/* ============================================================
   Island Calculator add-on (views/calculator.js)
   Safe hand-written recursive-descent arithmetic evaluator and
   lightweight popover controller attached to the expanded topbar.
   ============================================================ */
(() => {
  const MAX_LEN = 30;
  const PANEL_W = 260;
  const PANEL_H = 340;
  const PANEL_MARGIN = 6;
  const BACKSPACE_DELAY_MS = 380;
  const BACKSPACE_REPEAT_MS = 55;
  const KEY_FLASH_MS = 80;

  const I18N = {
    en: {
      title: 'Calculator',
      close: 'Close calculator',
      clear: 'Clear',
      backspace: 'Backspace',
      divide: 'Divide',
      multiply: 'Multiply',
      subtract: 'Subtract',
      add: 'Add',
      equals: 'Equals',
      decimal: 'Decimal point',
      div0: "Can't divide by zero",
      overflow: 'Result too large',
    },
    ar: {
      title: 'الآلة الحاسبة',
      close: 'إغلاق الآلة الحاسبة',
      clear: 'مسح',
      backspace: 'حذف',
      divide: 'قسمة',
      multiply: 'ضرب',
      subtract: 'طرح',
      add: 'جمع',
      equals: 'يساوي',
      decimal: 'فاصلة عشرية',
      div0: 'لا يمكن القسمة على صفر',
      overflow: 'النتيجة أكبر من الحد',
    },
  };

  function normalizeChar(ch) {
    if (!ch || ch.length !== 1) return null;
    const code = ch.charCodeAt(0);
    if (code >= 0x30 && code <= 0x39) return ch;
    if (code >= 0x0660 && code <= 0x0669) return String.fromCharCode(code - 0x0660 + 0x30);
    if (code >= 0x06f0 && code <= 0x06f9) return String.fromCharCode(code - 0x06f0 + 0x30);
    if (ch === '.' || ch === '\u066b') return '.';
    if (ch === '+') return '+';
    if (ch === '-' || ch === '\u2212') return '−';
    if (ch === '*' || ch === '×' || ch === 'x' || ch === 'X') return '×';
    if (ch === '/' || ch === '÷') return '÷';
    return null;
  }

  function isOp(ch) {
    return ch === '+' || ch === '−' || ch === '×' || ch === '÷';
  }

  function findLastBinaryOp(expr) {
    for (let i = expr.length - 1; i >= 0; i--) {
      const c = expr[i];
      if (!isOp(c)) continue;
      if (i === 0 && c === '−') continue; // leading unary minus
      if (i > 0 && (expr[i - 1] === 'e' || expr[i - 1] === 'E')) continue; // exponent sign inside committed scientific number
      return i;
    }
    return -1;
  }

  function endsWithBinaryOp(expr) {
    if (!expr) return false;
    const last = expr.length - 1;
    if (!isOp(expr[last])) return false;
    if (last === 0 && expr[last] === '−') return false;
    return true;
  }

  function formatNumber(n) {
    if (typeof n !== 'number' || !Number.isFinite(n)) return null;
    if (n === 0 || Object.is(n, -0)) return '0';
    const rounded = Number(n.toPrecision(12));
    if (!Number.isFinite(rounded)) return null;
    if (rounded === 0) return '0';
    let ascii;
    if (Math.abs(rounded) >= 1e15 || Math.abs(rounded) < 1e-6) {
      ascii = rounded.toExponential(8).replace(/\.?0+e/, 'e');
    } else {
      ascii = String(rounded);
    }
    return ascii.replace(/-/g, '−');
  }

  /**
   * Hand-written recursive-descent evaluator restricted strictly to +, -, *, / and decimals.
   * No eval, Function constructor, parentheses, %, or power operators.
   */
  function evalExpr(raw) {
    if (typeof raw !== 'string') return { ok: false, error: 'invalid' };
    let ascii = '';
    for (let i = 0; i < raw.length; i++) {
      const ch = raw[i];
      if (ch === 'e' || ch === 'E') {
        ascii += 'e';
        continue;
      }
      const norm = normalizeChar(ch);
      if (!norm) return { ok: false, error: 'invalid' };
      if (norm === '−') ascii += '-';
      else if (norm === '×') ascii += '*';
      else if (norm === '÷') ascii += '/';
      else ascii += norm;
    }

    if (!ascii || ascii === '-') return { ok: false, error: 'incomplete' };
    const tail = ascii[ascii.length - 1];
    if (tail === '+' || tail === '-' || tail === '*' || tail === '/' || tail === 'e') {
      return { ok: false, error: 'incomplete' };
    }

    let pos = 0;
    let hasOp = false;

    function parseNumber() {
      const start = pos;
      let sawDigit = false;
      while (pos < ascii.length && ascii[pos] >= '0' && ascii[pos] <= '9') {
        sawDigit = true;
        pos++;
      }
      if (pos < ascii.length && ascii[pos] === '.') {
        pos++;
        while (pos < ascii.length && ascii[pos] >= '0' && ascii[pos] <= '9') {
          sawDigit = true;
          pos++;
        }
      }
      if (!sawDigit) return { ok: false, error: 'invalid' };
      if (pos < ascii.length && ascii[pos] === 'e') {
        const expPos = pos;
        pos++;
        if (pos < ascii.length && (ascii[pos] === '+' || ascii[pos] === '-')) pos++;
        let expDigits = false;
        while (pos < ascii.length && ascii[pos] >= '0' && ascii[pos] <= '9') {
          expDigits = true;
          pos++;
        }
        if (!expDigits) {
          pos = expPos;
          return { ok: false, error: 'invalid' };
        }
      }
      const val = Number(ascii.slice(start, pos));
      if (!Number.isFinite(val)) return { ok: false, error: 'overflow' };
      return { ok: true, value: val };
    }

    function parsePrimary() {
      if (pos === 0 && ascii[pos] === '-') {
        pos++;
        const n = parseNumber();
        if (!n.ok) return n;
        return { ok: true, value: -n.value };
      }
      return parseNumber();
    }

    function parseTerm() {
      let left = parsePrimary();
      if (!left.ok) return left;
      while (pos < ascii.length && (ascii[pos] === '*' || ascii[pos] === '/')) {
        const op = ascii[pos++];
        hasOp = true;
        const right = parsePrimary();
        if (!right.ok) return right;
        if (op === '*') {
          const prod = left.value * right.value;
          if (!Number.isFinite(prod)) return { ok: false, error: 'overflow' };
          left = { ok: true, value: prod };
        } else {
          if (right.value === 0) return { ok: false, error: 'div0' };
          const quot = left.value / right.value;
          if (!Number.isFinite(quot)) return { ok: false, error: 'overflow' };
          left = { ok: true, value: quot };
        }
      }
      return left;
    }

    function parseExpr() {
      let left = parseTerm();
      if (!left.ok) return left;
      while (pos < ascii.length && (ascii[pos] === '+' || ascii[pos] === '-')) {
        const op = ascii[pos++];
        hasOp = true;
        const right = parseTerm();
        if (!right.ok) return right;
        const sum = op === '+' ? left.value + right.value : left.value - right.value;
        if (!Number.isFinite(sum)) return { ok: false, error: 'overflow' };
        left = { ok: true, value: sum };
      }
      return left;
    }

    const res = parseExpr();
    if (!res.ok) return res;
    if (pos !== ascii.length) return { ok: false, error: 'invalid' };
    const formatted = formatNumber(res.value);
    if (formatted === null) return { ok: false, error: 'overflow' };
    return { ok: true, value: res.value, formatted, hasOp };
  }

  function createEngine() {
    const eng = {
      expr: '',
      error: '',
      committed: false,

      reset() {
        this.expr = '';
        this.error = '';
        this.committed = false;
      },

      inputDigit(digit) {
        const d = normalizeChar(digit);
        if (!d || d < '0' || d > '9') return false;
        if (this.error) {
          this.reset();
        } else if (this.committed) {
          this.expr = '';
          this.committed = false;
        }
        if (this.expr.length >= MAX_LEN) return false;
        this.expr += d;
        return true;
      },

      inputDot() {
        if (this.error) {
          this.reset();
        } else if (this.committed) {
          this.expr = '';
          this.committed = false;
        }
        const lastOp = findLastBinaryOp(this.expr);
        const seg = this.expr.slice(lastOp + 1);
        if (seg.includes('.') || seg.includes('e')) return false;
        if (seg === '' || seg === '−') {
          if (this.expr.length + 2 > MAX_LEN) return false;
          this.expr += '0.';
          return true;
        }
        if (this.expr.length + 1 > MAX_LEN) return false;
        this.expr += '.';
        return true;
      },

      inputOp(rawOp) {
        const op = normalizeChar(rawOp);
        if (!op || !isOp(op)) return false;
        if (this.error) {
          this.reset();
        }
        this.committed = false;
        if (this.expr === '') {
          if (op === '−') {
            this.expr = '−';
            return true;
          }
          return false;
        }
        if (this.expr === '−') return false;
        if (endsWithBinaryOp(this.expr)) {
          this.expr = this.expr.slice(0, -1) + op;
          return true;
        }
        if (this.expr.length >= MAX_LEN) return false;
        this.expr += op;
        return true;
      },

      backspace() {
        if (this.error) {
          this.reset();
          return true;
        }
        if (!this.expr) return false;
        this.committed = false;
        this.expr = this.expr.slice(0, -1).replace(/e[+−-]?$/, '');
        return true;
      },

      preview() {
        if (this.error) return { text: '', error: this.error };
        if (!this.expr || this.committed || endsWithBinaryOp(this.expr)) {
          return { text: '', error: '' };
        }
        const res = evalExpr(this.expr);
        if (!res.ok) {
          if (res.error === 'div0' || res.error === 'overflow') {
            return { text: '', error: res.error };
          }
          return { text: '', error: '' };
        }
        if (!res.hasOp) return { text: '', error: '' };
        return { text: res.formatted, error: '' };
      },

      commit() {
        if (this.error || !this.expr || this.expr === '−' || endsWithBinaryOp(this.expr)) {
          return false;
        }
        const res = evalExpr(this.expr);
        if (!res.ok) {
          if (res.error === 'div0' || res.error === 'overflow') {
            this.error = res.error;
            this.committed = false;
            return true;
          }
          return false;
        }
        this.expr = res.formatted;
        this.error = '';
        this.committed = true;
        return true;
      },
    };
    return eng;
  }

  if (typeof module !== 'undefined' && module.exports) {
    module.exports = { MAX_LEN, normalizeChar, formatNumber, evalExpr, createEngine };
  }
  if (typeof document === 'undefined' || typeof window === 'undefined') return;

  const btn = document.getElementById('open-calc');
  const panel = document.getElementById('calc-panel');
  if (!btn || !panel) return;

  panel.innerHTML = `
    <div class="calc-clip">
      <canvas class="calc-bd" id="calc-bd" width="${PANEL_W}" height="${PANEL_H}"></canvas>
      <div class="calc-tint"></div>
      <div class="calc-deco"></div>
      <div class="calc-sheen"></div>
      <div class="calc-display">
        <div class="calc-expr empty" id="calc-expr" aria-live="off"></div>
        <div class="calc-res" id="calc-res" aria-live="polite"></div>
      </div>
      <div class="calc-grid" role="group" aria-label="Keypad">
        <button type="button" class="calc-key calc-fn" data-k="C" aria-label="Clear">C</button>
        <button type="button" class="calc-key calc-fn" data-k="back" aria-label="Backspace">&#9003;</button>
        <button type="button" class="calc-key calc-op" data-k="÷" aria-label="Divide">&#247;</button>
        <button type="button" class="calc-key calc-op" data-k="×" aria-label="Multiply">&#215;</button>
        <button type="button" class="calc-key" data-k="7">7</button>
        <button type="button" class="calc-key" data-k="8">8</button>
        <button type="button" class="calc-key" data-k="9">9</button>
        <button type="button" class="calc-key calc-op" data-k="−" aria-label="Subtract">&#8722;</button>
        <button type="button" class="calc-key" data-k="4">4</button>
        <button type="button" class="calc-key" data-k="5">5</button>
        <button type="button" class="calc-key" data-k="6">6</button>
        <button type="button" class="calc-key calc-op" data-k="+" aria-label="Add">+</button>
        <button type="button" class="calc-key" data-k="1">1</button>
        <button type="button" class="calc-key" data-k="2">2</button>
        <button type="button" class="calc-key" data-k="3">3</button>
        <button type="button" class="calc-key calc-eq" data-k="=" aria-label="Equals">=</button>
        <button type="button" class="calc-key calc-zero" data-k="0">0</button>
        <button type="button" class="calc-key" data-k="." aria-label="Decimal point">.</button>
      </div>
    </div>`;

  const exprEl = panel.querySelector('#calc-expr');
  const resEl = panel.querySelector('#calc-res');
  const gridEl = panel.querySelector('.calc-grid');
  const bdCanvas = panel.querySelector('#calc-bd');
  const bdCtx = bdCanvas ? bdCanvas.getContext('2d') : null;
  const eng = createEngine();

  let enabled = false;
  let btnWired = false;
  let isOpen = false;
  let closeTimer = null;
  let holdTimeout = null;
  let holdInterval = null;
  let lastPreviewKey = '';
  let panelPos = { left: 0, top: 0 };
  const flashTimers = new Map();

  function lang() {
    return window.state?.features?.prefsLang === 'ar' ? 'ar' : 'en';
  }

  function strings() {
    return I18N[lang()];
  }

  function updateLabels() {
    const t = strings();
    btn.title = t.title;
    btn.setAttribute('aria-label', t.title);
    panel.setAttribute('aria-label', t.title);
    const setAria = (k, label) => {
      const el = panel.querySelector(`[data-k="${k}"]`);
      if (el) el.setAttribute('aria-label', label);
    };
    setAria('C', t.clear);
    setAria('back', t.backspace);
    setAria('÷', t.divide);
    setAria('×', t.multiply);
    setAria('−', t.subtract);
    setAria('+', t.add);
    setAria('=', t.equals);
    setAria('.', t.decimal);
  }

  function renderDisplay() {
    const s = eng.expr;
    exprEl.textContent = s;
    exprEl.classList.toggle('empty', s.length === 0);
    exprEl.classList.toggle('sm1', s.length > 12 && s.length <= 18);
    exprEl.classList.toggle('sm2', s.length > 18 && s.length <= 24);
    exprEl.classList.toggle('sm3', s.length > 24);
    exprEl.scrollLeft = exprEl.scrollWidth;

    const pv = eng.preview();
    const t = strings();
    const nextText = pv.error ? (t[pv.error] || t.overflow) : pv.text;
    const isErr = Boolean(pv.error);
    const nextKey = `${isErr ? 'E' : 'V'}:${nextText}`;
    resEl.classList.toggle('err', isErr);
    resEl.dir = isErr && lang() === 'ar' ? 'rtl' : 'ltr';
    if (nextKey !== lastPreviewKey) {
      lastPreviewKey = nextKey;
      resEl.textContent = nextText;
      if (nextText && !document.body.classList.contains('no-anim')) {
        resEl.classList.add('fade');
        requestAnimationFrame(() => resEl.classList.remove('fade'));
      } else {
        resEl.classList.remove('fade');
      }
    }
  }

  function syncGlass() {
    if (!isOpen || !bdCtx) return;
    if (
      document.body.classList.contains('no-blur') ||
      document.body.classList.contains('th-solid') ||
      document.body.classList.contains('th-rec')
    ) {
      return;
    }
    const off = window.__getBackdropCanvas?.();
    if (!off || off.width <= 0 || off.height <= 0) return;
    if (bdCanvas.width !== PANEL_W || bdCanvas.height !== PANEL_H) {
      bdCanvas.width = PANEL_W;
      bdCanvas.height = PANEL_H;
    }
    bdCtx.clearRect(0, 0, PANEL_W, PANEL_H);
    const sx = Math.max(0, Math.min(panelPos.left, off.width - 1));
    const sy = Math.max(0, Math.min(panelPos.top, off.height - 1));
    const sw = Math.min(PANEL_W, off.width - sx);
    const sh = Math.min(PANEL_H, off.height - sy);
    if (sw > 0 && sh > 0) {
      bdCtx.drawImage(off, sx, sy, sw, sh, 0, 0, sw, sh);
    }
  }

  function positionPanel() {
    if (!isOpen) return;
    const isl = document.getElementById('island');
    const islRect = isl ? isl.getBoundingClientRect() : { left: 46, right: 594, top: 8, bottom: 240 };
    const btnRect = btn.getBoundingClientRect();
    const W = window.innerWidth;
    const H = window.innerHeight;
    const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), Math.max(lo, hi));

    const anchorRight = btnRect.width > 0 ? btnRect.right : islRect.right - 16;
    const left = Math.round(clamp(anchorRight - PANEL_W, PANEL_MARGIN, W - PANEL_W - PANEL_MARGIN));

    const belowTop = (btnRect.height > 0 ? btnRect.bottom : islRect.top + 42) + 8;
    let top;
    let slideY;
    let originY;
    if (belowTop + PANEL_H <= H - PANEL_MARGIN) {
      top = Math.round(belowTop);
      slideY = '-8px';
      originY = 'top';
    } else {
      top = Math.round(clamp(islRect.bottom - PANEL_H, PANEL_MARGIN, H - PANEL_H - PANEL_MARGIN));
      slideY = '8px';
      originY = 'bottom';
    }

    const btnMidX = btnRect.width > 0 ? btnRect.left + btnRect.width / 2 : left + PANEL_W - 24;
    const originX = Math.round(clamp(btnMidX - left, 20, PANEL_W - 20));
    panel.style.left = `${left}px`;
    panel.style.top = `${top}px`;
    panel.style.setProperty('--calc-slide-y', slideY);
    panel.style.setProperty('--calc-origin-x', `${originX}px`);
    panel.style.setProperty('--calc-origin-y', originY);
    panelPos = { left, top };

    window.nova?.calcRect?.({ x: left, y: top, width: PANEL_W, height: PANEL_H });
    syncGlass();
  }

  function flashKey(keyId) {
    if (!keyId || document.body.classList.contains('no-anim')) return;
    const el = panel.querySelector(`[data-k="${keyId}"]`);
    if (!el) return;
    el.classList.add('pressed');
    const prev = flashTimers.get(keyId);
    if (prev) clearTimeout(prev);
    flashTimers.set(
      keyId,
      setTimeout(() => {
        flashTimers.delete(keyId);
        el.classList.remove('pressed');
      }, KEY_FLASH_MS)
    );
  }

  function stopHoldRepeat() {
    if (holdTimeout) {
      clearTimeout(holdTimeout);
      holdTimeout = null;
    }
    if (holdInterval) {
      clearInterval(holdInterval);
      holdInterval = null;
    }
  }

  function handleKeyToken(k) {
    let changed = false;
    if (k === 'C') {
      eng.reset();
      changed = true;
    } else if (k === 'back') {
      changed = eng.backspace();
    } else if (k === '=') {
      changed = eng.commit();
    } else if (k === '.') {
      changed = eng.inputDot();
    } else if (isOp(k)) {
      changed = eng.inputOp(k);
    } else if (k >= '0' && k <= '9') {
      changed = eng.inputDigit(k);
    }
    if (changed) renderDisplay();
  }

  function onPadPointerDown(e) {
    const keyBtn = e.target.closest('.calc-key');
    if (!keyBtn || e.button !== 0) return;
    e.preventDefault();
    panel.focus({ preventScroll: true });
    const k = keyBtn.dataset.k;
    handleKeyToken(k);
    if (k === 'back') {
      stopHoldRepeat();
      holdTimeout = setTimeout(() => {
        holdTimeout = null;
        holdInterval = setInterval(() => {
          if (!isOpen) {
            stopHoldRepeat();
            return;
          }
          if (eng.backspace()) renderDisplay();
        }, BACKSPACE_REPEAT_MS);
      }, BACKSPACE_DELAY_MS);
    }
  }

  function onDocumentKeyDown(e) {
    if (!isOpen || !document.hasFocus()) return;
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const target = e.target;
    if (
      target &&
      target !== document.body &&
      target !== panel &&
      !panel.contains(target) &&
      (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')
    ) {
      return;
    }

    if (e.key === 'Escape') {
      if (!e.repeat) {
        e.preventDefault();
        close();
      }
      return;
    }
    if (e.key === 'Backspace') {
      e.preventDefault();
      flashKey('back');
      handleKeyToken('back');
      return;
    }
    if (e.key === 'Delete') {
      e.preventDefault();
      flashKey('C');
      handleKeyToken('C');
      return;
    }
    if (e.key === 'Enter' || e.key === '=') {
      e.preventDefault();
      if (!e.repeat) {
        flashKey('=');
        handleKeyToken('=');
      }
      return;
    }
    if (e.key === '.' || e.key === '\u066b' || e.code === 'NumpadDecimal') {
      e.preventDefault();
      flashKey('.');
      handleKeyToken('.');
      return;
    }

    let op = null;
    if (e.code === 'NumpadAdd' || e.key === '+') op = '+';
    else if (e.code === 'NumpadSubtract' || e.key === '-' || e.key === '−') op = '−';
    else if (e.code === 'NumpadMultiply' || e.key === '*' || e.key === '×' || e.key === 'x' || e.key === 'X') op = '×';
    else if (e.code === 'NumpadDivide' || e.key === '/' || e.key === '÷') op = '÷';
    if (op) {
      e.preventDefault();
      flashKey(op);
      handleKeyToken(op);
      return;
    }

    const norm = normalizeChar(e.key);
    if (norm && norm >= '0' && norm <= '9') {
      e.preventDefault();
      flashKey(norm);
      handleKeyToken(norm);
    }
  }

  function onDocumentPointerDown(e) {
    if (!isOpen) return;
    if (panel.contains(e.target) || btn.contains(e.target)) return;
    close();
  }

  function onWindowResize() {
    if (isOpen) positionPanel();
  }

  function onPanelWheel(e) {
    if (Math.abs(e.deltaY) > Math.abs(e.deltaX) && exprEl.scrollWidth > exprEl.clientWidth) {
      exprEl.scrollLeft += e.deltaY;
    }
    e.stopPropagation();
  }

  function onWindowFocus() {
    if (isOpen) panel.focus({ preventScroll: true });
  }

  function attachOpenListeners() {
    document.addEventListener('keydown', onDocumentKeyDown);
    document.addEventListener('pointerdown', onDocumentPointerDown, true);
    window.addEventListener('resize', onWindowResize);
    window.addEventListener('focus', onWindowFocus);
    window.addEventListener('island:backdrop', syncGlass);
    gridEl.addEventListener('pointerdown', onPadPointerDown);
    panel.addEventListener('wheel', onPanelWheel, { passive: true });
    window.addEventListener('pointerup', stopHoldRepeat);
    window.addEventListener('pointercancel', stopHoldRepeat);
    gridEl.addEventListener('pointerleave', stopHoldRepeat);
  }

  function detachOpenListeners() {
    document.removeEventListener('keydown', onDocumentKeyDown);
    document.removeEventListener('pointerdown', onDocumentPointerDown, true);
    window.removeEventListener('resize', onWindowResize);
    window.removeEventListener('focus', onWindowFocus);
    window.removeEventListener('island:backdrop', syncGlass);
    gridEl.removeEventListener('pointerdown', onPadPointerDown);
    panel.removeEventListener('wheel', onPanelWheel);
    window.removeEventListener('pointerup', stopHoldRepeat);
    window.removeEventListener('pointercancel', stopHoldRepeat);
    gridEl.removeEventListener('pointerleave', stopHoldRepeat);
  }

  function finishClose(fromBlur) {
    if (closeTimer) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    panel.hidden = true;
    panel.classList.remove('open');
    if (bdCtx) bdCtx.clearRect(0, 0, bdCanvas.width, bdCanvas.height);
    for (const tid of flashTimers.values()) clearTimeout(tid);
    flashTimers.clear();
    panel.querySelectorAll('.calc-key.pressed').forEach((el) => el.classList.remove('pressed'));
    window.nova?.calcRect?.(null);
    window.nova?.calcOpen?.(false, Boolean(fromBlur));
    window.setCalcHold?.(false);
    if (!fromBlur) {
      window.nova?.focusInput?.(false);
    }
  }

  function open() {
    if (!enabled || isOpen) return;
    if (closeTimer) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
    isOpen = true;
    btn.classList.add('on');
    btn.setAttribute('aria-expanded', 'true');
    updateLabels();
    renderDisplay();

    window.setCalcHold?.(true);
    window.nova?.calcOpen?.(true, false);
    window.nova?.focusInput?.(true);

    panel.hidden = false;
    positionPanel();
    attachOpenListeners();

    if (document.body.classList.contains('no-anim')) {
      panel.classList.add('open');
    } else {
      panel.classList.remove('open');
      void panel.offsetWidth;
      panel.classList.add('open');
    }
    panel.focus({ preventScroll: true });
    requestAnimationFrame(() => {
      if (isOpen) positionPanel();
    });
  }

  function close(immediate, fromBlur) {
    if (!isOpen && panel.hidden) return;
    isOpen = false;
    stopHoldRepeat();
    detachOpenListeners();
    btn.classList.remove('on');
    btn.setAttribute('aria-expanded', 'false');

    if (immediate || document.body.classList.contains('no-anim')) {
      panel.classList.remove('open');
      finishClose(fromBlur);
      return;
    }
    panel.classList.remove('open');
    closeTimer = setTimeout(() => {
      if (!isOpen) finishClose(fromBlur);
    }, 155);
  }

  function onBtnClick(e) {
    e.stopPropagation();
    if (!enabled) return;
    if (isOpen) close(false, false);
    else open();
  }

  function syncFeatureState() {
    const want = window.state?.features?.calculator !== false;
    enabled = want;
    btn.hidden = !want;
    if (want && !btnWired) {
      btn.addEventListener('click', onBtnClick);
      btnWired = true;
    } else if (!want && btnWired) {
      btn.removeEventListener('click', onBtnClick);
      btnWired = false;
    }
    if (!want && isOpen) {
      close(true, false);
    }
    if (want) {
      updateLabels();
      if (isOpen) {
        renderDisplay();
        positionPanel();
      }
    }
  }

  window.calcClose = (immediate) => close(immediate, false);
  window.__calcEngine = eng;
  window.addEventListener('island:features', syncFeatureState);
  window.addEventListener('island:pinned', (e) => {
    if (isOpen && e.detail && e.detail.pinned === false) close(true, false);
  });
  window.nova?.onCalcClose?.(() => close(false, true));

  syncFeatureState();
})();
