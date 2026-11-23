// Customizable Pre-Prayer & Iqama alerts with selectable notification sounds (1.8.15).
// Invoked from views/prayer.js's existing 1-second tick — zero extra intervals or threads.
(() => {
  const arMins = (m) => {
    if (m === 1) return 'دقيقة';
    if (m === 2) return 'دقيقتين';
    return `${m} ${m >= 3 && m <= 10 ? 'دقائق' : 'دقيقة'}`;
  };

  window.prayerExtraRemind = (d, now, at, clock, NAMES, fired) => {
    const f = window.state?.features || {};
    const soonMin = Math.max(0, Math.min(60, Number(f.prayerSoonMin ?? 10) || 0));
    const soonSound = f.prayerSoonSound || 'bell';
    const iqamaOn = f.iqamaAlert !== false;
    const iqamaDelay = Math.max(5, Math.min(45, Number(f.iqamaDelayMin ?? 15) || 15));
    const iqamaBefore = Math.max(0, Math.min(15, Number(f.iqamaBeforeMin ?? 2) || 0));
    const iqamaSound = f.iqamaSound || 'crystal';
    const PRAYERS = ['fajr', 'dhuhr', 'asr', 'maghrib', 'isha'];

    for (const k of PRAYERS) {
      const t = at(d.today.date, d.today.times[k]);
      const diff = t - now;
      const id = `${d.today.date}-${k}`;

      // 1) Pre-prayer alert (custom minutes before prayer + chosen alert sound)
      if (soonMin > 0 && diff > 0 && diff <= soonMin * 60000 && diff > (soonMin - 1) * 60000 && !fired.has(`${id}-soon`)) {
        fired.add(`${id}-soon`);
        window.playPrayerSound?.(soonSound);
        note('🕌', `${NAMES[k]} بعد ${arMins(soonMin)} (${clock(t)})`);
        showToast('prayer-soon', `${NAMES[k]} بعد ${arMins(soonMin)}`, clock(t), 0, 6500);
      }

      // 2) Iqama alert (custom minutes after adhan, triggered N minutes before iqama or at iqama)
      if (iqamaOn) {
        const iqamaTime = new Date(t.getTime() + iqamaDelay * 60000);
        const alertAt = iqamaTime.getTime() - iqamaBefore * 60000;
        const iqDiff = alertAt - now.getTime();
        if (iqDiff <= 0 && iqDiff > -60000 && !fired.has(`${id}-iqama`)) {
          fired.add(`${id}-iqama`);
          window.playPrayerSound?.(iqamaSound);
          const msg = iqamaBefore > 0
            ? `إقامة ${NAMES[k]} بعد ${arMins(iqamaBefore)}`
            : `حان وقت إقامة صلاة ${NAMES[k]}`;
          note('🕌', `${msg} (${clock(iqamaTime)})`);
          island.classList.add('prayer-alert');
          showToast('prayer', msg, clock(iqamaTime), 0, 7500);
          setTimeout(() => island.classList.remove('prayer-alert'), 7700);
        }
      }
    }
  };
})();
