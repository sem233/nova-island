// QuickClips — saved text snippets manager for Nova Island.
// Stores clips in userData/clips.json. Zero RAM when idle (no timers, no watchers).
const fs = require('fs');
const path = require('path');
const { app, clipboard, ipcMain } = require('electron');

const DATA_FILE = () => path.join(app.getPath('userData'), 'clips.json');
const MAX_CLIPS = 200;

let _clips = null; // lazy-loaded

function load() {
  if (_clips) return _clips;
  try {
    const raw = fs.readFileSync(DATA_FILE(), 'utf8');
    _clips = JSON.parse(raw);
    if (!Array.isArray(_clips)) _clips = [];
    const filtered = _clips.filter((c) => c && typeof c.text === 'string' && c.text !== '[object Promise]');
    if (filtered.length !== _clips.length) {
      _clips = filtered;
      save();
    }
  } catch {
    _clips = [];
  }
  return _clips;
}

function save() {
  try {
    fs.mkdirSync(path.dirname(DATA_FILE()), { recursive: true });
    fs.writeFileSync(DATA_FILE(), JSON.stringify(_clips, null, 2), 'utf8');
  } catch {}
}

function list() {
  const clips = load();
  // pinned first, then by createdAt desc
  return [...clips].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return b.createdAt - a.createdAt;
  });
}

function add(text) {
  if (typeof text !== 'string' || !text.trim() || text === '[object Promise]') return null;
  text = text.slice(0, 10000);
  const clips = load();
  // deduplicate: move existing to top instead of duplicating
  const idx = clips.findIndex((c) => c.text === text);
  if (idx !== -1) {
    const existing = clips.splice(idx, 1)[0];
    existing.createdAt = Date.now();
    clips.unshift(existing);
    save();
    return existing;
  }
  const clip = { id: `c:${Date.now()}:${Math.random().toString(36).slice(2, 7)}`, text, pinned: false, createdAt: Date.now() };
  clips.unshift(clip);
  // trim to max
  while (clips.length > MAX_CLIPS) clips.pop();
  save();
  return clip;
}

function remove(id) {
  _clips = load().filter((c) => c.id !== id);
  save();
}

function pin(id, pinned) {
  const c = load().find((c) => c.id === id);
  if (c) { c.pinned = !!pinned; save(); }
}

function edit(id, text) {
  if (typeof text !== 'string' || !text.trim()) return;
  const c = load().find((c) => c.id === id);
  if (c) { c.text = text.slice(0, 10000); c.updatedAt = Date.now(); save(); }
}

function clear(keepPinned = true) {
  _clips = keepPinned ? load().filter((c) => c.pinned) : [];
  save();
}

async function copyToClipboard(id) {
  const c = load().find((c) => c.id === id);
  if (c) {
    try {
      const res = clipboard.writeText(c.text);
      if (res && typeof res.then === 'function') await res;
    } catch {}
  }
  return !!c;
}

async function addFromClipboard() {
  let text = '';
  try {
    const res = clipboard.readText();
    text = (res && typeof res.then === 'function') ? await res : res;
  } catch {}
  text = String(text || '').trim();
  if (!text || text === '[object Promise]') return null;
  return add(text);
}

// ---------- IPC wiring ----------
function init(sendToIsland) {
  ipcMain.handle('clips-list',   () => list());
  ipcMain.handle('clips-add',    (_e, text) => { const c = add(text); sendToIsland(); return c; });
  ipcMain.handle('clips-add-clipboard', async () => { const c = await addFromClipboard(); if (c) sendToIsland(); return c; });
  ipcMain.handle('clips-remove', (_e, id)   => { remove(id); sendToIsland(); });
  ipcMain.handle('clips-pin',    (_e, id, pinned) => { pin(id, pinned); sendToIsland(); });
  ipcMain.handle('clips-edit',   (_e, id, text) => { edit(id, text); sendToIsland(); });
  ipcMain.handle('clips-clear',  (_e, keepPinned) => { clear(keepPinned !== false); sendToIsland(); });
  ipcMain.on('clips-copy', (_e, id) => copyToClipboard(id));
}

module.exports = { init, list, add, addFromClipboard, remove, pin, edit, clear, copyToClipboard };
