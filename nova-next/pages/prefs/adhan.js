// (1.8.7 / 1.8.15) Settings › Prayer & Dhikr:
// 1. Adhan audio files (local pick, play/stop, reset, plus curated online Adhan download & cache).
// 2. Pre-prayer & Iqama alert sound selectors with instant Web Audio preview.
(() => {
  const api = window.prefs;
  if (!api) return;

  // ---- Pre-prayer & Iqama sound selectors + Preview buttons ----
  const soundSelects = [...document.querySelectorAll('select[data-select-feature]')];
  function syncSelects(f) {
    if (!f) return;
    for (const sel of soundSelects) {
      const key = sel.dataset.selectFeature;
      if (f[key] != null && [...sel.options].some((o) => o.value === f[key])) {
        sel.value = f[key];
      }
    }
  }
  for (const sel of soundSelects) {
    sel.addEventListener('change', () => {
      const key = sel.dataset.selectFeature;
      api.setFeatures({ [key]: sel.value });
      if (typeof saved === 'function') saved();
      if (sel.id.includes('sound') && sel.value !== 'silent') window.playPrayerSound?.(sel.value);
    });
  }
  for (const btn of document.querySelectorAll('.pr-test[data-sound-for]')) {
    btn.addEventListener('click', () => {
      const sel = document.getElementById(btn.dataset.soundFor);
      if (sel) window.playPrayerSound?.(sel.value);
    });
  }
  const testPrayerBtn = document.getElementById('prayer-test-btn');
  if (testPrayerBtn && api.prayerTest) {
    testPrayerBtn.addEventListener('click', () => api.prayerTest());
  }
  api.onFeatures?.(syncSelects);
  api.features?.().then(syncSelects);

  // ---- Adhan audio rows (local files + curated online Adhan presets) ----
  if (!api.adhan) return;
  const audio = new Audio();
  let playing = null; // the row whose adhan plays

  function stop() {
    audio.pause();
    if (playing) playing.querySelector('.adhan-play').textContent = 'Play';
    playing = null;
  }
  audio.addEventListener('ended', stop);
  audio.addEventListener('error', stop);

  let presetsFilled = false;
  function fillPresets(presets) {
    if (presetsFilled || !Array.isArray(presets) || !presets.length) return;
    presetsFilled = true;
    for (const row of document.querySelectorAll('.adhan-row')) {
      const which = row.dataset.adhan;
      const sel = row.querySelector('.adhan-preset');
      if (!sel) continue;
      sel.innerHTML = '';
      const list = which === 'fajr'
        ? [...presets.filter((p) => p.fajrOnly), ...presets.filter((p) => !p.fajrOnly)]
        : presets.filter((p) => !p.fajrOnly);
      for (const p of list) {
        const opt = document.createElement('option');
        opt.value = p.id;
        opt.textContent = p.en;
        sel.append(opt);
      }
    }
  }

  function show(info) {
    if (!info) return;
    fillPresets(info.presets);
    for (const row of document.querySelectorAll('.adhan-row')) {
      const which = row.dataset.adhan;
      const i = info[which];
      if (!i) continue;
      const usesMain = which === 'fajr' && !i.custom && !i.hasDefault;
      const main = info.main;
      row.querySelector('.adhan-name').textContent = i.custom ? i.name
        : i.hasDefault ? (which === 'fajr' ? "Nova's Fajr adhan" : "Nova's adhan")
          : usesMain && (main.custom || main.hasDefault) ? 'Same as the adhan' : 'No file yet';
      row.querySelector('.adhan-reset').hidden = !i.custom;
      row.querySelector('.adhan-play').disabled = !(i.custom || i.hasDefault || main.custom || main.hasDefault);
    }
  }
  const refresh = async () => show(await api.adhan());

  for (const row of document.querySelectorAll('.adhan-row')) {
    const which = row.dataset.adhan;
    const play = row.querySelector('.adhan-play');
    const nameEl = row.querySelector('.adhan-name');
    play.addEventListener('click', () => {
      const was = playing;
      stop();
      if (was === row) return;
      audio.src = `/adhan${which === 'fajr' ? '-fajr' : ''}-audio?v=${Date.now()}`;
      audio.play().then(() => { playing = row; play.textContent = 'Stop'; }).catch(stop);
    });
    row.querySelector('.adhan-pick').addEventListener('click', async () => {
      stop();
      const r = await api.adhanPick(which);
      if (r && r.error) {
        nameEl.textContent = r.error === 'size' ? 'The file is too large (30 MB at most)'
          : r.error === 'type' ? 'Choose an audio file (mp3, m4a, ogg, wav)' : "Couldn't use the file";
      } else refresh();
    });
    row.querySelector('.adhan-reset').addEventListener('click', async () => {
      stop();
      await api.adhanReset(which);
      refresh();
    });
    const dlBtn = row.querySelector('.adhan-dl');
    const presetSel = row.querySelector('.adhan-preset');
    if (dlBtn && presetSel && api.adhanDownload) {
      dlBtn.addEventListener('click', async () => {
        const id = presetSel.value;
        if (!id) return;
        stop();
        dlBtn.disabled = true;
        const prevTxt = dlBtn.textContent;
        dlBtn.textContent = 'Downloading…';
        try {
          const r = await api.adhanDownload(which, id);
          if (r && r.ok) {
            api.setFeatures({ [which === 'fajr' ? 'adhanFajrPreset' : 'adhanPreset']: id, adhanSound: true });
            if (typeof saved === 'function') saved('Downloaded');
            await refresh();
          } else {
            nameEl.textContent = r?.error === 'invalid_audio' ? 'Downloaded audio file was invalid' : 'Download failed. Check your connection.';
          }
        } catch {
          nameEl.textContent = 'Download failed. Check your connection.';
        } finally {
          dlBtn.disabled = false;
          dlBtn.textContent = prevTxt;
        }
      });
    }
  }
  refresh();
})();
