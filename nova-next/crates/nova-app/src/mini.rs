// Floating mini player for YouTube videos popped out of the browser (mini.js).
// The page is served from a local http server so the YouTube embed gets a real origin
// and referrer (embeds refuse to play without one). The window is made when a video opens and
// destroyed when it closes; sizes and positions are in DIPs like Electron's.
use std::{
    cell::RefCell,
    io::{Read, Write},
    net::TcpListener,
    rc::Rc,
    sync::OnceLock,
};

use nova_web::{Bridge, Page};
use serde_json::{json, Value};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, CoTaskMemPWSTR, NavigationStartingEventHandler, NewWindowRequestedEventHandler};
use windows::{
    core::{w, Interface, PWSTR},
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::*,
    },
};

use crate::{app, screen, App};

const FILES: &[(&str, &str)] = &[("/index.html", "text/html"), ("/mini.css", "text/css"), ("/mini.js", "text/javascript")];
const PAD: f64 = 10.0; // transparent margin around the card (room for its shadow)
const MIN_W: f64 = 280.0;
const MAX_W: f64 = 960.0;
const SNAP_PX: f64 = 60.0; // snap to a screen edge when dropped this close
const EDGE_GAP: f64 = 16.0;
const T_HOVER: usize = 1;
const T_RESIZE: usize = 2;

/// The tab a video came from.
pub struct Session {
    pub id: String,
    pub t: f64,
    pub app_id: String,
    pub title: String,
}

struct Resize {
    corner_br: bool,
    start: (f64, f64),
    b: screen::Rect,
}

struct Win {
    hwnd: HWND,
    page: Rc<Page>,
    hovering: bool,
    resize: Option<Resize>,
}

thread_local! {
    static WIN: RefCell<Option<Win>> = const { RefCell::new(None) };
    static CURRENT: RefCell<Option<Session>> = const { RefCell::new(None) };
    static LAST_TIME: RefCell<f64> = const { RefCell::new(0.0) };
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn video_height(w: f64) -> f64 {
    (w * 9.0 / 16.0).round()
}

/// The server's port (started on first use, then kept for the app's life).
fn port() -> u16 {
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        let Ok(l) = TcpListener::bind("127.0.0.1:0") else { return 0 };
        let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let mut parts = head.split(' ');
                let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let path = target.split(['?', '#']).next().unwrap_or("");
                let file = FILES.iter().find(|(p, _)| *p == path).and_then(|(p, ty)| nova_web::asset(&format!("mini{p}")).map(|(b, _)| (b, *ty)));
                let _ = match (method, file) {
                    ("GET", Some((body, ty))) => s
                        .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {ty}; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
                        .and_then(|_| s.write_all(body)),
                    _ => s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                };
            }
        });
        port
    })
}

fn bounds(h: HWND) -> screen::Rect {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    let d = screen::rect_to_dip(r.left, r.top, r.right - r.left, r.bottom - r.top);
    screen::Rect { x: d.x.round(), y: d.y.round(), width: d.width.round(), height: d.height.round() }
}

fn set_bounds(h: HWND, b: screen::Rect) {
    let d = screen::matching(b);
    let (x, y) = d.to_phys(b.x, b.y);
    unsafe {
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, (b.width * d.scale).round() as i32, (b.height * d.scale).round() as i32, SWP_NOACTIVATE);
    }
}

fn save_bounds(a: &Rc<RefCell<App>>, h: HWND) {
    let b = bounds(h);
    let _ = a.borrow().core.store.save("mini.json", &json!({ "x": b.x, "y": b.y, "w": b.width - 2.0 * PAD }));
}

