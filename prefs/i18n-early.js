// Loaded in <head>, before anything is drawn (1.8.4): when the window opens in Arabic (?lang=ar),
// the page is right to left at once and stays hidden until the Arabic text is in place
// (prefs-i18n.js / tasks.js show it), so English never flashes first. A safety timer shows it
// anyway if the rest of the page failed to load.
(() => {
  if (new URLSearchParams(location.search).get('lang') !== 'ar') return;
  const html = document.documentElement;
  html.lang = 'ar';
  html.dir = 'rtl';
  html.classList.add('lang-ar', 'i18n-wait');
  // hidden by html.i18n-wait in prefs-2.css / tasks.css (the pages allow no inline styles)
  setTimeout(() => html.classList.remove('i18n-wait'), 1500);
})();
