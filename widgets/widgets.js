// Desktop Widgets Frontend Logic (Widgets Window)
(() => {
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => document.querySelectorAll(s);

  const AR_DAYS = ['الأَحَد', 'الإِثْنَيْن', 'الثُّلَاثَاء', 'الأَرْبِعَاء', 'الخَمِيس', 'الجُمُعَة', 'السَّبْت'];
  const AR_DAYS_PLAIN = ['الأحد', 'الإثنين', 'الثلاثاء', 'الأربعاء', 'الخميس', 'الجمعة', 'السبت'];
  const AR_MONTHS = ['يناير', 'فبراير', 'مارس', 'أبريل', 'مايو', 'يونيو', 'يوليو', 'أغسطس', 'سبتمبر', 'أكتوبر', 'نوفمبر', 'ديسمبر'];

  const ATHKAR = [
    'سُبْحَانَ اللَّهِ وَبِحَمْدِهِ',
    'سُبْحَانَ اللَّهِ الْعَظِيمِ',
    'الْحَمْدُ لِلَّهِ رَبِّ الْعَالَمِينَ',
    'لَا إِلَهَ إِلَّا اللَّهُ وَحْدَهُ لَا شَرِيكَ لَهُ',
    'أَسْتَغْفِرُ اللَّهَ وَأَتُوبُ إِلَيْهِ',
    'لَا حَوْلَ وَلَا قُوَّةَ إِلَّا بِاللَّهِ',
    'اللَّهُمَّ صَلِّ وَسَلِّمْ عَلَى نَبِيِّنَا مُحَمَّدٍ'
  ];

  let currentAthkarIdx = 0;
  let tasbihCount = Number(localStorage.getItem('tasbih_count') || 0);
  let cachedPrayer = null;

  function pad(n) {
    return String(n).padStart(2, '0');
  }

  // --- Tasbih Logic ---
  const phraseEl = $('#tasbih-phrase');
  const countEl = $('#tasbih-count');
  const circleBtn = $('#tasbih-circle-btn');
  const nextBtn = $('#tasbih-next-btn');
  const resetBtn = $('#tasbih-reset-btn');

  function renderTasbih() {
    if (phraseEl) phraseEl.textContent = ATHKAR[currentAthkarIdx];
    if (countEl) countEl.textContent = tasbihCount;
  }

  if (circleBtn) {
    circleBtn.addEventListener('click', () => {
      tasbihCount += 1;
      localStorage.setItem('tasbih_count', tasbihCount);
      renderTasbih();
    });
  }

  if (nextBtn) {
    nextBtn.addEventListener('click', () => {
      currentAthkarIdx = (currentAthkarIdx + 1) % ATHKAR.length;
      renderTasbih();
    });
  }

  if (resetBtn) {
    resetBtn.addEventListener('click', () => {
      tasbihCount = 0;
      localStorage.setItem('tasbih_count', 0);
      renderTasbih();
    });
  }

  // --- Live Clock & Dates ---
  function updateClock() {
    const now = new Date();
    const h = pad(now.getHours());
    const m = pad(now.getMinutes());
    const s = pad(now.getSeconds());
    const dayIdx = now.getDay();
    const dayCal = AR_DAYS[dayIdx];
    const dayPlain = AR_DAYS_PLAIN[dayIdx];
    const dayNum = String(now.getDate());
    const month = AR_MONTHS[now.getMonth()];
    const year = now.getFullYear();

    let hijriStr = '';
    try {
      hijriStr = new Intl.DateTimeFormat('ar-SA-u-ca-islamic-umalqura', {
        day: 'numeric',
        month: 'long',
        year: 'numeric'
      }).format(now);
    } catch {
      hijriStr = `${now.getDate()} هـ`;
    }

    const fullDateStr = `${hijriStr} | ${dayNum} ${month} ${year}`;

    // Style 1
    const w1Date = $('#w1-date');
    const w1Hour = $('#w1-hour');
    const w1Day = $('#w1-day');
    const w1Min = $('#w1-min');
    if (w1Date) w1Date.textContent = fullDateStr;
    if (w1Hour) w1Hour.textContent = h;
    if (w1Day) w1Day.textContent = dayCal;
    if (w1Min) w1Min.textContent = m;

    // Style 2
    const w2VHour = $('#w2-v-hour');
    const w2VMin = $('#w2-v-min');
    const w2VDay = $('#w2-v-day');
    const w2Pill = $('#w2-pill-badge');
    const w2DayNum = $('#w2-day-num');
    const w2Month = $('#w2-month');
    const w2Hijri = $('#w2-hijri');
    if (w2VHour) w2VHour.textContent = h;
    if (w2VMin) w2VMin.textContent = m;
    if (w2VDay) w2VDay.textContent = dayPlain;
    if (w2Pill) w2Pill.textContent = dayCal;
    if (w2DayNum) w2DayNum.textContent = dayNum;
    if (w2Month) w2Month.textContent = month;
    if (w2Hijri) w2Hijri.textContent = hijriStr;

    // Style 3
    const w3Day = $('#w3-day');
    const w3Clock = $('#w3-clock');
    if (w3Day) w3Day.textContent = dayCal;
    if (w3Clock) w3Clock.textContent = `${h}:${m}`;

    updatePrayerCountdown();
  }

  // --- Prayer Times Logic ---
  function renderPrayers(p) {
    if (!p) return;
    cachedPrayer = p;

    const next = p.next || p.active;
    const name = next ? (next.ar || next.name || 'الصلاة') : 'الصلاة';
    const time = next ? next.time : '--:--';

    // Next Prayer card
    const npName = $('#np-name');
    const npTime = $('#np-time');
    if (npName) npName.textContent = name;
    if (npTime) npTime.textContent = time;

    // Five prayers card
    const fpList = $('#fp-list');
    const fpDate = $('#fp-date');
    if (fpDate) fpDate.textContent = p.hijri || '';
    if (fpList && Array.isArray(p.times)) {
      fpList.innerHTML = '';
      p.times.forEach((item) => {
        const isNext = next && (item.name === next.name || item.ar === next.ar);
        const div = document.createElement('div');
        div.className = `fp-item${isNext ? ' active' : ''}`;
        div.innerHTML = `<span class="fp-name">${item.ar || item.name}</span><span class="fp-time">${item.time}</span>`;
        fpList.appendChild(div);
      });
    }

    // Style 1 Prayer
    const w1Prayer = $('#w1-prayer');
    if (w1Prayer) w1Prayer.innerHTML = `<b>${name}</b> · ${time}`;

    updatePrayerCountdown();
  }

  function updatePrayerCountdown() {
    if (!cachedPrayer) return;
    const next = cachedPrayer.next || cachedPrayer.active;
    if (!next) return;

    let leftSec = typeof next.secondsLeft === 'number' ? next.secondsLeft : 0;
    if (leftSec > 0) {
      const h = Math.floor(leftSec / 3600);
      const m = Math.floor((leftSec % 3600) / 60);
      const s = leftSec % 60;
      const cdStr = `${pad(h)}:${pad(m)}:${pad(s)}`;

      const npCd = $('#np-countdown');
      if (npCd) npCd.textContent = cdStr;

      const w3Prayer = $('#w3-prayer');
      if (w3Prayer) w3Prayer.innerHTML = `<b>${next.ar || next.name}</b> · باقي ${h}:${pad(m)}`;
    }

    const w3Dot = $('#w3-dot');
    if (w3Dot) {
      const now = new Date();
      const pctOfDay = ((now.getHours() * 60 + now.getMinutes()) / 1440) * 100;
      w3Dot.style.left = `${Math.min(95, Math.max(5, pctOfDay)).toFixed(1)}%`;
    }
  }

  // --- Features & Toggles ---
  function applyFeatures(f) {
    if (!f) return;
    const wrap = $('#widgets-wrap');
    if (wrap) {
      wrap.dataset.size = f.widgetSize || 'medium';
    }

    const toggle = (sel, show) => {
      const el = $(sel);
      if (el) el.hidden = !show;
    };

    toggle('#w-tasbih', f.widgetTasbih !== false);
    toggle('#w-next-prayer', f.widgetNextPrayer !== false);
    toggle('#w-five-prayers', f.widgetFivePrayers === true);
    toggle('#w-style1', f.widgetStyle1 !== false);
    toggle('#w-style2', f.widgetStyle2 === true);
    toggle('#w-style3', f.widgetStyle3 === true);
  }

  // --- Bridge Handlers ---
  if (window.widgetsBridge) {
    widgetsBridge.onFeatures(applyFeatures);
    widgetsBridge.onPrayer(renderPrayers);

    widgetsBridge.getFeatures().then(applyFeatures).catch(() => {});
    widgetsBridge.getPrayer().then(renderPrayers).catch(() => {});
  }

  renderTasbih();
  updateClock();
  setInterval(updateClock, 1000);
})();
