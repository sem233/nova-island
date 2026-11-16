// Video Downloader (MDM) UI controller for Nova Island
(() => {
  const $ = (s) => document.querySelector(s);
  const bridge = window.nova || window.bridge;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  const vdlBtn = $('#vdl-btn');
  const vdlList = $('#vdl-list');
  let vdlNote = null;
  let vdlTimer = null;
  let vdlProbeData = null;

  function getState() {
    return window.state || { media: { has: false } };
  }

  function doRender() {
    if (typeof window.render === 'function') window.render();
  }

  function fmtVdlSize(bytes, note) {
    if (!(bytes > 0)) return note || '';
    if (bytes < 1024 ** 2) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
    if (bytes < 1024 ** 3) {
      const mb = bytes / 1024 ** 2;
      return `${mb >= 100 ? Math.round(mb) : mb.toFixed(1).replace(/\.0$/, '')} MB`;
    }
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  }

  function vdlShow(text, kind = '') {
    const el = $('#m-artist');
    if (!el) return;
    clearTimeout(vdlTimer);
    vdlNote = text;
    el.textContent = text;
    el.classList.remove('err', 'ok');
    if (kind) el.classList.add(kind);
    vdlTimer = setTimeout(() => {
      vdlNote = null;
      el.classList.remove('err', 'ok');
      const m = getState().media;
      el.textContent = m?.has ? m.artist || m.album || m.app || '' : '';
    }, 4500);
  }

  async function startVdlChoice(choice) {
    const m = getState().media || {};
    if (!vdlBtn || vdlBtn.classList.contains('busy')) return;
    const probeUrl = vdlProbeData?.url || '';
    closeVdlPicker(true);
    vdlBtn.classList.add('busy');
    vdlShow(`بدء تحميل ${choice?.label || 'الفيديو'}…`);
    let res;
    try {
      res = await bridge.videoDownload({
        app: m.app || '',
        title: m.title || '',
        url: probeUrl,
        choice,
      });
    } catch {
      res = { ok: false, error: 'mdm-error' };
    }
    vdlBtn.classList.remove('busy');
    if (res?.ok) {
      vdlBtn.classList.add('done');
      setTimeout(() => vdlBtn.classList.remove('done'), 4000);
      vdlShow(`جاري تحميل ${res.label || choice?.label || 'الفيديو'} بنجاح ✓`, 'ok');
      vdlProbeData = null;
    } else {
      vdlShow(res?.message || 'تعذر بدء تحميل الفيديو', 'err');
    }
    doRender();
  }

  function renderVdlChoices(data) {
    vdlProbeData = data;
    const choices = data?.choices || [];
    const audioIco = '<svg class="s" viewBox="0 0 24 24"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>';
    if (vdlList) {
      vdlList.innerHTML = choices.map((c, i) => {
        const isAudio = c.kind === 'audio';
        const sz = fmtVdlSize(c.size, c.note);
        const tag = c.tag || '';
        const badge = tag ? `<i class="vdl-tag">${esc(tag)}</i>` : '';
        return `<button class="vdl-chip${isAudio ? ' audio' : ''}" data-idx="${i}" style="--i:${i}" title="${esc(c.label)}${sz ? ` · ${esc(sz)}` : ''}">` +
          `<span class="vdl-q">${isAudio ? audioIco : ''}<span>${esc(c.label)}</span>${badge}</span>` +
          `<span class="vdl-sz">${esc(sz)}</span>` +
          '</button>';
      }).join('');
    }
  }

  if (vdlList) {
    vdlList.addEventListener('click', (e) => {
      const btn = e.target.closest('.vdl-chip');
      if (!btn || !vdlProbeData?.choices) return;
      const choice = vdlProbeData.choices[Number(btn.dataset.idx)];
      if (choice) startVdlChoice(choice);
    });
  }

  function closeVdlPicker(silent = false) {
    const view = $('#v-media');
    if (!view || !view.classList.contains('picking')) return;
    view.classList.remove('picking');
    if (!silent) {
      clearTimeout(vdlTimer);
      vdlNote = null;
      const el = $('#m-artist');
      if (el) {
        el.classList.remove('err', 'ok');
        const m = getState().media;
        el.textContent = m?.has ? m.artist || m.album || m.app || '' : '';
      }
    }
  }

  if (vdlBtn) {
    vdlBtn.addEventListener('click', async () => {
      const m = getState().media || {};
      if (vdlBtn.classList.contains('busy')) return;
      const view = $('#v-media');
      if (view && view.classList.contains('picking')) {
        closeVdlPicker();
        return;
      }
      if (vdlProbeData && Array.isArray(vdlProbeData.choices) && vdlProbeData.choices.length) {
        renderVdlChoices(vdlProbeData);
        if (view) view.classList.add('picking');
        vdlShow('اختر جودة الفيديو أو MP3');
        doRender();
        return;
      }
      vdlBtn.classList.add('busy');
      vdlShow('جاري فحص رابط وجودات الفيديو…');
      let res;
      try {
        res = await bridge.videoProbe({ app: m.app || '', title: m.title || '' });
      } catch {
        res = { ok: false, error: 'mdm-error' };
      }
      vdlBtn.classList.remove('busy');
      if (res?.ok && Array.isArray(res.choices) && res.choices.length) {
        vdlProbeData = res;
        renderVdlChoices(res);
        if (view) view.classList.add('picking');
        vdlShow('اختر جودة الفيديو أو MP3');
      } else {
        const errMsg = res?.error === 'no-url'
          ? 'افتح الفيديو في المتصفح أو انسخ رابطه'
          : (res?.message || 'تعذر العثور على جودات الفيديو');
        vdlShow(errMsg, 'err');
      }
      doRender();
    });
  }

  // Close picker on track change
  if (bridge && typeof bridge.onMedia === 'function') {
    bridge.onMedia(() => {
      closeVdlPicker(true);
      vdlProbeData = null;
    });
  }
})();
