// Tasks view: to-do manager enabled by the Tasks plugin (.nvp).
(() => {
  const form = $('#task-add-form');
  const input = $('#task-input');
  const prioSelect = $('#task-priority');
  const listEl = $('#tasks-list');
  const emptyEl = $('#tasks-empty');
  const clearBtn = $('#task-clear-btn');
  const badge = $('#tasks-badge');

  let tasksData = [];

  function updateBadge() {
    if (!badge) return;
    const pendingCount = tasksData.filter((t) => !t.done).length;
    badge.textContent = pendingCount > 0 ? pendingCount : '';
    badge.hidden = pendingCount === 0;
  }

  function render(items) {
    tasksData = items || [];
    updateBadge();

    if (!listEl) return;
    listEl.innerHTML = '';

    if (!tasksData.length) {
      if (emptyEl) emptyEl.hidden = false;
      return;
    }
    if (emptyEl) emptyEl.hidden = true;

    tasksData.forEach((t) => {
      const item = document.createElement('div');
      item.className = `task-item${t.done ? ' done' : ''}`;
      item.dataset.id = t.id;

      // Checkbox
      const checkBtn = document.createElement('button');
      checkBtn.type = 'button';
      checkBtn.className = 'task-checkbox';
      checkBtn.title = t.done ? 'Mark pending' : 'Mark done';
      checkBtn.innerHTML = '<svg class="s" viewBox="0 0 24 24"><path d="m5 12 5 5L20 7"/></svg>';
      checkBtn.addEventListener('click', async (e) => {
        e.stopPropagation();
        await bridge.tasks.toggle(t.id);
        refresh();
      });

      // Text and meta
      const content = document.createElement('div');
      content.className = 'task-content';

      const textSpan = document.createElement('span');
      textSpan.className = 'task-text';
      textSpan.textContent = t.text;
      textSpan.dir = 'auto';

      const meta = document.createElement('div');
      meta.className = 'task-meta';

      const prio = document.createElement('span');
      prio.className = `task-prio prio-${t.priority || 'normal'}`;
      prio.textContent = t.priority === 'high' ? 'High' : t.priority === 'low' ? 'Low' : 'Normal';
      meta.appendChild(prio);

      if (t.due) {
        const dueSpan = document.createElement('span');
        dueSpan.className = 'task-due';
        dueSpan.textContent = `· ${t.due}`;
        meta.appendChild(dueSpan);
      }

      content.appendChild(textSpan);
      content.appendChild(meta);

      // Delete button
      const delBtn = document.createElement('button');
      delBtn.type = 'button';
      delBtn.className = 'task-del-btn';
      delBtn.title = 'Delete task';
      delBtn.innerHTML = '<svg class="s" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>';
      delBtn.addEventListener('click', async (e) => {
        e.stopPropagation();
        await bridge.tasks.remove(t.id);
        refresh();
      });

      item.appendChild(checkBtn);
      item.appendChild(content);
      item.appendChild(delBtn);

      listEl.appendChild(item);
    });
  }

  async function refresh() {
    if (!bridge.tasks) return;
    try {
      const list = await bridge.tasks.list();
      render(list);
    } catch {}
  }

  if (form) {
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      const text = input.value.trim();
      if (!text) return;
      const priority = prioSelect ? prioSelect.value : 'normal';
      input.value = '';
      await bridge.tasks.add(text, { priority });
      refresh();
    });
  }

  if (clearBtn) {
    clearBtn.addEventListener('click', async () => {
      await bridge.tasks.clear(true);
      refresh();
    });
  }

  // Handle plugin availability
  function checkPlugin(plugins) {
    const hasTasks = Array.isArray(plugins) && plugins.some((p) => p.kind === 'tasks');
    state.tasksTab = hasTasks;
    if (typeof refreshTabs === 'function') refreshTabs();
  }

  if (bridge.onPlugins) {
    bridge.onPlugins(checkPlugin);
  }

  if (bridge.plugins) {
    bridge.plugins().then(checkPlugin).catch(() => {});
  }

  if (bridge.tasks && bridge.tasks.onUpdate) {
    bridge.tasks.onUpdate(refresh);
  }

  refresh();
})();
