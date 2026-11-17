// Embeds the app's page files from the repo root, exactly as they are, and writes a SHA-256 manifest
// of them. The repo is only read. A changed page file changes MANIFEST, which the parity tests use to
// know the golden screenshots must be taken again.
// What 1.8 changes or adds (1.7 files are never edited):
//   nova-next/pages/<path>   a page file of 1.8's own; it replaces the repo's file of the same path
//   PATCHES                  a small edit to a repo file, made here; the build stops if the text it
//                            replaces isn't there exactly once (the 1.7 file changed: look again)
use std::{env, fmt::Write as _, fs, path::{Path, PathBuf}};

use sha2::{Digest, Sha256};

// (folder, extensions) served under https://nova.island/<path>
const SERVED: &[(&str, &[&str])] = &[
    ("", &["html", "css", "js"]),
    ("views", &["js"]),
    ("fonts", &["ttf"]),
    ("prefs", &["html", "css", "js"]),
    ("prefs/apps", &["png"]),
    ("mini", &["html", "css", "js"]),
];
// single files outside those folders
const EXTRA: &[&str] = &["build/icon.png", "build/icon.ico", "plugins-catalog.json"];
// never served: the preload files run as the bridge (PRELOAD_*), not as pages
const SKIP: &[&str] = &["preload.js", "prefs/preload.js", "mini/preload.js", "nova.js", "apps.js", "browser-downloads.js",
    "calendar.js", "downloads.js", "features.js", "log.js", "mini.js", "network.js", "news.js", "plugins.js", "prayer.js",
    "prefs-window.js", "privacy.js", "settings.js", "shelf.js", "signing.js", "smartclip.js", "system.js", "updater.js"];

