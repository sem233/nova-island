// Nova Island: the island window, its page, and the system bridge.
//   nova-island                      the app (data in --data <dir>; a dev build never touches the
//                                    real %APPDATA%\dynamic-nova unless --data says so)
//   nova-island --mock <query>       the promo staging (promo/src/island.html?<query>) instead,
//                                    with mock data and no bridge, to compare against 1.7
//   --bg rrggbb                      opaque background behind the page (comparisons)
//   --pos x,y                        (with --mock) place the window, physical pixels
//   --exit-after <ms> [--shot file]  close after a while (and save the window region as a BMP first)
//   --capture file.png               with --exit-after: save the page itself (WebView2 CapturePreview)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adhan;
mod bus;
mod core;
mod files;
mod glass;
mod idle_test;
mod island;
mod prefs;
mod radio;
mod mini;
pub mod clips;
pub mod mdm;
mod router;
mod screen;
mod shot;
mod startup;
mod tasks;
mod taskbar;
mod tray;
mod widgets;

use std::{
    sync::{Arc, Mutex},
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Duration, Instant},
};

use nova_core::{features::Features, settings::Store};
use nova_web::{Bridge, Page};
use serde_json::{json, Value};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use windows::{
    core::{Interface, HSTRING},
    Win32::{
        Foundation::*,
        UI::{
            HiDpi::*,
            Input::KeyboardAndMouse::{GetAsyncKeyState, RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, VK_LBUTTON, VK_RBUTTON},
            WindowsAndMessaging::*,
        },
    },
};

const WM_SYS: u32 = WM_APP + 1; // a nova-sys line is waiting
const WM_LATER_SIZE: u32 = WM_APP + 7; // a WM_SIZE that came while the app was busy
const WM_LATER_LAYOUT: u32 = WM_APP + 8; // the same for a display or scale change
const WM_SHOW_ISLAND: u32 = WM_APP + 9; // Nova was started again: show the island if it's hidden
const WM_CALC_BLUR: u32 = WM_APP + 10; // calculator lost window activation
const TIMER_CURSOR: usize = 1;
const TIMER_DRAG: usize = 2;
const TIMER_CAPTURE: usize = 3;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const HOTKEY_TOGGLE: i32 = 1;

/// (1.8.7) WM_SHOW_ISLAND's answer from a copy that's fine (its page alive).
const SHOWN_OK: usize = 0x4e4f5641; // "NOVA"

thread_local! {
    /// The island page is there and answering (false after its process failed, until it loads).
    static PAGE_ALIVE: Cell<bool> = const { Cell::new(true) };
    static CALC_PREV_FG: Cell<isize> = const { Cell::new(0) };
    static CALC_BTN_DOWN: Cell<bool> = const { Cell::new(false) };
    static PAGE_SUSPENDED: Cell<bool> = const { Cell::new(false) };
    static PAGE_LOW_MEM: Cell<bool> = const { Cell::new(false) };
}

pub fn remember_calc_prev_fg(island_hwnd: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        if !fg.0.is_null() && fg != island_hwnd {
            CALC_PREV_FG.with(|c| c.set(fg.0 as isize));
        }
    }
}

pub fn restore_calc_prev_fg(island_hwnd: HWND) {
    unsafe {
        let prev_raw = CALC_PREV_FG.with(|c| c.replace(0));
        if prev_raw != 0 && GetForegroundWindow() == island_hwnd {
            let prev = HWND(prev_raw as *mut _);
            if IsWindow(Some(prev)).as_bool() && !IsIconic(prev).as_bool() {
                let _ = SetForegroundWindow(prev);
            }
        }
    }
}

/// Syncs WebView2 memory target and system stats polling with whether the island is currently open.
fn sync_page_power(a: &Rc<RefCell<App>>) {
    let (open, slow, visible, page, system) = {
        let s = a.borrow();
        (s.core.is_open(), s.glass.perf(&s.core.features).slow(), s.core.win.visible(), s.page.clone(), s.system.clone())
    };
    if !visible {
        return;
    }
    let want_low = !open;
    if PAGE_LOW_MEM.with(|m| m.replace(want_low)) != want_low {
        if let Some(p) = page {
            p.set_low_memory(want_low);
        }
        system.set_stats_every(if slow || want_low { 5000 } else { 2000 });
    }
}

/// Signature of fields in `nova-sys` output that affect the island UI (excludes `idle`, `omni`, `browser`, `tabs`, `clip`, `rec`).
fn media_sig(v: &Value) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        v["has"], v["playing"], v["title"], v["artist"], v["album"], v["app"],
        v["dur"], v["pos"], v["thumb"], v["vol"], v["muted"], v["bright"],
        v["fs"], v["kl"], v["caps"], v["fgProc"], v["fgTitle"], v["bat"], v["ac"]
    )
}

/// The island page answers again ("island-ready") or stopped (its process failed).
pub fn set_page_alive(on: bool) {
    PAGE_ALIVE.with(|p| p.set(on));
}

/// Starts a new copy of Nova and ends this one (WebView2 gone, or a stuck copy ended): the new
/// copy makes its page from the start. Not for test runs (they end on their own).
fn restart_self(why: &str) {
    nova_core::log::error(&format!("restarting: {why}"));
    PAGE_ALIVE.with(|p| p.set(false));
    if arg("--exit-after").is_some() || !startup::installed() {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0);
}

pub struct App {
    pub core: core::Core,
    pub glass: glass::Glass,
    pub env: Option<ICoreWebView2Environment>,
    pub page: Option<Rc<Page>>,
    pub sys: Sender<String>,
    sys_lines: Receiver<String>,
    pub bus: bus::Bus,
    bus_rx: Receiver<bus::Msg>,
    pub shared_features: Arc<Mutex<Value>>,
    pub system: Arc<nova_services::system::System>,
    pub prayer: Arc<nova_services::prayer::Prayer>,
    pub network: Arc<nova_services::network::Network>,
    pub privacy: Arc<nova_services::privacy::Privacy>,
    pub apps: Arc<nova_services::apps::Apps>,
    pub shelf: Arc<nova_services::shelf::Shelf>,
    pub downloads: Arc<nova_services::downloads::Downloads>,
    pub news: Arc<nova_services::news::News>,
    pub updater: Arc<nova_services::updater::Updater>,
    pub app_notify: Arc<nova_services::appnotify::AppNotify>,
    pub calendar: Arc<nova_services::calendar::service::Calendar>,
    pub plugins: Arc<nova_services::plugins::Plugins>,
    /// the to-do list of the built-in Tasks plugin (1.8.3)
    pub tasks: Arc<nova_services::tasks::Tasks>,
    pub shots: Arc<nova_services::screenshot::Screenshots>,
    pub prefs: Option<prefs::Prefs>,
    pub radio: Option<prefs::Prefs>,
    /// the Tasks window (1.8.3), open only while it shows
    pub tasks_win: Option<prefs::Prefs>,
    /// The island's radio: {station, surah, playing, live}, for the Radio window.
    pub radio_now: Value,
    clip_seq: Option<u64>,
    pub clips: clips::ClipsManager,
    pub data_dir: PathBuf,
}

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

