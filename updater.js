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
let cleanupFn = null;

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

    const isHigherVersion = semverCmp(tag, currentVersion) > 0;
    const available = isHigherVersion;

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
    let lastSent = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      outStream.write(value);
      receivedBytes += value.length;

      const pct = totalBytes > 0 ? Math.round((receivedBytes / totalBytes) * 100) : 0;
      const now = Date.now();
      if (pct !== lastProgressPct || now - lastSent > 250) {
        lastProgressPct = pct;
        lastSent = now;
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

    try {
      const { exec } = require('child_process');
      exec(`powershell.exe -NoProfile -Command "Unblock-File -LiteralPath '${destPath}' -ErrorAction SilentlyContinue"`);
    } catch {}

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

function setCleanup(fn) {
  cleanupFn = fn;
}

function install() {
  if (!ready || !ready.path) {
    log.info('install called but no update is ready');
    return false;
  }

  const downloadedPath = path.resolve(ready.path);
  const currentPid = process.pid;
  const parentPid = process.ppid || '';

  // Portable exe location or current executable
  const targetExe = path.resolve(process.env.PORTABLE_EXECUTABLE_FILE || process.execPath);
  const targetDir = path.dirname(targetExe);
  const userDataDir = app?.getPath ? app.getPath('userData') : path.join(process.env.APPDATA || '', 'dynamic-nova');
  const updatesDir = path.join(userDataDir, 'updates');
  fs.mkdirSync(updatesDir, { recursive: true });

  const cmdScriptPath = path.join(updatesDir, 'apply-update.cmd');
  const logPath = path.join(updatesDir, 'updater.log');

  log.info(`installing update ${ready.version}: replacing ${targetExe} with ${downloadedPath}`);

  // Call cleanup hook to terminate background helper
  if (typeof cleanupFn === 'function') {
    try { cleanupFn(); } catch {}
  }

  const newVer = ready.version || 'latest';
  const cmdScript = `@echo off
chcp 65001 >nul
set "LOG=${logPath}"
set "DOWNLOADED=${downloadedPath}"
set "TARGET=${targetExe}"
set "TARGETDIR=${targetDir}"
set "ELECTRON_PID=${currentPid}"
set "PARENT_PID=${parentPid}"
set "NEW_VERSION=${newVer}"

echo ======================================== > "%LOG%"
echo [%date% %time%] Nova Island Updater starting >> "%LOG%"
echo [%date% %time%] Target: %TARGET% >> "%LOG%"
echo [%date% %time%] Downloaded: %DOWNLOADED% >> "%LOG%"
echo [%date% %time%] Electron PID: %ELECTRON_PID% >> "%LOG%"
echo [%date% %time%] Parent PID: %PARENT_PID% >> "%LOG%"

:: 1. Force terminate old Electron and NSIS processes
echo [%date% %time%] Terminating old processes... >> "%LOG%"
if not "%ELECTRON_PID%"=="" (
  taskkill /F /PID %ELECTRON_PID% /T >> "%LOG%" 2>&1
)
if not "%PARENT_PID%"=="" (
  taskkill /F /PID %PARENT_PID% /T >> "%LOG%" 2>&1
)
taskkill /F /IM nova-helper.exe >> "%LOG%" 2>&1

:: 2. Wait for file handles to release
echo [%date% %time%] Waiting for file handles to release... >> "%LOG%"
timeout /t 2 /nobreak >nul

:: 3. Clear Chromium SingletonLock to avoid phantom single-instance lock failure
echo [%date% %time%] Clearing SingletonLock... >> "%LOG%"
del /f /q "%APPDATA%\\dynamic-nova\\SingletonLock" >> "%LOG%" 2>&1
del /f /q "%APPDATA%\\dynamic-nova\\SingletonCookie" >> "%LOG%" 2>&1
del /f /q "%APPDATA%\\dynamic-nova\\SingletonSocket" >> "%LOG%" 2>&1

:: 4. Unblock the downloaded file to prevent Windows SmartScreen blocks
echo [%date% %time%] Unblocking downloaded binary... >> "%LOG%"
powershell.exe -NoProfile -Command "Unblock-File -LiteralPath '%DOWNLOADED%' -ErrorAction SilentlyContinue" >> "%LOG%" 2>&1

:: 5. Copy new binary to target with retries
echo [%date% %time%] Replacing target executable... >> "%LOG%"
set RETRY=0
:RETRY_COPY
set /a RETRY+=1
copy /y "%DOWNLOADED%" "%TARGET%" >> "%LOG%" 2>&1
if not errorlevel 1 (
  echo [%date% %time%] Successfully updated %TARGET% on attempt %RETRY% >> "%LOG%"
  set "TO_LAUNCH=%TARGET%"
  goto LAUNCH
)

if %RETRY% lss 20 (
  timeout /t 1 /nobreak >nul
  goto RETRY_COPY
)

echo [%date% %time%] Warning: Could not replace %TARGET% after %RETRY% attempts >> "%LOG%"
set "TO_LAUNCH=%DOWNLOADED%"

:LAUNCH
:: Also create a versioned copy in TARGETDIR if target had a different name
set "VERSIONED=%TARGETDIR%\\Nova.Island.%NEW_VERSION%.portable.exe"
if not "%TO_LAUNCH%"=="%VERSIONED%" (
  copy /y "%DOWNLOADED%" "%VERSIONED%" >> "%LOG%" 2>&1
)

:: Unblock the binary to be launched
powershell.exe -NoProfile -Command "Unblock-File -LiteralPath '%TO_LAUNCH%' -ErrorAction SilentlyContinue" >> "%LOG%" 2>&1

echo [%date% %time%] Launching %TO_LAUNCH%... >> "%LOG%"
cd /d "%TARGETDIR%"
start "" "%TO_LAUNCH%"

echo [%date% %time%] Updater finished successfully. >> "%LOG%"
exit
`;

  try {
    fs.writeFileSync(cmdScriptPath, cmdScript, 'utf8');

    const child = spawn('cmd.exe', ['/c', cmdScriptPath], {
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
  setCleanup,
  checkManual,
  download,
  install,
  retrySoon,
};
