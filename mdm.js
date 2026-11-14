// Built-in video & MP3 downloader for Nova Island (powered by yt-dlp + ffmpeg, with quality + size
// picker, highest-quality 320k MP3 extraction, HLS/.ts stream & segment support, and instant cancel).
// Runs with 0 MB extra background RAM when idle.
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { spawn } = require('child_process');
const log = require('./log');

let electronApp = null;
try {
  electronApp = require('electron').app;
} catch {}

const KEEP_DONE_MS = 60 * 60 * 1000;
const PROBE_TTL_MS = 8 * 60 * 1000;
const MAX_ITEMS = 25;
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';

let onChange = () => {};
const downloads = new Map(); // id -> item
const procs = new Map(); // id -> { kill() }
const probeCache = new Map(); // url -> { at, data }
let seq = 0;

function toolsDir() {
  if (process.env.NOVA_TOOLS_DIR) return process.env.NOVA_TOOLS_DIR;
  if (electronApp?.getPath) {
    try {
      return path.join(electronApp.getPath('userData'), 'tools');
    } catch {}
  }
  return path.join(__dirname, 'tools');
}

function downloadsDir() {
  if (process.env.NOVA_DOWNLOADS_DIR) return process.env.NOVA_DOWNLOADS_DIR;
  if (electronApp?.getPath) {
    try {
      return electronApp.getPath('downloads');
    } catch {}
  }
  return path.join(os.homedir(), 'Downloads');
}

function tempDir() {
  const d = path.join(os.tmpdir(), 'nova-video-dl');
  try {
    fs.mkdirSync(d, { recursive: true });
  } catch {}
  return d;
}

function isFile(p) {
  try {
    return !!p && fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

function pathDirs() {
  return String(process.env.PATH || process.env.Path || '')
    .split(path.delimiter)
    .map((s) => s.trim())
    .filter(Boolean);
}

// ---------- locating yt-dlp, ffmpeg, and JS runtime ----------
function pythonScriptDirs() {
  const out = [];
  for (const base of [
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'Programs', 'Python'),
    process.env.APPDATA && path.join(process.env.APPDATA, 'Python'),
  ]) {
    if (!base) continue;
    try {
      for (const d of fs.readdirSync(base)) out.push(path.join(base, d, 'Scripts'));
    } catch {}
  }
  return out;
}

function findYtdlp() {
  if (isFile(process.env.YTDLP_EXE)) return process.env.YTDLP_EXE;
  const dirs = [
    toolsDir(),
    path.join(__dirname, 'tools'),
    ...pythonScriptDirs(),
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'Microsoft', 'WinGet', 'Links'),
    process.env.USERPROFILE && path.join(process.env.USERPROFILE, 'scoop', 'shims'),
    'C:\\ProgramData\\chocolatey\\bin',
    ...pathDirs(),
  ].filter(Boolean);
  for (const d of dirs) {
    const p = path.join(d, 'yt-dlp.exe');
    if (isFile(p)) return p;
  }
  return '';
}

function findFfmpegDir() {
  if (isFile(process.env.FFMPEG_EXE)) return path.dirname(process.env.FFMPEG_EXE);
  const dirs = [
    path.join(toolsDir(), 'ffmpeg'),
    path.join(__dirname, 'tools', 'ffmpeg'),
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'Microsoft', 'WinGet', 'Links'),
    process.env.USERPROFILE && path.join(process.env.USERPROFILE, 'scoop', 'shims'),
    'C:\\ProgramData\\chocolatey\\bin',
    'C:\\ffmpeg\\bin',
    ...pathDirs(),
  ].filter(Boolean);
  for (const d of dirs) {
    if (isFile(path.join(d, 'ffmpeg.exe'))) return d;
  }
  return '';
}

function jsRuntimeArgs() {
  // Build a broad list of dirs to search for JS runtimes (Electron runs with a stripped PATH)
  const dirs = [
    ...pathDirs(),
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'Microsoft', 'WinGet', 'Links'),
    process.env.ProgramFiles && path.join(process.env.ProgramFiles, 'nodejs'),
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, 'Programs', 'nodejs'),
    process.env.APPDATA && path.join(process.env.APPDATA, 'npm'),
    process.env.USERPROFILE && path.join(process.env.USERPROFILE, '.deno', 'bin'),
    process.env.USERPROFILE && path.join(process.env.USERPROFILE, 'scoop', 'shims'),
    'C:\\Program Files\\nodejs',
    'C:\\Program Files (x86)\\nodejs',
  ].filter(Boolean);
  // Check all dirs for deno/node/bun — deno first (preferred by yt-dlp)
  for (const [name, exe] of [['deno', 'deno.exe'], ['node', 'node.exe'], ['bun', 'bun.exe']]) {
    for (const d of dirs) {
      const p = path.join(d, exe);
      if (isFile(p)) return ['--js-runtimes', `${name}:${p}`];
    }
  }
  return [];
}

// ---------- auto-installing yt-dlp & ffmpeg on first need (verified by SHA-256) ----------
function sumFor(sumsText, fileName) {
  for (const line of String(sumsText).split(/\r?\n/)) {
    const m = line.trim().match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (m && m[2].trim() === fileName) return m[1].toLowerCase();
  }
  return '';
}

async function fetchToFile(url, dest, expectedSha256 = '') {
  const r = await fetch(url, {
    headers: { 'user-agent': 'NovaIsland/1.7' },
    signal: AbortSignal.timeout(180000),
  });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  const buf = Buffer.from(await r.arrayBuffer());
  if (expectedSha256) {
    const got = crypto.createHash('sha256').update(buf).digest('hex').toLowerCase();
    if (got !== expectedSha256.toLowerCase()) throw new Error('checksum mismatch');
  }
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = `${dest}.tmp.${process.pid}`;
  fs.writeFileSync(tmp, buf);
  fs.renameSync(tmp, dest);
}