fn app() -> Option<Rc<RefCell<App>>> {
    APP.with(|a| a.borrow().clone())
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// "Stay in place" over a browser: the whole island at half opacity (new in 1.8), so the tabs
/// under it show through. Added from here, so the page files stay as they are.
const DIM_JS: &str = "(on => { let s = document.getElementById('nova-dim'); if (!s) { s = document.createElement('style'); s.id = 'nova-dim'; s.textContent = 'html { transition: opacity .22s ease } html.nova-dim { opacity: .62 } html:has(body.no-anim) { transition: none }'; document.head.appendChild(s); } document.documentElement.classList.toggle('nova-dim', on); })";

/// Shows another app's notification in the island's toast, the way news items show.
/// Another app's notification in the island's toast: its icon where the bell goes, a soft glow in
/// its colour from that side, and the sender first ("Ahmed: text"; the app's name only when the
/// notification has no title). A long message scrolls from its start to its end (right to left
/// for Arabic) and the toast stays until it has been read. A click, or "Open", opens the app.
/// The look is added from here, so the page files stay as they are.
const APP_NOTIFY_JS: &str = r#"(n => {
  if (!document.getElementById('nova-appnote')) {
    const s = document.createElement('style');
    s.id = 'nova-appnote';
    s.textContent = `
      .l-toast.appnote { border-radius: inherit; background: radial-gradient(130% 220% at 0% 50%, rgba(var(--app-rgb), .4), rgba(var(--app-rgb), 0) 62%); box-shadow: inset 0 0 0 1px rgba(var(--app-rgb), .35); }
      .l-toast.appnote .t-left { min-width: 0; }
      .l-toast.appnote #t-label { overflow: hidden; text-overflow: clip; font-weight: 600; }
      .l-toast.appnote #t-label > span { display: inline-block; will-change: transform; }
      .l-toast.appnote #t-value:not(:empty) { height: 24px; padding: 0 12px; border-radius: 99px; display: inline-flex; align-items: center; background: rgba(var(--app-rgb), .9); color: #fff; font-weight: 700; cursor: pointer; }
      .l-toast.appnote .batt, .l-toast.appnote .t-bar, .l-vtoast.appnote .v-bar { display: none; }
      .l-toast.appnote .t-shot { display: block; width: 22px; height: 22px; max-width: 22px; border-radius: 6px; box-shadow: none; object-fit: contain; }
      .l-toast.appnote.noicon .t-shot { display: none; }
      .l-toast.appnote.noicon .ico-bell, .l-vtoast.appnote .ico-bell { display: block; color: rgb(var(--app-rgb)); }`;
    document.head.appendChild(s);
  }
  const head = n.title || n.app;
  const text = (n.body ? `${head}: ${n.body}` : head) + (n.more ? ` (+${n.more})` : '');
  showToast('appnote', text, n.id != null ? 'Open' : '', 0, 6000);
  const t = document.querySelector('.l-toast');
  t.classList.toggle('noicon', !n.icon);
  if (n.icon) document.querySelector('.t-shot').src = n.icon;
  t.style.setProperty('--app-rgb', Array.isArray(n.rgb) ? n.rgb.join(', ') : 'var(--accent-rgb)');
  if (n.id != null) state.toastAction = () => bridge.appNotifyOpen(n.id);
  requestAnimationFrame(() => {
    const label = document.getElementById('t-label');
    if (!label || label.textContent !== text) return;
    const span = document.createElement('span');
    span.textContent = text;
    label.textContent = '';
    label.append(span);
    const over = span.offsetWidth - label.clientWidth;
    if (over <= 4) return;
    const rtl = getComputedStyle(label).direction === 'rtl';
    const ms = Math.max(2500, (over / 45) * 1000);
    span.animate([{ transform: 'translateX(0)' }, { transform: `translateX(${rtl ? over : -over}px)` }], { duration: ms, delay: 1200, fill: 'forwards', easing: 'linear' });
    const total = 1200 + ms + 2000;
    if (total > 6000) {
      state.toastUntil = Date.now() + total;
      clearTimeout(toastTimer);
      toastTimer = setTimeout(render, total + 50);
      render();
    }
  });
})"#;

/// The catch-up after sleep or a lost connection (network.onBack and powerMonitor 'resume').
static WAKE: std::sync::OnceLock<Arc<dyn Fn() + Send + Sync>> = std::sync::OnceLock::new();

fn main() {
    // a failure must never be silent: a panic (panic=abort) or a startup error goes to nova.log and
    // to a message on screen, so a PC where the island doesn't appear tells us why
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        fail(&format!("crash: {what}{at}"));
    }));
    if let Err(e) = run() {
        fail(&e);
        std::process::exit(1);
    }
}

/// Writes the error to nova.log (CRASH) and shows it, unless this is a test run.
fn fail(e: &str) {
    if nova_core::log::path().is_none() {
        nova_core::log::init(data_dir(), &format!("start {} webview2", env!("CARGO_PKG_VERSION")));
    }
    nova_core::log::write("CRASH", e);
    eprintln!("nova-island: {e}");
    // only an installed copy shows it: dev and test runs just log
    if !startup::installed() {
        return;
    }
    let webview = e.to_ascii_lowercase().contains("webview");
    let hint = if webview { "\n\nNova Island needs Microsoft Edge WebView2 Runtime. Install it from https://go.microsoft.com/fwlink/p/?LinkId=2124703 and open Nova Island again." } else { "" };
    let log = nova_core::log::path().map(|p| p.display().to_string()).unwrap_or_default();
    let text = format!("Nova Island couldn't start.\n\n{e}{hint}\n\nDetails are in {log}");
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), windows::core::w!("Nova Island"), MB_OK | MB_ICONERROR | MB_TOPMOST);
    }
}

/// Where settings live: --data, else the real folder only in an installed build.
/// tests: the island's page keeps a forced scale (--test-no-pin)
static TEST_NO_PIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

thread_local! {
    /// The page `send` talks to (tests: idle_test closes it and makes it again).
    static PAGE_SLOT: RefCell<Option<Rc<RefCell<Option<Rc<Page>>>>>> = const { RefCell::new(None) };
}
fn clear_page_slot() {
    PAGE_SLOT.with(|s| if let Some(p) = s.borrow().as_ref() { *p.borrow_mut() = None });
}
fn set_page_slot(page: Rc<Page>) {
    PAGE_SLOT.with(|s| if let Some(p) = s.borrow().as_ref() { *p.borrow_mut() = Some(page) });
}

fn data_dir() -> PathBuf {
    if let Some(d) = arg("--data") {
        return PathBuf::from(d);
    }
    // a copy run from Cargo's target folder keeps its own data beside it (<target>\data), never
    // the installed app's settings
    if cfg!(debug_assertions) || startup::dev_build() {
        if let Some(target) = std::env::current_exe().ok().and_then(|p| p.parent().and_then(|d| d.parent()).map(|d| d.to_path_buf())) {
            return target.join("data");
        }
    }
    nova_core::settings::dir()
}

