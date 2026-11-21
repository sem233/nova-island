// Tasks on the island (1.8.3, the built-in Tasks plugin): the Tasks tab lists the next open tasks
// (late and today first, then by time and priority), each ticked off with one click; Open shows the
// Tasks window. A reminder comes as the big card, in its tag's colour, with Done, Snooze and Open.
(() => {
  const listEl = $('#tk-list');
  const countEl = $('#tk-count');
  const badge = $('#tasks-badge');
  const TAG_ORDER = { urgent: 0, high: 1, medium: 2, low: 3 };
  const TAG_NAME = { low: 'Low', medium: 'Medium', high: 'High', urgent: 'Urgent' };
  const SHOW = 6;
  let tasks = [];
  let on = false;

  const DAY = 86400000;
  const startOfDay = () => { const d = new Date(); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); };
  const clock = (ms) => new Date(ms).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  function when(ms) {
    const day = new Date(ms);
    const d0 = new Date(day.getFullYear(), day.getMonth(), day.getDate()).getTime();
    const today = startOfDay();
    if (d0 === today) return clock(ms);
    if (d0 === today + DAY) return `Tomorrow ${clock(ms)}`;
    if (d0 === today - DAY) return `Yesterday ${clock(ms)}`;
    return day.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
  }
  const late = (x) => !x.done && x.due && x.due < Date.now();
  const order = (a, b) => (a.due ?? Infinity) - (b.due ?? Infinity) || TAG_ORDER[a.tag] - TAG_ORDER[b.tag] || b.created - a.created;

  function render() {
    const open = tasks.filter((x) => !x.done).sort(order);
    const lateN = open.filter(late).length;
    const todayN = open.filter((x) => x.due != null && x.due < startOfDay() + DAY).length;
    countEl.textContent = !open.length ? '' : lateN ? `${lateN} late · ${open.length} open` : todayN ? `${todayN} today · ${open.length} open` : `${open.length} open`;
    badge.classList.toggle('on', lateN > 0);
    listEl.innerHTML = '';
    if (!open.length) {
      listEl.innerHTML = '<div class="tk-empty"><b>All clear</b><span>Add tasks in the Tasks window</span></div>';
      return;
    }
    for (const x of open.slice(0, SHOW)) {
      const row = document.createElement('div');
      row.className = `tk-item tg-${x.tag}${late(x) ? ' late' : ''}`;
      row.innerHTML = '<button class="tk-check" aria-label="Done"><svg class="s" viewBox="0 0 24 24"><path d="m6 12.5 4 4 8-9"/></svg></button>' +
        `<span class="tk-title" dir="auto"><span class="tk-txt"></span></span>${x.due ? `<span class="tk-when">${when(x.due)}</span>` : ''}<span class="tk-tag">${TAG_NAME[x.tag]}</span>`;
      row.querySelector('.tk-txt').textContent = x.title;
      row.querySelector('.tk-check').addEventListener('click', (e) => {
        e.stopPropagation();
        if (row.classList.contains('finishing')) return;
        // (1.8.4) struck through, then the row slides away and closes up; saved once it's gone
        row.classList.add('finishing');
        setTimeout(() => bridge.taskDone(x.id, true), 720);
      });
      row.addEventListener('click', () => bridge.tasksOpen());
      listEl.append(row);
    }
    if (open.length > SHOW) {
      const more = document.createElement('button');
      more.className = 'tk-more';
      more.textContent = `+${open.length - SHOW} more`;
      more.addEventListener('click', () => bridge.tasksOpen());
      listEl.append(more);
    }
  }
  $('#tk-open').addEventListener('click', () => bridge.tasksOpen());

  // ---- reminders: the big card ----
  const queue = [];
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  let showing = null;
  let nextTimer = null;
  function scheduleNext(ms = 2000) {
    clearTimeout(nextTimer);
    nextTimer = null;
    if (!on || showing || !queue.length) return;
    nextTimer = setTimeout(() => {
      nextTimer = null;
      showNext();
    }, ms);
  }
  function showNext() {
    if (showing || !queue.length) return;
    if (!state.visible || window.sceneShowing?.() || window.cardShowing?.()) {
      scheduleNext(2000);
      return;
    }
    const x = queue.shift();
    if (!tasks.some((y) => y.id === x.id && !y.done)) return showNext();
    showing = x.id;
    const due = x.due ? (late(x) && Date.now() - x.due > 60000 ? `Was due ${when(x.due)}` : `Due ${when(x.due)}`) : '';
    const html =
      `<div class="tk-card"><div class="tk-card-title" dir="auto">${esc(x.title)}</div>` +
      `<div class="tk-card-meta">${due ? `<span>${esc(due)}</span>` : ''}<span class="tk-card-tag">${TAG_NAME[x.tag]}</span>` +
      `${x.notes ? `<span class="tk-card-note" dir="auto">${esc(x.notes.split('\n')[0].slice(0, 90))}</span>` : ''}</div>` +
      '<div class="tk-card-acts"><button data-tk="done" class="tk-primary">Done ✓</button><button data-tk="snooze">Snooze 10 min</button><button data-tk="open">Open</button></div></div>';
    window.showCard?.({ kind: `task tk-${x.tag}`, badge: '✓ Task', html, ms: 30000 });
  }
  document.addEventListener('click', (e) => {
    const b = e.target.closest('[data-tk]');
    if (!b || !showing) return;
    e.stopPropagation();
    const id = showing;
    if (b.dataset.tk === 'done') bridge.taskDone(id, true);
    if (b.dataset.tk === 'snooze') bridge.taskSnooze(id, 10);
    if (b.dataset.tk === 'open') bridge.tasksOpen();
    $('#rm-close')?.click();
  });
  // the card left (closed, timed out or acted on): the next waiting reminder
  window.addEventListener('island:mode', (e) => {
    if (e.detail.mode === 'expanded' && on) render();
    if (showing && e.detail.prev === 'reminder' && e.detail.mode !== 'reminder') {
      showing = null;
      scheduleNext(800);
    } else if (!showing && queue.length) {
      scheduleNext(400);
    }
  });
  bridge.onTaskDue((x) => {
    if (!on || !x?.id || queue.some((q) => q.id === x.id) || showing === x.id) return;
    queue.push(x);
    showNext();
  });

  // ---- the plugin: on, off, removed ----
  function apply(list) {
    const now = (list || []).some((p) => p.kind === 'tasks');
    on = now;
    if (state.tasksTab !== now) {
      state.tasksTab = now;
      refreshTabs();
    }
    if (!now) {
      queue.length = 0;
      clearTimeout(nextTimer);
      nextTimer = null;
    }
  }
  bridge.onTasks((l) => { tasks = Array.isArray(l) ? l : []; render(); });
  bridge.tasksList?.().then((l) => { tasks = Array.isArray(l) ? l : []; render(); });
  bridge.onPlugins(apply);
  bridge.plugins().then(apply);
  setInterval(() => {
    if (on && (lastMode === 'expanded' || tasks.length > 0)) render();
  }, 60000);
})();
