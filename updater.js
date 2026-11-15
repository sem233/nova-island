// Updates for Nova Island from GitHub Releases (sem233/nova-island).
// Supports manual check, progress streaming, in-place portable auto-update and restart.
const { app } = require('electron');
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const log = require('./log');

const REPO_OWNER = 'sem233';
const REPO_NAME = 'nova-island';
const GITHUB_API_RELEASE = `https://api.github.com/repos/${REPO_OWNER}/${REPO_NAME}/releases/latest`;
const EVERY_MS = 6 * 60 * 60 * 1000;
const FIRST_CHECK_MS = 60 * 1000;

// Local build timestamp for comparing re-uploaded builds under the same tag
const BUILD_TIME = (() => {
  try {
    const pkg = require('./package.json');
    return pkg.buildTime || fs.statSync(path.join(__dirname, 'package.json')).mtimeMs;
  } catch {
    return Date.now();
  }
})();

let enabled = false;
let ready = null; // { version, path }
let latestReleaseInfo = null; // { release, asset }
let downloading = false;
let timer = null;
let send = () => {};

function semverCmp(a, b) {
  const pa = (a || '').replace(/^v/, '').split('.').map(n => parseInt(n, 10) || 0);
  const pb = (b || '').replace(/^v/, '').split('.').map(n => parseInt(n, 10) || 0);
  for (let i = 0; i < Math.max(pa.length, pb.length, 3); i++) {
    const na = pa[i] || 0;
    const nb = pb[i] || 0;
    if (na > nb) return 1;
    if (na < nb) return -1;
  }
  return 0;
}