pub fn schedule_capture(hwnd: HWND, ms: u32) {
    unsafe {
        SetTimer(Some(hwnd), TIMER_CAPTURE, ms, None);
    }
}

/// captureBackdrop(), then the next one (scheduleCapture)
fn capture_tick(a: &Rc<RefCell<App>>, hwnd: HWND) {
    capture_now(a);
    let s = a.borrow();
    let next = s.glass.next_delay(s.core.active(), &s.core.features);
    drop(s);
    schedule_capture(hwnd, next);
}

/// One capture, skipped while dragging (it would make the move stutter), locked, hidden, or
/// when the glass is off.
pub fn capture_now(a: &Rc<RefCell<App>>) {
    let mut s = a.borrow_mut();
    // (1.8.7) nor while it's the thin line over a browser's tabs: nothing of the glass shows
    if s.core.dragging() || s.core.is_line() || s.glass.screen_locked || !s.core.win.visible() || !s.glass.glass_live(&s.core.features) {
        return;
    }
    // a screenshot is being taken (Win+Shift+S): no glass picture now, so the island is in it
    if matches!(nova_sys::win::foreground_info().0.to_ascii_lowercase().as_str(), "snippingtool" | "screenclippinghost") {
        return;
    }
    let (Some(env), Some(page)) = (s.env.clone(), s.page.clone()) else { return };
    let (x, y) = s.core.win.pos();
    let (ww, wh) = s.core.win.size();
    let display = screen::matching(screen::Rect { x, y, width: ww, height: wh });
    let slow = s.glass.perf(&s.core.features).slow();
    let hwnd = s.core.win.hwnd;
    s.glass.capture(hwnd, &display, ww, wh, slow, &env, &page.webview);
}

/// What follows a change of features, the battery or a recorder (nova.js onChange / evalSaver /
/// applyPerf / updateProtection).
pub fn after_change(a: &Rc<RefCell<App>>) {
    let mut s = a.borrow_mut();
    let hwnd = s.core.win.hwnd;
    let app = &mut *s;
    if let Some(on) = app.glass.eval_saver(&app.core.features) {
        let pct = app.glass.battery.0;
        (app.core.send)("saver", serde_json::json!({ "on": on, "pct": pct }));
    }
    let mut capture_in = None;
    if app.glass.apply_perf(&app.core.features) {
        let f = app.glass.island_features(&app.core.features);
        (app.core.send)("features", f);
        capture_in = Some(300);
    }
    if app.glass.update_protection(hwnd, &app.core.features) {
        capture_in = Some(capture_in.unwrap_or(150).min(150));
    }
    drop(s);
    if let Some(ms) = capture_in {
        schedule_capture(hwnd, ms);
    }
}