/// Snap the card to nearby screen edges after the user drops the window.
fn snap(a: &Rc<RefCell<App>>, h: HWND) {
    let b = bounds(h);
    let wa = screen::matching(b).work_area;
    let (mut x, mut y) = (b.x, b.y);
    let (cl, ct, cr, cb) = (b.x + PAD, b.y + PAD, b.x + b.width - PAD, b.y + b.height - PAD);
    if (cl - wa.x).abs() < SNAP_PX {
        x = wa.x + EDGE_GAP - PAD;
    }
    if (wa.x + wa.width - cr).abs() < SNAP_PX {
        x = wa.x + wa.width - b.width - EDGE_GAP + PAD;
    }
    if (ct - wa.y).abs() < SNAP_PX {
        y = wa.y + EDGE_GAP - PAD;
    }
    if (wa.y + wa.height - cb).abs() < SNAP_PX {
        y = wa.y + wa.height - b.height - EDGE_GAP + PAD;
    }
    x = clamp(x, wa.x - PAD, wa.x + wa.width - b.width + PAD);
    y = clamp(y, wa.y - PAD, wa.y + wa.height - b.height + PAD);
    if x != b.x || y != b.y {
        set_bounds(h, screen::Rect { x, y, ..b });
    }
    save_bounds(a, h);
}

fn poll_hover(h: HWND) {
    let (px, py) = screen::cursor();
    let b = bounds(h);
    let now = px >= b.x && px <= b.x + b.width && py >= b.y && py <= b.y + b.height;
    let page = WIN.with(|w| {
        let mut w = w.borrow_mut();
        let w = w.as_mut()?;
        (w.hovering != now).then(|| {
            w.hovering = now;
            w.page.clone()
        })
    });
    if let Some(p) = page {
        p.event("mini-hover", json!(now));
    }
}

fn resize_tick(h: HWND) {
    let Some((br, start, b)) = WIN.with(|w| w.borrow().as_ref().and_then(|w| w.resize.as_ref().map(|r| (r.corner_br, r.start, r.b)))) else { return };
    let (px, _) = screen::cursor();
    let dx = px - start.0;
    let w0 = b.width - 2.0 * PAD;
    let vw = clamp(if br { w0 + dx } else { w0 - dx }, MIN_W, MAX_W);
    let (width, height) = (vw + 2.0 * PAD, video_height(vw) + 2.0 * PAD);
    // grow away from the screen edge the window sits against
    let wa = screen::matching(b).work_area;
    let x = if br { b.x } else { b.x + b.width - width };
    let lower_half = b.y + b.height / 2.0 > wa.y + wa.height / 2.0;
    let y = if lower_half { b.y + b.height - height } else { b.y };
    set_bounds(h, screen::Rect { x, y, width, height });
}

extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match m {
            WM_TIMER if wp.0 == T_HOVER => poll_hover(h),
            WM_TIMER if wp.0 == T_RESIZE => resize_tick(h),
            WM_EXITSIZEMOVE => {
                if let Some(a) = app() {
                    snap(&a, h);
                }
            }
            WM_SIZE => {
                // the same for the mini player's own state (1.8.2)
                let page = match WIN.with(|w| w.try_borrow().map(|w| w.as_ref().map(|w| w.page.clone()))) {
                    Ok(p) => p,
                    Err(_) => {
                        let _ = PostMessageW(Some(h), WM_SIZE, WPARAM(0), LPARAM(0));
                        None
                    }
                };
                if let Some(p) = page {
                    let mut r = RECT::default();
                    let _ = GetClientRect(h, &mut r);
                    let _ = p.controller.SetBounds(r);
                }
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                return LRESULT(0);
            }
            WM_CLOSE => {
                destroy();
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(h, m, wp, lp)
    }
}