async function fetchLatestRelease() {
  const r = await fetch(GITHUB_API_RELEASE, {
    headers: { 'User-Agent': 'Nova-Island-Updater' },
    signal: AbortSignal.timeout(15000),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

async function checkManual(force = false) {
  try {
    const release = await fetchLatestRelease();
    const tag = release.tag_name || '';
    const currentVersion = app?.getVersion ? app.getVersion() : require('./package.json').version;
    const assets = Array.isArray(release.assets) ? release.assets : [];
    const asset = assets.find((a) => a.name.endsWith('.exe')) || assets[0];

    if (!asset) {
      return { ok: false, error: 'No executable asset found in latest release' };
    }

    const remoteAssetTime = new Date(asset.updated_at || asset.created_at).getTime();
    const isHigherVersion = semverCmp(tag, currentVersion) > 0;
    const isNewerBuild = semverCmp(tag, currentVersion) === 0 && (remoteAssetTime > (BUILD_TIME + 60000));
    const available = isHigherVersion || isNewerBuild;

    latestReleaseInfo = { release, asset };

    return {
      ok: true,
      available,
      version: tag,
      currentVersion,
      assetName: asset.name,
      sizeMb: (asset.size / (1024 * 1024)).toFixed(1),
      updatedAt: asset.updated_at,
      notes: release.body || '',
      ready: !!ready && ready.version === tag,
    };
  } catch (err) {
    log.info(`update check error: ${err.message}`);
    return { ok: false, error: err.message };
  }
}

async function download() {
  if (downloading) return { ok: false, error: 'Download already in progress' };
  if (!latestReleaseInfo?.asset) {
    const checkRes = await checkManual(true);
    if (!checkRes.ok || !latestReleaseInfo?.asset) {
      return { ok: false, error: checkRes.error || 'No asset to download' };
    }
  }

  const asset = latestReleaseInfo.asset;
  const version = latestReleaseInfo.release.tag_name;
  downloading = true;

  try {
    const userDataDir = app?.getPath ? app.getPath('userData') : path.join(process.env.APPDATA || '', 'dynamic-nova');
    const updatesDir = path.join(userDataDir, 'updates');
    fs.mkdirSync(updatesDir, { recursive: true });
    const destPath = path.join(updatesDir, 'Nova.Island.pending.exe');

    send('update-status', { status: 'starting', version, assetName: asset.name });

    const res = await fetch(asset.browser_download_url, {
      headers: { 'User-Agent': 'Nova-Island-Updater' },
      signal: AbortSignal.timeout(300000), // 5 minutes timeout for large exe
    });
    if (!res.ok) throw new Error(`Download HTTP ${res.status}`);

    const totalBytes = Number(res.headers.get('content-length')) || asset.size || 0;
    let receivedBytes = 0;
    const reader = res.body.getReader();
    const outStream = fs.createWriteStream(destPath);

    let lastProgressPct = -1;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      outStream.write(value);
      receivedBytes += value.length;

      const pct = totalBytes > 0 ? Math.round((receivedBytes / totalBytes) * 100) : 0;
      if (pct !== lastProgressPct) {
        lastProgressPct = pct;
        send('update-progress', {
          pct,
          version,
          receivedMb: (receivedBytes / (1024 * 1024)).toFixed(1),
          totalMb: (totalBytes / (1024 * 1024)).toFixed(1),
        });
      }
    }

    await new Promise((resolve, reject) => {
      outStream.end((err) => (err ? reject(err) : resolve()));
    });

    ready = { version, path: destPath };
    downloading = false;

    send('update-ready', { version });
    send('update', { version });
    return { ok: true, version, path: destPath };
  } catch (err) {
    downloading = false;
    log.info(`update download error: ${err.message}`);
    send('update-status', { status: 'error', error: err.message });
    return { ok: false, error: err.message };
  }
}

function install() {
  if (!ready || !ready.path) {
    log.info('install called but no update is ready');
    return false;
  }

  const downloadedPath = path.resolve(ready.path);
  const currentPid = process.pid;

  // Portable exe location or current executable
  const targetExe = path.resolve(process.env.PORTABLE_EXECUTABLE_FILE || process.execPath);
  const targetDir = path.dirname(targetExe);
  const userDataDir = app?.getPath ? app.getPath('userData') : path.join(process.env.APPDATA || '', 'dynamic-nova');
  const updatesDir = path.join(userDataDir, 'updates');
  fs.mkdirSync(updatesDir, { recursive: true });

  const psScriptPath = path.join(updatesDir, 'apply-update.ps1');

  log.info(`installing update ${ready.version}: replacing ${targetExe} with ${downloadedPath}`);

  // Create robust powershell worker script
  const psScript = `
# Nova Island Update Installer Script
$ErrorActionPreference = 'SilentlyContinue'

$electronPid = ${currentPid}
$downloaded = '${downloadedPath.replace(/'/g, "''")}'
$target = '${targetExe.replace(/'/g, "''")}'
$targetDir = '${targetDir.replace(/'/g, "''")}'

# 1. Wait for the Electron child process to exit
$proc = Get-Process -Id $electronPid -ErrorAction SilentlyContinue
if ($proc) {
  $proc.WaitForExit(15000)
}

# 2. Wait until NSIS wrapper and all processes holding the target file have exited
# and we can open the target file with exclusive write access
$targetUnlocked = $false
for ($i = 0; $i -lt 50; $i++) {
  Start-Sleep -Milliseconds 400
  $locking = Get-Process | Where-Object {
    try { $_.Path -eq $target -or $_.Id -eq $electronPid } catch { $false }
  }
  if ($locking) { continue }

  try {
    if (Test-Path -LiteralPath $target) {
      $stream = [System.IO.File]::Open($target, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::None)
      $stream.Close()
      $stream.Dispose()
    }
    $targetUnlocked = $true
    break
  } catch {
    # Still locked by quitting process or antivirus
  }
}

# Additional short pause to allow Windows kernel to release single-instance mutexes
Start-Sleep -Milliseconds 600

# 3. Replace the executable with the newly downloaded binary
$replaced = $false
if ($downloaded -ne $target) {
  for ($i = 0; $i -lt 15; $i++) {
    try {
      Copy-Item -LiteralPath $downloaded -Destination $target -Force -ErrorAction Stop
      $replaced = $true
      break
    } catch {
      Start-Sleep -Milliseconds 500
    }
  }
} else {
  $replaced = $true
}

# 4. Launch the updated application
if ($replaced -and (Test-Path -LiteralPath $target)) {
  Start-Process -FilePath $target -WorkingDirectory $targetDir
} else {
  # Fallback: Copy to "Nova Island.exe" in target folder or run downloaded directly
  $fallbackPath = Join-Path $targetDir "Nova Island.exe"
  try {
    Copy-Item -LiteralPath $downloaded -Destination $fallbackPath -Force -ErrorAction Stop
    Start-Process -FilePath $fallbackPath -WorkingDirectory $targetDir
  } catch {
    Start-Process -FilePath $downloaded -WorkingDirectory (Split-Path -Parent $downloaded)
  }
}
`;

  try {
    fs.writeFileSync(psScriptPath, psScript, 'utf8');

    const child = spawn('powershell.exe', [
      '-NoProfile',
      '-ExecutionPolicy', 'Bypass',
      '-WindowStyle', 'Hidden',
      '-File', psScriptPath,
    ], {
      detached: true,
      stdio: 'ignore',
    });
    child.unref();

    setTimeout(() => {
      try {
        const { BrowserWindow } = require('electron');
        for (const w of BrowserWindow.getAllWindows()) {
          try {
            w.removeAllListeners('close');
            w.destroy();
          } catch {}
        }
      } catch {}
      app.exit(0);
    }, 250);
    return true;
  } catch (err) {
    log.info(`failed to launch installer: ${err.message}`);
    return false;
  }
}

function checkBackground() {
  if (!enabled) return;
  if (ready) {
    send('update', { version: ready.version });
    return;
  }
  checkManual(false).then((res) => {
    if (res.ok && res.available) {
      log.info(`background update available: ${res.version}`);
      send('update', { version: res.version });
    }
  }).catch(() => {});
}

function start(sendFn, on) {
  send = sendFn;
  setEnabled(on);
}

function setEnabled(on) {
  enabled = !!on;
  clearTimeout(timer);
  clearInterval(timer);
  if (!enabled) return;
  timer = setTimeout(() => {
    checkBackground();
    timer = setInterval(checkBackground, EVERY_MS);
  }, FIRST_CHECK_MS);
}

function retrySoon() {
  if (enabled) setTimeout(checkBackground, 60 * 1000);
}

module.exports = {
  start,
  setEnabled,
  checkManual,
  download,
  install,
  retrySoon,
};