let ytdlpPromise = null;
async function ensureYtdlp(onState = () => {}) {
  const found = findYtdlp();
  if (found) return found;
  if (ytdlpPromise) return ytdlpPromise;
  ytdlpPromise = (async () => {
    try {
      onState('Setting up video downloader (one-time)…');
      const base = 'https://github.com/yt-dlp/yt-dlp/releases/latest/download';
      const sumsRes = await fetch(`${base}/SHA2-256SUMS`, {
        headers: { 'user-agent': 'NovaIsland/1.7' },
        signal: AbortSignal.timeout(20000),
      });
      const sums = sumsRes.ok ? await sumsRes.text() : '';
      const sha = sumFor(sums, 'yt-dlp.exe');
      const dest = path.join(toolsDir(), 'yt-dlp.exe');
      await fetchToFile(`${base}/yt-dlp.exe`, dest, sha);
      log.info(`[video-dl] installed yt-dlp.exe to ${dest}`);
      return dest;
    } finally {
      ytdlpPromise = null;
    }
  })();
  return ytdlpPromise;
}

function pickFfmpegAsset(releases) {
  const re = /^ffmpeg-n(\d+\.\d+)(?:\.\d+)?-(?:latest|\d+-g[0-9a-f]+)-win64-lgpl-shared-(\d+\.\d+)\.zip$/i;
  for (const rel of Array.isArray(releases) ? releases : []) {
    const assets = Array.isArray(rel?.assets) ? rel.assets : [];
    const sums = assets.find((a) => a.name === 'checksums.sha256');
    if (!sums) continue;
    const match = assets.find((a) => {
      const m = String(a?.name || '').match(re);
      return m && m[1] === m[2];
    });
    if (match) return { asset: match, sumsUrl: sums.browser_download_url };
  }
  return null;
}

function extractBinFromZip(zipPath, outDir) {
  return new Promise((resolve, reject) => {
    fs.mkdirSync(outDir, { recursive: true });
    const tar = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe');
    const ps = spawn(tar, ['-xf', zipPath, '--strip-components=2', '-C', outDir, '*/bin/*'], { windowsHide: true });
    ps.on('error', reject);
    ps.on('close', (code) => (code === 0 && isFile(path.join(outDir, 'ffmpeg.exe')) ? resolve(outDir) : reject(new Error(`tar exited ${code}`))));
  });
}

let ffmpegPromise = null;
async function ensureFfmpegDir(onState = () => {}) {
  const found = findFfmpegDir();
  if (found) return found;
  if (ffmpegPromise) return ffmpegPromise;
  ffmpegPromise = (async () => {
    try {
      onState('Setting up HD merger (one-time)…');
      const r = await fetch('https://api.github.com/repos/BtbN/FFmpeg-Builds/releases?per_page=5', {
        headers: { accept: 'application/vnd.github+json', 'user-agent': 'NovaIsland/1.7' },
        signal: AbortSignal.timeout(20000),
      });
      if (!r.ok) return '';
      const pick = pickFfmpegAsset(await r.json());
      if (!pick) return '';
      const sumsRes = await fetch(pick.sumsUrl, {
        headers: { 'user-agent': 'NovaIsland/1.7' },
        signal: AbortSignal.timeout(20000),
      });
      const sha = sumsRes.ok ? sumFor(await sumsRes.text(), pick.asset.name) : '';
      const outDir = path.join(toolsDir(), 'ffmpeg');
      const zipPath = path.join(toolsDir(), 'ffmpeg-lgpl.zip');
      await fetchToFile(pick.asset.browser_download_url, zipPath, sha);
      try {
        await extractBinFromZip(zipPath, outDir);
      } finally {
        try { fs.rmSync(zipPath, { force: true }); } catch {}
      }
      log.info(`[video-dl] installed ffmpeg to ${outDir}`);
      return isFile(path.join(outDir, 'ffmpeg.exe')) ? outDir : '';
    } catch (err) {
      log.info(`[video-dl] ffmpeg setup skipped: ${err.message}`);
      return '';
    } finally {
      ffmpegPromise = null;
    }
  })();
  return ffmpegPromise;
}

// ---------- process tree cleanup ----------
function killTree(proc) {
  if (!proc) return;
  if (typeof proc.abort === 'function') {
    try { proc.abort(); } catch {}
    return;
  }
  if (proc.killed) return;
  try {
    if (process.platform === 'win32' && proc.pid) {
      spawn('taskkill', ['/PID', String(proc.pid), '/T', '/F'], { windowsHide: true });
    } else {
      proc.kill('SIGTERM');
    }
  } catch {}
}

// ---------- live downloads state for Nova Island ----------
let notifyTimer = null;
function notify(immediate = false) {
  if (immediate) {
    clearTimeout(notifyTimer);
    notifyTimer = null;
    onChange();
    return;
  }
  if (notifyTimer) return;
  notifyTimer = setTimeout(() => {
    notifyTimer = null;
    onChange();
  }, 200);
}

function prune() {
  const now = Date.now();
  for (const [id, d] of downloads) {
    if (d.finishedAt && now - d.finishedAt > KEEP_DONE_MS) downloads.delete(id);
  }
  while (downloads.size > MAX_ITEMS) {
    const oldest = [...downloads.values()].find((d) => !['downloading', 'processing', 'queued'].includes(d.status));
    if (!oldest) break;
    downloads.delete(oldest.id);
  }
}

function items() {
  prune();
  return [...downloads.values()].map((d) => ({
    id: d.id,
    name: d.name,
    path: d.path || '',
    thumbnail: d.thumbnail || '',
    createdAt: d.createdAt,
    downloaded: d.downloaded,
    total: d.total,
    speed: d.speed,
    eta: d.eta,
    status: d.status,
    stage: d.stage || '',
    error: d.error || '',
  }));
}

