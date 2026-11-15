// Desktop Widgets Window: renders interactive glass widgets directly on the Windows desktop.
const path = require('path');
const { BrowserWindow, screen, ipcMain } = require('electron');
const settings = require('./settings');
const features = require('./features');

let win = null;
let prayerModule = null;

function getSavedPos() {
  const saved = settings.load('widgets.json', {});
  const wa = screen.getPrimaryDisplay().workArea;
  const defWidth = 470;
  const defHeight = 680;
  const x = Number.isFinite(saved.x) ? saved.x : wa.x + wa.width - defWidth - 30;
  const y = Number.isFinite(saved.y) ? saved.y : wa.y + 40;
  return { x, y, width: defWidth, height: defHeight };
}

function savePos() {
  if (!win || win.isDestroyed()) return;
  const b = win.getBounds();
  settings.save('widgets.json', { x: b.x, y: b.y });
}

function resetPos() {
  const wa = screen.getPrimaryDisplay().workArea;
  const defWidth = 470;
  const x = wa.x + wa.width - defWidth - 30;
  const y = wa.y + 40;
  if (win && !win.isDestroyed()) {
    win.setPosition(x, y);
    savePos();
  }
}

function send(channel, data) {
  if (win && !win.isDestroyed()) {
    win.webContents.send(channel, data);
  }
}

function createWindow() {
  if (win && !win.isDestroyed()) return;

  const pos = getSavedPos();
  win = new BrowserWindow({
    x: pos.x,
    y: pos.y,
    width: pos.width,
    height: pos.height,
    frame: false,
    transparent: true,
    resizable: true,
    hasShadow: false,
    skipTaskbar: true,
    backgroundColor: '#00000000',
    title: 'Nova Desktop Widgets',
    webPreferences: {
      preload: path.join(__dirname, 'widgets', 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  win.loadFile(path.join(__dirname, 'widgets', 'index.html'));
  win.setAlwaysOnTop(false);

  win.on('moved', savePos);
  win.on('closed', () => { win = null; });
}

function sync() {
  const f = features.get();
  if (f.desktopWidgets) {
    if (!win || win.isDestroyed()) createWindow();
    send('features', f);
    if (prayerModule) {
      send('prayer', prayerModule.snapshot());
    }
  } else {
    if (win && !win.isDestroyed()) {
      win.close();
      win = null;
    }
  }
}

function start(prayer) {
  prayerModule = prayer;
  features.listen(() => sync());

  ipcMain.on('widgets-reset-pos', resetPos);
  ipcMain.handle('widgets-features', () => features.get());
  ipcMain.handle('widgets-prayer', () => (prayerModule ? prayerModule.snapshot() : null));

  sync();
}

module.exports = { start, resetPos, sync };
