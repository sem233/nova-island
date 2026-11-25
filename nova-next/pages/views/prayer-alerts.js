// Customizable Pre-Prayer & Iqama alerts with selectable notification sounds (1.8.15 / 1.8.16).
// Invoked from views/prayer.js's tick & native Rust TIMER_PRAYER wake — zero extra intervals.
(() => {
  const bootAt = Date.now();
  let featuresLoaded = false;
  let queuedIqama = null;

  window.addEventListener('island:features', () => { featuresLoaded = true; });
  if (window.state?.features && Object.keys(window.state.features).length > 0) {
    featuresLoaded = true;
  }

  const arMins = (m) => {
    if (m === 1) return 'دقيقة';
    if (m === 2) return 'دقيقتين';
    return `${m} ${m >= 3 && m <= 10 ? 'دقائق' : 'دقيقة'}`;
  };

  function presentToast(kind, msg, timeText, sound, glow, ms) {
    window.playPrayerSound?.(sound);
    note('🕌', `${msg} (${timeText})`);
    if (glow) {
      island.classList.add('prayer-alert');
      setTimeout(() => island.classList.remove('prayer-alert'), ms + 250);
    }
    showToast(kind, msg, timeText, 0, ms);
    // Clickable dismissal keeps the prayer alert visible even over Chrome tabs or hover.
    state.toastAction = () => {
      state.toastUntil = 0;
      render();
    };
    render();
    if (typeof reportRect === 'function') reportRect();
  }

  function deliverIqama(item) {
    if (!item) return;
    // If Adhan audio is actively playing in the scene, wait for it to finish rather than
    // being swallowed silently behind the scene or talking over the Adhan.
    if (window.sceneShowing?.()) {
      if (window.adhanPlaying?.()) {
        queuedIqama = item;
        return;
      }
      window.endScene?.();
    }
    presentToast('prayer', item.msg, item.timeText, item.sound, true, 8500);
  }

  window.addEventListener('island:adhan-ended', () => {
    if (!queuedIqama) return;
    const item = queuedIqama;
    queuedIqama = null;
    setTimeout(() => deliverIqama(item), 350);
  });

  window.prayerExtraRemind = (d, now, at, clock, NAMES, fired) => {
    if (!featuresLoaded) return;
    const f = window.state?.features || {};
    const soonMin = Math.max(0, Math.min(60, Number(f.prayerSoonMin ?? 10) || 0));
    const soonSound = f.prayerSoonSound || 'bell';
    const iqamaOn = f.iqamaAlert !== false;
    const iqamaDelay = Math.max(5, Math.min(45, Number(f.iqamaDelayMin ?? 15) || 15));
    const iqamaBefore = Math.max(0, Math.min(15, Number(f.iqamaBeforeMin ?? 2) || 0));
    const iqamaSound = f.iqamaSound || 'crystal';
    const PRAYERS = ['fajr', 'dhuhr', 'asr', 'maghrib', 'isha'];
    const nowMs = now.getTime();
    const CATCHUP_MS = 120_000;

    for (const k of PRAYERS) {
      const t = at(d.today.date, d.today.times[k]);
      const tMs = t.getTime();
      const id = `${d.today.date}-${k}`;

      // 1) Pre-prayer alert (fires at adhan - soonMin, with 120s catch-up; skips alerts from before app boot)
      if (soonMin > 0) {
        const soonAt = tMs - soonMin * 60000;
        const soonDiff = soonAt - nowMs;
        const soonKey = `${id}-soon-${soonMin}`;
        if (soonDiff <= 0 && soonDiff > -CATCHUP_MS && nowMs < tMs - 15000 && soonAt >= bootAt - 15000 && !fired.has(soonKey)) {
          fired.add(soonKey);
          const leftMin = Math.max(1, Math.round((tMs - nowMs) / 60000));
          presentToast('prayer-soon', `${NAMES[k]} بعد ${arMins(leftMin)}`, clock(t), soonSound, false, 8000);
        }
      }

      // 2) Iqama alert (custom delay after adhan, triggered N min before iqama or at iqama)
      if (iqamaOn) {
        const iqamaMs = tMs + iqamaDelay * 60000;
        const alertAt = tMs + Math.max(1, iqamaDelay - iqamaBefore) * 60000;
        const iqDiff = alertAt - nowMs;
        const iqKey = `${id}-iqama-${iqamaDelay}-${iqamaBefore}`;
        if (iqDiff <= 0 && iqDiff > -CATCHUP_MS && alertAt >= bootAt - 15000 && !fired.has(iqKey)) {
          fired.add(iqKey);
          const msg = iqamaBefore > 0
            ? `إقامة ${NAMES[k]} بعد ${arMins(iqamaBefore)}`
            : `حان وقت إقامة صلاة ${NAMES[k]}`;
          deliverIqama({ msg, timeText: clock(new Date(iqamaMs)), sound: iqamaSound });
        }
      }
    }
  };

  // Live preview from Settings › Prayer & Dhikr › "Test prayer alert"
  window.testPrayerAlert = () => {
    const f = window.state?.features || {};
    const soonMin = Math.max(1, Number(f.prayerSoonMin ?? 5) || 5);
    const sound = f.prayerSoonSound && f.prayerSoonSound !== 'silent'
      ? f.prayerSoonSound
      : (f.iqamaSound && f.iqamaSound !== 'silent' ? f.iqamaSound : 'bell');
    const nextText = window.nextPrayerText?.() || 'الصلاة القادمة';
    presentToast('prayer-soon', `تجربة التنبيه · ${nextText}`, `قبل ${arMins(soonMin)}`, sound, true, 6500);
  };
})();
