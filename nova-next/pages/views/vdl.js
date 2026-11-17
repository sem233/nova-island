// Video Downloader (MDM) UI controller for Nova Island — v1.8.8
(() => {
  const $ = (s) => document.querySelector(s);
  const bridge = window.nova || window.bridge;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  const vdlBtn = $('#vdl-btn');
  const vdlList = $('#vdl-list');
  let vdlNote = null;
  let vdlTimer = null;
  let vdlProbeData = null;
  let vdlProbeKey = '';
  let vdlProbeTime = 0;
  let vdlInFlight = null;
  let vdlAutoProbeTimer = null;
  let vdlHoverTimer = null;
  let probeSeq = 0;
  let lastTrackKey = '';
  let vdlWantsOpenAfterProbe = false;
  let activeDownloadId = null;

  function getState() {
    return window.state || { media: { has: false } };
  }

  function trackKeyOf(m) {
    if (!m || !m.has) return '';
    return `${m.title || ''}|${m.artist || ''}|${m.app || ''}`;
  }

  function doRender() {
    if (typeof window.render === 'function') window.render();
    if (typeof window.reportRect === 'function') {
      requestAnimationFrame(() => window.reportRect());
    }
  }

  function alignDrawer() {
    const head = $('.media .m-head');
    const drawer = $('#vdl-drawer');
    if (head && drawer) {
      const top = Math.max(44, Math.ceil(head.offsetHeight + 4));
      drawer.style.top = `${top}px`;
    }
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

  function clearVdlNote() {
    clearTimeout(vdlTimer);
    vdlNote = null;
    window.__vdlNote = null;
    const el = $('#m-artist');
    if (el) {
      el.classList.remove('err', 'ok');
      const m = getState().media;
      el.textContent = m?.has ? m.artist || m.album || m.app || '' : '';
    }
  }

  function vdlShow(text, kind = '', autoClearMs = 4500) {
    const el = $('#m-artist');
    if (!el) return;
    clearTimeout(vdlTimer);
    vdlNote = text;
    window.__vdlNote = text;
    el.textContent = text;
    el.classList.remove('err', 'ok');
    if (kind) el.classList.add(kind);
    if (kind && autoClearMs > 0) {
      vdlTimer = setTimeout(clearVdlNote, autoClearMs);
    }
  }

  function clearVdlToast() {
    const st = getState();
    if (st.toast && String(st.toast).startsWith('vdl-')) {
      st.toast = '';
      st.toastUntil = 0;
      st.toastAction = null;
      if (typeof window.render === 'function') window.render();
    }
  }

  function notifyQualitiesReady(res, key, seq) {
    if (seq !== probeSeq || key !== lastTrackKey) return;
    vdlProbeData = res;
    vdlProbeKey = key;
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
      if (seq !== probeSeq || key !== lastTrackKey) return;
      st.toast = '';
      st.toastUntil = 0;
      st.toastAction = null;
      if (typeof window.selectTab === 'function') window.selectTab('media');
      openVdlPicker(res);
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
    alignDrawer();
  }

  function openVdlPicker(data) {
    const view = $('#v-media');
    if (!view || !data?.choices?.length) return false;
    renderVdlChoices(data);
    view.classList.add('picking');
    if (typeof window.setVdlHold === 'function') {
      window.setVdlHold(true);
    }
    vdlShow('اختر جودة الفيديو أو MP3', '', 0);
    doRender();
    setTimeout(() => {
      alignDrawer();
      if (typeof window.reportRect === 'function') window.reportRect();
    }, 60);
    return true;
  }

  function closeVdlPicker(silent = false) {
    vdlWantsOpenAfterProbe = false;
    const view = $('#v-media');
    const wasPicking = !!(view && view.classList.contains('picking'));
    if (view) view.classList.remove('picking');
    if (typeof window.setVdlHold === 'function') {
      window.setVdlHold(false);
    }
    if (!silent) clearVdlNote();
    if (wasPicking) {
      doRender();
      setTimeout(() => {
        if (typeof window.reportRect === 'function') window.reportRect();
      }, 380);
    }
  }

  function runProbe(m, key, seq, passive) {
    if (vdlInFlight && vdlInFlight.key === key && vdlInFlight.seq === seq) {
      if (!passive && vdlInFlight.passive) {
        return vdlInFlight.promise.then((res) => {
          if (seq !== probeSeq || key !== lastTrackKey) return null;
          if (res?.ok && Array.isArray(res.choices) && res.choices.length) return res;
          return runProbe(m, key, seq, false);
        });
      }
      return vdlInFlight.promise;
    }

    const req = {
      app: m?.app || '',
      title: m?.title || '',
      passive: !!passive,
    };
    const slot = { key, seq, passive: !!passive, promise: null };
    const promise = Promise.resolve()
      .then(() => bridge.videoProbe(req))
      .catch(() => ({ ok: false, error: 'mdm-error' }))
      .finally(() => {
        if (vdlInFlight === slot) vdlInFlight = null;
      });
    slot.promise = promise;
    vdlInFlight = slot;
    return promise;
  }

  async function startVdlChoice(choice) {
    const m = getState().media || {};
    if (!vdlBtn || vdlBtn.classList.contains('busy')) return;
    const key = lastTrackKey;
    const seq = probeSeq;
    const probeUrl = vdlProbeData?.url || '';
    const sourcePage = vdlProbeData?.sourcePage || '';
    const titleHint = vdlProbeData?.title || m.title || '';
    const thumbnail = vdlProbeData?.thumbnail || m.art || '';
    closeVdlPicker(true);
    vdlBtn.classList.add('busy');
    vdlShow(`جاري بدء تحميل ${choice?.label || 'الفيديو'}…`, '', 0);
    let res;
    try {
      res = await bridge.videoDownload({
        app: m.app || '',
        title: m.title || '',
        url: probeUrl,
        sourcePage,
        titleHint,
        thumbnail,
        choice,
      });
    } catch {
      res = { ok: false, error: 'mdm-error' };
    }
    vdlBtn.classList.remove('busy');
    if (seq !== probeSeq || key !== lastTrackKey) return;
    if (res?.ok) {
      activeDownloadId = res.id || null;
      vdlBtn.classList.add('done');
      setTimeout(() => vdlBtn.classList.remove('done'), 4000);
      vdlShow(`بدأ تحميل ${res.label || choice?.label || 'الفيديو'} ✓`, 'ok', 4500);
      // Keep vdlProbeData so user can re-pick another quality or retry after cancel
    } else {
      vdlShow(res?.message || 'تعذر بدء تحميل الفيديو', 'err', 4500);
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

  function scheduleAutoProbe(m, key, seq) {
    clearTimeout(vdlAutoProbeTimer);
    if (!m?.has || (vdlProbeData && vdlProbeKey === key)) return;
    vdlAutoProbeTimer = setTimeout(async () => {
      if (seq !== probeSeq || key !== lastTrackKey) return;
      const curM = getState().media || {};
      if (!curM.has || (vdlProbeData && vdlProbeKey === key)) return;
      const res = await runProbe(curM, key, seq, true);
      if (seq !== probeSeq || key !== lastTrackKey) return;
      if (res?.ok && Array.isArray(res.choices) && res.choices.length) {
        vdlProbeData = res;
        vdlProbeKey = key;
        vdlProbeTime = Date.now();
        vdlBtn?.classList.add('ready');
        const island = document.getElementById('island');
        if (island) {
          island.classList.add('dl-flash');
          setTimeout(() => island.classList.remove('dl-flash'), 1600);
        }
        if (vdlWantsOpenAfterProbe) {
          vdlWantsOpenAfterProbe = false;
          vdlBtn?.classList.remove('busy');
          const mode = typeof window.currentMode === 'function' ? window.currentMode() : 'expanded';
          if (mode === 'expanded') openVdlPicker(res);
          else notifyQualitiesReady(res, key, seq);
        }
      }
    }, 1200);
  }

  if (vdlBtn) {
    vdlBtn.addEventListener('mouseenter', () => {
      const m = getState().media || {};
      const key = lastTrackKey;
      const seq = probeSeq;
      if (!m.has || (vdlProbeData && vdlProbeKey === key)) return;
      clearTimeout(vdlHoverTimer);
      vdlHoverTimer = setTimeout(async () => {
        if (seq !== probeSeq || key !== lastTrackKey) return;
        const curM = getState().media || {};
        if (!curM.has || (vdlProbeData && vdlProbeKey === key)) return;
        const res = await runProbe(curM, key, seq, true);
        if (seq !== probeSeq || key !== lastTrackKey) return;
        if (res?.ok && Array.isArray(res.choices) && res.choices.length) {
          vdlProbeData = res;
          vdlProbeKey = key;
          vdlProbeTime = Date.now();
          vdlBtn.classList.add('ready');
        }
      }, 250);
    });

    vdlBtn.addEventListener('mouseleave', () => {
      clearTimeout(vdlHoverTimer);
    });

    vdlBtn.addEventListener('click', async (e) => {
      e.stopPropagation();
      const view = $('#v-media');
      if (view && view.classList.contains('picking')) {
        closeVdlPicker(false);
        return;
      }
      if (vdlBtn.classList.contains('busy')) return;

      const m = getState().media || {};
      const key = lastTrackKey;
      const seq = probeSeq;

      if (
        vdlProbeData &&
        vdlProbeKey === key &&
        Array.isArray(vdlProbeData.choices) &&
        vdlProbeData.choices.length &&
        Date.now() - vdlProbeTime < 300000
      ) {
        openVdlPicker(vdlProbeData);
        return;
      }

      vdlWantsOpenAfterProbe = true;
      vdlBtn.classList.add('busy');
      vdlShow('جاري البحث عن الجودات…', '', 0);
      doRender();

      const res = await runProbe(m, key, seq, false);
      if (seq !== probeSeq || key !== lastTrackKey) return;
      vdlBtn.classList.remove('busy');

      if (res?.ok && Array.isArray(res.choices) && res.choices.length) {
        vdlProbeData = res;
        vdlProbeKey = key;
        vdlProbeTime = Date.now();
        vdlBtn.classList.add('ready');
        vdlWantsOpenAfterProbe = false;
        const mode = typeof window.currentMode === 'function' ? window.currentMode() : 'expanded';
        if (mode === 'expanded') {
          openVdlPicker(res);
        } else {
          notifyQualitiesReady(res, key, seq);
        }
      } else {
        vdlWantsOpenAfterProbe = false;
        const errMsg = res?.error === 'no-url'
          ? 'افتح الفيديو في المتصفح أو انسخ رابطه'
          : (res?.message || 'تعذر العثور على جودات الفيديو');
        vdlShow(errMsg, 'err', 6500);
      }
      doRender();
    });
  }

  window.addEventListener('island:mode', () => {
    setTimeout(() => {
      if (typeof window.reportRect === 'function') window.reportRect();
    }, 100);
  });

  if (bridge && typeof bridge.onMedia === 'function') {
    bridge.onMedia((m) => {
      const key = trackKeyOf(m);
      if (key !== lastTrackKey) {
        lastTrackKey = key;
        probeSeq += 1;
        const seq = probeSeq;
        clearTimeout(vdlAutoProbeTimer);
        clearTimeout(vdlHoverTimer);
        vdlInFlight = null;
        vdlProbeData = null;
        vdlProbeKey = '';
        vdlProbeTime = 0;
        vdlWantsOpenAfterProbe = false;
        activeDownloadId = null;
        if (vdlList) vdlList.innerHTML = '';
        vdlBtn?.classList.remove('busy', 'ready', 'done');
        closeVdlPicker(true);
        clearVdlNote();
        clearVdlToast();
        if (m?.has) scheduleAutoProbe(m, key, seq);
      }
    });
  }

  if (bridge && typeof bridge.onVideoDl === 'function') {
    bridge.onVideoDl((d) => {
      const view = $('#v-media');
      if (view && view.classList.contains('picking')) return;
      if (d?.id && activeDownloadId && d.id !== activeDownloadId) return;
      if (d?.pct != null) {
        vdlShow(`جاري التحميل ${Math.round(d.pct)}%`, '', 0);
      } else if (d?.done) {
        activeDownloadId = null;
        vdlShow('اكتمل التحميل بنجاح ✓', 'ok', 4500);
        vdlBtn?.classList.remove('busy');
        vdlBtn?.classList.add('done');
        setTimeout(() => vdlBtn?.classList.remove('done'), 4000);
      } else if (d?.text) {
        vdlShow(String(d.text), '', 0);
      }
    });
  }
})();
