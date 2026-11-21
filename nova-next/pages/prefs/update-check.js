// Settings › General › Check for updates (1.8.6) & Fork updates (1.8.10):
// Original check keeps querying NOVA-TECH-KIT/nova-island; the fork row checks sem233/nova-island.
// Text is English; prefs-i18n.js applies Arabic translations dynamically.
(() => {
  if (!window.prefs?.checkUpdates) return;

  let current = '';
  let lastCheckedSource = null;

  function makeUpdaterRow({ rowId, btnId, source, checkLabel, idleMsg, showVersion }) {
    const row = document.getElementById(rowId);
    const btn = document.getElementById(btnId);
    const msg = row?.querySelector('.upd-msg');
    const ver = row?.querySelector('.upd-ver');
    if (!row || !btn || !msg || !ver) return null;

    let rowReady = null;
    const say = (text, version, tone) => {
      msg.textContent = text;
      ver.textContent = version || '';
      row.dataset.tone = tone || '';
    };
    const idle = () => {
      btn.disabled = false;
      btn.textContent = rowReady ? 'Update now' : checkLabel;
      btn.classList.toggle('accent', !!rowReady);
    };

    const failText = (reason) => {
      if (reason === 'not-found') {
        return source === 'fork'
          ? 'Repository is private or release manifest is missing (HTTP 404)'
          : 'No release manifest found on upstream (HTTP 404)';
      }
      if (reason === 'checksum') return 'Downloaded file verification failed (SHA-512 mismatch)';
      if (reason === 'invalid-manifest') return 'Release manifest could not be read';
      return "Couldn't check. Check your internet connection and try again";
    };

    btn.addEventListener('click', () => {
      if (rowReady) {
        btn.disabled = true;
        btn.textContent = 'Updating…';
        window.prefs.updateNow();
        return;
      }
      lastCheckedSource = source;
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
        if (!rowReady && !row.dataset.tone) {
          say(idleMsg, showVersion ? verStr : '', '');
        }
      },
      onResult: (r) => {
        if (r?.state === 'ready') {
          rowReady = r.version;
          say('A new version is ready', r.version, 'new');
        } else if (r?.state === 'latest') {
          say('You have the latest version', current || r.version, 'ok');
        } else {
          say(failText(r?.reason), '', 'bad');
        }
        idle();
      },
      onReady: (v, failed) => {
        if (failed) {
          if (!v) rowReady = null;
          idle();
          return;
        }
        if (!v) return;
        rowReady = v;
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
    const src = r?.source || lastCheckedSource || 'official';
    const target = rows.find((item) => item.source === src) || rows[0];
    target?.onResult(r);
  });

  window.prefs.onUpdate?.((u) => {
    const src = u?.source || lastCheckedSource || 'official';
    if (u?.failed) {
      rows.forEach((item) => item.onReady(u.version, true));
      return;
    }
    if (u?.version) {
      const target = rows.find((item) => item.source === src) || rows[0];
      target?.onReady(u.version, false);
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
    if (s?.ready) {
      const target = rows.find((item) => item.source === (s.source || 'official')) || rows[0];
      target?.onReady(s.ready, false);
    }
  });
})();
