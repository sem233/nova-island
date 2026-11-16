// QuickClips tab — saved text snippets with copy, pin, delete, instant clipboard paste, and island pin.
(() => {
  const list        = document.getElementById('qc-list');
  const empty       = document.getElementById('qc-empty');
  const pasteBtn    = document.getElementById('qc-paste-btn');
  const addBtn      = document.getElementById('qc-add-btn');
  const pinIslandBtn= document.getElementById('qc-pin-island-btn');
  const clearBtn    = document.getElementById('qc-clear-btn');
  const addPanel    = document.getElementById('qc-add-panel');
  const textarea    = document.getElementById('qc-new-text');
  const saveBtn     = document.getElementById('qc-save-btn');
  const cancelBtn   = document.getElementById('qc-cancel-btn');

  let clips = [];
  let copyingId = null;
  const bridge = window.nova || window.bridge;

  // Sync island pin state with button UI
  function updatePinIslandBtn() {
    if (!pinIslandBtn) return;
    const isPinned = !!(window.state && window.state.pinned);
    pinIslandBtn.classList.toggle('active', isPinned);
    pinIslandBtn.title = isPinned ? 'Island pinned (الجزيرة مثبتة) — click to unpin' : 'Pin island open (تثبيت الجزيرة لتظل مفتوحة)';
  }

  if (pinIslandBtn) {
    pinIslandBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      if (typeof window.togglePin === 'function') {
        window.togglePin();
      } else if (window.state) {
        window.state.pinned = !window.state.pinned;
        if (!window.state.pinned) window.state.hover = false;
        bridge.setPinned(window.state.pinned);
        if (typeof window.render === 'function') window.render();
      }
      updatePinIslandBtn();
    });
  }

  window.addEventListener('island:pinned', updatePinIslandBtn);
  window.addEventListener('island:mode', updatePinIslandBtn);
  updatePinIslandBtn();

  // ---- render clips list ----
  function render(data) {
    clips = Array.isArray(data) ? data : clips;
    const pinned   = clips.filter((c) => c.pinned);
    const unpinned = clips.filter((c) => !c.pinned);

    const existing = new Map([...list.querySelectorAll('.qc-row')].map((el) => [el.dataset.id, el]));
    const order = [...pinned, ...unpinned];
    const seen = new Set();

    order.forEach((clip) => {
      seen.add(clip.id);
      let row = existing.get(clip.id);
      if (!row) {
        row = makeRow(clip);
        list.appendChild(row);
        requestAnimationFrame(() => row.classList.add('qc-in'));
      } else {
        // update pin status
        const pinBtn = row.querySelector('.qc-pin');
        if (pinBtn) {
          pinBtn.dataset.pinned = clip.pinned ? '1' : '0';
          pinBtn.title = clip.pinned ? 'Unpin from top' : 'Pin to top (تثبيت بالأعلى)';
        }
        list.appendChild(row);
      }
    });

    // remove deleted
    for (const [id, el] of existing) {
      if (!seen.has(id)) {
        el.classList.add('qc-out');
        el.addEventListener('animationend', () => el.remove(), { once: true });
      }
    }

    if (empty) empty.hidden = clips.length > 0;
  }

  function preview(text) {
    const p = String(text || '').trim();
    return p.length > 160 ? p.slice(0, 157) + '…' : p;
  }

  function makeRow(clip) {
    const row = document.createElement('div');
    row.className = 'qc-row';
    row.dataset.id = clip.id;
    if (clip.pinned) row.classList.add('is-pinned');

    const pre = document.createElement('div');
    pre.className = 'qc-text';
    pre.dir = 'auto';
    pre.textContent = preview(clip.text);
    pre.title = 'Click to copy';

    const actions = document.createElement('div');
    actions.className = 'qc-actions';

    // Copy button
    const copyBtn = document.createElement('button');
    copyBtn.className = 'qc-btn qc-copy';
    copyBtn.title = 'Copy';
    copyBtn.innerHTML = `<svg class="s" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;

    copyBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      if (copyingId === clip.id) return;
      copyingId = clip.id;
      bridge.clips.copy(clip.id);

      copyBtn.classList.add('qc-copied');
      copyBtn.innerHTML = `<svg class="s" viewBox="0 0 24 24"><path d="M20 6 9 17l-5-5"/></svg>`;
      row.classList.add('qc-flash');

      setTimeout(() => {
        copyBtn.classList.remove('qc-copied');
        copyBtn.innerHTML = `<svg class="s" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>`;
        row.classList.remove('qc-flash');
        copyingId = null;
      }, 1400);
    });

    // Pin button
    const pinBtn = document.createElement('button');
    pinBtn.className = 'qc-btn qc-pin';
    pinBtn.dataset.pinned = clip.pinned ? '1' : '0';
    pinBtn.title = clip.pinned ? 'Unpin' : 'Pin to top (تثبيت بالأعلى)';
    pinBtn.innerHTML = `<svg class="s" viewBox="0 0 24 24"><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></svg>`;

    pinBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const nowPinned = pinBtn.dataset.pinned !== '1';
      pinBtn.dataset.pinned = nowPinned ? '1' : '0';
      row.classList.toggle('is-pinned', nowPinned);
      await bridge.clips.pin(clip.id, nowPinned);
      const updated = await bridge.clips.list();
      render(updated);
    });

    // Delete button
    const delBtn = document.createElement('button');
    delBtn.className = 'qc-btn qc-del';
    delBtn.title = 'Delete';
    delBtn.innerHTML = `<svg class="s" viewBox="0 0 24 24"><path d="M18 6 6 18M6 6l12 12"/></svg>`;
    delBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      row.classList.add('qc-out');
      row.addEventListener('animationend', async () => {
        await bridge.clips.remove(clip.id);
        const updated = await bridge.clips.list();
        render(updated);
      }, { once: true });
    });

    actions.append(copyBtn, pinBtn, delBtn);
    row.append(pre, actions);

    // Clicking anywhere on the text card also copies
    row.addEventListener('click', () => copyBtn.click());

    return row;
  }

  // ---- instant clipboard paste button ----
  if (pasteBtn) {
    pasteBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      pasteBtn.disabled = true;
      try {
        const added = await bridge.clips.addFromClipboard();
        if (added) {
          pasteBtn.classList.add('qc-btn-success');
          setTimeout(() => pasteBtn.classList.remove('qc-btn-success'), 1200);
          const updated = await bridge.clips.list();
          render(updated);
          // Highlight / flash the newly pasted row
          const newRow = list.querySelector(`[data-id="${added.id}"]`);
          if (newRow) {
            newRow.classList.add('qc-flash');
            setTimeout(() => newRow.classList.remove('qc-flash'), 1200);
          }
        } else {
          pasteBtn.classList.add('qc-btn-empty');
          setTimeout(() => pasteBtn.classList.remove('qc-btn-empty'), 1200);
        }
      } catch (err) {
        console.error('[clips paste]', err);
      } finally {
        pasteBtn.disabled = false;
      }
    });
  }

  // ---- manual add panel with focus management ----
  function openAddPanel() {
    if (window.state) window.state.editingClip = true;
    addPanel.hidden = false;
    textarea.value = '';
    addPanel.classList.remove('qc-panel-out');
    addPanel.classList.add('qc-panel-in');

    if (typeof window.reportRect === 'function') window.reportRect();

    // Request window focus so the user can type / paste immediately!
    bridge.focusInput(true);
    setTimeout(() => {
      textarea.focus();
      if (typeof window.reportRect === 'function') window.reportRect();
    }, 60);
  }

  function closeAddPanel() {
    if (window.state) window.state.editingClip = false;
    bridge.focusInput(false);
    addPanel.classList.remove('qc-panel-in');
    addPanel.classList.add('qc-panel-out');
    addPanel.addEventListener('animationend', () => {
      addPanel.hidden = true;
      addPanel.classList.remove('qc-panel-out');
      if (typeof window.reportRect === 'function') window.reportRect();
    }, { once: true });
    if (typeof window.reportRect === 'function') window.reportRect();
  }

  if (addBtn) {
    addBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      if (!addPanel.hidden) { closeAddPanel(); return; }
      openAddPanel();
    });
  }

  if (cancelBtn) cancelBtn.addEventListener('click', (e) => { e.stopPropagation(); closeAddPanel(); });

  if (saveBtn) {
    saveBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const text = textarea.value.trim();
      if (!text) return;
      await bridge.clips.add(text);
      const updated = await bridge.clips.list();
      render(updated);
      closeAddPanel();
    });
  }

  // Enable keyboard input on pointerdown
  if (textarea) {
    textarea.addEventListener('pointerdown', () => {
      bridge.focusInput(true);
      setTimeout(() => textarea.focus(), 50);
    });

    textarea.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        saveBtn.click();
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        closeAddPanel();
      }
    });
  }

  // ---- clear all unpinned clips ----
  if (clearBtn) {
    clearBtn.addEventListener('click', async () => {
      clearBtn.disabled = true;
      await bridge.clips.clear(true);
      const updated = await bridge.clips.list();
      render(updated);
      clearBtn.disabled = false;
    });
  }

  // ---- updates from main process ----
  bridge.clips.onUpdate((data) => render(data));

  // Initial load
  bridge.clips.list().then(render);
})();
