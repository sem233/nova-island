// Video Downloader (MDM) UI controller for Nova Island
// Derived from the battle-tested mdm.js / renderer.js workflow:
// - Hover pre-fetch on download button (300ms) for instant 0ms response when clicked
// - Stable in-memory drawer: does NOT vanish or dismiss on periodic media playback ticks
// - Keeps the island pinned while picking quality
// - Formatted sizes and crisp chips for all detected video resolutions & MP3 (320k)
(() => {
  const $ = (s) => document.querySelector(s);
  const bridge = window.nova || window.bridge;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  const vdlBtn = $('#vdl-btn');
  const vdlList = $('#vdl-list');
  let vdlNote = null;
  let vdlTimer = null;
  let vdlProbeData = null;
  let vdlPrefetching = false;
  let vdlPrefetched = false;
  let vdlProbeTime = 0;
  let vdlHoverTimer = null;
  let lastTrackKey = '';

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
    if (kind) {
      vdlTimer = setTimeout(() => {
        vdlNote = null;
        el.classList.remove('err', 'ok');
        const m = getState().media;
        el.textContent = m?.has ? m.artist || m.album || m.app || '' : '';
      }, 4500);
    }
  }

  function notifyQualitiesReady(res) {
    vdlProbeData = res;
    vdlPrefetched = true;
    vdlProbeTime = Date.now();
    const island = document.getElementById('island');
    if (island) {
      island.classList.add('dl-flash');
      setTimeout(() => island.classList.remove('dl-flash'), 2500);
    }
    if (typeof window.showToast === 'function') {
      window.showToast('vdl-ready', 'جودات التحميل جاهزة', 'اختر الجودة ⬇', 1, 9000);
    }
    const st = getState();
    st.toastAction = () => {
      const view = $('#v-media');
      if (view) {
        renderVdlChoices(res);
        view.classList.add('picking');
        if (typeof window.togglePin === 'function') window.togglePin(true);
        vdlShow('اختر جودة الفيديو أو MP3');
        doRender();
      }
    };
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

  function closeVdlPicker(silent = false) {
    const view = $('#v-media');
    if (!view || !view.classList.contains('picking')) return;
    view.classList.remove('picking');
    if (typeof window.togglePin === 'function') {
      window.togglePin(false);
    }
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
      vdlPrefetched = false;
    } else {
      vdlShow(res?.message || 'تعذر بدء تحميل الفيديو', 'err');
    }
    doRender();
  }

  if (vdlList) {
    vdlList.addEventListener('click', (e) => {
      const btn = e.target.closest('.vdl-chip');
      if (!btn || !vdlProbeData?.choices) return;
      const choice = vdlProbeData.choices[Number(btn.dataset.idx)];
      if (choice) startVdlChoice(choice);
    });
  }

  // Hover prefetch: probe in background on button hover so qualities appear instantly on click
  if (vdlBtn) {
    vdlBtn.addEventListener('mouseenter', () => {
      const m = getState().media || {};
      if (!m.has || vdlPrefetching || vdlPrefetched || vdlProbeData) return;
      vdlHoverTimer = setTimeout(async () => {
        if (!getState().media?.has || vdlPrefetching || vdlPrefetched) return;
        vdlPrefetching = true;
        try {
          const res = await bridge.videoProbe({ app: m.app || '', title: m.title || '' });
          if (res?.ok && Array.isArray(res.choices) && res.choices.length) {
            vdlProbeData = res;
            vdlPrefetched = true;
            vdlProbeTime = Date.now();
          }
        } catch {}
        vdlPrefetching = false;
      }, 300);
    });

    vdlBtn.addEventListener('mouseleave', () => {
      clearTimeout(vdlHoverTimer);
    });

    vdlBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const m = getState().media || {};
      if (vdlBtn.classList.contains('busy')) return;

      const view = $('#v-media');
      if (view && view.classList.contains('picking')) {
        closeVdlPicker();
        return;
      }

      // If we already have fresh probe data (e.g. from hover prefetch) -> show instantly!
      if (vdlProbeData && Array.isArray(vdlProbeData.choices) && vdlProbeData.choices.length && (Date.now() - vdlProbeTime < 300000)) {
        renderVdlChoices(vdlProbeData);
        if (view) view.classList.add('picking');
        if (typeof window.togglePin === 'function') window.togglePin(true);
        vdlShow('اختر جودة الفيديو أو MP3');
        doRender();
        return;
      }

      // Otherwise start probe now with animated busy feedback
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
        vdlPrefetched = true;
        vdlProbeTime = Date.now();
        const currentMode = typeof window.currentMode === 'function' ? window.currentMode() : 'expanded';
        if (currentMode === 'expanded') {
          renderVdlChoices(res);
          if (view) view.classList.add('picking');
          if (typeof window.togglePin === 'function') window.togglePin(true);
          vdlShow('اختر جودة الفيديو أو MP3');
        } else {
          notifyQualitiesReady(res);
        }
      } else {
        const errMsg = res?.error === 'no-url'
          ? 'افتح الفيديو في المتصفح أو انسخ رابطه'
          : (res?.message || 'تعذر العثور على جودات الفيديو');
        vdlShow(errMsg, 'err');
      }
      doRender();
    });
  }

  // Auto-open qualities drawer if user hovers/expands island while freshly ready
  window.addEventListener('island:mode', (e) => {
    if (
      e.detail?.mode === 'expanded' &&
      vdlProbeData?.choices?.length &&
      vdlPrefetched &&
      !$('#v-media')?.classList.contains('picking') &&
      Date.now() - vdlProbeTime < 45000
    ) {
      const view = $('#v-media');
      if (view) {
        renderVdlChoices(vdlProbeData);
        view.classList.add('picking');
        if (typeof window.togglePin === 'function') window.togglePin(true);
        vdlShow('اختر جودة الفيديو أو MP3');
      }
    }
  });

  // Only close picker when the actual TRACK changes (title/artist/app change),
  // NEVER on periodic media playback ticks!
  if (bridge && typeof bridge.onMedia === 'function') {
    bridge.onMedia((m) => {
      const key = `${m?.title || ''}|${m?.artist || ''}|${m?.app || ''}`;
      if (key !== lastTrackKey) {
        lastTrackKey = key;
        closeVdlPicker(true);
        vdlProbeData = null;
        vdlPrefetched = false;
        vdlPrefetching = false;
      }
    });
  }

  if (bridge && typeof bridge.onVideoDl === 'function') {
    bridge.onVideoDl((d) => {
      if (vdlBtn?.classList.contains('busy') && d?.text) vdlShow(String(d.text));
    });
  }
})();
