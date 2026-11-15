// Tasks manager for Nova Island — powers the Tasks (.nvp) plugin.
// Stores to-do items in userData/tasks.json.
const fs = require('fs');
const path = require('path');
const { app, ipcMain } = require('electron');

const DATA_FILE = () => path.join(app.getPath('userData'), 'tasks.json');

let _tasks = null;

function load() {
  if (_tasks) return _tasks;
  try {
    const raw = fs.readFileSync(DATA_FILE(), 'utf8');
    _tasks = JSON.parse(raw);
    if (!Array.isArray(_tasks)) _tasks = [];
  } catch {
    _tasks = [];
  }
  if (!_tasks.length) {
    _tasks = [
      {
        id: 't:welcome:1',
        text: 'مرحباً بك في قائمة المهام (Tasks) على نوفا آيلاند 📝',
        priority: 'high',
        due: 'اليوم',
        done: false,
        createdAt: Date.now(),
      },
      {
        id: 't:welcome:2',
        text: 'يمكنك تحديد المهمة كمنجزة بالضغط على المربع أو حذفها أو إضافة مهمة جديدة',
        priority: 'normal',
        due: '',
        done: false,
        createdAt: Date.now() - 1000,
      }
    ];
    save();
  }
  return _tasks;
}

function save() {
  try {
    fs.mkdirSync(path.dirname(DATA_FILE()), { recursive: true });
    fs.writeFileSync(DATA_FILE(), JSON.stringify(_tasks, null, 2), 'utf8');
  } catch {}
}

const PRIO_WEIGHT = { urgent: 4, high: 3, normal: 2, low: 1 };

function list() {
  const tasks = load();
  return [...tasks].sort((a, b) => {
    if (!!a.done !== !!b.done) return a.done ? 1 : -1;
    const pa = PRIO_WEIGHT[a.priority] || 2;
    const pb = PRIO_WEIGHT[b.priority] || 2;
    if (pa !== pb) return pb - pa;
    return (b.createdAt || 0) - (a.createdAt || 0);
  });
}

function add(text, opts = {}) {
  if (typeof text !== 'string' || !text.trim()) return null;
  const tasks = load();
  const priority = ['urgent', 'high', 'normal', 'low'].includes(opts.priority) ? opts.priority : 'normal';
  const due = typeof opts.due === 'string' ? opts.due.trim().slice(0, 50) : '';
  const task = {
    id: `t:${Date.now()}:${Math.random().toString(36).slice(2, 6)}`,
    text: text.trim().slice(0, 300),
    priority,
    due,
    done: false,
    createdAt: Date.now(),
  };
  tasks.unshift(task);
  save();
  return task;
}

function toggle(id, done) {
  const t = load().find((x) => x.id === id);
  if (t) {
    t.done = typeof done === 'boolean' ? done : !t.done;
    t.completedAt = t.done ? Date.now() : null;
    save();
    return t;
  }
  return null;
}

function remove(id) {
  _tasks = load().filter((x) => x.id !== id);
  save();
  return true;
}

function clear(completedOnly = true) {
  _tasks = completedOnly ? load().filter((x) => !x.done) : [];
  save();
}

function init(sendToIsland) {
  ipcMain.handle('tasks-list', () => list());
  ipcMain.handle('tasks-add', (_e, text, opts) => { const t = add(text, opts); if (sendToIsland) sendToIsland(); return t; });
  ipcMain.handle('tasks-toggle', (_e, id, done) => { const t = toggle(id, done); if (sendToIsland) sendToIsland(); return t; });
  ipcMain.handle('tasks-remove', (_e, id) => { remove(id); if (sendToIsland) sendToIsland(); return true; });
  ipcMain.handle('tasks-clear', (_e, compOnly) => { clear(compOnly !== false); if (sendToIsland) sendToIsland(); return true; });
}

module.exports = { init, list, add, toggle, remove, clear };
