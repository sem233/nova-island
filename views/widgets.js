// Widgets view: Islamic Clock & Prayer Widgets from user designs with S/M/L size controls.
(() => {
  const AR_DAYS = ['الأَحَد', 'الإِثْنَيْن', 'الثُّلَاثَاء', 'الأَرْبِعَاء', 'الخَمِيس', 'الجُمُعَة', 'السَّبْت'];
  const AR_DAYS_PLAIN = ['الأحد', 'الإثنين', 'الثلاثاء', 'الأربعاء', 'الخميس', 'الجمعة', 'السبت'];
  const AR_MONTHS = ['يناير', 'فبراير', 'مارس', 'أبريل', 'مايو', 'يونيو', 'يوليو', 'أغسطس', 'سبتمبر', 'أكتوبر', 'نوفمبر', 'ديسمبر'];

  let cachedPrayer = null;

  function pad(n) {
    return String(n).padStart(2, '0');
  }

  function updateClock() {
    const now = new Date();
    const h = pad(now.getHours());
    const m = pad(now.getMinutes());
    const dayIdx = now.getDay();
    const dayCal = AR_DAYS[dayIdx];
    const dayPlain = AR_DAYS_PLAIN[dayIdx];
    const dayNum = String(now.getDate());
    const month = AR_MONTHS[now.getMonth()];
    const year = now.getFullYear();

    // Hijri date string
    let hijriStr = '';
    try {
      const hParts = new Intl.DateTimeFormat('ar-SA-u-ca-islamic-umalqura', {
        day: 'numeric',
        month: 'long',
        year: 'numeric'
      }).format(now);
      hijriStr = hParts;
    } catch {
      hijriStr = `${now.getDate()} هـ`;
    }

    const fullDateStr = `${hijriStr} | ${dayNum} ${month} ${year}`;

    // Update Style 1 elements
    const w1Date = $('#w1-date');
    const w1Hour = $('#w1-hour');
    const w1Day = $('#w1-day');
    const w1Min = $('#w1-min');
    if (w1Date) w1Date.textContent = fullDateStr;
    if (w1Hour) w1Hour.textContent = h;
    if (w1Day) w1Day.textContent = dayCal;
    if (w1Min) w1Min.textContent = m;

    // Update Style 2 elements
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

    // Update Style 3 elements
    const w3Day = $('#w3-day');
    const w3Clock = $('#w3-clock');
    if (w3Day) w3Day.textContent = dayCal;
    if (w3Clock) w3Clock.textContent = `${h}:${m}`;

    updatePrayerDetails();
  }

  function updatePrayerDetails() {
    if (!cachedPrayer) return;

    const next = cachedPrayer.next || cachedPrayer.active;
    const name = next ? (next.ar || next.name || 'الصلاة') : 'الصلاة';
    const time = next ? next.time : '--:--';
    const leftSec = next && typeof next.secondsLeft === 'number' ? next.secondsLeft : 0;

    const leftH = Math.floor(leftSec / 3600);
    const leftM = Math.floor((leftSec % 3600) / 60);
    const countdown = `${leftH}:${pad(leftM)}`;

    // Style 1 Prayer info
    const w1Prayer = $('#w1-prayer');
    if (w1Prayer) {
      w1Prayer.innerHTML = `<b>${name}</b> · ${time}`;
    }

    // Style 3 Prayer info and Timeline dot
    const w3Prayer = $('#w3-prayer');
    if (w3Prayer) {
      w3Prayer.innerHTML = `<b>${name}</b> · باقي ${countdown}`;
    }

    const w3Dot = $('#w3-dot');
    if (w3Dot) {
      // Progress calculation throughout day or between prayers
      const now = new Date();
      const pctOfDay = ((now.getHours() * 60 + now.getMinutes()) / 1440) * 100;
      w3Dot.style.left = `${Math.min(95, Math.max(5, pctOfDay)).toFixed(1)}%`;
    }
  }

  // Prayer updates from bridge
  if (bridge.onPrayer) {
    bridge.onPrayer((p) => {
      cachedPrayer = p;
      updatePrayerDetails();
    });
  }

  if (bridge.prayerSnapshot) {
    bridge.prayerSnapshot().then((p) => {
      cachedPrayer = p;
      updatePrayerDetails();
    }).catch(() => {});
  }

  // Toolbar Handlers: Style Switcher & Size Control
  function initControls() {
    const container = $('#widgets-container');
    if (!container) return;

    // Style selection
    const styleBtns = $$('.wstyle-btn');
    styleBtns.forEach((btn) => {
      btn.addEventListener('click', () => {
        styleBtns.forEach((b) => b.classList.remove('active'));
        btn.classList.add('active');

        const style = btn.dataset.style;
        const c1 = $('#wcard-1');
        const c2 = $('#wcard-2');
        const c3 = $('#wcard-3');

        if (style === 'all') {
          if (c1) c1.hidden = false;
          if (c2) c2.hidden = false;
          if (c3) c3.hidden = false;
        } else {
          if (c1) c1.hidden = style !== '1';
          if (c2) c2.hidden = style !== '2';
          if (c3) c3.hidden = style !== '3';
        }
      });
    });

    // Size selection (Small, Medium, Large)
    const sizeBtns = $$('.wsize-btn');
    sizeBtns.forEach((btn) => {
      btn.addEventListener('click', () => {
        sizeBtns.forEach((b) => b.classList.remove('active'));
        btn.classList.add('active');

        const size = btn.dataset.size;
        container.dataset.size = size;
      });
    });
  }

  // Start ticker
  updateClock();
  setInterval(updateClock, 1000);
  initControls();
})();
