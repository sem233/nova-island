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
  const pa = (a || '').replace(/^v/, '').split('.').map(Number);
  const pb = (b || '').replace(/^v/, '').split('.').map(Number);
  for (let i = 0; i < 3; i++) {
    const na = pa[i] || 0;
    const nb = pb[i] || 0;
    if (na !== nb) return na > nb ? 1 : -1;
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
    const isNewerBuild = remoteAssetTime > (BUILD_TIME + 60000);
    const available = force || isHigherVersion || isNewerBuild;

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

  const downloadedPath = ready.path;
  const currentPid = process.pid;

  // Portable exe location or current executable
  const targetExe = process.env.PORTABLE_EXECUTABLE_FILE || process.execPath;
  log.info(`installing update ${ready.version}: replacing ${targetExe} with ${downloadedPath}`);

  // Create detached powershell worker to replace and launch
  const psScript = `
    $currentPid = ${currentPid};
    $downloaded = '${downloadedPath.replace(/'/g, "''")}';
    $target = '${targetExe.replace(/'/g, "''")}';

    # Wait for the running instance to quit
    $proc = Get-Process -Id $currentPid -ErrorAction SilentlyContinue;
    if ($proc) { $proc.WaitForExit(10000); }
    Start-Sleep -Milliseconds 900;

    # Copy new binary over target
    if ($downloaded -ne $target) {
      try {
        Copy-Item -Path $downloaded -Destination $target -Force;
      } catch {
        Start-Sleep -Seconds 1;
        Copy-Item -Path $downloaded -Destination $target -Force;
      }
    }

    # Start the updated app
    Start-Process -FilePath $target;
  `;

  try {
    const child = spawn('powershell.exe', [
      '-NoProfile',
      '-NonInteractive',
      '-WindowStyle', 'Hidden',
      '-Command', psScript,
    ], {
      detached: true,
      stdio: 'ignore',
    });
    child.unref();

    setTimeout(() => {
      app.quit();
    }, 200);
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
