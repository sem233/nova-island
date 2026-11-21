// Nova: two eyes in the idle pill that look at the cursor and blink. They appear only when the
// cursor lingers beside the island for a few seconds, and never stand in the way of opening it:
// once the cursor is on the island (hover), the island opens as usual and the eyes are gone.
(() => {
  const DWELL_MS = 3000;
  const eyes = $('.eyes');
  const pupils = [...$$('.eye b')];
  let near = null;
  let nearSince = 0;
  let dwellTimer = null;

  let blinkTimer = null;
  let dblBlinkTimer = null;

  function blink() {
    eyes.classList.add('blink');
    setTimeout(() => eyes.classList.remove('blink'), 130);
  }

  function stopBlink() {
    clearTimeout(blinkTimer);
    clearTimeout(dblBlinkTimer);
    blinkTimer = null;
    dblBlinkTimer = null;
  }

  function scheduleBlink() {
    if (blinkTimer || !island.classList.contains('eyes') || state.features.noAnimations || !state.visible) return;
    blinkTimer = setTimeout(() => {
      blinkTimer = null;
      if (!island.classList.contains('eyes') || state.features.noAnimations || !state.visible) return;
      blink();
      if (Math.random() < 0.2) dblBlinkTimer = setTimeout(blink, 260);
      scheduleBlink();
    }, 2400 + Math.random() * 3600);
  }

  function apply() {
    const lingered = near && Date.now() - nearSince >= DWELL_MS;
    const show = feat('novaEyes') && !light() && !anchor.vertical && lingered && !state.hover && lastMode === 'idle';
    if (island.classList.contains('eyes') !== !!show) island.classList.toggle('eyes', !!show);
    if (show && !state.features.noAnimations && state.visible) scheduleBlink();
    else stopBlink();
  }

  bridge.onNear((n) => {
    if (!n) {
      near = null;
      nearSince = 0;
      clearTimeout(dwellTimer);
      apply();
      return;
    }
    if (!near) {
      nearSince = Date.now();
      clearTimeout(dwellTimer);
      dwellTimer = setTimeout(apply, DWELL_MS + 20);
    }
    near = n;
    // pupils move up to 2.5px toward the cursor
    const d = Math.hypot(n.dx, n.dy) || 1;
    const k = Math.min(1, d / 120);
    const px = (n.dx / d) * 2.5 * k;
    const py = (n.dy / d) * 2.5 * k;
    pupils.forEach((p) => {
      p.style.setProperty('--px', `${px.toFixed(2)}px`);
      p.style.setProperty('--py', `${py.toFixed(2)}px`);
    });
    apply();
  });

  bridge.onHover(apply);
  window.addEventListener('island:features', apply);
  window.addEventListener('island:mode', apply);
})();