fn create(a: &Rc<RefCell<App>>) -> Result<(), String> {
    let env = a.borrow().env.clone().ok_or("no WebView2")?;
    let saved = a.borrow().core.store.load("mini.json").unwrap_or(Value::Null);
    let wa = screen::primary().work_area;
    let vw = clamp(saved["w"].as_f64().filter(|w| *w != 0.0).unwrap_or(420.0), MIN_W, MAX_W);
    let (width, height) = (vw + 2.0 * PAD, video_height(vw) + 2.0 * PAD);
    let x = saved["x"].as_f64().unwrap_or(wa.x + wa.width - width - EDGE_GAP + PAD);
    let y = saved["y"].as_f64().unwrap_or(wa.y + wa.height - height - EDGE_GAP + PAD);
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let cls = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: HINSTANCE(inst.0), lpszClassName: w!("NovaIslandMini"), hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?, ..Default::default() };
        RegisterClassW(&cls);
        // frameless, transparent, no taskbar button, floating on top
        CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP, w!("NovaIslandMini"), w!("Mini Player"), WS_POPUP, 0, 0, 1, 1, None, None, Some(HINSTANCE(inst.0)), None)
            .map_err(|e| e.to_string())?
    };
    set_bounds(hwnd, screen::Rect { x, y, width, height });
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let pg = Rc::new(Page::create(&env, hwnd, r.right, r.bottom, Some(Bridge::Mini), |msg| {
        if let Some(a) = app() {
            crate::router::handle(&a, msg, crate::router::Src::Mini);
        }
    })?);
    let own = format!("http://127.0.0.1:{}/", port());
    unsafe {
        // the bar (-webkit-app-region: drag) moves the window
        if let Ok(s) = pg.webview.Settings().and_then(|s| s.cast::<ICoreWebView2Settings9>()) {
            let _ = s.SetIsNonClientRegionSupportEnabled(true);
        }
        // links in the embed (video title, logo) open in the default browser, not an app window
        let mut token = 0;
        pg.webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        crate::bus::open_external(&CoTaskMemPWSTR::from(uri).to_string());
                        args.SetHandled(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| e.to_string())?;
        pg.webview
            .add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    if let Some(args) = args {
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        let uri = CoTaskMemPWSTR::from(uri).to_string();
                        if !uri.starts_with(&own) {
                            args.SetCancel(true)?;
                            crate::bus::open_external(&uri);
                        }
                    }
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| e.to_string())?;
        let _ = SetTimer(Some(hwnd), T_HOVER, 120, None);
    }
    pg.on_console(|line| nova_core::log::error(&format!("[mini] {line}")))?;
    WIN.with(|w| *w.borrow_mut() = Some(Win { hwnd, page: pg, hovering: false, resize: None }));
    snap(a, hwnd); // keep a restored position on screen
    Ok(())
}

/// Hand the video back to the tab at the time the mini player reached.
fn finish(time: f64, resume: bool) {
    let Some(s) = CURRENT.with(|c| c.borrow_mut().take()) else { return };
    let Some(a) = app() else { return };
    let sys = a.borrow().sys.clone();
    // back in the tab at the second the mini player reached
    let _ = sys.send(format!("seekapp {} {:.2}", s.app_id, time.max(0.0)));
    if resume {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = sys.send(format!("playapp {}", s.app_id));
        });
    }
}

