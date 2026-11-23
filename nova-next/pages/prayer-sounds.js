// Shared synthesized notification sounds for Prayer & Iqama alerts (1.8.15).
// Uses one shared Web Audio API context — zero MP3 RAM footprint, works offline in island & Settings.
(() => {
  let ctx = null;
  const PROFILES = {
    bell: { notes: [[0, 784], [0.32, 659.25], [0.64, 523.25]], opts: { peak: 0.2, attack: 0.025, decay: 1.35, type: 'sine' } },
    chime: { notes: [[0, 587.33], [0.2, 880], [0.42, 1174.66]], opts: { peak: 0.17, attack: 0.02, decay: 0.95, type: 'triangle' } },
    crystal: { notes: [[0, 880], [0.16, 1318.5], [0.34, 1108.73], [0.54, 1760]], opts: { peak: 0.15, attack: 0.015, decay: 0.85, type: 'sine' } },
    takbir: { notes: [[0, 698.46], [0.18, 880], [0.52, 698.46], [0.7, 1046.5]], opts: { peak: 0.2, attack: 0.02, decay: 0.65, type: 'sine' } },
    pulse: { notes: [[0, 523.25], [0.3, 659.25]], opts: { peak: 0.16, attack: 0.04, decay: 0.9, type: 'sine' } },
  };

  window.playPrayerSound = (kind = 'bell') => {
    if (!kind || kind === 'silent') return;
    const p = PROFILES[kind] || PROFILES.bell;
    if (typeof window.playTones === 'function') {
      window.playTones(p.notes, p.opts);
      return;
    }
    try {
      ctx ||= new AudioContext();
      if (ctx.state === 'suspended') ctx.resume();
      const t0 = ctx.currentTime;
      const { peak, attack, decay, type } = p.opts;
      for (const [d, f] of p.notes) {
        const o = ctx.createOscillator();
        const g = ctx.createGain();
        o.type = type;
        o.frequency.value = f;
        g.gain.setValueAtTime(0.0001, t0 + d);
        g.gain.exponentialRampToValueAtTime(peak, t0 + d + attack);
        g.gain.exponentialRampToValueAtTime(0.0001, t0 + d + decay);
        o.connect(g).connect(ctx.destination);
        o.start(t0 + d);
        o.stop(t0 + d + decay + 0.05);
      }
    } catch {}
  };
})();
