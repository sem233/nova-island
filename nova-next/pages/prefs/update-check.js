// Settings › General › Check for updates (1.8.6) & Fork updates (1.8.10):
// Original check keeps querying NOVA-TECH-KIT/nova-island; the fork row checks sem233/nova-island.
// Text is English; prefs-i18n.js applies Arabic translations dynamically.
(() => {
  if (!window.prefs?.checkUpdates) return;

  let current = '';
  let ready = null;
  let activeSource = null;

  function makeUpdaterRow({ rowId, btnId, source, checkLabel, idleMsg, showVersion }) {
    const row = document.getElementById(rowId);
    const btn = document.getElementById(btnId);
    const msg = row?.querySelector('.upd-msg');
    const ver = row?.querySelector('.upd-ver');
    if (!row || !btn || !msg || !ver) return null;

    const say = (text, version, tone) => {
      msg.textContent = text;
      ver.textContent = version || '';
      row.dataset.tone = tone || '';
    };
    const idle = () => {
      btn.disabled = false;
      btn.textContent = ready ? 'Update now' : checkLabel;
      btn.classList.toggle('accent', !!ready);
    };

    btn.addEventListener('click', () => {
      if (ready) {
        btn.disabled = true;
        btn.textContent = 'Updating…';
        window.prefs.updateNow();
        return;
      }
      activeSource = source;
      btn.disabled = true;
      btn.textContent = 'Checking…';
      say('Checking…', '', '');
      window.prefs.checkUpdates(source);
    });

    return {
      source,
      setStoreHidden: (store) => {
        row.hidden = !!store;
      },
      onGeneral: (verStr) => {
        row.hidden = false;
        if (!ready && !row.dataset.tone) {
          say(idleMsg, showVersion ? verStr : '', '');
        }
      },
      onResult: (r) => {
        if (r?.state === 'ready') {
          ready = r.version;
          say('A new version is ready', r.version, 'new');
        } else if (r?.state === 'latest') {
          say('You have the latest version', current || r.version, 'ok');
        } else {
          say("Couldn't check. Check your internet connection and try again", '', 'bad');
        }
        idle();
      },
      onReady: (v) => {
        if (!v) return;
        ready = v;
        say('A new version is ready', v, 'new');
        idle();
      },
    };
  }

  const rows = [
    makeUpdaterRow({
      rowId: 'update-check-row',
      btnId: 'upd-check',
      source: 'official',
      checkLabel: 'Check now',
      idleMsg: 'Version',
      showVersion: true,
    }),
    makeUpdaterRow({
      rowId: 'update-check-fork-row',
      btnId: 'upd-check-fork',
      source: 'fork',
      checkLabel: 'Check fork',
      idleMsg: 'sem233/nova-island',
      showVersion: false,
    }),
  ].filter(Boolean);

  if (!rows.length) return;

  window.prefs.onUpdateCheck?.((r) => {
    const src = r?.source || activeSource || 'official';
    const target = rows.find((item) => item.source === src) || rows[0];
    target?.onResult(r);
  });

  window.prefs.onUpdate?.((u) => {
    if (u?.version) {
      const target = rows.find((item) => item.source === activeSource) || rows[0];
      target?.onReady(u.version);
    }
  });

  const general = (g) => {
    if (!g) return;
    current = g.version || current;
    rows.forEach((item) => {
      item.setStoreHidden(g.store);
      if (!g.store) item.onGeneral(current);
    });
  };
  window.prefs.general().then(general);
  window.prefs.onGeneral?.(general);
  window.prefs.updateState?.().then((s) => {
    if (s?.ready) rows[0]?.onReady(s.ready);
  });
})();
