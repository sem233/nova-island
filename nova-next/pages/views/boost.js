// Windows Cache Cleaner & RAM Booster UI (views/boost.js, 1.8.16).
// Zero background polling: scans stats only when opened and executes cleanup on a Rust worker thread.
(() => {
  const btn = document.getElementById('open-boost');
  const host = document.querySelector('.l-expanded');
  if (!btn || !host || !window.nova?.boostStats) return;

  const CIRC = 2 * Math.PI * 42; // 263.89
  let open = false;
  let busy = false;

  const T = {
    en: {
      title: 'RAM & Cache Boost',
      close: 'Close boost panel',
      avail: 'Available RAM',
      cache: 'Windows Cache',
      go: 'Boost & Clean',
      working: 'Optimizing…',
      ready: 'One click to clean temp cache & free RAM',
      btnTip: 'Clean cache & boost RAM',
      cleaned: (ram, cache, files) => `Freed ${ram} RAM · ${cache} cache (${files} files)`,
      gb: (mb) => (mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`),
      mb: (v) => (v >= 1024 ? `${(v / 1024).toFixed(2)} GB` : `${v.toFixed(v < 10 ? 1 : 0)} MB`),
    },
    ar: {
      title: 'تنظيف الكاش وتسريع الرامات',
      close: 'إغلاق لوحة التنظيف',
      avail: 'الرام المتاحة',
      cache: 'كاش الويندوز',
      go: 'تنظيف وتسريع',
      working: 'جارٍ التنظيف…',
      ready: 'ضغطة واحدة لتفريغ الكاش وتسريع الرامات',
      btnTip: 'تنظيف الكاش وتسريع الرامات',
      cleaned: (ram, cache, files) => `تم تفريغ ${ram} رام · ${cache} كاش (${files} ملف)`,
      gb: (mb) => (mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`),
      mb: (v) => (v >= 1024 ? `${(v / 1024).toFixed(2)} GB` : `${v.toFixed(v < 10 ? 1 : 0)} MB`),
    },
  };
  const tr = () => (window.state?.features?.prefsLang === 'ar' ? T.ar : T.en);

  const sheet = document.createElement('section');
  sheet.id = 'boost-sheet';
  sheet.className = 'boost-sheet';
  sheet.setAttribute('role', 'region');
  sheet.setAttribute('aria-label', 'RAM & Cache Boost');
  sheet.hidden = true;
  sheet.innerHTML =
    '<div class="boost-head">' +
      '<span class="boost-title"><svg class="s" viewBox="0 0 24 24" aria-hidden="true"><path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/></svg><span id="boost-ttl"></span></span>' +
      '<button type="button" class="boost-close" id="boost-close" aria-label="Close"><svg class="s" viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6 6 18M6 6l12 12"/></svg></button>' +
    '</div>' +
    '<div class="boost-body">' +
      '<div class="boost-gauge" id="boost-gauge">' +
        '<svg viewBox="0 0 100 100" aria-hidden="true"><circle cx="50" cy="50" r="42" class="trk"/><circle cx="50" cy="50" r="42" class="val" id="boost-ring"/></svg>' +
        '<div class="boost-gauge-txt"><b id="boost-pct">--%</b><small id="boost-used">-- / --</small></div>' +
      '</div>' +
      '<div class="boost-right">' +
        '<div class="boost-chips">' +
          '<div class="boost-chip" id="boost-ram-chip"><span id="boost-avail-lbl"></span><b id="boost-avail">--</b></div>' +
          '<div class="boost-chip" id="boost-cache-chip"><span id="boost-cache-lbl"></span><b id="boost-cache">--</b></div>' +
        '</div>' +
        '<div class="boost-foot">' +
          '<button type="button" class="boost-go" id="boost-go">' +
            '<svg class="s" viewBox="0 0 24 24" aria-hidden="true"><path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/></svg>' +
            '<progress class="boost-spin" aria-label="Optimizing"></progress>' +
            '<span id="boost-go-txt"></span>' +
          '</button>' +
          '<span class="boost-msg" id="boost-msg" role="status" aria-live="polite"></span>' +
        '</div>' +
      '</div>' +
    '</div>';
  host.append(sheet);

  const ring = sheet.querySelector('#boost-ring');
  const gauge = sheet.querySelector('#boost-gauge');
  const pctEl = sheet.querySelector('#boost-pct');
  const usedEl = sheet.querySelector('#boost-used');
  const availEl = sheet.querySelector('#boost-avail');
  const cacheEl = sheet.querySelector('#boost-cache');
  const cacheChip = sheet.querySelector('#boost-cache-chip');
  const ramChip = sheet.querySelector('#boost-ram-chip');
  const goBtn = sheet.querySelector('#boost-go');
  const goTxt = sheet.querySelector('#boost-go-txt');
  const msgEl = sheet.querySelector('#boost-msg');
  const closeBtn = sheet.querySelector('#boost-close');

  function syncLang() {
    const t = tr();
    const ar = window.state?.features?.prefsLang === 'ar';
    sheet.dir = ar ? 'rtl' : 'ltr';
    sheet.lang = ar ? 'ar' : 'en';
    sheet.setAttribute('aria-label', t.title);
    sheet.querySelector('#boost-ttl').textContent = t.title;
    sheet.querySelector('#boost-avail-lbl').textContent = t.avail;
    sheet.querySelector('#boost-cache-lbl').textContent = t.cache;
    closeBtn.setAttribute('aria-label', t.close);
    closeBtn.title = t.close;
    btn.title = t.btnTip;
    btn.setAttribute('aria-label', t.btnTip);
    if (!busy) goTxt.textContent = t.go;
    if (!msgEl.classList.contains('ok') && !busy) msgEl.textContent = t.ready;
  }

  function applyVisibility() {
    const enabled = window.state?.features?.ramBooster !== false;
    btn.hidden = !enabled;
    if (!enabled && open) setOpen(false);
    syncLang();
  }

  function drawGauge(pct, usedMb, totalMb, availMb) {
    const t = tr();
    const p = Math.max(0, Math.min(100, Number(pct) || 0));
    pctEl.textContent = `${p}%`;
    usedEl.textContent = `${t.gb(usedMb)} / ${t.gb(totalMb)}`;
    availEl.textContent = t.gb(availMb);
    ring.style.strokeDashoffset = String(CIRC * (1 - p / 100));
    gauge.classList.toggle('warn', p >= 65 && p < 85);
    gauge.classList.toggle('hot', p >= 85);
  }

  async function refresh() {
    try {
      const s = await window.nova.boostStats();
      if (!s) return;
      drawGauge(s.ramPct, s.usedMb, s.totalMb, s.availMb);
      cacheEl.textContent = tr().mb(Number(s.cacheMb) || 0);
      cacheChip.classList.remove('done');
      ramChip.classList.remove('done');
    } catch {}
  }

  function setOpen(next) {
    if (open === next) return;
    open = next;
    btn.classList.toggle('active', open);
    btn.setAttribute('aria-expanded', String(open));
    if (open) {
      syncLang();
      msgEl.classList.remove('ok');
      msgEl.textContent = tr().ready;
      sheet.hidden = false;
      requestAnimationFrame(() => sheet.classList.add('open'));
      refresh();
    } else {
      sheet.classList.remove('open');
      setTimeout(() => { if (!open) sheet.hidden = true; }, 190);
    }
  }

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    setOpen(!open);
  });
  closeBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    setOpen(false);
  });

  goBtn.addEventListener('click', async (e) => {
    e.stopPropagation();
    if (busy) return;
    busy = true;
    const t = tr();
    goBtn.disabled = true;
    goBtn.classList.add('busy');
    goTxt.textContent = t.working;
    msgEl.classList.remove('ok');
    msgEl.textContent = t.working;
    try {
      const r = await window.nova.boostRun();
      if (r) {
        drawGauge(r.afterPct, r.afterUsedMb, r.totalMb, r.availMb);
        const ramTxt = t.gb(Number(r.ramFreedMb) || 0);
        const cacheTxt = t.mb(Number(r.cacheFreedMb) || 0);
        cacheEl.textContent = `✓ ${cacheTxt}`;
        cacheChip.classList.add('done');
        if ((r.ramFreedMb || 0) > 0) ramChip.classList.add('done');
        msgEl.classList.add('ok');
        msgEl.textContent = t.cleaned(ramTxt, cacheTxt, r.filesCleaned || 0);
        window.celebrate?.(['#30d158', '#60a5fa', '#93e4c8']);
      }
    } catch {
      msgEl.textContent = t.ready;
    } finally {
      busy = false;
      goBtn.disabled = false;
      goBtn.classList.remove('busy');
      goTxt.textContent = tr().go;
    }
  });

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && open) setOpen(false);
  });
  window.addEventListener('island:mode', (e) => {
    if (e.detail?.mode !== 'expanded' && open) setOpen(false);
  });
  window.addEventListener('island:tab', () => {
    if (open) setOpen(false);
  });
  window.addEventListener('island:features', applyVisibility);
  applyVisibility();
})();
