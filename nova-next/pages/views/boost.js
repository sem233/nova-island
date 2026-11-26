// Windows Cache Cleaner & RAM Booster UI (views/boost.js, 1.8.18).
// Zero idle polling: scans on open, syncs live with bridge.onStats while open, and streams
// real-time Rust `boost-progress` snapshots so the circular RAM ring empties smoothly in lockstep
// with actual OS cache deletion & physical memory reclamation.
(() => {
  const btn = document.getElementById('open-boost');
  const host = document.querySelector('.l-expanded');
  if (!btn || !host || !window.nova?.boostStats) return;

  let open = false;
  let busy = false;
  let holdUntil = 0;
  let curPct = 0;
  let curUsed = 0;
  let curTotal = 0;
  let curAvail = 0;
  let animId = 0;
  let reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches || false;
  window.matchMedia?.('(prefers-reduced-motion: reduce)')?.addEventListener?.('change', (e) => {
    reducedMotion = e.matches;
  });

  // LRI/PDI Unicode directional isolates keep "1.6 GB" / "312 MB" crisp inside Arabic RTL text
  const ltr = (s) => `\u2066${s}\u2069`;
  const fmtGb = (mb) => (mb >= 1000 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`);
  const fmtMb = (v) => (v >= 1000 ? `${(v / 1024).toFixed(2)} GB` : `${v.toFixed(v < 10 ? 1 : 0)} MB`);
  const fmtUsedPair = (usedMb, totalMb) => {
    if (totalMb >= 1024) {
      return `${(usedMb / 1024).toFixed(1)} / ${(totalMb / 1024).toFixed(1)} GB`;
    }
    return `${Math.round(usedMb)} / ${Math.round(totalMb)} MB`;
  };

  const T = {
    en: {
      title: 'RAM & Cache Boost',
      close: 'Close boost panel',
      ringLabel: 'RAM usage',
      ringAction: 'Click to clean cache & free RAM',
      avail: 'Available RAM',
      cache: 'System & App Cache',
      go: 'Boost & Clean',
      working: 'Boosting…',
      ready: 'Deep clean Windows/browser caches & free RAM',
      btnTip: 'Clean cache & boost RAM',
      stages: {
        cache: 'Cleaning Windows temp cache…',
        temp: 'Cleaning Windows temp cache…',
        gpu: 'Cleaning GPU shader caches…',
        browser: 'Cleaning browser caches…',
        apps: 'Cleaning app caches…',
        dns: 'Flushing DNS network cache…',
        ram: (p, tot) => (tot > 0 ? `Freeing RAM (${p}/${tot} apps)…` : 'Freeing RAM…'),
        memory: 'Compacting system memory…',
        done: 'Optimization complete',
      },
      summary: (r) => {
        const drop = r.beforePct > r.afterPct ? `${r.beforePct}% → ${r.afterPct}% · ` : '';
        const parts = [];
        if (r.ramFreedMb > 0) parts.push(`${fmtGb(r.ramFreedMb)} RAM`);
        if (r.cacheFreedMb > 0) parts.push(`${fmtMb(r.cacheFreedMb)} cache`);
        if (!parts.length && r.wsTrimmedMb > 0) parts.push(`${fmtGb(r.wsTrimmedMb)} trimmed`);
        if (!parts.length) return `Already optimized (${r.procsTrimmed || 0} apps checked)`;
        return `${drop}Freed ${parts.join(' · ')}`;
      },
      detail: (r) =>
        `RAM: ${r.beforePct}% → ${r.afterPct}% (${fmtGb(r.ramFreedMb)} free, ${fmtGb(r.wsTrimmedMb)} working set trimmed) | ` +
        `Cache: ${fmtMb(r.cacheFreedMb)} (${r.filesCleaned} files) | Apps: ${r.procsTrimmed} | DNS: ${r.dnsFlushed ? 'flushed' : 'skipped'}` +
        (r.standbyPurged ? ' | Standby: purged' : ''),
    },
    ar: {
      title: 'تنظيف الكاش وتسريع الرامات',
      close: 'إغلاق لوحة التنظيف',
      ringLabel: 'استهلاك الرامات',
      ringAction: 'اضغط لتنظيف الكاش وتفريغ الرامات',
      avail: 'الرام المتاحة',
      cache: 'كاش الويندوز والمتصفح',
      go: 'تنظيف وتسريع',
      working: 'جارٍ التسريع…',
      ready: 'ضغطة لتنظيف الكاش وتفريغ الرام',
      btnTip: 'تنظيف الكاش وتسريع الرامات',
      stages: {
        cache: 'جارٍ تنظيف ملفات الكاش المؤقتة…',
        temp: 'جارٍ تنظيف كاش الويندوز…',
        gpu: 'جارٍ تنظيف كاش كارت الشاشة…',
        browser: 'جارٍ تنظيف كاش المتصفحات…',
        apps: 'جارٍ تنظيف كاش التطبيقات…',
        dns: 'جارٍ تفريغ كاش الشبكة DNS…',
        ram: (p, tot) => (tot > 0 ? `تفريغ الرامات (${ltr(`${p}/${tot}`)} تطبيق)…` : 'جارٍ تفريغ الرامات…'),
        memory: 'جارٍ ضغط وتحسين الذاكرة…',
        done: 'اكتمل التنظيف والتسريع',
      },
      summary: (r) => {
        const drop = r.beforePct > r.afterPct ? `${ltr(`${r.beforePct}%`)} ← ${ltr(`${r.afterPct}%`)} · ` : '';
        const parts = [];
        if (r.ramFreedMb > 0) parts.push(`${ltr(fmtGb(r.ramFreedMb))} رام`);
        if (r.cacheFreedMb > 0) parts.push(`${ltr(fmtMb(r.cacheFreedMb))} كاش`);
        if (!parts.length && r.wsTrimmedMb > 0) parts.push(`${ltr(fmtGb(r.wsTrimmedMb))} مضغوطة`);
        if (!parts.length) return `الجهاز محسّن بالفعل (${r.procsTrimmed || 0} تطبيق)`;
        return `${drop}فُرّغ ${parts.join(' · ')}`;
      },
      detail: (r) =>
        `الرام: ${ltr(`${r.beforePct}%`)} ← ${ltr(`${r.afterPct}%`)} (${ltr(fmtGb(r.ramFreedMb))} متاحة، ${ltr(fmtGb(r.wsTrimmedMb))} ضغط عمليات) | ` +
        `الكاش: ${ltr(fmtMb(r.cacheFreedMb))} (${r.filesCleaned} ملف) | التطبيقات: ${r.procsTrimmed}` +
        (r.standbyPurged ? ' | تم تفريغ Standby' : ''),
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
        '<div class="boost-halo" aria-hidden="true"></div>' +
        '<div class="boost-ring-vis" id="boost-ring-vis" aria-hidden="true"></div>' +
        '<progress class="boost-sr" id="boost-ring" value="0" max="100" aria-label="RAM usage" aria-valuetext="0%"></progress>' +
        '<div class="boost-gauge-txt" aria-hidden="true"><b id="boost-pct" dir="ltr">--%</b><small id="boost-used" dir="ltr">-- / --</small></div>' +
        '<button type="button" class="boost-ring-hit" id="boost-ring-hit" aria-label="Boost & Clean"></button>' +
      '</div>' +
      '<div class="boost-right">' +
        '<div class="boost-chips">' +
          '<div class="boost-chip" id="boost-ram-chip"><span id="boost-avail-lbl"></span><b id="boost-avail" dir="ltr">--</b></div>' +
          '<div class="boost-chip" id="boost-cache-chip"><span id="boost-cache-lbl"></span><b id="boost-cache" dir="ltr">--</b></div>' +
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

  const ringSr = sheet.querySelector('#boost-ring');
  const ringVis = sheet.querySelector('#boost-ring-vis');
  const ringHit = sheet.querySelector('#boost-ring-hit');
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
    ringSr.setAttribute('aria-label', t.ringLabel);
    ringHit.setAttribute('aria-label', t.ringAction);
    ringHit.title = t.ringAction;
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

  function setRingTarget(p) {
    const rounded = Math.max(0, Math.min(100, Math.round(Number(p) || 0)));
    ringSr.setAttribute('value', String(rounded));
    ringSr.setAttribute('aria-valuetext', `${rounded}%`);
    ringSr.style.setProperty('--boost-val', String(rounded));
    ringVis.style.setProperty('--boost-val', String(rounded));
    gauge.classList.toggle('warn', rounded >= 70 && rounded < 85);
    gauge.classList.toggle('hot', rounded >= 85);
    return rounded;
  }

  function paintText(pct, usedMb, totalMb, availMb) {
    pctEl.textContent = `${Math.max(0, Math.min(100, Math.round(Number(pct) || 0)))}%`;
    if (totalMb > 0) {
      usedEl.textContent = fmtUsedPair(usedMb, totalMb);
      availEl.textContent = fmtGb(availMb);
    }
  }

  // Smoothly eases displayed RAM counters in lockstep with live Rust progress snapshots
  function drawGauge(pct, usedMb, totalMb, availMb, immediate = false) {
    const targetPct = Math.max(0, Math.min(100, Number(pct) || 0));
    const targetUsed = Number(usedMb) || 0;
    const targetTotal = Number(totalMb) || curTotal;
    const targetAvail = Number(availMb) || 0;
    if (animId) { cancelAnimationFrame(animId); animId = 0; }

    setRingTarget(targetPct);

    if (immediate || reducedMotion || curTotal === 0) {
      curPct = targetPct;
      curUsed = targetUsed;
      curTotal = targetTotal;
      curAvail = targetAvail;
      paintText(curPct, curUsed, curTotal, curAvail);
      return;
    }

    const fromPct = curPct;
    const fromUsed = curUsed;
    const fromAvail = curAvail;
    curTotal = targetTotal;
    const start = performance.now();
    const dur = 200;
    const step = (now) => {
      const k = Math.min(1, (now - start) / dur);
      const e = 1 - Math.pow(1 - k, 3);
      curPct = fromPct + (targetPct - fromPct) * e;
      curUsed = fromUsed + (targetUsed - fromUsed) * e;
      curAvail = fromAvail + (targetAvail - fromAvail) * e;
      paintText(curPct, curUsed, curTotal, curAvail);
      if (k < 1) {
        animId = requestAnimationFrame(step);
      } else {
        animId = 0;
        curPct = targetPct;
        curUsed = targetUsed;
        curAvail = targetAvail;
      }
    };
    animId = requestAnimationFrame(step);
  }

  async function refresh() {
    try {
      const s = await window.nova.boostStats();
      if (!s || busy) return;
      drawGauge(s.ramPct, s.usedMb, s.totalMb, s.availMb, curTotal === 0);
      const mbStr = fmtMb(Number(s.cacheMb) || 0);
      cacheEl.textContent = s.cachePartial ? `${mbStr}+` : mbStr;
      cacheChip.classList.remove('done', 'active');
      ramChip.classList.remove('done', 'active');
      gauge.classList.remove('boosted');
    } catch {}
  }

  function setOpen(next) {
    if (open === next) return;
    open = next;
    btn.classList.toggle('active', open);
    btn.setAttribute('aria-expanded', String(open));
    if (open) {
      syncLang();
      if (!busy) {
        msgEl.classList.remove('ok');
        msgEl.textContent = tr().ready;
        msgEl.title = '';
      }
      sheet.hidden = false;
      requestAnimationFrame(() => sheet.classList.add('open'));
      refresh();
    } else {
      sheet.classList.remove('open');
      setTimeout(() => { if (!open) sheet.hidden = true; }, 190);
    }
  }

  // Live streaming progress handler from Rust `boost::run_with_progress`
  window.nova.onBoostProgress?.((ev) => {
    if (!ev || !busy) return;
    const t = tr();
    if (typeof ev.ramPct === 'number') {
      drawGauge(ev.ramPct, ev.usedMb, ev.totalMb, ev.availMb, false);
    }
    const stage = ev.stage || 'cache';
    const isCacheStage = stage === 'cache' || stage === 'temp' || stage === 'gpu' || stage === 'browser' || stage === 'apps' || stage === 'dns';
    const isRamStage = stage === 'ram' || stage === 'memory';
    cacheChip.classList.toggle('active', isCacheStage);
    ramChip.classList.toggle('active', isRamStage);

    if (typeof ev.cacheFreedMb === 'number' && ev.cacheFreedMb > 0) {
      cacheEl.textContent = `${fmtMb(ev.cacheFreedMb)} (${ev.filesCleaned || 0})`;
    }
    const labelFn = t.stages[stage];
    if (typeof labelFn === 'function') {
      msgEl.textContent = labelFn(ev.procsTrimmed || 0, ev.procsTotal || 0);
    } else if (typeof labelFn === 'string') {
      msgEl.textContent = labelFn;
    }
  });

  // Stay in sync with live system stats while open and idle (zero extra polling!)
  window.nova.onStats?.((s) => {
    if (!open || busy || performance.now() < holdUntil || !s || !s.ramTotal) return;
    const totalMb = Math.round(s.ramTotal / 1048576);
    const usedMb = Math.round(s.ramUsed / 1048576);
    const availMb = Math.max(0, totalMb - usedMb);
    const pct = Math.round((s.ram || 0) * 100);
    drawGauge(pct, usedMb, totalMb, availMb, false);
  });

  async function runBoost(e) {
    e?.stopPropagation?.();
    if (busy) return;
    busy = true;
    const t = tr();
    goBtn.disabled = true;
    ringHit.disabled = true;
    goBtn.classList.add('busy');
    gauge.classList.remove('boosted');
    gauge.classList.add('boosting');
    cacheChip.classList.remove('done');
    ramChip.classList.remove('done');
    goTxt.textContent = t.working;
    msgEl.classList.remove('ok');
    msgEl.textContent = t.stages.ram(0, 0);
    msgEl.title = '';
    try {
      const r = await window.nova.boostRun();
      if (r) {
        holdUntil = performance.now() + 2800;
        drawGauge(r.afterPct, r.afterUsedMb, r.totalMb, r.availMb, false);
        const cacheTxt = fmtMb(Number(r.cacheFreedMb) || 0);
        cacheEl.textContent = r.cacheFreedMb > 0 ? `✓ ${cacheTxt}` : '✓ 0 MB';
        cacheChip.classList.remove('active');
        ramChip.classList.remove('active');
        cacheChip.classList.add('done');
        if ((r.ramFreedMb || 0) > 0 || (r.wsTrimmedMb || 0) > 0) ramChip.classList.add('done');
        gauge.classList.add('boosted');
        msgEl.classList.add('ok');
        msgEl.textContent = tr().summary(r);
        msgEl.title = tr().detail(r);
        if ((r.ramFreedMb || 0) >= 64 || (r.cacheFreedMb || 0) >= 10) {
          window.celebrate?.(['#30d158', '#60a5fa', '#93e4c8']);
        }
      }
    } catch {
      msgEl.textContent = tr().ready;
    } finally {
      busy = false;
      goBtn.disabled = false;
      ringHit.disabled = false;
      goBtn.classList.remove('busy');
      gauge.classList.remove('boosting');
      cacheChip.classList.remove('active');
      ramChip.classList.remove('active');
      goTxt.textContent = tr().go;
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
  goBtn.addEventListener('click', runBoost);
  ringHit.addEventListener('click', runBoost);

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
