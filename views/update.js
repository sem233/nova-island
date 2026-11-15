// Updates View: checks GitHub releases (sem233/nova-island), streams download,
// shows live progress, and restarts into the new version.
(() => {
  const COUNTDOWN_S = 6;
  let timer = null;
  let checking = false;

  const headerBtn = $('#open-update');
  const sysBtn = $('#sys-update-btn');
  const sysStatus = $('#sys-update-status');
  const progWrap = $('#sys-update-prog');
  const progBar = $('#sys-update-bar');
  const progLbl = $('#sys-update-prog-lbl');
  const updateDot = $('#update-dot');

  function setChecking(on) {
    checking = on;
    if (headerBtn) headerBtn.classList.toggle('spinning', on);
    if (sysBtn) sysBtn.classList.toggle('spinning', on);
    if (sysStatus && on) sysStatus.textContent = 'Checking for updates on GitHub… (جاري البحث)';
  }

  function startCountdown(version) {
    if (timer) return;
    let left = COUNTDOWN_S;
    const show = () => {
      showToast('updating', `Updating to ${version} · Restarting`, `${left}s`, 0, left * 1000 + 800);
      state.toastAction = show;
    };
    show();
    if (sysStatus) sysStatus.textContent = `Update ready! Restarting in ${left}s…`;

    timer = setInterval(() => {
      left -= 1;
      if (left > 0) {
        $('#t-value').textContent = `${left}s`;
        $('#vt-value').textContent = `${left}s`;
        if (sysStatus) sysStatus.textContent = `Update ready! Restarting in ${left}s…`;
        return;
      }
      clearInterval(timer);
      timer = null;
      bridge.installUpdate();
    }, 1000);
  }

  async function triggerUpdateFlow(force = false) {
    if (checking) return;
    setChecking(true);

    try {
      const res = await bridge.updater.check(force);
      setChecking(false);

      if (!res.ok) {
        const errMsg = res.error || 'Failed to check updates';
        if (sysStatus) sysStatus.textContent = `Check failed: ${errMsg}`;
        showToast('err', 'تعذر فحص التحديثات', errMsg.slice(0, 30), 0, 4500);
        return;
      }

      if (!res.available) {
        if (sysStatus) sysStatus.textContent = `Nova Island is up to date (${res.currentVersion || res.version})`;
        showToast('check', 'التطبيق على أحدث إصدار', res.version, 1, 3500);
        return;
      }

      // Update available!
      if (updateDot) updateDot.hidden = false;
      if (sysStatus) sysStatus.textContent = `Found ${res.version} (${res.sizeMb} MB). Downloading…`;
      showToast('dl-start', `تحديث جديد ${res.version}`, `حجم ${res.sizeMb} MB · جاري التحميل`, 0, 4000);

      if (progWrap) progWrap.hidden = false;
      if (progBar) progBar.style.width = '0%';
      if (progLbl) progLbl.textContent = '0%';

      // Start downloading
      const dlRes = await bridge.updater.download();
      if (!dlRes.ok) {
        if (sysStatus) sysStatus.textContent = `Download error: ${dlRes.error}`;
        showToast('err', 'فشل تحميل التحديث', dlRes.error || '', 0, 4500);
        if (progWrap) progWrap.hidden = true;
        return;
      }

      // Download complete!
      if (progWrap) progWrap.hidden = true;
      if (updateDot) updateDot.hidden = true;
      startCountdown(dlRes.version);
    } catch (err) {
      setChecking(false);
      if (sysStatus) sysStatus.textContent = `Error: ${err.message}`;
      showToast('err', 'خطأ في التحديث', err.message, 0, 4000);
    }
  }

  // Progress listener
  if (bridge.updater && bridge.updater.onProgress) {
    bridge.updater.onProgress((p) => {
      const pct = p.pct || 0;
      if (progWrap) progWrap.hidden = false;
      if (progBar) progBar.style.width = `${pct}%`;
      if (progLbl) progLbl.textContent = `${pct}% (${p.receivedMb}/${p.totalMb} MB)`;
      if (sysStatus) sysStatus.textContent = `Downloading update: ${pct}% (${p.receivedMb}/${p.totalMb} MB)`;
    });
  }

  // Background update ready notification
  bridge.onUpdate((u) => {
    if (timer) return;
    if (updateDot) updateDot.hidden = false;
    showToast('update', `Update ready · ${u.version}`, 'Restart now', 0, 20000);
    state.toastAction = () => startCountdown(u.version);
  });

  // Header button click
  if (headerBtn) {
    headerBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      triggerUpdateFlow(true);
    });
  }

  // System view button click
  if (sysBtn) {
    sysBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      triggerUpdateFlow(true);
    });
  }
})();