function cleanStem(title) {
  return (
    String(title || 'Video')
      .replace(/\.[a-z0-9]{2,5}$/i, '')
      .replace(/[<>:"/\\|?*\x00-\x1f]/g, ' ')
      .replace(/\s+/g, ' ')
      .trim()
      .slice(0, 140) || 'Video'
  );
}

function cleanName(title, ext = 'mp4') {
  return `${cleanStem(title)}.${ext}`;
}

function num(v) {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

// ---------- probing video qualities & file sizes (with MP3 & .ts support) ----------
function buildChoicesFromInfo(raw) {
  const duration = num(raw?.duration);
  const formats = (Array.isArray(raw?.formats) ? raw.formats : []).filter((f) => f && f.has_drm !== true);

  const estSize = (f) => {
    const exact = num(f.filesize) || num(f.filesize_approx);
    if (exact > 0) return exact;
    const rate = num(f.tbr) || num(f.vbr) + num(f.abr);
    if (duration > 0 && rate > 0) return Math.round(duration * rate * 125);
    return 0;
  };

  // Best audio stream size (for DASH video-only formats that get merged with audio)
  const audioOnly = formats
    .filter((f) => f.acodec && f.acodec !== 'none' && (!f.vcodec || f.vcodec === 'none'))
    .sort((a, b) => num(b.abr || b.tbr) - num(a.abr || a.tbr));
  const bestAudio = audioOnly[0] || null;
  const bestAudioSize = bestAudio ? estSize(bestAudio) : duration > 0 ? Math.round(duration * 140 * 125) : 0;

  // Group video streams by height
  const byHeight = new Map();
  let anyTs = false;
  for (const f of formats) {
    if (f.vcodec === 'none') continue;
    const isTs = /m3u8/i.test(f.protocol || '') || /^ts$/i.test(f.ext || '');
    if (isTs) anyTs = true;
    const h = Math.round(num(f.height));
    if (!h) continue;
    const hasAudio = f.acodec && f.acodec !== 'none';
    const vSize = estSize(f);
    const totalSize = vSize > 0 ? (hasAudio ? vSize : vSize + bestAudioSize) : 0;
    const fps = Math.round(num(f.fps));
    const tbr = num(f.tbr) || num(f.vbr);
    const prev = byHeight.get(h);
    // Prefer a stream at this height that has a known size, higher fps, and higher bitrate
    const score = (totalSize > 0 ? 1e9 : 0) + fps * 1e5 + tbr;
    if (!prev || score > prev.score) {
      byHeight.set(h, { height: h, fps, size: totalSize, isTs, score });
    }
  }

  const choices = [];
  const heights = [...byHeight.keys()].sort((a, b) => b - a);
  for (const h of heights) {
    const v = byHeight.get(h);
    const label = h >= 2160 ? '4K' : h >= 1440 ? '2K' : `${h}p`;
    const tag = v.fps > 30 ? `${v.fps}fps` : v.isTs ? 'TS' : h >= 2160 ? '2160p' : h >= 1440 ? '1440p' : h >= 720 ? 'HD' : '';
    choices.push({
      id: `v-${h}`,
      label,
      tag,
      height: h,
      size: v.size,
      kind: 'video',
      selector: `bv*[height=${h}]+ba/b[height=${h}]/bv*[height<=${h}]+ba/b[height<=${h}]/b`,
    });
  }

  // Generic / HLS/.ts streams that didn't report a height
  if (!choices.length) {
    const topSize = num(raw?.filesize) || num(raw?.filesize_approx) || formats.reduce((m, f) => Math.max(m, estSize(f)), 0);
    choices.push({
      id: 'v-best',
      label: anyTs ? 'TS Stream' : 'Best HD',
      tag: anyTs ? 'TS' : 'MP4',
      height: 0,
      size: topSize,
      kind: 'video',
      selector: 'bv*+ba/b',
    });
  }

  // Highest quality MP3 (320 kbps)
  const mp3Size = duration > 0 ? Math.round(duration * 320 * 125) : bestAudioSize;
  choices.push({
    id: 'mp3',
    label: 'MP3',
    tag: '320k',
    height: 0,
    size: mp3Size,
    kind: 'audio',
    selector: 'mp3:0',
  });

  return choices;
}

function runYtdlpJson(ytdlpExe, url, referer = '') {
  return new Promise((resolve, reject) => {
    let origin = '';
    try { origin = new URL(referer || url).origin; } catch {}
    const args = [
      '--ignore-config',
      '--no-exec',
      '--no-update',
      '-J',
      '--no-playlist',
      '--no-warnings',
      '--encoding', 'utf-8',
      '--referer', referer || url,
      ...(origin ? ['--add-header', `Origin:${origin}`] : []),
      '--user-agent', UA,
      ...jsRuntimeArgs(),
      '--',
      url,
    ];
    const proc = spawn(ytdlpExe, args, {
      windowsHide: true,
      env: { ...process.env, PYTHONIOENCODING: 'utf-8' },
    });
    let out = '';
    let err = '';
    const timer = setTimeout(() => {
      killTree(proc);
      reject(new Error('timeout'));
    }, 35000);
    proc.stdout.setEncoding('utf8');
    proc.stderr.setEncoding('utf8');
    proc.stdout.on('data', (d) => (out += d));
    proc.stderr.on('data', (d) => (err += d));
    proc.on('error', (e) => {
      clearTimeout(timer);
      reject(e);
    });
    proc.on('close', (code) => {
      clearTimeout(timer);
      if (code === 0 && out.trim()) {
        try {
          const line = out.trim().split(/\r?\n/).pop();
          resolve({ raw: JSON.parse(line), jsonText: line });
          return;
        } catch {}
      }
      const msg = err
        .split(/\r?\n/)
        .map((l) => l.trim())
        .find((l) => l.startsWith('ERROR:'))
        ?.replace(/^ERROR:\s*(\[[^\]]+\]\s*[^:]+:\s*)?/, '') || 'No video found on this page';
      reject(new Error(msg));
    });
  });
}

// ---------- Streaming-site sniffer (Electron BrowserWindow HLS/DASH detector) ----------
// Opens the page in a hidden window, intercepts HLS/DASH manifest requests, then tries
// yt-dlp on the manifest URL.  Called automatically when yt-dlp can't handle the page.
function sniffStreamingPage(pageUrl, pageTitle, onState, ytdlpExe) {
  return new Promise((resolve) => {
    let electron;
    try { electron = require('electron'); } catch { resolve(null); return; }
    const { BrowserWindow, session } = electron;
    if (!BrowserWindow || !session) { resolve(null); return; }

    const MANIFEST_RE = /\.(m3u8?|mpd)([\?#&]|$)/i;
    const UA_STR = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36';

    const ses = session.fromPartition(`persist:nova-sniff-${Date.now()}`, { cache: false });
    ses.setUserAgent(UA_STR);

    let candidates = [];
    let closed = false;
    let bw = null;
    let resolveTimer = null;
    let clickInterval = null;

    const cleanup = (result) => {
      if (closed) return;
      closed = true;
      clearTimeout(resolveTimer);
      clearInterval(clickInterval);
      resolve(result);
      try { if (bw && !bw.isDestroyed()) bw.destroy(); } catch {}
      setTimeout(() => { try { ses.clearCache(); } catch {} }, 4000);
    };

    const pickBest = () => {
      if (!candidates.length) return null;
      // Prefer master manifests (e.g. master.m3u8)
      const masters = candidates.filter((u) => /master/i.test(u));
      if (masters.length) return masters[0];
      // Next, prefer non-segment manifests
      const nonSeg = candidates.filter((u) => !/([/_-]seg|[/_-]chunk|[/_-]piece|[/_-]part|[/_-]frag)[/_-]?\d+/i.test(u));
      const pool = nonSeg.length ? nonSeg : candidates;
      return pool.sort((a, b) => a.length - b.length)[0];
    };

    const schedulePick = () => {
      if (resolveTimer) return;
      resolveTimer = setTimeout(() => {
        const best = pickBest();
        log.info(`[stream-sniff] resolving best manifest: ${best}`);
        cleanup(best);
      }, 1800);
    };

    const timeout = setTimeout(() => {
      const best = pickBest();
      log.info(`[stream-sniff] timeout reached, best so far: ${best}`);
      cleanup(best);
    }, 30000);

    const onUrlSeen = (url) => {
      if (closed || !url || !MANIFEST_RE.test(url)) return;
      if (!candidates.includes(url)) {
        log.info(`[stream-sniff] candidate manifest found: ${url}`);
        candidates.push(url);
      }
      schedulePick();
    };

    // Catch manifests as soon as requested (faster than waiting for completion)
    ses.webRequest.onBeforeRequest({ urls: ['<all_urls>'] }, (details, callback) => {
      onUrlSeen(details.url);
      callback({});
    });

    ses.webRequest.onCompleted({ urls: ['<all_urls>'] }, (details) => {
      onUrlSeen(details.url);
    });

    bw = new BrowserWindow({
      show: false,
      width: 1280,
      height: 720,
      webPreferences: {
        session: ses,
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        autoplayPolicy: 'no-user-gesture-required',
        webSecurity: false,
      },
    });

    bw.webContents.on('render-process-gone', () => { if (!candidates.length) cleanup(null); });
    bw.on('closed', () => { if (!closed && !candidates.length) cleanup(null); });

    const clickAllFrames = () => {
      if (closed || !bw || bw.isDestroyed()) return;
      const script = `
        (function(){
          try {
            var vids = document.querySelectorAll('video');
            for (var i = 0; i < vids.length; i++) {
              if (vids[i].paused && vids[i].play) {
                vids[i].muted = true;
                vids[i].play().catch(function(){});
              }
            }
            var sel = [
              'video','button[class*="play"]','button[aria-label*="play" i]',
              'button[title*="play" i]','.play-btn','#play-btn',
              '.jw-icon-playback','.vjs-play-control','[data-plyr="play"]',
              'button.play','a.play-button','.btn-play','.ytp-play-button',
              '[class*="PlayButton"]','[class*="play-button"]','[id*="play"]',
              '.jw-display-icon-container','.vjs-big-play-button',
              '.plyr__control--overlaid','div[class*="player"]'
            ];
            for (var s = 0; s < sel.length; s++) {
              var els = document.querySelectorAll(sel[s]);
              for (var k = 0; k < els.length; k++) {
                try { els[k].click(); } catch(e){}
              }
            }
            document.body && document.body.click && document.body.click();
          } catch(e){}
        })()
      `;
      try {
        bw.webContents.executeJavaScript(script).catch(() => {});
        const main = bw.webContents.mainFrame;
        if (main && Array.isArray(main.frames)) {
          for (const f of main.frames) {
            try { f.executeJavaScript(script).catch(() => {}); } catch(e){}
          }
        }
      } catch(e){}
    };

    bw.webContents.on('did-finish-load', () => {
      setTimeout(clickAllFrames, 1200);
      setTimeout(clickAllFrames, 3000);
    });

    bw.webContents.on('did-frame-finish-load', () => {
      setTimeout(clickAllFrames, 800);
    });

    clickInterval = setInterval(clickAllFrames, 2500);

    bw.loadURL(pageUrl, {
      userAgent: UA_STR,
      extraHeaders: `Referer: ${pageUrl}\nAccept-Language: en-US,en;q=0.9\n`,
    });
  });
}

async function probeVideo(page, title = '', onState = () => {}) {
  if (!/^https?:\/\//i.test(page)) return { ok: false, error: 'no-url' };

  const cached = probeCache.get(page);
  if (cached && Date.now() - cached.at < PROBE_TTL_MS) {
    return { ok: true, ...cached.data };
  }

  let ytdlpExe;
  try {
    ytdlpExe = await ensureYtdlp(onState);
  } catch {
    return { ok: false, error: 'no-ytdlp' };
  }
  if (!ytdlpExe) return { ok: false, error: 'no-ytdlp' };

  // Warm up ffmpeg in background if not installed yet (never block probing)
  ensureFfmpegDir().catch(() => {});

  onState('Loading qualities & sizes…');
  try {
    const { raw, jsonText } = await runYtdlpJson(ytdlpExe, page);
    const infoFile = path.join(tempDir(), `info-${process.pid}-${Date.now()}.json`);
    try {
      fs.writeFileSync(infoFile, jsonText, 'utf8');
    } catch {}
    const choices = buildChoicesFromInfo(raw);
    const data = {
      url: page,
      title: raw.title || title || 'Video',
      thumbnail: /^https?:\/\//i.test(raw.thumbnail || '') ? raw.thumbnail : '',
      duration: num(raw.duration),
      infoFile,
      choices,
    };
    probeCache.set(page, { at: Date.now(), data });
    return { ok: true, ...data };
  } catch (err) {
    // Direct .ts / .m3u8 fallback if yt-dlp couldn't parse a raw segment or stream URL
    if (/\.(ts|m3u8)(\?|$)/i.test(page)) {
      const data = {
        url: page,
        title: title || 'TS Video',
        thumbnail: '',
        duration: 0,
        infoFile: '',
        choices: [
          { id: 'ts', label: 'TS Video', tag: 'TS', height: 0, size: 0, kind: 'video', selector: 'ts:direct' },
          { id: 'mp3', label: 'MP3', tag: '320k', height: 0, size: 0, kind: 'audio', selector: 'mp3:0' },
        ],
      };
      return { ok: true, ...data };
    }

    // --- Streaming-site fallback: sniff HLS/DASH in a hidden browser window ---
    const isStreamingSite = !/\.(mp4|mkv|webm|mov|avi|flv|ts|m3u8|mpd)(\?|$)/i.test(page);
    if (isStreamingSite) {
      onState('Opening video player to detect stream…');
      log.info(`[stream-sniff] yt-dlp unsupported, trying browser sniff for: ${page}`);
      try {
        const manifestUrl = await sniffStreamingPage(page, title, onState, ytdlpExe);
        if (manifestUrl) {
          onState('Stream detected — loading qualities…');
          log.info(`[stream-sniff] probing manifest: ${manifestUrl}`);
          // Try yt-dlp on the manifest URL with the original page as referer
          try {
            const { raw: mRaw, jsonText: mJson } = await runYtdlpJson(ytdlpExe, manifestUrl, page);
            const mInfo = path.join(tempDir(), `info-${process.pid}-sniff-${Date.now()}.json`);
            try { fs.writeFileSync(mInfo, mJson, 'utf8'); } catch {}
            const choices = buildChoicesFromInfo(mRaw);
            const data = {
              url: manifestUrl,
              title: mRaw.title || title || 'Video',
              thumbnail: /^https?:\/\//i.test(mRaw.thumbnail || '') ? mRaw.thumbnail : '',
              duration: num(mRaw.duration),
              infoFile: mInfo,
              choices,
              _sourcePageUrl: page,
            };
            probeCache.set(page, { at: Date.now(), data });
            if (manifestUrl && manifestUrl !== page) probeCache.set(manifestUrl, { at: Date.now(), data });
            return { ok: true, ...data };
          } catch (mErr) {
            log.info(`[stream-sniff] yt-dlp probe on manifest failed: ${mErr.message}, offering stream direct`);
            // yt-dlp can't parse the manifest either, offer direct stream download
            const data = {
              url: manifestUrl,
              title: title || 'Video',
              thumbnail: '',
              duration: 0,
              infoFile: '',
              choices: [
                { id: 'hls', label: 'HLS Stream', tag: 'HD', height: 0, size: 0, kind: 'video', selector: 'ts:direct' },
                { id: 'mp3', label: 'MP3', tag: '320k', height: 0, size: 0, kind: 'audio', selector: 'mp3:0' },
              ],
              _sourcePageUrl: page,
            };
            probeCache.set(page, { at: Date.now(), data });
            if (manifestUrl && manifestUrl !== page) probeCache.set(manifestUrl, { at: Date.now(), data });
            return { ok: true, ...data };
          }
        }
      } catch (sniffErr) {
        log.info(`[stream-sniff] sniff error: ${sniffErr.message}`);
      }
    }

    log.info(`[video-dl] probe failed for ${page}: ${err.message}`);
    return { ok: false, error: 'no-video', message: err.message.slice(0, 120) };
  }
}

// ---------- Direct / sequential .ts segment downloader ----------
async function downloadTsDirect(item, ffmpegDir, asMp3 = false) {
  const ac = new AbortController();
  procs.set(item.id, { abort: () => ac.abort() });
  item.status = 'downloading';
  item.stage = 'starting';
  notify(true);

  const outDir = downloadsDir();
  const tmpDir = tempDir();
  fs.mkdirSync(outDir, { recursive: true });

  const isHlsManifest = /\.(m3u8?|mpd)([\?#]|$)/i.test(item.url);
  const finalExt = asMp3 ? 'mp3' : 'mp4';
  const finalPath = path.join(outDir, cleanName(item.baseTitle || item.name, finalExt));
  const ffmpegExe = ffmpegDir && path.join(ffmpegDir, 'ffmpeg.exe');

  // ---- HLS/DASH manifest: use ffmpeg directly to remux the stream ----
  if (isHlsManifest && ffmpegExe && isFile(ffmpegExe)) {
    item.stage = '';
    notify(true);
    const ref = item.referer || item._sourcePageUrl || item.url;
    let origin = '';
    try { origin = new URL(ref).origin; } catch {}
    try {
      await new Promise((resolve, reject) => {
        const ffArgs = [
          '-y',
          '-user_agent', UA,
          ...(ref ? ['-referer', ref] : []),
          ...(origin ? ['-headers', `Origin: ${origin}\r\n`] : []),
          '-reconnect', '1',
          '-reconnect_streamed', '1',
          '-reconnect_delay_max', '5',
          '-i', item.url,
          ...(asMp3
            ? ['-vn', '-c:a', 'libmp3lame', '-b:a', '320k']
            : ['-c', 'copy', '-bsf:a', 'aac_adtstoasc']),
          finalPath,
        ];
        const ff = spawn(ffmpegExe, ffArgs, { windowsHide: true });
        procs.set(item.id, ff);
        let errBuf = '';
        let lastDl = 0;
        const t0 = Date.now();
        ff.stderr.setEncoding('utf8');
        // Parse ffmpeg progress from stderr: "size= 1234kB time=..."
        ff.stderr.on('data', (d) => {
          errBuf += d;
          const sizeMatch = errBuf.match(/size=\s*(\d+)kB/i);
          const timeMatch = errBuf.match(/time=(\d+):(\d+):([\d.]+)/);
          if (sizeMatch) {
            const bytes = parseInt(sizeMatch[1], 10) * 1024;
            if (bytes > lastDl) {
              lastDl = bytes;
              item.downloaded = bytes;
              item.total = Math.max(item.total || 0, bytes);
              const dt = Math.max(0.5, (Date.now() - t0) / 1000);
              item.speed = bytes / dt;
              item.status = 'downloading';
              item.stage = '';
              notify(false);
            }
          }
          // Keep errBuf to last 500 chars to avoid memory growth
          if (errBuf.length > 500) errBuf = errBuf.slice(-500);
        });
        ff.on('error', reject);
        ff.on('close', (c) => {
          if (ac.signal.aborted) { resolve(); return; }
          if (c === 0) resolve();
          else reject(new Error(`ffmpeg exited ${c}`));
        });
      });

      if (ac.signal.aborted) return;
      procs.delete(item.id);
      const st = fs.statSync(finalPath);
      item.path = finalPath;
      item.name = path.basename(finalPath);
      item.downloaded = item.total = st.size;
      item.status = 'completed';
      item.speed = 0;
      item.eta = null;
      item.finishedAt = Date.now();
      log.info(`[video-dl] hls-direct finished: ${finalPath}`);
      notify(true);
    } catch (err) {
      procs.delete(item.id);
      if (item._canceling) return;
      item.status = 'error';
      item.speed = 0;
      item.eta = null;
      item.error = err.message.slice(0, 120);
      item.finishedAt = Date.now();
      notify(true);
    }
    return;
  }

  const rawTs = path.join(tmpDir, `${item.id.replace(':', '-')}-${Date.now()}.ts`);
  const rawExt = asMp3 ? 'mp3' : ffmpegDir ? 'mp4' : 'ts';

  try {
    // Check if the URL is a numbered .ts segment (e.g. seg-1-v1-a1.ts or chunk_001.ts)
    const segMatch = item.url.match(/^(.*?(?:seg(?:ment)?|chunk|piece|part|frag)?[-_]?)(\d+)([-_][^/?#]*)?(\.ts(?:\?.*)?)$/i);
    const outStream = fs.createWriteStream(rawTs);
    const t0 = Date.now();

    if (segMatch) {
      const [, prefix, startStr, mid = '', suffix] = segMatch;
      const padLen = startStr.startsWith('0') ? startStr.length : 0;
      let idx = parseInt(startStr, 10);
      item.stage = '';
      while (!ac.signal.aborted && idx < 10000) {
        const numStr = padLen ? String(idx).padStart(padLen, '0') : String(idx);
        const segUrl = `${prefix}${numStr}${mid}${suffix}`;
        const r = await fetch(segUrl, {
          headers: { 'user-agent': UA, referer: item.url },
          signal: ac.signal,
        });
        if (!r.ok) {
          if (idx === parseInt(startStr, 10)) throw new Error(`HTTP ${r.status}`);
          break; // end of segments
        }
        const buf = Buffer.from(await r.arrayBuffer());
        if (!buf.length) break;
        outStream.write(buf);
        item.downloaded += buf.length;
        item.total = item.downloaded;
        const dt = Math.max(0.2, (Date.now() - t0) / 1000);
        item.speed = item.downloaded / dt;
        notify(false);
        idx++;
      }
    } else {
      const r = await fetch(item.url, {
        headers: { 'user-agent': UA, referer: item.url },
        signal: ac.signal,
      });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      item.total = num(r.headers.get('content-length'));
      item.stage = '';
      for await (const chunk of r.body) {
        if (ac.signal.aborted) break;
        const buf = Buffer.from(chunk);
        outStream.write(buf);
        item.downloaded += buf.length;
        if (item.downloaded > item.total) item.total = item.downloaded;
        const dt = Math.max(0.2, (Date.now() - t0) / 1000);
        item.speed = item.downloaded / dt;
        notify(false);
      }
    }
    await new Promise((r) => outStream.end(r));

    if (ac.signal.aborted) {
      try { fs.rmSync(rawTs, { force: true }); } catch {}
      return;
    }

    const ffmpegExe = ffmpegDir && path.join(ffmpegDir, 'ffmpeg.exe');
    if (ffmpegExe && isFile(ffmpegExe)) {
      item.status = 'processing';
      item.speed = 0;
      item.eta = null;
      notify(true);
      await new Promise((resolve, reject) => {
        const ffArgs = asMp3
          ? ['-y', '-i', rawTs, '-vn', '-c:a', 'libmp3lame', '-b:a', '320k', finalPath]
          : ['-y', '-i', rawTs, '-c', 'copy', '-bsf:a', 'aac_adtstoasc', finalPath];
        const ff = spawn(ffmpegExe, ffArgs, { windowsHide: true });
        procs.set(item.id, ff);
        ff.on('error', reject);
        ff.on('close', (c) => (c === 0 ? resolve() : reject(new Error(`ffmpeg exited ${c}`))));
      });
      try { fs.rmSync(rawTs, { force: true }); } catch {}
    } else {
      fs.renameSync(rawTs, finalPath);
    }

    procs.delete(item.id);
    const st = fs.statSync(finalPath);
    item.path = finalPath;
    item.name = path.basename(finalPath);
    item.downloaded = item.total = st.size;
    item.status = 'completed';
    item.speed = 0;
    item.eta = null;
    item.finishedAt = Date.now();
    notify(true);
  } catch (err) {
    procs.delete(item.id);
    try { fs.rmSync(rawTs, { force: true }); } catch {}
    if (item._canceling) return;
    if (item._pausing) {
      item.status = 'paused';
      item.speed = 0;
      item.eta = null;
      notify(true);
      return;
    }
    item.status = 'error';
    item.speed = 0;
    item.eta = null;
    item.error = err.message || 'TS download failed';
    notify(true);
  }
}

// ---------- spawning yt-dlp for the chosen quality or MP3 ----------
function spawnDownload(item, ytdlpExe, ffmpegDir, onState = () => {}, onMeta = () => {}, usePreloaded = true) {
  if (item.selector === 'ts:direct') {
    onMeta({ ok: true, how: 'started', label: item.label || 'TS' });
    downloadTsDirect(item, ffmpegDir, item.kind === 'audio');
    return;
  }

  const outDir = downloadsDir();
  const tmpDir = tempDir();
  fs.mkdirSync(outDir, { recursive: true });

  const isAudio = item.kind === 'audio' || String(item.selector).startsWith('mp3:');
  const formatSel = isAudio ? 'ba/b' : ffmpegDir ? item.selector || 'bv*+ba/b' : 'b';
  const hasInfo = usePreloaded && item.infoFile && isFile(item.infoFile);

  const args = [
    '--ignore-config',
    '--no-exec',
    '--no-update',
    '--no-playlist',
    '--no-simulate',
    '--continue',
    '--progress',       // force progress output even when not on a tty (Electron has no tty)
    '--newline',        // one line per update instead of \r overwrite
    '--encoding', 'utf-8',
    '--referer', item.referer || item.url,
    ...(item.referer ? ['--add-header', `Origin:${new URL(item.referer).origin}`] : []),
    '--user-agent', UA,
    '-N', '8',
    '--hls-prefer-native',
    '-f', formatSel,
    ...(ffmpegDir ? ['--ffmpeg-location', ffmpegDir] : []),
    ...(isAudio
      ? ['-x', '--audio-format', 'mp3', '--audio-quality', '0']
      : ffmpegDir
        ? ['--merge-output-format', 'mp4/mkv']
        : []),
    ...jsRuntimeArgs(),
    '-P', `home:${outDir}`,
    '-P', `temp:${tmpDir}`,
    '-o', `%(title).150B [${item.tag || item.label || 'HD'}].%(ext)s`,
    '--print', 'before_dl:NOVA_META\t%(title)s\t%(thumbnail)s\t%(resolution)s\t%(height)s\t%(fps)s\t%(filesize,filesize_approx)s\t%(ext)s',
    '--print', 'after_move:NOVA_FILE\t%(filepath)s',
    '--progress-template', 'download:NOVA_PROG\t%(progress.downloaded_bytes)s\t%(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t%(progress.speed)s\t%(progress.eta)s',
    '--progress-template', 'postprocess:NOVA_POST\t%(progress.status)s',
    ...(hasInfo ? ['--load-info-json', item.infoFile] : ['--', item.url]),
  ];

  item.status = 'downloading';
  item.stage = 'starting';
  item.error = '';
  item.speed = 0;
  item.eta = null;
  item._pausing = false;
  item._canceling = false;
  item._streamDl = 0;
  item._streamTotal = 0;
  item._doneBytes = 0;
  notify(true);

  const proc = spawn(ytdlpExe, args, {
    windowsHide: true,
    env: { ...process.env, PYTHONIOENCODING: 'utf-8' },
  });
  procs.set(item.id, proc);

  let lastErr = '';
  let metaSettled = false;
  const settleMeta = (val) => {
    if (metaSettled) return;
    metaSettled = true;
    onMeta(val);
  };

  if (hasInfo) {
    // We already know the title, thumbnail, and quality from the probe!
    settleMeta({ ok: true, how: 'started', label: item.label });
  }

  const handleLine = (raw) => {
    const line = raw.trim();
    if (!line) return;
    if (line.startsWith('NOVA_META\t')) {
      const [, title, thumb, , , , size, ext] = line.split('\t');
      const e = isAudio ? 'mp3' : ext && ext !== 'NA' ? ext : 'mp4';
      if (title && title !== 'NA') {
        item.baseTitle = title;
        item.name = cleanName(`${title} [${item.label || 'HD'}]`, e);
      }
      if (thumb && /^https?:\/\//i.test(thumb)) item.thumbnail = thumb;
      const total = num(size);
      if (total > item.total && !isAudio) item.total = total;
      item.stage = '';
      onState(`Downloading ${item.label}…`);
      notify(true);
      settleMeta({ ok: true, how: 'started', label: item.label });
      return;
    }
    if (line.startsWith('NOVA_PROG\t')) {
      const [, dlStr, totStr, estStr, spdStr, etaStr] = line.split('\t');
      const dl = num(dlStr);
      const tot = num(totStr) || num(estStr);
      const spd = num(spdStr);
      const eta = Number(etaStr);

      // Detect stream reset: yt-dlp started a new stream (video → audio)
      // Reset happens when downloaded bytes jump back significantly
      const streamReset = item._streamDl > 1024 && dl < item._streamDl * 0.3;
      if (streamReset) {
        // Commit completed stream bytes to _doneBytes
        item._doneBytes += item._streamTotal > 0 ? item._streamTotal : item._streamDl;
        item._streamDl = 0;
        item._streamTotal = 0;
        // Keep existing total — it already includes estimated sizes for both streams
      }
      if (dl > item._streamDl) item._streamDl = dl;
      if (tot > item._streamTotal) item._streamTotal = tot;

      item.downloaded = item._doneBytes + item._streamDl;
      const combinedTotal = item._doneBytes + item._streamTotal;
      // Only grow total, never shrink (avoids bar jumping back)
      if (combinedTotal > item.total) item.total = combinedTotal;
      if (item.total > 0 && item.downloaded > item.total) item.total = item.downloaded;

      item.speed = spd;
      item.eta = Number.isFinite(eta) && eta >= 0 ? Math.round(eta) : null;
      item.stage = '';
      if (item.status !== 'processing') item.status = 'downloading';
      notify(false);
      settleMeta({ ok: true, how: 'started', label: item.label || 'best quality' });
      return;
    }
    if (line.startsWith('NOVA_POST\t') || line.startsWith('[Merger]') || line.startsWith('[ExtractAudio]') || line.startsWith('[Fixup')) {
      item.status = 'processing';
      item.stage = '';
      item.speed = 0;
      item.eta = null;
      notify(true);
      return;
    }
    if (line.startsWith('NOVA_FILE\t')) {
      const file = line.slice('NOVA_FILE\t'.length).trim();
      if (file) {
        item.path = file;
        item.name = path.basename(file);
      }
      return;
    }
    if (line.startsWith('ERROR:')) {
      lastErr = line.replace(/^ERROR:\s*(\[[^\]]+\]\s*[^:]+:\s*)?/, '').trim().slice(0, 140);
    }
  };

  let outBuf = '';
  let errBuf = '';
  proc.stdout.setEncoding('utf8');
  proc.stderr.setEncoding('utf8');
  proc.stdout.on('data', (chunk) => {
    outBuf += chunk;
    const parts = outBuf.split(/[\r\n]+/);
    outBuf = parts.pop() || '';
    parts.forEach(handleLine);
  });
  proc.stderr.on('data', (chunk) => {
    errBuf += chunk;
    const parts = errBuf.split(/[\r\n]+/);
    errBuf = parts.pop() || '';
    parts.forEach(handleLine);
  });

  proc.on('error', (err) => {
    procs.delete(item.id);
    if (item._canceling) return;
    item.status = 'error';
    item.stage = '';
    item.speed = 0;
    item.eta = null;
    item.error = err.message;
    notify(true);
    settleMeta({ ok: false, error: 'mdm-error' });
  });

  proc.on('close', (code) => {
    if (outBuf.trim()) handleLine(outBuf);
    if (errBuf.trim()) handleLine(errBuf);
    procs.delete(item.id);

    if (item._canceling) return;
    if (item._pausing) {
      item.status = 'paused';
      item.stage = '';
      item.speed = 0;
      item.eta = null;
      notify(true);
      settleMeta({ ok: true, how: 'paused', label: item.label || 'best quality' });
      return;
    }

    // If --load-info-json failed (e.g. expired signed URLs), retry once directly from item.url
    if (code !== 0 && hasInfo) {
      item.infoFile = '';
      spawnDownload(item, ytdlpExe, ffmpegDir, onState, onMeta, false);
      return;
    }

    if (code === 0) {
      try {
        if (item.path && fs.existsSync(item.path)) {
          const st = fs.statSync(item.path);
          if (st.size > 0) item.downloaded = item.total = st.size;
        }
      } catch {}
      if (!item.total && item.downloaded) item.total = item.downloaded;
      item.status = 'completed';
      item.stage = '';
      item.speed = 0;
      item.eta = null;
      item.finishedAt = Date.now();
      log.info(`[video-dl] finished ${item.name} -> ${item.path}`);
      notify(true);
      settleMeta({ ok: true, how: 'started', label: item.label || 'best quality' });
    } else {
      item.status = 'error';
      item.stage = '';
      item.speed = 0;
      item.eta = null;
      item.error = lastErr || 'Could not download this video';
      log.info(`[video-dl] failed (${code}): ${item.error}`);
      notify(true);
      settleMeta({ ok: false, error: lastErr ? 'no-video' : 'mdm-error', message: item.error });
    }
  });
}

// ---------- starting a download (specific quality choice or best quality) ----------
async function downloadBest(page, title, onState = () => {}, choice = null, sourceUrl = '') {
  if (!/^https?:\/\//i.test(page)) return { ok: false, error: 'no-url' };

  let ytdlpExe;
  try {
    ytdlpExe = await ensureYtdlp(onState);
  } catch (err) {
    log.info(`[video-dl] yt-dlp setup failed: ${err.message}`);
    return { ok: false, error: 'no-ytdlp' };
  }
  if (!ytdlpExe) return { ok: false, error: 'no-ytdlp' };

  const ffmpegDir = await ensureFfmpegDir(onState);

  // Resolve probe cache using page URL, manifest URL, or source page URL
  let cached = probeCache.get(page)?.data;
  if (!cached && sourceUrl) cached = probeCache.get(sourceUrl)?.data;
  if (!cached) {
    for (const [k, v] of probeCache) {
      if (
        k === page ||
        k === sourceUrl ||
        v.data?.url === page ||
        v.data?.url === sourceUrl ||
        v.data?._sourcePageUrl === page ||
        v.data?._sourcePageUrl === sourceUrl
      ) {
        cached = v.data;
        break;
      }
    }
  }

  const sourcePage = cached?._sourcePageUrl || sourceUrl || page;
  // Use the manifest URL if one was found by the stream sniffer
  if (cached?.url && cached.url !== page) page = cached.url;

  const isAudio = choice?.kind === 'audio' || String(choice?.selector || '').startsWith('mp3:');
  const label = choice?.label ? (choice.tag ? `${choice.label} ${choice.tag}` : choice.label) : 'best quality';
  const selector = choice?.selector || (ffmpegDir ? 'bv*+ba/b' : 'b');
  const ext = isAudio ? 'mp3' : 'mp4';
  const baseTitle = cached?.title || title || 'Video';

  const id = `v:${++seq}`;
  const item = {
    id,
    url: page,
    referer: sourcePage,
    _sourcePageUrl: sourcePage,
    baseTitle,
    name: cleanName(`${baseTitle} [${choice?.label || 'HD'}]`, ext),
    path: '',
    thumbnail: cached?.thumbnail || '',
    label,
    tag: choice?.label || 'HD',
    selector,
    kind: isAudio ? 'audio' : 'video',
    infoFile: cached?.infoFile || '',
    createdAt: Date.now(),
    downloaded: 0,
    total: num(choice?.size),
    speed: 0,
    eta: null,
    status: 'downloading',
    stage: 'starting',
    error: '',
    finishedAt: 0,
    _ytdlpExe: ytdlpExe,
    _ffmpegDir: ffmpegDir,
  };
  downloads.set(id, item);

  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve({ ok: true, how: 'started', label }), 4000);
    spawnDownload(item, ytdlpExe, ffmpegDir, onState, (res) => {
      clearTimeout(timer);
      resolve(res);
    });
  });
}

function cleanTempFor(item) {
  try {
    const tDir = tempDir();
    const stem = cleanStem(item.baseTitle || item.name).slice(0, 40).toLowerCase();
    for (const f of fs.readdirSync(tDir)) {
      if (stem && f.toLowerCase().includes(stem)) {
        try { fs.rmSync(path.join(tDir, f), { force: true }); } catch {}
      }
    }
  } catch {}
}

function control(id, action) {
  const item = downloads.get(id);
  if (!item) return;
  if (action === 'cancel') {
    item._canceling = true;
    const proc = procs.get(id);
    if (proc) {
      procs.delete(id);
      killTree(proc);
    }
    downloads.delete(id);
    setTimeout(() => cleanTempFor(item), 400);
    notify(true);
    return;
  }
  if (action === 'pause') {
    const proc = procs.get(id);
    if (proc) {
      item._pausing = true;
      item.status = 'paused';
      item.speed = 0;
      item.eta = null;
      notify(true);
      killTree(proc);
    }
  } else if (action === 'resume') {
    if (procs.has(id) || !['paused', 'error'].includes(item.status)) return;
    const ytdlpExe = item._ytdlpExe || findYtdlp();
    const ffmpegDir = item._ffmpegDir || findFfmpegDir();
    if (!ytdlpExe) return;
    spawnDownload(item, ytdlpExe, ffmpegDir, () => {}, () => {}, false);
  }
}

function stopAll() {
  for (const proc of procs.values()) killTree(proc);
  procs.clear();
}

function start(changed) {
  onChange = changed;
}

module.exports = { start, items, control, probeVideo, downloadBest, stopAll, findYtdlp, findFfmpegDir, ensureFfmpegDir };