fn schedule_cursor(hwnd: HWND, ms: u32) {
    unsafe {
        SetTimer(Some(hwnd), TIMER_CURSOR, ms, None);
    }
}

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_MOUSEACTIVATE if GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 != 0 => return LRESULT(MA_NOACTIVATE as isize),
            // Windows sends these from inside SetWindowPos, while the island's own code is moving
            // the window and holds the app (on a display scaled past 100% the first placing also
            // resizes it). Reaching for the app then crashed 1.8.0 and 1.8.1 at start (issue 5):
            // now a busy app gets the work a moment later, from the message queue.
            WM_SIZE | WM_LATER_SIZE => {
                if let Some(a) = app() {
                    match a.try_borrow() {
                        Ok(s) => {
                            if let Some(p) = s.page.clone() {
                                drop(s);
                                let mut r = RECT::default();
                                let _ = GetClientRect(h, &mut r);
                                let _ = p.controller.SetBounds(r);
                                // a new size comes with a new screen scale: the page follows that
                                // scale only (not Windows' Text size)
                                if !TEST_NO_PIN.load(std::sync::atomic::Ordering::Relaxed) {
                                    p.pin_scale(GetDpiForWindow(h) as f64 / 96.0);
                                }
                            }
                        }
                        Err(_) => {
                            let _ = PostMessageW(Some(h), WM_LATER_SIZE, WPARAM(0), LPARAM(0));
                        }
                    }
                }
                if m == WM_LATER_SIZE {
                    return LRESULT(0);
                }
            }
            WM_TIMER => {
                if let Some(a) = app() {
                    if w.0 == TIMER_CURSOR {
                        let mut s = a.borrow_mut();
                        let was = s.core.active();
                        s.core.poll_cursor();
                        if s.core.calc_active() {
                            let down = (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0
                                || (GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16 & 0x8000) != 0;
                            let prev = CALC_BTN_DOWN.with(|c| c.replace(down));
                            if down && !prev && !s.core.inside() && s.core.prepare_calc_blur() {
                                CALC_PREV_FG.with(|c| c.set(0));
                                (s.core.send)("calc-close", Value::Null);
                            }
                        } else {
                            CALC_BTN_DOWN.with(|c| c.set(false));
                        }
                        let now = s.core.active();
                        let next = s.core.next_poll_ms();
                        drop(s);
                        schedule_cursor(h, next);
                        sync_page_power(&a);
                        if now && !was {
                            schedule_capture(h, 60);
                        }
                    } else if w.0 == TIMER_CAPTURE {
                        let _ = KillTimer(Some(h), TIMER_CAPTURE);
                        capture_tick(&a, h);
                    } else if w.0 == TIMER_DRAG {
                        a.borrow_mut().core.follow_cursor();
                    }
                }
                return LRESULT(0);
            }
            // powerMonitor 'resume'
            WM_POWERBROADCAST if w.0 as u32 == PBT_APMRESUMEAUTOMATIC => {
                if let Some(wake) = WAKE.get() {
                    wake();
                }
            }
            WM_SHOW_ISLAND => {
                if let Some(a) = app() {
                    match a.try_borrow_mut() {
                        Ok(mut s) => {
                            if s.core.user_hidden {
                                s.core.user_hidden = false;
                                s.core.update_visibility();
                            }
                        }
                        // busy this moment: once it's free (the answer below still says it's fine)
                        Err(_) => {
                            let _ = PostMessageW(Some(h), WM_SHOW_ISLAND, WPARAM(0), LPARAM(0));
                        }
                    }
                    // (1.8.7) the answer to a second launch: fine only with a page that's alive
                    if PAGE_ALIVE.with(|p| p.get()) {
                        return LRESULT(SHOWN_OK as isize);
                    }
                }
                return LRESULT(0);
            }
            WM_HOTKEY if w.0 as i32 == HOTKEY_TOGGLE => {
                if let Some(a) = app() {
                    a.borrow_mut().core.toggle();
                }
            }
            bus::WM_BUS => {
                if let Some(a) = app() {
                    drain_bus(&a);
                }
            }
            WM_SYS => {
                if let Some(a) = app() {
                    drain_sys(&a);
                }
            }
            tray::WM_TRAY if matches!(l.0 as u32, WM_LBUTTONUP | WM_RBUTTONUP) => menu(h),
            WM_WTSSESSION_CHANGE => {
                // 7: locked, 8: unlocked
                if let Some(a) = app() {
                    match w.0 {
                        7 => a.borrow_mut().glass.screen_locked = true,
                        8 => {
                            a.borrow_mut().glass.screen_locked = false;
                            schedule_capture(h, 300);
                        }
                        _ => {}
                    }
                }
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED | WM_LATER_LAYOUT => {
                if let Some(a) = app() {
                    match a.try_borrow_mut() {
                        Ok(mut s) => s.core.layout(),
                        // moved onto a display with another scale while the app was busy
                        Err(_) => {
                            let _ = PostMessageW(Some(h), WM_LATER_LAYOUT, WPARAM(0), LPARAM(0));
                        }
                    }
                }
                if m != WM_DISPLAYCHANGE {
                    return LRESULT(0);
                }
            }
            WM_ACTIVATE if (w.0 & 0xffff) as u32 == WA_INACTIVE => {
                if let Some(a) = app() {
                    if let Ok(s) = a.try_borrow() {
                        if s.core.calc_active() {
                            let _ = PostMessageW(Some(h), WM_CALC_BLUR, WPARAM(0), LPARAM(0));
                        }
                    } else {
                        let _ = PostMessageW(Some(h), WM_CALC_BLUR, WPARAM(0), LPARAM(0));
                    }
                }
            }
            WM_CALC_BLUR => {
                if let Some(a) = app() {
                    if GetForegroundWindow() != h {
                        let mut s = a.borrow_mut();
                        if s.core.prepare_calc_blur() {
                            CALC_PREV_FG.with(|c| c.set(0));
                            (s.core.send)("calc-close", Value::Null);
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                tray::remove(h);
                PostQuitMessage(0);
            }
            _ => {}
        }
        DefWindowProcW(h, m, w, l)
    }
}

/// The tray / right-click menu.
pub fn menu(h: HWND) {
    let Some(a) = app() else { return };
    let state = {
        let s = a.borrow();
        tray::MenuState { shown: !s.core.user_hidden, vertical: s.core.prefs.get("vertical").and_then(Value::as_bool).unwrap_or(false), startup: (startup::installed() && !startup::store()).then(startup::starts_with_windows) }
    };
    let id = tray::popup(h, &state);
    match id {
        tray::ID_SHOW => {
            let mut s = a.borrow_mut();
            s.core.user_hidden = !s.core.user_hidden;
            s.core.update_visibility();
        }
        tray::ID_VERTICAL => a.borrow_mut().core.set_vertical(!state.vertical),
        tray::ID_RESET => a.borrow_mut().core.reset_position(),
        tray::ID_STARTUP => startup::set_start_with_windows(!state.startup.unwrap_or(false)),
        tray::ID_SETTINGS => prefs::open(None),
        tray::ID_TOGGLE => (a.borrow().core.send)("toggle", Value::Null),
        tray::ID_QUIT => unsafe {
            let _ = DestroyWindow(h);
            return;
        },
        _ => {}
    }
    if id != 0 {
        router::notify_general(&a);
    }
}

/// Lines from nova-sys: what main reads from them, then the whole line to the island as "media".
/// Events and invoke answers from the service threads, handed to the page.
fn drain_bus(a: &Rc<RefCell<App>>) {
    loop {
        let m = match a.borrow().bus_rx.try_recv() {
            Ok(m) => m,
            Err(_) => return,
        };
        match m {
            bus::Msg::Event(ch, v) => {
                // the Settings page gets its own channels too
                if prefs::EVENTS.contains(&ch.as_str()) {
                    prefs::send(a, &ch, v.clone());
                }
                if ch == "plugins" {
                    radio::plugins_changed(a);
                    tasks::plugins_changed(a);
                }
                // the Tasks window shows the list too
                if ch == "tasks" {
                    tasks::to_window(a, v.clone());
                    // ...and the desktop widget, at once (tasks.json is saved before this comes)
                    nova_render::widget::redraw("tasks");
                }
                // another app's notification: the island's own toast shows it ("App · title: text")
                if ch == "app-notify" {
                    if let Some(p) = a.borrow().page.clone() {
                        p.run(&format!("{APP_NOTIFY_JS}({v})"));
                    }
                    continue;
                }
                let page = a.borrow().page.clone();
                if let Some(p) = page {
                    p.event(&ch, v);
                }
            }
            bus::Msg::Resolve(id, v) => router::answer(a, id, v),
            bus::Msg::Ui(f) => f(a),
        }
    }
}

/// After any change of features: the copy the services read, and what follows in nova.js
/// features.onChange (stats pace, prayer recalculation, clipboard history).
pub fn features_changed(a: &Rc<RefCell<App>>, before: &Value) {
    let s = a.borrow();
    let now = s.core.features.get();
    *s.shared_features.lock().unwrap() = now.clone();
    let page = s.prefs.as_ref().map(|p| p.page.clone());
    if let Some(p) = page {
        p.event("features", now.clone());
    }
    let slow = s.glass.perf(&s.core.features).slow();
    let low = !s.core.is_open();
    s.system.set_stats_every(if slow || low { 5000 } else { 2000 });
    if now["asrHanafi"] != before["asrHanafi"] || now["hijriAdjust"] != before["hijriAdjust"] {
        s.prayer.refresh();
    }
    if now["clipboardHistory"] == false && before["clipboardHistory"] != false {
        s.system.clear_clips();
    }
    s.network.set_enabled(now["netToasts"] == true);
    s.privacy.set_enabled(now["privacyDots"] == true);
    s.downloads.set_enabled(now["downloads"] == true);
    s.downloads.set_folders(now["folderDownloads"] == true);
    if now["news"] != before["news"] {
        s.news.set_enabled(now["news"] == true);
    }
    if now["appNotify"] != before["appNotify"] {
        if now["appNotify"] == true {
            nova_sys::notify::request();
        }
        s.app_notify.set_enabled(now["appNotify"] == true);
    }
    if now["autoUpdate"] != before["autoUpdate"] {
        s.updater.set_enabled(now["autoUpdate"] == true);
    }
    // Settings › Time format: the desktop widgets redraw at once, and their pictures in Settings
    if now["clock24"] != before["clock24"] {
        drop(s);
        nova_render::desktop::time_format(now["clock24"] == true);
        crate::prefs::send(a, "widgets", crate::widgets::get_with_previews());
    }
}

fn drain_sys(a: &Rc<RefCell<App>>) {
    loop {
        let line = match a.borrow().sys_lines.try_recv() {
            Ok(l) => l,
            Err(_) => return,
        };
        let Ok(mut data) = serde_json::from_str::<Value>(&line) else { continue };
        // tests: --test-fake-media "Title|Artist" plays a made-up track (nothing is heard), so the
        // compact island can be seen without playing anything
        if let Some(fake) = arg("--test-fake-media") {
            if data.get("has").is_some() {
                let (title, artist) = fake.split_once('|').unwrap_or((fake.as_str(), ""));
                data["has"] = json!(true);
                data["playing"] = json!(true);
                data["title"] = json!(title);
                data["artist"] = json!(artist);
                data["album"] = json!("");
                data["app"] = json!("Spotify.exe");
                data["dur"] = json!(240);
                data["pos"] = json!(60);
                if let Some(o) = data.as_object_mut() {
                    o.remove("thumb");
                }
            }
        }
        if data.get("focusapp").is_some() || data.get("running").is_some() {
            continue; // the Apps tab's answers (step 6)
        }
        // the clipboard counter: whatever was there at start is old news
        if let Some(seq) = data.get("clip").and_then(Value::as_f64) {
            let seq = seq as u64;
            let mut s = a.borrow_mut();
            let prev = s.clip_seq.replace(seq);
            if prev.is_some_and(|p| p != seq) {
                let private = data.get("clipPriv").and_then(Value::as_bool).unwrap_or(false);
                let (system, shots) = (s.system.clone(), s.shots.clone());
                drop(s);
                system.clipboard_changed(private);
                if !private {
                    std::thread::spawn(move || shots.clipboard_changed());
                }
            }
        }
        let mut s = a.borrow_mut();
        if let Some(o) = data.get("omni") {
            s.core.on_omni(o);
        }
        // (1.8.7) the tabs are read (UI Automation, every second) only for "Stay in place"
        nova_sys::tabs::WANTED.store(!s.core.features.bool("addrIsland") && !s.core.features.bool("browserLine"), std::sync::atomic::Ordering::Relaxed);
        // ...and the address bar only for "Address bar", from the island's default place
        nova_sys::omni::WANTED.store(s.core.omni_allowed(), std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = data.get("tabs") {
            s.core.on_tabs(t);
        }
        // tests: --test-browser '{"x":..,"y":..,"w":..,"h":..}' (physical pixels) stands for the
        // browser window in front; --test-browser-blink MS: every other MS it's gone
        static TEST_BROWSER: std::sync::OnceLock<Option<Value>> = std::sync::OnceLock::new();
        if let Some(b) = TEST_BROWSER.get_or_init(|| arg("--test-browser").and_then(|j| serde_json::from_str(&j).ok())).as_ref() {
            let blink = arg("--test-browser-blink").and_then(|v| v.parse::<u128>().ok()).filter(|ms| *ms > 0);
            let gone = blink.is_some_and(|ms| (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0) / ms) % 2 == 1);
            s.core.on_browser(if gone { &Value::Null } else { b });
        } else if let Some(b) = data.get("browser") {
            s.core.on_browser(b);
        }
        let mut changed = false;
        if let Some(pct) = data.get("bat").and_then(Value::as_i64) {
            let ac = data.get("ac").and_then(Value::as_bool) != Some(false);
            if (pct, ac) != s.glass.battery {
                s.glass.battery = (pct, ac);
                changed = true;
            }
        }
        if let Some(rec) = data.get("rec").and_then(Value::as_bool) {
            if rec != s.glass.recording {
                s.glass.recording = rec;
                nova_core::log::info(&format!("screen recorder {}", if rec { "running: glass paused" } else { "gone: glass back" }));
                (s.core.send)("recording", serde_json::json!(rec));
                changed = true;
            }
        }
        if let Some(fs) = data.get("fs").and_then(Value::as_bool) {
            if fs != s.core.fullscreen_hidden {
                s.core.fullscreen_hidden = fs;
                s.core.update_visibility();
            }
        }
        s.core.win.keep_on_top();
        drop(s);
        if changed {
            after_change(a);
        }
        // (1.8.7) the Nova strip on the taskbar, when it's on
        taskbar::on_line(a, &data);
        if PAGE_SUSPENDED.with(|p| p.get()) {
            continue;
        }
        let sig = media_sig(&data);
        let idle_lt_5 = data.get("idle").and_then(Value::as_i64).is_some_and(|i| i < 5);
        let should_push = {
            thread_local! {
                static LAST_MEDIA: RefCell<(String, bool, Instant)> = RefCell::new((String::new(), true, Instant::now() - Duration::from_secs(60)));
            }
            LAST_MEDIA.with(|cell| {
                let mut last = cell.borrow_mut();
                if sig != last.0 || idle_lt_5 != last.1 || last.2.elapsed() >= Duration::from_secs(5) {
                    *last = (sig, idle_lt_5, Instant::now());
                    true
                } else {
                    false
                }
            })
        };
        if should_push {
            let page = a.borrow().page.clone();
            if let Some(p) = page {
                p.event("media", data);
            }
        }
    }
}

fn run() -> Result<(), String> {
    unsafe {
        let _ = windows::Win32::System::Ole::OleInitialize(None);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mock = arg("--mock");
    if let Some(spec) = arg("--glass-test") {
        let (x, y) = spec.split_once(',').map(|(a, b)| (a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0))).unwrap_or((0.0, 0.0));
        let out = std::env::args().skip_while(|a| a != "--glass-test").nth(2).unwrap_or("glass.bgra".into());
        return glass::test_capture(x, y, &out);
    }
    let dir = data_dir();
    nova_core::log::init(dir.clone(), &format!("start {} webview2 ({})", env!("CARGO_PKG_VERSION"), if startup::store() { "store" } else { "direct" }));
    // one running copy: a second launch only brings the island forward
    if mock.is_none() && !startup::first_instance(if startup::installed() { "single" } else { "dev" }) {
        // the island window shows up a moment after the copy starts; a copy that never made one
        // is stuck, and is logged so a report shows it
        let mut other = startup::other_window();
        for _ in 0..30 {
            if other.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            other = startup::other_window();
        }
        match other {
            Some(h) => unsafe {
                // as 1.7: a second launch (the Start menu, the startup entry) leaves the island as it
                // is; it only shows it again if it was hidden. 1.8.0 to 1.8.3 sent it Ctrl+Alt+I,
                // which pinned it open until Ctrl+Alt+I was pressed again (1.8.4)
                // (1.8.7) and it must answer: a copy that's stuck (or lost its page, see
                // on_process_failed) made every click on Nova's icon do nothing at all
                let mut answer = 0usize;
                let ok = SendMessageTimeoutW(h, WM_SHOW_ISLAND, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG | SMTO_BLOCK, 3000, Some(&mut answer)).0 != 0;
                if ok && answer == SHOWN_OK {
                    nova_core::log::info("already running: the island shows if it was hidden");
                } else {
                    nova_core::log::error(&format!("already running but not answering ({}): restarting it", if ok { "no page" } else { "stuck" }));
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(h, Some(&mut pid));
                    if let Ok(p) = windows::Win32::System::Threading::OpenProcess(windows::Win32::System::Threading::PROCESS_TERMINATE | windows::Win32::System::Threading::PROCESS_SYNCHRONIZE, false, pid) {
                        let _ = windows::Win32::System::Threading::TerminateProcess(p, 1);
                        let _ = windows::Win32::System::Threading::WaitForSingleObject(p, 5000);
                        let _ = windows::Win32::Foundation::CloseHandle(p);
                    }
                    restart_self("the running copy didn't answer");
                }
            },
            None => return Err("another copy of Nova Island is running but has no island window. End \"Nova Island\" in Task Manager and open it again.".into()),
        }
        return Ok(());
    }
    let hwnd = island::create(Some(wndproc)).map_err(|e| format!("the island window: {e}"))?;
    nova_core::log::info("startup: island window");
    // tests: start at another size, as a display scaled past 100% does, so the first placing
    // resizes the window from inside the app's own code (issue 5)
    if std::env::args().any(|a| a == "--test-scaled-start") {
        unsafe {
            let _ = SetWindowPos(hwnd, None, 0, 0, 800, 400, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
    let win = island::Window::new(hwnd);
    let mut features = Features::load(Store::new(dir.clone()));
    // 1.8.1: the water reminder is off until someone turns it on. features.json keeps every
    // option, so who chose it can't be told apart: it goes off once for everyone, and stays as
    // set after that (the marker file).
    let water_once = dir.join("water-off-1.8.1");
    if !water_once.exists() {
        features.set(&json!({ "water": false }));
        let _ = std::fs::write(&water_once, b"");
        nova_core::log::info("water reminder: off by default from 1.8.1");
    }
    // 1.8.8: the legacy Address Bar docking mode (addrIsland) was removed from Settings UI in 1.8.7;
    // turn off stale enabled values once so opening a browser never squishes the island into a line.
    let addr_once = dir.join("addr-off-1.8.8");
    if !addr_once.exists() {
        features.set(&json!({ "addrIsland": false }));
        let _ = std::fs::write(&addr_once, b"");
        nova_core::log::info("addrIsland: off by default from 1.8.8");
    }
    startup::refresh();

    // the page: events go through `send` once the page exists
    let page_slot: Rc<RefCell<Option<Rc<Page>>>> = Rc::default();
    PAGE_SLOT.with(|s| *s.borrow_mut() = Some(page_slot.clone()));
    let send_slot = page_slot.clone();
    let send: Box<dyn Fn(&str, Value)> = Box::new(move |ch, data| {
        if let Some(p) = send_slot.borrow().as_ref() {
            if ch == core::DIM {
                p.run(&format!("{DIM_JS}({})", data == true));
            } else if ch == "visibility" {
                let show = data == true;
                if show && PAGE_SUSPENDED.with(|s| s.replace(false)) {
                    p.set_suspended(false);
                    PAGE_LOW_MEM.with(|m| m.set(false));
                }
                p.event(ch, data);
                if !show && !PAGE_SUSPENDED.with(|s| s.get()) && arg("--test-idle").is_none() {
                    PAGE_SUSPENDED.with(|s| s.set(true));
                    PAGE_LOW_MEM.with(|m| m.set(true));
                    p.set_suspended(true);
                }
            } else {
                p.event(ch, data);
            }
        }
    });

    // nova-sys on its own thread; its lines wake the window
    let (line_tx, line_rx) = channel::<String>();
    let hwnd_raw = hwnd.0 as isize;
    let sys = nova_sys::start(move |line| {
        if line_tx.send(line).is_ok() {
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_SYS, WPARAM(0), LPARAM(0));
            }
        }
    });

    // the services, on their own threads, talking to the page through the bus
    let (bus, bus_rx) = bus::Bus::new(hwnd);
    {
        let bus_vdl = bus.clone();
        crate::mdm::set_progress_cb(move |val| {
            bus_vdl.event("video-dl", val);
        });
    }
    let shared_features = Arc::new(Mutex::new(features.get()));
    let ctx = nova_services::ctx::Ctx { emit: bus.emitter(), dir: dir.clone(), features: shared_features.clone() };
    let system = nova_services::system::System::new(ctx.clone());
    let prayer = nova_services::prayer::Prayer::new(ctx.clone(), system.clone());
    {
        let prayer = prayer.clone();
        let bus2 = bus.clone();
        system.on_location_change(move |info| {
            prayer.refresh();
            bus2.event("location", info);
        });
    }
    let news = nova_services::news::News::new(ctx.clone());
    let network = nova_services::network::Network::new(ctx.clone());
    let privacy = nova_services::privacy::Privacy::new(ctx.clone());
    // updates: the installed app only (the Store updates its own); a test feed for checks
    let feed = arg("--update-feed");
    let pending = match &feed {
        Some(_) => dir.join("updater-pending"),
        None => PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("nova-island-updater").join("pending"),
    };
    let updater = nova_services::updater::Updater::new(ctx.clone(), feed.as_deref().unwrap_or(nova_services::updater::FEED), pending, feed.as_ref().map(|_| 3000));
    {
        // waking from sleep or getting the internet back: weather, prayer times, news and a failed
        // update check catch up at once (5 s for Wi-Fi to reconnect; several triggers count once)
        let (system, prayer, news, updater) = (system.clone(), prayer.clone(), news.clone(), updater.clone());
        let gen = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let wake = move || {
            let my = gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let (system, prayer, news, updater, gen) = (system.clone(), prayer.clone(), news.clone(), updater.clone(), gen.clone());
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                if gen.load(std::sync::atomic::Ordering::SeqCst) != my {
                    return;
                }
                system.refresh_now();
                prayer.retry();
                updater.retry_soon();
                news.retry_soon();
            });
        };
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let _ = WAKE.set(wake.clone());
        network.on_back(move || wake());
    }
    let apps = nova_services::apps::Apps::new(ctx.clone());
    let shelf = nova_services::shelf::Shelf::new(ctx.clone());
    let downloads = nova_services::downloads::Downloads::new(ctx.clone());
    nova_services::downloads::register_mdm_source(crate::mdm::items, crate::mdm::control);
    {
        let dl_notify = downloads.clone();
        crate::mdm::set_on_change(move || dl_notify.publish_now());
    }
    let calendar = nova_services::calendar::service::Calendar::new(ctx.clone());
    let plugins = nova_services::plugins::Plugins::new(ctx.clone());
    let tasks = nova_services::tasks::Tasks::new(ctx.clone());
    let app_notify = nova_services::appnotify::AppNotify::new(ctx.clone());
    let shots = nova_services::screenshot::Screenshots::new(ctx.clone(), shelf.clone());
    let app_rc = Rc::new(RefCell::new(App { core: core::Core::new(win, send, Store::new(dir.clone()), features), glass: glass::Glass::new(), env: None, page: None, sys, sys_lines: line_rx, bus, bus_rx, shared_features, system, prayer, network, privacy, apps, shelf, downloads, news, calendar, plugins, shots, updater, app_notify, prefs: None, radio: None, tasks_win: None, tasks, radio_now: serde_json::json!({}), clip_seq: None, clips: clips::ClipsManager::new(dir.clone()), data_dir: dir.clone() }));
    APP.with(|a| *a.borrow_mut() = Some(app_rc.clone()));

    // HardwareMediaKeyHandling off, like nova.js: the island's own audio (Quran Radio, the adhan)
    // must not show up as a Windows media session, or the island shows itself as "now playing"
    nova_core::log::info("startup: services");
    adhan::apply();
    // tests: --devtools-port N drives the pages from a script (its own WebView folder, so it
    // doesn't join the installed app's browser, which would ignore the port)
    let base_args = "--autoplay-policy=no-user-gesture-required --disable-features=HardwareMediaKeyHandling --disable-background-networking --disable-sync --disable-default-apps --disable-component-update --disable-extensions --js-flags=--max-old-space-size=64";
    let (web_dir, web_args) = match arg("--devtools-port") {
        Some(port) => (format!("{}\\Nova Island\\EBWebView-test", std::env::var("LOCALAPPDATA").unwrap_or_default()), format!("{base_args} --remote-debugging-port={port}")),
        None => (format!("{}\\Nova Island\\EBWebView", std::env::var("LOCALAPPDATA").unwrap_or_default()), base_args.to_string()),
    };
    let env = nova_web::environment(&web_dir, &web_args).map_err(|e| format!("WebView2: {e}"))?;
    nova_core::log::info("startup: webview2");
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let page = Rc::new(Page::create(&env, hwnd, r.right, r.bottom, mock.is_none().then_some(Bridge::Nova), move |msg| {
        if let Some(a) = app() {
            router::handle(&a, msg, router::Src::Island);
        }
    }).map_err(|e| format!("WebView2 page: {e}"))?);
    nova_core::log::info("startup: page");
    // the island's page at the screen's scale only (see Page::pin_scale)
    page.pin_scale(unsafe { GetDpiForWindow(hwnd) } as f64 / 96.0);
    // tests: --test-text-scale 1.25 draws the island as a 125% Windows Text size made WebView2
    // do before 1.8.6; with --test-no-pin it stays so (the bug), without it the pin undoes it
    if let Some(t) = arg("--test-text-scale").and_then(|v| v.parse::<f64>().ok()) {
        let s = unsafe { GetDpiForWindow(hwnd) } as f64 / 96.0;
        page.force_scale(s * t);
        if std::env::args().any(|x| x == "--test-no-pin") {
            TEST_NO_PIN.store(true, std::sync::atomic::Ordering::Relaxed);
        } else {
            page.pin_scale(s);
        }
    }
    *page_slot.borrow_mut() = Some(page.clone());
    app_rc.borrow_mut().page = Some(page.clone());
    app_rc.borrow_mut().env = Some(env.clone());
    page.on_console(|line| nova_core::log::error(&format!("[renderer] {line}")))?;
    // (1.8.7) the page's process failed: load it again; the whole of WebView2 gone: start a new
    // copy of Nova and close this one (a page can't be made again without it). The page's own
    // crashes and hangs (a GPU driver, an antivirus ending msedgewebview2.exe, no memory left)
    // used to leave an invisible island until the PC restarted.
    {
        let pg = Rc::downgrade(&page);
        let reloads = Cell::new(0u32);
        page.on_process_failed(move |kind, gone| {
            nova_core::log::error(&format!("[webview] {kind}"));
            if !kind.starts_with("GPU") && !kind.starts_with("a helper") {
                set_page_alive(false);
            }
            if gone || reloads.get() >= 5 {
                restart_self(if gone { "WebView2 stopped" } else { "the page kept failing" });
                return;
            }
            // GPU / helper processes come back by themselves; the page's own need a reload
            if kind.starts_with("page") || kind.starts_with("frame") {
                reloads.set(reloads.get() + 1);
                if let Some(p) = pg.upgrade() {
                    p.reload();
                }
            }
        })?;
    }
    if let Some(bg) = arg("--bg").and_then(|h| u32::from_str_radix(&h, 16).ok()) {
        page.set_background((bg >> 16) as u8, (bg >> 8) as u8, bg as u8, 255)?;
    }

    match &mock {
        Some(query) => {
            let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().map_err(|e| e.to_string())?;
            let repo = repo.to_string_lossy().trim_start_matches(r"\\?\").to_string();
            unsafe {
                let wv3: ICoreWebView2_3 = page.webview.cast().map_err(|e| e.to_string())?;
                wv3.SetVirtualHostNameToFolderMapping(&HSTRING::from("nova.dev"), &HSTRING::from(repo), COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS).map_err(|e| e.to_string())?;
            }
            page.navigate(&format!("https://nova.dev/promo/src/island.html?{query}"))?;
            if let Some((x, y)) = arg("--pos").and_then(|p| p.split_once(',').map(|(a, b)| (a.parse::<f64>().unwrap_or(0.0), b.parse::<f64>().unwrap_or(0.0)))) {
                app_rc.borrow().core.win.set_pos(x, y);
            }
            app_rc.borrow().core.win.show_inactive();
        }
        None => {
            page.navigate(&format!("{}index.html", nova_web::ORIGIN))?;
            let mut s = app_rc.borrow_mut();
            s.core.place_window();
            s.core.update_visibility();
            drop(s);
            tray::add(hwnd);
            unsafe {
                if RegisterHotKey(Some(hwnd), HOTKEY_TOGGLE, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'I' as u32).is_err() {
                    nova_core::log::warn("Ctrl+Alt+I is taken by another app");
                }
            }
            schedule_cursor(hwnd, 40);
            unsafe {
                let _ = windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(hwnd, 0);
            }
            after_change(&app_rc);
            idle_test::start(&app_rc);
            schedule_capture(hwnd, 300);
            if arg("--prefs-pages").is_some() {
                router::record_start();
            }
            if std::env::args().any(|x| x == "--open-settings") {
                // tests: --settings-page widgets opens that page
                prefs::open(arg("--settings-page"));
            }
            // tests: a video in the mini player, as if popped out of a tab
            if let Some(v) = arg("--open-mini") {
                mini::open(&app_rc, mini::Session { id: v, t: 30.0, app_id: "NovaTest".into(), title: "Mini player test".into() });
            }
            let s = app_rc.borrow();
            s.system.start();
            s.prayer.start();
            s.network.start();
            s.privacy.start();
            s.apps.init();
            s.shelf.start();
            s.downloads.start();
            s.news.start();
            s.calendar.start();
            s.plugins.start();
            s.tasks.start();
            if s.core.features.get()["appNotify"] == true {
                nova_sys::notify::request();
            }
            s.app_notify.start();
            // the desktop widgets, as Settings › Widgets left them (native: Direct2D, no WebView)
            crate::widgets::start(&app_rc);
            // tests: a sample notification from an app (by its AppUserModelId), to see the look
            if let Some(id) = arg("--fake-notify") {
                let bus = s.bus.clone();
                let text = arg("--fake-text").unwrap_or_else(|| "Test message from Nova Island".into());
                let title = arg("--fake-title").unwrap_or_else(|| "Ahmed".into());
                std::thread::spawn(move || {
                    let (icon, rgb) = nova_sys::icons::start_icon(&id, 44).map(|(i, c)| (json!(i), json!(c))).unwrap_or((json!(""), json!(null)));
                    std::thread::sleep(std::time::Duration::from_millis(2500));
                    bus.event("app-notify", json!({ "app": "Test", "title": title, "body": text, "more": 0, "id": 1, "icon": icon, "rgb": rgb }));
                });
            }
            if (startup::installed() && !startup::store()) || arg("--update-feed").is_some() {
                s.updater.start();
            }
        }
    }

    // tests: close Settings after this long (memory check)
    let mut prefs_close_at = arg("--prefs-close-after").and_then(|v| v.parse::<u64>().ok()).map(|ms| Instant::now() + Duration::from_millis(ms));
    let exit_at = arg("--exit-after").and_then(|v| v.parse::<u64>().ok()).map(|ms| Instant::now() + Duration::from_millis(ms));
    let mut msg = MSG::default();
    loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    // an update nobody restarted for goes in as the app quits
                    app_rc.borrow().updater.install_on_quit();
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if prefs_close_at.is_some_and(|t| Instant::now() >= t) {
                prefs_close_at = None;
                prefs::close();
            }
            if exit_at.is_some_and(|t| Instant::now() >= t) {
                if let Some(file) = arg("--capture-prefs") {
                    let pg = app_rc.borrow().prefs.as_ref().map(|p| p.page.clone());
                    if let Some(pg) = pg {
                        if let Some(js) = arg("--eval-prefs") {
                            println!("eval-prefs: {}", pg.eval(&js)?);
                        }
                        pg.capture_png(&file)?;
                    }
                }
                // tests: act on the island page, then let it run a while before --eval / --capture
                if let Some(js) = arg("--eval-wait") {
                    println!("eval-wait: {}", page.eval(&js)?);
                    pump(4000);
                }
                // tests: Settings opened from the island page (inside a WebView2 event)
                if std::env::args().any(|x| x == "--check-open-settings") {
                    page.eval("setTimeout(() => nova.openSettings(), 500), 1")?;
                    pump(5000);
                    println!("settings open: {}", app_rc.borrow().prefs.is_some());
                }
                // tests: each Settings page as a picture, and what the page was given
                if let Some(dir) = arg("--prefs-pages") {
                    let pg = app_rc.borrow().prefs.as_ref().map(|p| p.page.clone());
                    if let Some(pg) = pg {
                        for id in ["general", "island", "themes-page", "faith", "alerts", "connections", "plugins", "support", "more"] {
                            pg.eval(&format!("document.querySelector('[data-page=\"{id}\"]').click(); window.scrollTo(0, 0); 1"))?;
                            pump(900);
                            pg.capture_png(&format!("{dir}/{id}-rust.png"))?;
                        }
                    }
                    let _ = std::fs::write(format!("{dir}/record.json"), serde_json::to_string_pretty(&router::record_take()).unwrap_or_default());
                }
                // tests: the Radio window
                if let Some(pg) = prefs::page(&app_rc, prefs::Kind::Radio) {
                    if let Some(js) = arg("--eval-radio") {
                        println!("eval-radio: {}", pg.eval(&js)?);
                        pump(2500);
                    }
                    if let Some(file) = arg("--capture-radio") {
                        pg.capture_png(&file)?;
                    }
                }
                // tests: the Tasks window (1.8.3)
                if let Some(pg) = prefs::page(&app_rc, prefs::Kind::Tasks) {
                    if let Some(js) = arg("--eval-tasks") {
                        println!("eval-tasks: {}", pg.eval(&js)?);
                        pump(1500);
                    }
                    if let Some(file) = arg("--capture-tasks") {
                        pg.capture_png(&file)?;
                    }
                }
                if let Some(pg) = mini::page() {
                    if let Some(js) = arg("--eval-mini") {
                        println!("eval-mini: {}", pg.eval(&js)?);
                    }
                    if let (Some(file), Some(h)) = (arg("--shot-mini"), mini::hwnd()) {
                        let mut wr = RECT::default();
                        let _ = GetWindowRect(h, &mut wr);
                        shot::save_bmp(wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top, &file)?;
                    }
                }
                if let Some(js) = arg("--eval") {
                    println!("eval: {}", page.eval(&js)?);
                }
                if let Some(file) = arg("--capture") {
                    page.capture_png(&file)?;
                }
                if let Some(file) = arg("--shot") {
                    let mut wr = RECT::default();
                    let _ = GetWindowRect(hwnd, &mut wr);
                    shot::save_bmp(wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top, &file)?;
                }
                router::print_summary();
                let _ = page.controller.Close();
                let _ = DestroyWindow(hwnd);
                return Ok(());
            }
            let _ = MsgWaitForMultipleObjects(None, false, 10, QS_ALLINPUT);
        }
    }
}