pub fn open(a: &Rc<RefCell<App>>, s: Session) {
    let last = LAST_TIME.with(|t| *t.borrow());
    let other = CURRENT.with(|c| c.borrow().as_ref().is_some_and(|c| c.id != s.id));
    if other {
        finish(last, false); // previous video stays paused at its spot
    }
    LAST_TIME.with(|t| *t.borrow_mut() = s.t);
    let q = format!("v={}&t={:.2}&title={}", query(&s.id), s.t, query(&s.title));
    CURRENT.with(|c| *c.borrow_mut() = Some(s));
    if WIN.with(|w| w.borrow().is_none()) {
        if let Err(e) = create(a) {
            nova_core::log::error(&format!("[mini] can't open: {e}"));
            return;
        }
    }
    let Some((hwnd, page)) = WIN.with(|w| w.borrow().as_ref().map(|w| (w.hwnd, w.page.clone()))) else { return };
    let _ = page.navigate(&format!("http://127.0.0.1:{}/index.html?{q}", port()));
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// URLSearchParams encoding (application/x-www-form-urlencoded).
fn query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The window is gone: the tab picks up where the mini player stopped.
fn destroy() {
    let Some(w) = WIN.with(|w| w.borrow_mut().take()) else { return };
    unsafe {
        let _ = KillTimer(Some(w.hwnd), T_HOVER);
        let _ = KillTimer(Some(w.hwnd), T_RESIZE);
        let _ = w.page.controller.Close();
        let _ = DestroyWindow(w.hwnd);
    }
    finish(LAST_TIME.with(|t| *t.borrow()), true);
}

pub fn hwnd() -> Option<HWND> {
    WIN.with(|w| w.borrow().as_ref().map(|w| w.hwnd))
}

pub fn page() -> Option<Rc<Page>> {
    WIN.with(|w| w.borrow().as_ref().map(|w| w.page.clone()))
}

/// Messages from the mini page (mini/preload.js).
pub fn channel(a: &Rc<RefCell<App>>, channel: &str, args: &[Value]) {
    let arg0 = args.first().cloned().unwrap_or(Value::Null);
    match channel {
        "mini-time" => {
            if let Some(t) = arg0.as_f64() {
                LAST_TIME.with(|l| *l.borrow_mut() = t);
            }
        }
        "mini-close" => {
            if let Some(t) = arg0["time"].as_f64() {
                LAST_TIME.with(|l| *l.borrow_mut() = t);
            }
            // !!resume
            let resume = match &arg0["resume"] {
                Value::Null => false,
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
                Value::String(s) => !s.is_empty(),
                _ => true,
            };
            finish(LAST_TIME.with(|t| *t.borrow()), resume);
            destroy();
        }
        "mini-resize-start" => {
            let Some(hwnd) = WIN.with(|w| w.borrow().as_ref().map(|w| w.hwnd)) else { return };
            let r = Resize { corner_br: arg0.as_str() != Some("bl"), start: screen::cursor(), b: bounds(hwnd) };
            WIN.with(|w| {
                if let Some(w) = w.borrow_mut().as_mut() {
                    w.resize = Some(r);
                }
            });
            unsafe {
                let _ = SetTimer(Some(hwnd), T_RESIZE, 10, None);
            }
        }
        "mini-resize-end" => {
            let Some(hwnd) = WIN.with(|w| w.borrow_mut().as_mut().and_then(|w| w.resize.take().map(|_| w.hwnd))) else { return };
            unsafe {
                let _ = KillTimer(Some(hwnd), T_RESIZE);
            }
            snap(a, hwnd);
        }
        _ => {}
    }
}

const BROWSER_PROCS: &[&str] = &["chrome", "msedge", "brave", "vivaldi", "opera"];

pub fn browser_proc(app_id: &str) -> Option<&'static str> {
    let id = app_id.to_lowercase();
    if id.contains("chrome") {
        Some("chrome")
    } else if id.contains("edge") || id.contains("msedge") {
        Some("msedge")
    } else if id.contains("brave") {
        Some("brave")
    } else if id.contains("vivaldi") {
        Some("vivaldi")
    } else if id.contains("opera") {
        Some("opera")
    } else {
        None
    }
}

/// /^[\w.!\s-]+$/
pub fn app_id_ok(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'!' | b'-' | b' '))
}

/// /(?:[?&]v=|youtu\.be\/|\/shorts\/|\/live\/|\/embed\/)([\w-]{11})/
pub fn youtube_id(url: &str) -> Option<String> {
    const MARKS: &[&str] = &["?v=", "&v=", "youtu.be/", "/shorts/", "/live/", "/embed/"];
    let b = url.as_bytes();
    for i in 0..b.len() {
        for m in MARKS {
            if b[i..].starts_with(m.as_bytes()) {
                let s = i + m.len();
                if b.len() >= s + 11 && b[s..s + 11].iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')) {
                    return Some(url[s..s + 11].to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=3").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.youtube.com/watch?list=x&v=dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://www.youtube.com/shorts/abcdefghijk").as_deref(), Some("abcdefghijk"));
        assert_eq!(youtube_id("https://www.youtube.com/watch?v=short"), None);
        assert_eq!(query("a b&ج"), "a+b%26%D8%AC");
        assert!(app_id_ok("Chrome") && app_id_ok("MSEdge") && !app_id_ok("a b") && !app_id_ok(""));
        assert_eq!(browser_proc("Chrome"), Some("chrome"));
        assert_eq!(browser_proc("Spotify.exe"), None);
    }
}