const OVERLAY: &str = "nova-next/pages";
// preload files in the overlay: bridges, not pages
const OVERLAY_PRELOADS: &[&str] = &["radio/preload.js", "tasks/preload.js"];
const PATCHES: &[(&str, &str, &str)] = &[
    // Quran Radio plays surahs from mp3quran's server (1.8)
    ("index.html", "https://qurango.net https://*.qurango.net", "https://qurango.net https://*.qurango.net https://cdn.mp3quran.net"),
    // the island's own styles in 1.8 (style-2.css), after 1.7's, plus QuickClips, MDM & Calculator
    ("index.html", "  <link rel=\"stylesheet\" href=\"style.css\">\n", "  <link rel=\"stylesheet\" href=\"style.css\">\n  <link rel=\"stylesheet\" href=\"style-2.css\">\n  <link rel=\"stylesheet\" href=\"style-quickclips.css\">\n  <link rel=\"stylesheet\" href=\"style-calculator.css\">\n"),
    // the radio window and the radio's state for it, and opening the app of a notification (1.8)
    (
        "preload.js",
        "  pluginSet: (id, key, value) => ipcRenderer.send('plugin-setting', id, key, value),\n",
        "  pluginSet: (id, key, value) => ipcRenderer.send('plugin-setting', id, key, value),\n  radioOpen: () => ipcRenderer.send('radio-open'),\n  appNotifyOpen: (id) => ipcRenderer.send('app-notify-open', id),\n  radioNow: (s) => ipcRenderer.send('radio-now', s),\n  radioSuwar: () => ipcRenderer.invoke('radio-suwar'),\n  onRadioPlay: on('radio-play'),\n  videoProbe: (req) => ipcRenderer.invoke('video-probe', req),\n  videoDownload: (req) => ipcRenderer.invoke('video-download', req),\n  onVideoDl: on('video-dl'),\n  clips: {\n    list: () => ipcRenderer.invoke('clips-list'),\n    add: (t) => ipcRenderer.invoke('clips-add', t),\n    addFromClipboard: () => ipcRenderer.invoke('clips-add-clipboard'),\n    remove: (id) => ipcRenderer.invoke('clips-remove', id),\n    pin: (id, p) => ipcRenderer.invoke('clips-pin', id, p),\n    edit: (id, t) => ipcRenderer.invoke('clips-edit', id, t),\n    clear: (p) => ipcRenderer.invoke('clips-clear', p),\n    copy: (id) => ipcRenderer.send('clips-copy', id),\n    onUpdate: on('clips'),\n  },\n",
    ),
    // other apps' notifications on the island, off by default (1.8)
    (
        "prefs/index.html",
        "<label class=\"row\"><b>Mic &amp; camera dots</b><input type=\"checkbox\" class=\"sw\" data-feature=\"privacyDots\"></label>\n",
        "<label class=\"row\"><b>Mic &amp; camera dots</b><input type=\"checkbox\" class=\"sw\" data-feature=\"privacyDots\"></label>\n            <label class=\"row\"><div class=\"txt\"><b>App notifications on the island</b><small>WhatsApp, Telegram and other apps. Turn off Windows' own banners to see each one once</small></div><input type=\"checkbox\" class=\"sw\" data-feature=\"appNotify\"></label>\n",
    ),
    // Apps tab: the open apps first (1.8)
    (
        "prefs/index.html",
        "            <label class=\"row\"><b>New track card</b><input type=\"checkbox\" class=\"sw\" data-feature=\"trackPeek\"></label>\n          </div>\n        </div>\n",
        "            <label class=\"row\"><b>New track card</b><input type=\"checkbox\" class=\"sw\" data-feature=\"trackPeek\"></label>\n          </div>\n        </div>\n        <div class=\"group\">\n          <h2>Apps</h2>\n          <div class=\"card\">\n            <label class=\"row\"><div class=\"txt\"><b>Open apps first</b><small>Apps with a window open come first in the Apps tab</small></div><input type=\"checkbox\" class=\"sw\" data-feature=\"appsRunningFirst\"></label>\n          </div>\n        </div>\n",
    ),
    // Settings: its own 1.8 styles, and Telegram beside Facebook (1.8)
    ("prefs/index.html", "  <link rel=\"stylesheet\" href=\"prefs.css\">
", "  <link rel=\"stylesheet\" href=\"prefs.css\">
  <link rel=\"stylesheet\" href=\"prefs-2.css\">
"),
    (
        "prefs/index.html",
        "      <button class=\"side-social\" data-support=\"facebook\" title=\"facebook.com/nova4kit\">
        <svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M14 8.5V6.8c0-.8.2-1.3 1.4-1.3H17V2.6A21 21 0 0 0 14.6 2.5C12.2 2.5 10.6 4 10.6 6.6v1.9H8v3.2h2.6V21.5H14v-9.8h2.6l.4-3.2H14Z\"/></svg>
        <span>Nova on Facebook</span>
      </button>
",
        "      <div class=\"side-socials\">
      <button class=\"side-social\" data-support=\"facebook\" title=\"facebook.com/nova4kit\">
        <svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M14 8.5V6.8c0-.8.2-1.3 1.4-1.3H17V2.6A21 21 0 0 0 14.6 2.5C12.2 2.5 10.6 4 10.6 6.6v1.9H8v3.2h2.6V21.5H14v-9.8h2.6l.4-3.2H14Z\"/></svg>
        <span>Facebook</span>
      </button>
      <button class=\"side-social tg\" data-support=\"telegram\" title=\"t.me/nova4apps\">
        <svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M21.4 3.7 2.9 10.9c-1.2.5-1.2 1.2-.2 1.5l4.7 1.5 1.8 5.5c.2.6.4.9.9.9.4 0 .6-.2.9-.5l2.3-2.2 4.8 3.5c.9.5 1.5.2 1.7-.8l3.2-15c.3-1.3-.5-1.9-1.6-1.6Zm-11.9 11-.4 3.6-1.4-4.6L18 7.3l-8.5 7.4Z\"/></svg>
        <span>Telegram</span>
      </button>
      </div>
",
    ),
    // Reminders and scenes (water, the adhan) that never left (1.8.1): hovering pauses their
    // countdown, and a card that opened under a mouse resting there (the browser's tabs) counted
    // as hovered, so it stayed until the mouse moved. Now only a mouse that moved since the card
    // came pauses it. scene.js keeps the record for both.
    (
        "views/scene.js",
        "  let lastTick = Date.now();\n",
        r#"  let lastTick = Date.now();
  let shownAt = 0;
  // the mouse's last real move over the island: synthetic moves (the window starting to take
  // the mouse under a cursor that stands still) repeat the last point and don't count
  let lastPt = null;
  let movedAt = 0;
  document.addEventListener('mousemove', (e) => {
    const p = [e.screenX, e.screenY];
    if (lastPt && Math.hypot(p[0] - lastPt[0], p[1] - lastPt[1]) > 3) movedAt = Date.now();
    lastPt = p;
  }, { passive: true });
  // a new card starts from where the mouse is now
  window.addEventListener('island:mode', () => { lastPt = null; });
  window.mouseMovedSince = (t) => movedAt >= t;
"#,
    ),
    ("views/scene.js", "    state.sceneUntil = Date.now() + total;\n", "    state.sceneUntil = Date.now() + total;\n    shownAt = Date.now();\n"),
    // Settings › Plugins: Tasks gets its own icon (1.7 had one icon, the radio's, for every plugin)
    (
        "prefs/prefs.js",
        "<span class=\"plug-ico\">${PLUG_ICON}</span>",
        "<span class=\"plug-ico${p.kind === 'tasks' || p.id === 'tasks' ? ' tasks' : ''}\">${p.kind === 'tasks' || p.id === 'tasks' ? '<svg viewBox=\"0 0 24 24\"><rect x=\"3\" y=\"3\" width=\"18\" height=\"18\" rx=\"5\"/><path d=\"m8 12.5 3 3 5-6\"/></svg>' : PLUG_ICON}</span>",
    ),
    // Tasks on the island (1.8.3): a tab after Timer while the plugin is on, its view, its script, plus QuickClips & MDM
    (
        "index.html",
        "            <button role=\"tab\" data-tab=\"system\" class=\"tab\" aria-label=\"System\">",
        "            <button role=\"tab\" data-tab=\"tasks\" class=\"tab\" aria-label=\"Tasks\" hidden>\n              <svg class=\"s\" viewBox=\"0 0 24 24\"><rect x=\"3\" y=\"3\" width=\"18\" height=\"18\" rx=\"5\"/><path d=\"m8 12.5 3 3 5-6\"/></svg><span>Tasks</span><i class=\"badge\" id=\"tasks-badge\"></i>\n            </button>\n            <button role=\"tab\" data-tab=\"quickclips\" class=\"tab\" aria-label=\"Quick Clips\">\n              <svg class=\"s\" viewBox=\"0 0 24 24\"><path d=\"M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2\"/><rect x=\"8\" y=\"2\" width=\"8\" height=\"4\" rx=\"1\"/><path d=\"m9 14 2 2 4-4\"/></svg><span>Clips</span>\n            </button>\n            <button role=\"tab\" data-tab=\"system\" class=\"tab\" aria-label=\"System\">",
    ),
    (
        "index.html",
        "        <section id=\"v-system\" class=\"view\" role=\"tabpanel\">",
        r#"        <section id="v-tasks" class="view" role="tabpanel">
          <div class="tk">
            <div class="tk-head">
              <b>Tasks</b><span class="sub" id="tk-count"></span>
              <button class="pill small" id="tk-open">Open</button>
            </div>
            <div class="tk-list" id="tk-list"></div>
          </div>
        </section>
        <section id="v-quickclips" class="view qc-view" role="tabpanel">
          <div class="qc-toolbar">
            <button id="qc-paste-btn" class="qc-action-btn primary" title="Paste text from clipboard instantly">
              <svg class="s" viewBox="0 0 24 24"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/><path d="m9 14 2 2 4-4"/></svg>
              <span>Paste Clipboard</span>
            </button>
            <button id="qc-add-btn" class="qc-action-btn" title="Write a custom clip">
              <svg class="s" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>
              <span>New</span>
            </button>
            <div class="qc-spacer"></div>
            <button id="qc-pin-island-btn" class="qc-icon-btn" title="Pin island open">
              <svg class="s" viewBox="0 0 24 24"><line x1="12" y1="17" x2="12" y2="22"/><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a1 1 0 0 0 0-2H8a1 1 0 0 0 0 2h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"/></svg>
            </button>
            <button id="qc-clear-btn" class="qc-icon-btn" title="Clear unpinned clips">
              <svg class="s" viewBox="0 0 24 24"><path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6"/></svg>
            </button>
          </div>
          <div id="qc-add-panel" class="qc-add-panel" hidden>
            <textarea id="qc-new-text" class="qc-textarea" placeholder="Type or paste your text here… (Ctrl+Enter to save)" rows="3"></textarea>
            <div class="qc-add-actions">
              <button id="qc-save-btn" class="pill small primary">Save Clip</button>
              <button id="qc-cancel-btn" class="pill small">Cancel</button>
            </div>
          </div>
          <div id="qc-list" class="qc-list"></div>
          <div class="qc-empty" id="qc-empty">
            <svg class="s" viewBox="0 0 24 24"><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><rect x="8" y="2" width="8" height="4" rx="1"/></svg>
            <span>No clips yet — click <b>Paste Clipboard</b> or <b>New</b></span>
          </div>
        </section>
        <section id="v-system" class="view" role="tabpanel">"#,
    ),
    (
        "index.html",
        "                <button id=\"pip-btn\" class=\"mini-btn pip\" title=\"Pop out into a floating mini player\" aria-label=\"Open mini player\">\n                  <svg class=\"s ico\" viewBox=\"0 0 24 24\"><path d=\"M21 10V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4\"/><rect x=\"12\" y=\"13\" width=\"10\" height=\"7\" rx=\"2\"/></svg>\n                  <span class=\"spin\" aria-hidden=\"true\"></span>\n                </button>\n",
        "                <button id=\"pip-btn\" class=\"mini-btn pip\" title=\"Pop out into a floating mini player\" aria-label=\"Open mini player\">\n                  <svg class=\"s ico\" viewBox=\"0 0 24 24\"><path d=\"M21 10V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4\"/><rect x=\"12\" y=\"13\" width=\"10\" height=\"7\" rx=\"2\"/></svg>\n                  <span class=\"spin\" aria-hidden=\"true\"></span>\n                </button>\n                <button id=\"vdl-btn\" class=\"mini-btn vdl\" title=\"Download Video / Extract MP3 (تحميل الفيديو أو الصوت)\" aria-label=\"Download video\">\n                  <svg class=\"s ico\" viewBox=\"0 0 24 24\"><path d=\"M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3\"/></svg>\n                  <svg class=\"s ico-x\" viewBox=\"0 0 24 24\"><path d=\"M18 6 6 18M6 6l12 12\"/></svg>\n                  <span class=\"spin\" aria-hidden=\"true\"></span>\n                </button>\n",
    ),
    (
        "index.html",
        "                  <span id=\"vol-pct\" class=\"vol-pct\"></span>\n                </div>\n              </div>\n            </div>\n",
        "                  <span id=\"vol-pct\" class=\"vol-pct\"></span>\n                </div>\n              </div>\n              <div class=\"vdl-drawer\" id=\"vdl-drawer\"><div class=\"vdl-list\" id=\"vdl-list\"></div></div>\n            </div>\n",
    ),
    ("index.html", "  <script src=\"views/radio.js\"></script>\n", "  <script src=\"views/radio.js\"></script>\n  <script src=\"views/tasks.js\"></script>\n  <script src=\"views/quickclips.js\"></script>\n  <script src=\"views/vdl.js\"></script>\n  <script src=\"views/calculator.js\"></script>\n"),
    ("renderer.js", "t === 'radio' ? !state.radioTab : ", "t === 'radio' ? !state.radioTab : t === 'tasks' ? !state.tasksTab : t === 'quickclips' ? false : "),
    (
        "renderer.js",
        "const NO_DRAG = 'button, input, .slider, .file, .app, .pick, .picker, .clip-item, .shelf-items';",
        "const NO_DRAG = 'button, input, textarea, .slider, .file, .app, .pick, .picker, .clips, .clip-item, .clip-row, .clip-list, .clip-full, .clip-full-text, .shelf-items, .vdl-drawer, .vdl-list, .dl-list, .tk-list, .news-list, .qc-list, .qc-textarea, .qc-add-panel, .qc-action-btn, .qc-icon-btn, .qc-btn, .qc-row';",
    ),
    (
        "renderer.js",
        "  if (e.target.closest('.picker-list, .clip-list, .shelf-items') || state.vol === null) return;\n",
        "  if (e.target?.closest?.('.picker-list, .clips, .clip-list, .clip-full, .clip-full-text, .clip-row, .shelf-items, .dl-list, .tk-list, .qc-list, .vdl-list, .news-list') || state.vol === null) return;\n",
    ),
    (
        "renderer.js",
        "bridge.onToggle(() => {\n  state.pinned = !state.pinned;\n",
        "window.state = state;\nwindow.render = render;\nwindow.setVdlHold = (on) => {\n  const hold = !!on;\n  if (state.vdlHold === hold) return hold;\n  state.vdlHold = hold;\n  bridge.setPinned(Boolean(state.pinned || state.vdlHold || state.calcHold));\n  render();\n  return hold;\n};\nwindow.setCalcHold = (on) => {\n  const hold = !!on;\n  if (state.calcHold === hold) return hold;\n  state.calcHold = hold;\n  bridge.setPinned(Boolean(state.pinned || state.vdlHold || state.calcHold));\n  render();\n  return hold;\n};\nwindow.togglePin = (force) => {\n  state.pinned = typeof force === 'boolean' ? force : !state.pinned;\n  if (!state.pinned) { state.hover = false; state.vdlHold = false; state.calcHold = false; }\n  bridge.setPinned(Boolean(state.pinned || state.vdlHold || state.calcHold));\n  render();\n  window.dispatchEvent(new CustomEvent('island:pinned', { detail: { pinned: state.pinned } }));\n  return state.pinned;\n};\nbridge.onToggle(() => {\n  window.togglePin();\n",
    ),
    (
        "renderer.js",
        "  if (state.toastAction && state.toast && now < state.toastUntil && !state.pinned) return 'toast';\n  // a reminder stays put under the cursor so it can be read (hovering pauses it)\n  if (state.reminderUntil > now && !state.pinned && !state.dragging) {\n    return state.toast && now < state.toastUntil ? 'toast' : 'reminder';\n  }\n  if (!state.moving && (state.hover || state.pinned || state.dragging)) return 'expanded';\n",
        "  if (state.toastAction && state.toast && now < state.toastUntil && !state.pinned && !state.vdlHold && !state.calcHold) return 'toast';\n  // a reminder stays put under the cursor so it can be read (hovering pauses it)\n  if (state.reminderUntil > now && !state.pinned && !state.vdlHold && !state.calcHold && !state.dragging) {\n    return state.toast && now < state.toastUntil ? 'toast' : 'reminder';\n  }\n  if (!state.moving && (state.hover || state.pinned || state.vdlHold || state.calcHold || state.dragging)) return 'expanded';\n",
    ),
    (
        "renderer.js",
        "    $('#m-title').textContent = $('#p-title').textContent = m.title || 'Unknown';\n    const sub = m.artist || m.album || appName(m.app);\n    $('#m-artist').textContent = $('#p-artist').textContent = sub;\n",
        "    $('#m-title').textContent = $('#p-title').textContent = m.title || 'Unknown';\n    const sub = m.artist || m.album || appName(m.app);\n    if (!window.__vdlNote) $('#m-artist').textContent = sub;\n    $('#p-artist').textContent = sub;\n",
    ),
    // Tasks (1.8.3): the island's list, finishing one, snoozing a reminder, opening the window
    (
        "preload.js",
        "  onShelf: on('shelf'),\n",
        "  onShelf: on('shelf'),\n  onTasks: on('tasks'),\n  onTaskDue: on('task-due'),\n  tasksList: () => ipcRenderer.invoke('tasks-list'),\n  taskDone: (id, done) => ipcRenderer.send('tasks-done', id, done),\n  taskSnooze: (id, minutes) => ipcRenderer.send('tasks-snooze', id, minutes),\n  tasksOpen: () => ipcRenderer.send('tasks-open'),\n",
    ),
    // Clipboard (1.8.3): a clip's whole text, and pinning one (views/system.js in the overlay)
    (
        "preload.js",
        "    clearClips: () => ipcRenderer.send('clip-clear'),\n",
        "    clearClips: () => ipcRenderer.send('clip-clear'),\n    clipFull: (i) => ipcRenderer.invoke('clip-full', i),\n    pinClip: (i, on) => ipcRenderer.send('clip-pin', i, on),\n",
    ),
    // (1.8.4) the early part of the Arabic layer, in <head>: right to left and hidden until the
    // Arabic is in place, so English doesn't flash first (pages/prefs/i18n-early.js)
    ("prefs/index.html", "  <link rel=\"stylesheet\" href=\"prefs-2.css\">
", "  <link rel=\"stylesheet\" href=\"prefs-2.css\">
  <script src=\"i18n-early.js\"></script>
"),
    ("prefs/index.html", "<small>Kept in memory only</small>", "<small>Kept in memory only; clips you pin are saved on this PC</small>"),
    // Desktop widgets (1.8): Settings › Widgets, after Island, the panel for the desktop's widgets
    // (pages/prefs/widgets.js fills it; the widgets themselves are native, nova-render)
    (
        "prefs/index.html",
        "      <a href=\"#themes-page\" data-page=\"themes-page\">",
        "      <a href=\"#widgets\" data-page=\"widgets\"><svg viewBox=\"0 0 24 24\"><rect x=\"3\" y=\"3\" width=\"18\" height=\"8\" rx=\"2.5\"/><rect x=\"3\" y=\"13\" width=\"8\" height=\"8\" rx=\"2.5\"/><rect x=\"13\" y=\"13\" width=\"8\" height=\"8\" rx=\"2.5\"/></svg>Widgets</a>\n      <a href=\"#themes-page\" data-page=\"themes-page\">",
    ),
    (
        "prefs/index.html",
        "      <section id=\"themes-page\" class=\"page\">\n",
        "      <section id=\"widgets\" class=\"page\">\n        <h1>Widgets</h1>\n        <div class=\"group\">\n          <div class=\"card\">\n            <label class=\"row\"><div class=\"txt\"><b>Show widgets on the desktop</b><small>Click a widget below to put it on your desktop. Drag it there to place it; right-click it for more</small></div><input type=\"checkbox\" class=\"sw\" id=\"wg-on\"></label>\n          </div>\n        </div>\n        <div class=\"group\">\n          <h2>On your desktop</h2>\n          <div class=\"card\" id=\"wg-now\"></div>\n        </div>\n        <div class=\"group\">\n          <h2>Add a widget</h2>\n          <div class=\"wg-gallery\" id=\"wg-gallery\"></div>\n        </div>\n        <div class=\"group\">\n          <h2>Add a ready group</h2>\n          <div class=\"wg-gallery\" id=\"wg-groups\"></div>\n        </div>\n        <div class=\"group\">\n          <div class=\"card\">\n            <div class=\"row\"><div class=\"txt\"><b>Put them back in place</b><small>Every widget to the first clear place from the top-right corner of the screen</small></div><button class=\"btn\" id=\"wg-reset\">Reset places</button></div>\n          </div>\n        </div>\n      </section>\n      <section id=\"themes-page\" class=\"page\">\n",
    ),
    ("prefs/index.html", "  <script src=\"prefs.js\"></script>\n", "  <script src=\"prefs.js\"></script>\n  <script src=\"widgets.js\"></script>\n"),
    (
        "prefs/preload.js",
        "  onGeneral: on('general'),\n",
        "  onGeneral: on('general'),\n  widgets: () => ipcRenderer.invoke('widgets-get'),\n  doWidgets: (action) => ipcRenderer.invoke('widgets-do', action),\n  onWidgets: on('widgets'),\n",
    ),
    // The island's timer (1.8): any length (− and + beside Start) and a 30-second alarm at the
    // end, until the island is touched (pages/views/timer-plus.js); each end calls it
    ("index.html", "  <script src=\"views/ready.js\"></script>\n", "  <script src=\"views/timer-plus.js\"></script>\n  <script src=\"views/ready.js\"></script>\n"),
    (
        "renderer.js",
        "    showToast('done', 'Timer Done', '', 0, 4000);\n    island.classList.add('alert');\n    setTimeout(() => island.classList.remove('alert'), 1000);\n    beep();\n",
        "    showToast('done', 'Timer Done', '', 0, window.timerAlarmMs || 4000);\n    if (window.timerAlarm) window.timerAlarm(beep); else beep();\n",
    ),
    (
        "renderer.js",
        "      showToast('done', 'Break over', 'Focus', 0, 9000);\n      state.toastAction = () => startPhase('focus');\n      beep();\n",
        "      showToast('done', 'Break over', 'Focus', 0, window.timerAlarmMs || 9000);\n      state.toastAction = () => startPhase('focus');\n      if (window.timerAlarm) window.timerAlarm(beep); else beep();\n",
    ),
    (
        "renderer.js",
        "      playTones([[0, 784], [0.2, 988]], { peak: 0.2, decay: 0.6 });\n",
        "      const done = () => playTones([[0, 784], [0.2, 988]], { peak: 0.2, decay: 0.6 });\n      if (window.timerAlarm) window.timerAlarm(done); else done();\n",
    ),
    // Updates that can't be missed (1.8.6): Settings' bar at the top of every page with "Update
    // now" (pages/prefs/update-bar.js), and the island's way to hear it (pages/views/update.js)
    ("prefs/index.html", "  <script src=\"widgets.js\"></script>\n", "  <script src=\"widgets.js\"></script>\n  <script src=\"update-bar.js\"></script>\n  <script src=\"update-check.js\"></script>\n"),
    // Settings › General › Check for updates (1.8.6), under Auto-update (pages/prefs/update-check.js)
    (
        "prefs/index.html",
        "            <label class=\"row\" id=\"update-row\"><b>Auto-update</b><input type=\"checkbox\" class=\"sw\" data-feature=\"autoUpdate\"></label>\n",
        "            <label class=\"row\" id=\"update-row\"><b>Auto-update</b><input type=\"checkbox\" class=\"sw\" data-feature=\"autoUpdate\"></label>\n            <div class=\"row\" id=\"update-check-row\"><div class=\"txt\"><b>Check for updates</b><small id=\"upd-status\"><span class=\"upd-msg\">Version</span> <span class=\"upd-ver\" data-no-i18n></span></small></div><button class=\"btn\" id=\"upd-check\">Check now</button></div>\n            <div class=\"row\" id=\"update-check-fork-row\"><div class=\"txt\"><b>Check fork updates</b><small id=\"upd-fork-status\"><span class=\"upd-msg\">sem233/nova-island</span> <span class=\"upd-ver\" data-no-i18n></span></small></div><button class=\"btn\" id=\"upd-check-fork\">Check fork</button></div>\n",
    ),
    (
        "prefs/preload.js",
        "  onWidgets: on('widgets'),\n",
        "  onWidgets: on('widgets'),\n  updateState: () => ipcRenderer.invoke('update-state'),\n  updateNow: () => ipcRenderer.send('update-now'),\n  onUpdate: on('update'),\n  checkUpdates: (src) => ipcRenderer.send('update-check', src),\n  onUpdateCheck: on('update-check'),\n",
    ),
    ("preload.js", "  onUpdate: on('update'),\n", "  onUpdate: on('update'),\n  onUpdateNow: on('update-now'),\n"),
    // Copy the reminder (1.8.6): a button beside the card's close one (pages/views/reminders-copy.js)
    (
        "index.html",
        "          <button class=\"rm-x\" id=\"rm-close\" aria-label=\"Close\" title=\"Close\"><svg class=\"s\" viewBox=\"0 0 24 24\"><path d=\"M18 6 6 18M6 6l12 12\"/></svg></button>\n",
        "          <span class=\"rm-tools\">\n            <button class=\"rm-x rm-copy\" id=\"rm-copy\" aria-label=\"Copy\" title=\"Copy\"><svg class=\"s\" viewBox=\"0 0 24 24\"><rect x=\"8\" y=\"8\" width=\"12\" height=\"12\" rx=\"2.5\"/><path d=\"M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2\"/></svg></button>\n            <button class=\"rm-x\" id=\"rm-close\" aria-label=\"Close\" title=\"Close\"><svg class=\"s\" viewBox=\"0 0 24 24\"><path d=\"M18 6 6 18M6 6l12 12\"/></svg></button>\n          </span>\n",
    ),
    ("index.html", "  <script src=\"views/timer-plus.js\"></script>\n", "  <script src=\"views/timer-plus.js\"></script>\n  <script src=\"views/reminders-copy.js\"></script>\n"),
    ("preload.js", "  onUpdateNow: on('update-now'),\n", "  onUpdateNow: on('update-now'),\n  copyText: (t) => ipcRenderer.send('copy-text', t),\n"),
    // Settings in Arabic (1.8.3): the language at the top of General, and the layer that puts the
    // Arabic over the page (pages/prefs/prefs-i18n.js), after prefs.js has filled it
    (
        "prefs/index.html",
        "            <label class=\"row\"><b>Show island</b>",
        "            <div class=\"row\"><b>Language</b>\n              <div class=\"seg\" role=\"radiogroup\" aria-label=\"Language\" data-no-i18n>\n                <button role=\"radio\" data-lang=\"en\" lang=\"en\">English</button>\n                <button role=\"radio\" data-lang=\"ar\" lang=\"ar\">العربية</button>\n              </div>\n            </div>\n            <label class=\"row\"><b>Show island</b>",
    ),
    ("prefs/index.html", "  <script src=\"prefs.js\"></script>\n", "  <script src=\"prefs.js\"></script>\n  <script src=\"prefs-i18n.js\"></script>\n"),
    // Water, spaced as set (1.8.1): a reminder closed or left to time out came back 15 minutes
    // later, so an ignored one showed four times an hour. Now the next one always waits the full
    // interval from Settings, and none come once the day's goal is reached.
    (
        "views/water.js",
        "      onClose: () => { activeMs = Math.max(0, everyMs() - SNOOZE_MS); },\n",
        "      onClose: () => { activeMs = 0; },\n",
    ),
    (
        "views/water.js",
        "    if (activeMs < everyMs()) return;\n",
        "    if (activeMs < everyMs()) return;\n    if (load() >= goal()) { activeMs = 0; return; } // the day's goal is reached: no more today\n",
    ),
    // A scene's card with no scene (1.8.1): the card shows one frame after the scene starts, and
    // when the page wasn't drawing (the island hidden or paused) the scene could end first; the
    // late frame then showed the card for good, its bar full, over whatever came next (a dua).
    // Now the frame shows it only for its own scene, and the timer takes away any card left over.
    (
        "views/scene.js",
        "    requestAnimationFrame(() => card.classList.add('on'));\n",
        "    requestAnimationFrame(() => { if (current === s) card.classList.add('on'); });\n",
    ),
    (
        "views/scene.js",
        "    lastTick = now;\n    if (!current) return;\n",
        "    lastTick = now;\n    if (!current) {\n      if (card.classList.contains('on')) card.classList.remove('on');\n      return;\n    }\n",
    ),
    ("views/scene.js", "    if (state.hover) state.sceneUntil += dt;\n", "    if (state.hover && window.mouseMovedSince(shownAt)) state.sceneUntil += dt;\n"),
    ("views/reminders.js", "    state.reminderUntil = Date.now() + total;\n", "    state.reminderUntil = Date.now() + total;\n    shownAt = Date.now();\n"),
    ("views/reminders.js", "  let lastTick = Date.now();\n", "  let lastTick = Date.now();\n  let shownAt = 0;\n"),
    (
        "views/reminders.js",
        "      const reading = state.hover && lastMode === 'reminder';\n",
        "      const reading = state.hover && lastMode === 'reminder' && (window.mouseMovedSince?.(shownAt) ?? true);\n",
    ),
    // Time format (1.8): 12 or 24 hours, in General under the orientation. pages/time-format.js
    // makes every time the island and Settings write follow it; the widgets read it too.
    ("index.html", "  <script src=\"renderer.js\"></script>\n", "  <script src=\"time-format.js\"></script>\n  <script src=\"renderer.js\"></script>\n"),
    (
        "prefs/index.html",
        "            <div class=\"row\"><div class=\"txt\"><b>In the browser</b>",
        "            <div class=\"row\"><b>Time format</b>\n              <div class=\"seg\" role=\"radiogroup\" aria-label=\"Time format\">\n                <button role=\"radio\" data-clock=\"12\">12-hour</button>\n                <button role=\"radio\" data-clock=\"24\">24-hour</button>\n              </div>\n            </div>\n            <div class=\"row\"><div class=\"txt\"><b>In the browser</b>",
    ),
    ("prefs/index.html", "  <script src=\"prefs.js\"></script>\n  <script src=\"prefs-i18n.js\"></script>\n", "  <script src=\"../time-format.js\"></script>\n  <script src=\"prefs.js\"></script>\n  <script src=\"prefs-i18n.js\"></script>\n"),
    // Nova's eyes: after the cursor has stayed right by the island 2 s (1.7: 3 s). 1.8.6 waited
    // 10 s, but its reach was a wide band over the browser's tabs, so they still came up with
    // nobody near; 1.8.7 measures from the island's edge (core.rs NEAR_EDGE_PX). The light
    // island's own rule said the same.
    ("views/nova.js", "  const DWELL_MS = 3000;\n", "  const DWELL_MS = 2000;\n"),
    // ...and when each of the page's own timers is next due, so the sleeping page wakes for it
    ("views/reminders.js", "  setInterval(tick, 250);\n", "  setInterval(tick, 250);\n  window.reminderNextAt = () => (feat('reminders') ? nextAt : 0);\n"),
    ("views/water.js", "  const everyMs = () => (state.features.waterEvery || 60) * 60e3;\n", "  const everyMs = () => (state.features.waterEvery || 60) * 60e3;\n  window.waterDueIn = () => (feat('water') ? everyMs() - activeMs : Infinity);\n"),
    ("views/companion.js", "  setInterval(flush, 1000);\n", "  setInterval(flush, 1000);\n  window.careDueAt = () => (feat('wellbeing') ? activeSince + WORK_MS : Infinity);\n"),
    (
        "views/calendar.js",
        "  bridge.onCalendar(update);\n",
        "  window.meetingNextNotice = () => {\n    if (!feat('meetings')) return Infinity;\n    const now = Date.now();\n    let at = Infinity;\n    for (const ev of events) for (const t of [...NOTICE_MIN.map((m) => ev.start - m * 60000), ev.start, ev.start - 3 * 3600e3, ev.end]) if (t > now - 1000) at = Math.min(at, t);\n    return at;\n  };\n  window.meetingSlide = () => {\n    const now = Date.now();\n    const next = feat('meetings') ? events.find((e) => e.end > now) : null;\n    return next ? { title: next.title, start: next.start, end: next.end } : null;\n  };\n  bridge.onCalendar(update);\n",
    ),
    // (1.8.7) The prayer tab picks "today" by the place's date, not the PC's: with the city in
    // another time zone (Tokyo, the PC in Cairo at 22:00) the times for the 13th and 14th came,
    // the tab looked for the 12th and stayed on "loading". "Today / tomorrow" and the next row
    // go by the place's date too.
    (
        "views/prayer.js",
        "  const ymd = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;\n",
        r#"  const ymd = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  // the date where the place is (the PC may be in another time zone)
  const placeYmd = (d, tz) => {
    if (!tz) return ymd(d);
    try {
      const p = Object.fromEntries(new Intl.DateTimeFormat('en-US', { timeZone: tz, year: 'numeric', month: 'numeric', day: 'numeric' }).formatToParts(d).map((x) => [x.type, Number(x.value)]));
      return `${p.year}-${pad(p.month)}-${pad(p.day)}`;
    } catch {
      return ymd(d);
    }
  };
"#,
    ),
    ("views/prayer.js", "    const today = ymd(new Date());\n", "    const today = placeYmd(new Date(), data.today?.tz || data.tomorrow?.tz || '');\n"),
    ("views/prayer.js", "next.time.toDateString() === t.toDateString()", "placeYmd(next.time, zone) === placeYmd(t, zone)"),
    ("views/prayer.js", "next.time.toDateString() === now.toDateString()", "placeYmd(next.time, zone) === placeYmd(now, zone)"),
    // (1.8.7) Madar: graphite and mint, a solid theme (its island details are in style-2.css)
    (
        "themes.css",
        "  --th-prev: #0b0f0c;\n}\n",
        "  --th-prev: #0b0f0c;\n}\n[data-theme=\"madar\"] {\n  --th-bg: linear-gradient(180deg, #1d2026, #16181d);\n  --th-deco: radial-gradient(70% 120% at 85% -40%, rgba(147, 228, 200, .07), transparent 70%);\n  --th-line: rgba(255, 255, 255, .13); --th-line-w: 1px;\n  --th-text: #f2f4f6; --th-soft: rgba(214, 220, 226, .66); --th-hi: #93e4c8;\n  --th-prev: #15171b;\n}\n",
    ),
    ("prefs/prefs.js", "  ['retro', 'Retro', 'Pixel green'],\n", "  ['retro', 'Retro', 'Pixel green'],\n  ['madar', 'Madar', 'Graphite and mint'],\n"),
    // (1.8.7) Settings › In the browser: a third choice, a thin line over the tabs
    ("prefs/index.html", "                <button role=\"radio\" data-browser=\"stay\">Stay in place</button>\n", "                <button role=\"radio\" data-browser=\"stay\">Stay in place</button>\n                <button role=\"radio\" data-browser=\"line\">Thin line</button>\n"),
    ("prefs/prefs.js", "String((b.dataset.browser === 'addr') === !!f.addrIsland)", "String(b.dataset.browser === (f.addrIsland ? 'addr' : f.browserLine ? 'line' : 'stay'))"),
    ("prefs/prefs.js", "  $('#browser-hint').textContent = f.addrIsland\n    ? 'Moves into the address bar, from its default place at the top'\n    : 'Stays where it is; clicks go to the tabs below';\n", "  $('#browser-hint').textContent = f.addrIsland\n    ? 'Moves into the address bar, from its default place at the top'\n    : f.browserLine\n      ? 'A thin line over the tabs; point at it to open the island'\n      : 'Stays where it is; clicks go to the tabs below';\n"),
    ("prefs/prefs.js", "  api.setFeatures({ addrIsland: b.dataset.browser === 'addr' });\n", "  api.setFeatures({ addrIsland: b.dataset.browser === 'addr', browserLine: b.dataset.browser === 'line' });\n"),
    // ...and the line a little longer and thicker than the address bar's was, so it is seen
    ("renderer.js", "[w, h] = anchor.addr === 'line' ? [90, 3] : [anchor.sw, anchor.sh];", "[w, h] = anchor.addr === 'line' ? [120, 4] : [anchor.sw, anchor.sh];"),
    // (1.8.7) Settings › General: the Nova strip on the taskbar (nova-app/src/taskbar.rs). Off for
    // now and marked "Coming soon" (nova-core features.rs keeps taskbarStrip off)
    ("prefs/index.html", "            <label class=\"row\"><b>Hide in full screen</b>", "            <label class=\"row soon\"><div class=\"txt\"><b>Show on the taskbar</b><small>Coming soon</small></div><input type=\"checkbox\" class=\"sw\" disabled></label>\n            <label class=\"row\"><b>Hide in full screen</b>"),
    // (1.8.7) the taskbar strip on: a verse / dhikr / du'a card shows over the strip instead of
    // the island (companion cards, with their own HTML, stay the island's)
    ("preload.js", "  copyText: (t) => ipcRenderer.send('copy-text', t),\n", "  copyText: (t) => ipcRenderer.send('copy-text', t),\n  stripCard: (c) => ipcRenderer.send('strip-card', c),\n"),
    ("views/reminders.js", "  function show(item) {\n    layer.className", "  function show(item) {\n    if (item.html == null && feat('taskbarStrip') && bridge.stripCard) {\n      bridge.stripCard({ name: item.badge || KIND_NAME[item.kind] || '', text: item.text, ref: item.ref || '', quran: isQuran(item), ms: item.ms || showMs() });\n      schedule();\n      return;\n    }\n    layer.className"),
    // (1.8.7) simpler Settings: the companion page goes (its features stay on, see nova-core
    // features.rs FIXED_ON); the water reminder moves to Alerts & Privacy, before Privacy
    ("prefs/index.html", r##"      <a href="#companion" data-page="companion"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2M9 9h.01M15 9h.01"/></svg>Companion</a>
"##, ""),
    ("prefs/index.html", r##"      <!-- COMPANION -->
      <section id="companion" class="page">
        <h1>Companion</h1>
        <div class="group">
          <h2>Nova</h2>
          <div class="card">
            <label class="row"><b>Eyes</b><input type="checkbox" class="sw" data-feature="novaEyes"></label>
            <label class="row"><b>At the adhan</b><input type="checkbox" class="sw" data-feature="novaPrayer"></label>
            <label class="row"><b>Focus sessions</b><input type="checkbox" class="sw" data-feature="novaFocus"></label>
            <label class="row"><b>Celebrations</b><input type="checkbox" class="sw" data-feature="celebrate"></label>
          </div>
        </div>
        <div class="group">
          <h2>Moments</h2>
          <div class="card">
            <label class="row"><b>Morning brief</b><input type="checkbox" class="sw" data-feature="morningBrief"></label>
            <label class="row"><b>Welcome back</b><input type="checkbox" class="sw" data-feature="welcomeBack"></label>
            <label class="row"><b>Break reminders</b><input type="checkbox" class="sw" data-feature="wellbeing"></label>
            <label class="row"><b>Network</b><input type="checkbox" class="sw" data-feature="netToasts"></label>
            <label class="row"><div class="txt"><b>Smart clipboard</b><small>Links, colors, math</small></div><input type="checkbox" class="sw" data-feature="smartClipboard"></label>
          </div>
        </div>
        <div class="group">
          <h2>Water</h2>
          <div class="card">
            <label class="row"><b>Water reminder</b><input type="checkbox" class="sw" data-feature="water"></label>
            <div class="row sub" data-needs="water"><b>Every</b>
              <div class="stepper" data-num="waterEvery" data-steps="15,30,45,60,90,120,180,240" data-unit="min"><button data-dir="-1" aria-label="Less">−</button><output></output><button data-dir="1" aria-label="More">+</button></div>
            </div>
            <div class="row sub" data-needs="water"><b>Daily goal</b>
              <div class="stepper" data-num="waterGoal" data-steps="4,5,6,7,8,9,10,11,12,14,16" data-unit="glass"><button data-dir="-1" aria-label="Less">−</button><output></output><button data-dir="1" aria-label="More">+</button></div>
            </div>
          </div>
        </div>
      </section>

"##, ""),
    ("prefs/index.html", r##"        <div class="group">
          <h2>Privacy</h2>"##, r##"        <div class="group">
          <h2>Water</h2>
          <div class="card">
            <label class="row"><b>Water reminder</b><input type="checkbox" class="sw" data-feature="water"></label>
            <div class="row sub" data-needs="water"><b>Every</b>
              <div class="stepper" data-num="waterEvery" data-steps="15,30,45,60,90,120,180,240" data-unit="min"><button data-dir="-1" aria-label="Less">−</button><output></output><button data-dir="1" aria-label="More">+</button></div>
            </div>
            <div class="row sub" data-needs="water"><b>Daily goal</b>
              <div class="stepper" data-num="waterGoal" data-steps="4,5,6,7,8,9,10,11,12,14,16" data-unit="glass"><button data-dir="-1" aria-label="Less">−</button><output></output><button data-dir="1" aria-label="More">+</button></div>
            </div>
          </div>
        </div>
        <div class="group">
          <h2>Privacy</h2>"##),
    // (1.8.7) In the browser: Address bar is off the list (its code stays for now); thin line or stay
    ("prefs/index.html", r##"                <button role="radio" data-browser="addr">Address bar</button>
"##, ""),
    // (1.8.7) internet use: live down / up and the ping (nova-services netspeed.rs, pages/views/netspeed.js)
    ("index.html", r##"<b id="net-val">…</b></div>
"##, r##"<b id="net-val">…</b></div>
                <div class="net-live"><span id="net-live"></span><span id="net-ping"></span></div>
"##),
    ("index.html", r##"        <div class="info-slide mt-slide" data-feat="meetings" hidden>
"##, r##"        <div class="info-slide net-slide" data-feat="slideNet">
          <div class="side"><svg class="s i-ico" viewBox="0 0 24 24"><path d="M12 4v15M6 13l6 6 6-6"/></svg><b id="i-rx" class="i-strong">0 KB/s</b></div>
          <div class="side"><svg class="s i-ico" viewBox="0 0 24 24"><path d="M12 20V5M6 11l6-6 6 6"/></svg><b id="i-tx" class="i-strong">0 KB/s</b><span id="i-ping" class="i-soft"></span></div>
        </div>
        <div class="info-slide mt-slide" data-feat="meetings" hidden>
"##),
    ("index.html", r##"  <script src="views/network.js"></script>
"##, r##"  <script src="views/network.js"></script>
  <script src="views/netspeed.js"></script>
"##),
    ("prefs/index.html", r##"            <label class="row"><b>Next prayer</b><input type="checkbox" class="sw" data-feature="slidePrayer"></label>
"##, r##"            <label class="row"><b>Next prayer</b><input type="checkbox" class="sw" data-feature="slidePrayer"></label>
            <label class="row"><b>Internet speed</b><input type="checkbox" class="sw" data-feature="slideNet"></label>
"##),
    ("prefs/index.html", r##"            <label class="row"><b>Caps Lock</b><input type="checkbox" class="sw" data-feature="capsToast"></label>
"##, r##"            <label class="row"><b>Caps Lock</b><input type="checkbox" class="sw" data-feature="capsToast"></label>
            <label class="row"><div class="txt"><b>Slow internet</b><small>When the ping stays over 200 ms for a minute</small></div><input type="checkbox" class="sw" data-feature="slowNetToast"></label>
"##),
    // (1.8.7) the adhan's sound (nova-app adhan.rs, pages/views/adhan.js, pages/prefs/adhan.js)
    ("prefs/index.html", r##"            <label class="row"><b>Hanafi Asr</b><input type="checkbox" class="sw" data-feature="asrHanafi"></label>
"##, r##"            <label class="row"><b>Hanafi Asr</b><input type="checkbox" class="sw" data-feature="asrHanafi"></label>
            <label class="row"><div class="txt"><b>Adhan sound</b><small>Plays with the prayer alert, which lasts as long as the adhan</small></div><input type="checkbox" class="sw" data-feature="adhanSound"></label>
            <div class="row sub adhan-row" data-adhan="main"><div class="txt"><b>Adhan</b><small class="adhan-name">Nova's adhan</small></div>
              <div class="adhan-btns"><button class="btn adhan-play">Play</button><button class="btn adhan-pick">Choose file</button><button class="btn adhan-reset" hidden>Default</button></div>
            </div>
            <div class="row sub adhan-row" data-adhan="fajr"><div class="txt"><b>Fajr adhan</b><small class="adhan-name">Nova's Fajr adhan</small></div>
              <div class="adhan-btns"><button class="btn adhan-play">Play</button><button class="btn adhan-pick">Choose file</button><button class="btn adhan-reset" hidden>Default</button></div>
            </div>
"##),
    ("prefs/index.html", r##"  <script src="widgets.js"></script>
"##, r##"  <script src="widgets.js"></script>
  <script src="adhan.js"></script>
"##),
    ("prefs/preload.js", r##"  onGeneral: on('general'),
  widgets:"##, r##"  onGeneral: on('general'),
  adhan: () => ipcRenderer.invoke('adhan-get'),
  adhanPick: (which) => ipcRenderer.invoke('adhan-pick', which),
  adhanReset: (which) => ipcRenderer.invoke('adhan-reset', which),
  widgets:"##),
    ("views/prayer.js", r##"          ms: 45000,
        });
"##, r##"          ms: 45000,
          ...(window.adhanScene?.() || {}),
        });
"##),
    ("views/prayer.js", r##"          showToast('prayer', `حان الآن موعد صلاة ${NAMES[k]}`, clock(t), 0, 9000);
          setTimeout(() => island.classList.remove('prayer-alert'), 9200);
"##, r##"          const tms = window.adhanToast?.() || 9000;
          showToast('prayer', `حان الآن موعد صلاة ${NAMES[k]}`, clock(t), 0, tms);
          setTimeout(() => island.classList.remove('prayer-alert'), tms + 200);
"##),
    ("preload.js", r##"  network: () => ipcRenderer.invoke('net-get'),
"##, r##"  network: () => ipcRenderer.invoke('net-get'),
  onAdhanFile: on('adhan-file'),
  calcOpen: (on, blur) => ipcRenderer.send('calc-open', on, blur),
  calcRect: (r) => ipcRenderer.send('calc-rect', r),
  onCalcClose: on('calc-close'),
"##),
    (
        "index.html",
        "            <button class=\"gear bell\" id=\"open-news\" title=\"News from Nova\" aria-label=\"News from Nova\">",
        "            <button class=\"gear calc-btn\" id=\"open-calc\" title=\"Calculator\" aria-label=\"Calculator\" aria-expanded=\"false\" hidden><svg class=\"s\" viewBox=\"0 0 24 24\"><rect x=\"4\" y=\"2.5\" width=\"16\" height=\"19\" rx=\"3\"/><path d=\"M8 7h8\"/><path d=\"M8 12h2M14 12h2M8 16h2M14 16h2\"/></svg></button>\n            <button class=\"gear bell\" id=\"open-news\" title=\"News from Nova\" aria-label=\"News from Nova\">",
    ),
    (
        "index.html",
        "  <script src=\"time-format.js\"></script>\n  <script src=\"renderer.js\"></script>\n",
        "  <section id=\"calc-panel\" class=\"calc-panel\" dir=\"ltr\" lang=\"en\" role=\"dialog\" aria-modal=\"false\" aria-label=\"Calculator\" tabindex=\"-1\" hidden></section>\n  <script src=\"time-format.js\"></script>\n  <script src=\"renderer.js\"></script>\n",
    ),
    (
        "renderer.js",
        "function sizeCanvases() {\n  [bd, off].forEach((c) => {\n    c.width = window.innerWidth;\n    c.height = window.innerHeight;\n  });\n  bd.style.width = `${window.innerWidth}px`;\n  bd.style.height = `${window.innerHeight}px`;\n}",
        "window.__getBackdropCanvas = () => off;\nfunction sizeCanvases() {\n  let prev = null;\n  if (off.width > 0 && off.height > 0 && window.innerWidth === off.width) {\n    prev = document.createElement('canvas');\n    prev.width = off.width;\n    prev.height = off.height;\n    prev.getContext('2d').drawImage(off, 0, 0);\n  }\n  [bd, off].forEach((c) => {\n    c.width = window.innerWidth;\n    c.height = window.innerHeight;\n  });\n  if (prev) {\n    offCtx.drawImage(prev, 0, 0);\n    bdCtx.drawImage(prev, 0, 0);\n  }\n  bd.style.width = `${window.innerWidth}px`;\n  bd.style.height = `${window.innerHeight}px`;\n}",
    ),
    (
        "renderer.js",
        "  fadeFrames = 10;\n  wake(300);\n});",
        "  fadeFrames = 10;\n  wake(300);\n  window.dispatchEvent(new CustomEvent('island:backdrop'));\n});",
    ),
    (
        "prefs/index.html",
        "            <label class=\"row\"><div class=\"txt\"><b>Open apps first</b><small>Apps with a window open come first in the Apps tab</small></div><input type=\"checkbox\" class=\"sw\" data-feature=\"appsRunningFirst\"></label>\n          </div>\n        </div>\n",
        "            <label class=\"row\"><div class=\"txt\"><b>Open apps first</b><small>Apps with a window open come first in the Apps tab</small></div><input type=\"checkbox\" class=\"sw\" data-feature=\"appsRunningFirst\"></label>\n          </div>\n        </div>\n        <div class=\"group\">\n          <h2>Tools</h2>\n          <div class=\"card\">\n            <label class=\"row\"><b>Calculator</b><input type=\"checkbox\" class=\"sw\" data-feature=\"calculator\"></label>\n          </div>\n        </div>\n",
    ),
    ("index.html", r##"  <script src="views/netspeed.js"></script>
"##, r##"  <script src="views/netspeed.js"></script>
  <script src="views/adhan.js"></script>
"##),
    ("index.html", "https://qurango.net https://*.qurango.net https://cdn.mp3quran.net", "https://qurango.net https://*.qurango.net https://cdn.mp3quran.net 'self'"),
];

/// The file's bytes with this file's PATCHES made (each must match exactly once).
fn patched(rel: &str, bytes: Vec<u8>) -> Vec<u8> {
    let mine: Vec<_> = PATCHES.iter().filter(|(f, _, _)| *f == rel).collect();
    if mine.is_empty() {
        return bytes;
    }
    let mut text = String::from_utf8(bytes).unwrap().replace("\r\n", "\n");
    for (_, from, to) in mine {
        let n = text.matches(from).count();
        assert!(n == 1, "patch for {rel}: expected one match of {from:?}, found {n} (the 1.7 file changed)");
        text = text.replace(from, to);
    }
    text.into_bytes()
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            out.push(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
        }
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "ttf" => "font/ttf",
        "png" => "image/png",
        "mp3" => "audio/mpeg",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir.join("../../..").canonicalize().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut files: Vec<String> = Vec::new();
    for (dir, exts) in SERVED {
        let folder = root.join(dir);
        let mut names: Vec<String> = fs::read_dir(&folder).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        for name in names {
            let rel = if dir.is_empty() { name.clone() } else { format!("{dir}/{name}") };
            let ext = name.rsplit('.').next().unwrap_or("");
            if exts.contains(&ext) && !SKIP.contains(&rel.as_str()) {
                files.push(rel);
            }
        }
    }
    files.extend(EXTRA.iter().map(|s| s.to_string()));
    // the top-level html/css/js are the island page only; drop anything else that is not a page file
    files.retain(|f| f.contains('/') || matches!(f.as_str(), "index.html" | "style.css" | "themes.css" | "renderer.js") || EXTRA.contains(&f.as_str()));

    // 1.8's own page files: added, or in place of the repo's
    let overlay = root.join(OVERLAY);
    let mut own: Vec<String> = Vec::new();
    if overlay.exists() {
        walk(&overlay, &overlay, &mut own);
        own.sort();
        println!("cargo:rerun-if-changed={}", overlay.to_string_lossy());
    }
    for rel in &own {
        if !OVERLAY_PRELOADS.contains(&rel.as_str()) && !files.contains(rel) {
            files.push(rel.clone());
        }
    }

    // where a file's bytes come from: the overlay, a patched copy, or the repo
    let abs = |rel: &str| -> String {
        let src = if own.iter().any(|o| o == rel) { overlay.join(rel) } else { root.join(rel) };
        println!("cargo:rerun-if-changed={}", src.to_string_lossy());
        if PATCHES.iter().any(|(f, _, _)| *f == rel) {
            let copy = out_dir.join("patched").join(rel);
            fs::create_dir_all(copy.parent().unwrap()).unwrap();
            fs::write(&copy, patched(rel, fs::read(&src).unwrap())).unwrap();
            return copy.to_string_lossy().replace('\\', "/");
        }
        src.to_string_lossy().replace('\\', "/")
    };

    let mut code = String::from("pub static FILES: &[(&str, &[u8], &str)] = &[\n");
    let mut manifest = String::new();
    for rel in &files {
        let path = abs(rel);
        let bytes = fs::read(Path::new(&path)).unwrap();
        writeln!(manifest, "{:x}  {rel}", Sha256::digest(&bytes)).unwrap();
        writeln!(code, "    ({rel:?}, include_bytes!({path:?}), {:?}),", mime(rel)).unwrap();
    }
    code.push_str("];\n");
    for (name, rel) in [("PRELOAD_NOVA", "preload.js"), ("PRELOAD_PREFS", "prefs/preload.js"), ("PRELOAD_MINI", "mini/preload.js"), ("PRELOAD_RADIO", "radio/preload.js"), ("PRELOAD_TASKS", "tasks/preload.js")] {
        let path = abs(rel);
        let bytes = fs::read(Path::new(&path)).unwrap();
        writeln!(manifest, "{:x}  {rel}", Sha256::digest(&bytes)).unwrap();
        writeln!(code, "pub static {name}: &str = include_str!({path:?});").unwrap();
    }
    writeln!(code, "pub static MANIFEST: &str = {manifest:?};").unwrap();
    fs::write(out_dir.join("assets.rs"), code).unwrap();
}