/// Runs the message loop for `ms` (tests).
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    while Instant::now() < end {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = MsgWaitForMultipleObjects(None, false, 10, QS_ALLINPUT);
        }
    }
}

/// A link from news or the island: nova://plugins/<id> and nova://settings/<page> open Settings
/// (step 7), https links the browser.
pub fn open_nova_link(link: &str) {
    if let Some(rest) = link.strip_prefix("nova://plugins/") {
        prefs::open(Some(format!("plugins:{rest}")));
    } else if let Some(rest) = link.strip_prefix("nova://settings/") {
        prefs::open(Some(rest.to_string()));
    } else {
        bus::open_external(link);
    }
}

/// Starts or stops following the cursor while the island is dragged (8 ms, like nova.js).
pub fn drag_timer(hwnd: HWND, on: bool) {
    unsafe {
        if on {
            SetTimer(Some(hwnd), TIMER_DRAG, 8, None);
        } else {
            let _ = KillTimer(Some(hwnd), TIMER_DRAG);
        }
    }
}

/// "Copy report" (log.js report + nova.js copy-report): versions, state, and the last log lines.
pub fn report(a: &Rc<RefCell<App>>) -> String {
    let s = a.borrow();
    let f = s.core.features.get();
    let on = f.as_object().map(|o| o.values().filter(|v| **v == Value::Bool(true)).count()).unwrap_or(0);
    let wv = s.env.as_ref().and_then(|e| unsafe {
        let mut p = windows::core::PWSTR::null();
        e.BrowserVersionString(&mut p).ok().map(|_| webview2_com::CoTaskMemPWSTR::from(p).to_string())
    }).unwrap_or_default();
    let mut v = windows::Win32::System::SystemInformation::OSVERSIONINFOW { dwOSVersionInfoSize: std::mem::size_of::<windows::Win32::System::SystemInformation::OSVERSIONINFOW>() as u32, ..Default::default() };
    unsafe {
        // RtlGetVersion tells the real Windows version (GetVersionEx is capped by the manifest)
        windows::core::link!("ntdll.dll" "system" fn RtlGetVersion(info: *mut windows::Win32::System::SystemInformation::OSVERSIONINFOW) -> i32);
        RtlGetVersion(&mut v);
    }
    let (total, _) = nova_sys::sysinfo::memory();
    let installed = startup::installed();
    nova_core::log::report(
        &format!("Nova Island {} ({})", env!("CARGO_PKG_VERSION"), if installed { "installed" } else { "dev" }),
        &format!("WebView2 {wv}"),
        &format!("Windows {}.{}.{} · x64 · {} GB", v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber, (total / 1e9).round()),
        &[
            ("bridge", r#"{"state":"ok","detail":"","engine":"built-in"}"#.to_string()),
            ("location", s.system.location_info()["mode"].as_str().unwrap_or("auto").to_string()),
            ("features", format!("{on} on")),
            ("vertical", s.core.prefs.get("vertical").and_then(Value::as_bool).unwrap_or(false).to_string()),
            ("calendar", if s.calendar.has_url() { "linked" } else { "none" }.to_string()),
        ],
    )
}
