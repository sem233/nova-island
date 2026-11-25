// (1.8.7 / 1.8.16) The adhan's sound with the prayer alert (Settings › Prayer & Dhikr › Adhan sound).
// The alert lasts as long as the adhan; its card has a speaker button that stops it (and the alert),
// or, with the sound off, plays it. Fajr has its own adhan. views/prayer.js fires "island:adhan"
// (which prayer) and then asks window.adhanScene() / adhanToast().
(() => {
  // the Fajr adhan and the one for the other prayers, each ready with its length
  const sounds = {};
  for (const which of ['main', 'fajr']) {
    const audio = new Audio();
    audio.preload = 'metadata';
    sounds[which] = { audio, ready: false };
  }
  function load() {
    for (const [which, s] of Object.entries(sounds)) {
      s.ready = false;
      s.audio.src = `/adhan${which === 'fajr' ? '-fajr' : ''}-audio?v=${Date.now()}`;
      s.audio.load();
    }
  }
  let cur = sounds.main;
  window.addEventListener('island:adhan', (e) => { cur = e.detail?.prayer === 'fajr' ? sounds.fajr : sounds.main; });

  // the alert's length: the adhan and a moment after it (8 min at most, whatever the file)
  const lengthMs = () => (Number.isFinite(cur.audio.duration) && cur.audio.duration > 0)
    ? Math.min(Math.round(cur.audio.duration * 1000) + 1500, 8 * 60000)
    : 45000;

  const SPEAKER = '<svg class="s" viewBox="0 0 24 24" aria-hidden="true"><path d="M11 5 6 9H3v6h3l5 4V5Z"/><path d="M15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13"/></svg>';
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.className = 'sc-adhan';
  btn.hidden = true;
  btn.innerHTML = SPEAKER;
  $('#sc-btn')?.after(btn);

  let inAlert = false;
  let lastScene = null;
  const playing = () => Object.values(sounds).some((s) => !s.audio.paused);

  function mark() {
    const on = playing();
    btn.classList.toggle('on', on);
    btn.title = on ? 'إيقاف الأذان' : 'تشغيل الأذان';
    btn.setAttribute('aria-label', btn.title);
  }
  function play() {
    if (!cur.ready) load();
    cur.audio.currentTime = 0;
    cur.audio.play().catch(() => {});
  }
  function stop() {
    for (const s of Object.values(sounds)) {
      s.audio.pause();
      try { s.audio.currentTime = 0; } catch {}
    }
  }
  for (const s of Object.values(sounds)) {
    s.audio.addEventListener('loadedmetadata', () => { s.ready = Number.isFinite(s.audio.duration) && s.audio.duration > 0; });
    s.audio.addEventListener('error', () => { s.ready = false; });
    s.audio.addEventListener('play', mark);
    s.audio.addEventListener('pause', mark);
    // the adhan is over: close the alert scene and notify any queued Iqama alert
    s.audio.addEventListener('ended', () => {
      mark();
      if (inAlert && window.sceneShowing?.()) window.endScene?.();
      window.dispatchEvent(new CustomEvent('island:adhan-ended'));
    });
  }
  load();
  bridge.onAdhanFile?.(load);

  // remember the adhan's scene (to start it again, as long as the adhan, from the button) and show
  // the button only on it (scene.js loads after this file: wrap once every script has run)
  function wrap() {
    const playScene = window.playScene;
    if (!playScene || playScene.adhanWrapped) return;
    window.playScene = (s) => {
      const shown = playScene(s);
      inAlert = !!(shown && s.adhan);
      lastScene = inAlert ? s : null;
      btn.hidden = !(inAlert && cur.ready);
      if (!inAlert && playing()) stop();
      mark();
      return shown;
    };
    window.playScene.adhanWrapped = true;
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', wrap);
  else wrap();

  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (playing()) {
      stop();
      window.endScene?.();
      window.dispatchEvent(new CustomEvent('island:adhan-ended'));
    } else if (lastScene) {
      play();
      window.playScene({ ...lastScene, ms: lengthMs() });
    }
  });

  window.adhanWillPlay = () => !!(feat('adhanSound') && cur.ready);
  window.adhanPlaying = playing;

  // the scene's options from views/prayer.js: the sound on, it plays and sets the length
  window.adhanScene = () => {
    const base = {
      adhan: true,
      onClose: () => {
        stop();
        window.dispatchEvent(new CustomEvent('island:adhan-ended'));
      },
    };
    if (feat('adhanSound') && cur.ready) {
      play();
      return { ...base, ms: lengthMs() };
    }
    return base;
  };
  // no room for the scene (a side-docked island): the plain toast, as long as the adhan
  window.adhanToast = () => {
    if (!feat('adhanSound') || !cur.ready) return 0;
    play();
    return lengthMs();
  };
})();
