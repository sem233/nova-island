// Copied image / screenshot toast (1.8.15): shows a sleek 5-second notification in the island with
// a one-click "Pin" button that floats the captured image at 1:1 size above other apps.
(() => {
  bridge.onScreenshot((s) => {
    if (!s) return;
    const shotImg = $('.t-shot');
    if (shotImg && s.thumb) shotImg.src = s.thumb;
    const ar = state.features?.prefsLang === 'ar';
    const portrait = s.height > s.width;

    if (s.autoPinned) {
      showToast('shot-auto', ar ? 'تم تثبيت اللقطة' : 'Pinned on screen', '✓', 0, 3200);
      return;
    }

    if (s.canPin && bridge.shotPin) {
      const label = ar
        ? (portrait ? 'تم التقاط صورة' : 'تم التقاط لقطة شاشة')
        : (portrait ? 'Image captured' : 'Screenshot captured');
      const actionText = ar ? '📌 تثبيت' : '📌 Pin';
      showToast('shot', label, actionText, 0, 5000);
      state.toastAction = () => bridge.shotPin();
      render();
      if (typeof reportRect === 'function') reportRect();
      return;
    }

    if (s.auto) {
      showToast('shot-auto', portrait ? 'Image saved' : 'Screenshot saved', 'Added to Shelf ✓', 0, 4200);
      state.toastAction = () => {
        selectTab('shelf');
        state.hover = true;
        render();
      };
      render();
      if (typeof reportRect === 'function') reportRect();
      return;
    }

    showToast('shot', portrait ? 'Image copied' : 'Screenshot copied', feat('tabShelf') ? 'Add to Shelf' : '', 0, 5000);
    if (feat('tabShelf')) {
      state.toastAction = () => bridge.shotToShelf();
      render();
      if (typeof reportRect === 'function') reportRect();
    }
  });
  bridge.onShotSaved((s) => showToast('shot-saved', s.name, 'On the Shelf', 0, 2600));
})();
