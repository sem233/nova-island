// Universal Island Pin button in the expanded topbar (1.8.15): locks the entire island open across
// every tab, synced with Ctrl+Alt+I and QuickClips' pin control.
(() => {
  const btn = document.getElementById('pin-island');
  if (!btn) return;

  function sync() {
    const pinned = !!(window.state && window.state.pinned);
    const ar = window.state?.features?.prefsLang === 'ar';
    btn.classList.toggle('active', pinned);
    btn.setAttribute('aria-pressed', String(pinned));
    const tip = pinned
      ? (ar ? 'الجزيرة مثبّتة — اضغط لإلغاء التثبيت' : 'Island pinned — click to unpin')
      : (ar ? 'تثبيت الجزيرة لتظل مفتوحة' : 'Pin island open');
    btn.title = tip;
    btn.setAttribute('aria-label', tip);
  }

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (typeof window.togglePin === 'function') {
      window.togglePin();
    } else if (window.state && window.nova?.setPinned) {
      window.state.pinned = !window.state.pinned;
      if (!window.state.pinned) window.state.hover = false;
      window.nova.setPinned(window.state.pinned);
      window.render?.();
    }
    sync();
  });

  window.addEventListener('island:pinned', sync);
  window.addEventListener('island:mode', sync);
  window.addEventListener('island:features', sync);
  sync();
})();
