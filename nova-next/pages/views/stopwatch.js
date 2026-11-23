// Stopwatch (count-up timer) inside the Timer tab (1.8.15): Start, Pause, Resume, Reset.
// Shows live count-up on the collapsed compact pill while running; runs zero timers when stopped.
(() => {
  const presets = document.querySelector('#v-timer .presets');
  const startBtn = document.getElementById('tm-start');
  const resetBtn = document.getElementById('tm-reset');
  const focusBtn = document.getElementById('tm-focus');
  if (!presets || !startBtn || !resetBtn) return;

  const swBtn = document.createElement('button');
  swBtn.type = 'button';
  swBtn.id = 'tm-sw';
  swBtn.className = 'sw-btn';
  swBtn.setAttribute('aria-pressed', 'false');
  swBtn.title = 'Stopwatch (عداد تصاعدي)';
  swBtn.innerHTML =
    '<svg class="s" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="13" r="8"/><path d="M12 9v4l2.5 2M9 2h6M19 5l1.5 1.5"/></svg><span>Stopwatch</span>';
  presets.append(swBtn);

  const sw = { mode: false, running: false, elapsedMs: 0, startedAt: 0 };
  let loop = null;

  const elapsedNow = () => (sw.running ? sw.elapsedMs + Math.max(0, Date.now() - sw.startedAt) : sw.elapsedMs);
  const fmtUp = (ms) => {
    const totalSec = Math.floor(ms / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = String(totalSec % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
  };

  window.stopwatchActive = () => sw.mode && (sw.running || sw.elapsedMs > 0);

  function setStepButtonsHidden(hide) {
    const minus = document.getElementById('tm-minus');
    const plus = document.getElementById('tm-plus');
    if (minus) minus.hidden = hide;
    if (plus) plus.hidden = hide;
  }

  function stopLoop() {
    if (loop) {
      clearInterval(loop);
      loop = null;
    }
  }

  function drawSw() {
    if (!sw.mode) return;
    const ms = elapsedNow();
    const text = fmtUp(ms);
    const digits = document.getElementById('tm-digits');
    const cTime = document.getElementById('c-time');
    const vcTime = document.getElementById('vc-time');
    const ring = document.getElementById('tm-ring');
    const stEl = document.getElementById('tm-state');
    if (digits && digits.textContent !== text) digits.textContent = text;
    if (cTime && cTime.textContent !== text) cTime.textContent = text;
    if (vcTime && vcTime.textContent !== text) vcTime.textContent = text;
    if (ring) {
      const frac = ((ms / 1000) % 60) / 60;
      const offset = ms > 0 ? 276.46 * (1 - frac) : 276.46;
      ring.style.strokeDashoffset = String(offset);
    }
    startBtn.textContent = sw.running ? 'Pause' : ms > 0 ? 'Resume' : 'Start';
    if (stEl) stEl.textContent = sw.running ? 'Stopwatch' : ms > 0 ? 'Paused' : 'Stopwatch';
    resetBtn.disabled = ms === 0 && !sw.running;
    swBtn.classList.toggle('sel', true);
    swBtn.setAttribute('aria-pressed', 'true');
  }

  function syncLoop() {
    stopLoop();
    if (sw.mode && sw.running) {
      loop = setInterval(drawSw, 500);
    }
  }

  function enterStopwatch() {
    if (sw.mode) return;
    // Clear any running countdown or focus timer cleanly via renderer.js's reset handler
    if (window.state?.timer?.total || window.state?.timer?.running) {
      resetBtn.disabled = false;
      resetBtn.click();
    }
    document.getElementById('island')?.classList.remove('focus-on', 'break-on');
    document.querySelectorAll('[data-min]').forEach((b) => b.classList.remove('sel'));
    focusBtn?.classList.remove('sel');
    sw.mode = true;
    sw.running = false;
    sw.elapsedMs = 0;
    sw.startedAt = 0;
    setStepButtonsHidden(true);
    drawSw();
    window.render?.();
  }

  function exitStopwatch() {
    if (!sw.mode) return;
    sw.mode = false;
    sw.running = false;
    sw.elapsedMs = 0;
    sw.startedAt = 0;
    stopLoop();
    swBtn.classList.remove('sel');
    swBtn.setAttribute('aria-pressed', 'false');
    setStepButtonsHidden(false);
  }

  swBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (sw.mode) {
      exitStopwatch();
      if (typeof window.render === 'function') {
        document.getElementById('tm-digits').textContent = '0:00';
        document.getElementById('tm-state').textContent = 'Pick a time';
        document.getElementById('tm-ring').style.strokeDashoffset = '276.46';
        startBtn.textContent = 'Start';
        resetBtn.disabled = true;
        window.render();
      }
    } else {
      enterStopwatch();
    }
  });

  // Intercept Start / Reset only when Stopwatch mode is active (capture phase runs before renderer.js)
  startBtn.addEventListener('click', (e) => {
    if (!sw.mode) return;
    e.stopImmediatePropagation();
    if (sw.running) {
      sw.elapsedMs = elapsedNow();
      sw.running = false;
    } else {
      sw.startedAt = Date.now();
      sw.running = true;
    }
    drawSw();
    syncLoop();
    window.render?.();
  }, true);

  resetBtn.addEventListener('click', (e) => {
    if (!sw.mode) return;
    e.stopImmediatePropagation();
    sw.running = false;
    sw.elapsedMs = 0;
    sw.startedAt = 0;
    stopLoop();
    drawSw();
    window.render?.();
  }, true);

  // Clicking any countdown preset or Focus leaves Stopwatch mode cleanly
  document.querySelectorAll('[data-min]').forEach((b) => b.addEventListener('click', exitStopwatch, true));
  focusBtn?.addEventListener('click', exitStopwatch, true);
  window.addEventListener('island:mode', () => { if (sw.mode) drawSw(); });
})();
