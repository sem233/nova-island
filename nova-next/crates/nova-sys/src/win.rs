//! The window in front, full screen, keyboard layout and Caps Lock, the clipboard's change counter
//! and private copies, idle time, battery, screen recorders, and the browser in front (Win in
//! NovaHelper.cs). Rectangles are physical pixels; nova-app converts them to DIPs.
use windows::{
    core::{w, PWSTR},
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, RECT},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW},
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            Memory::{GlobalLock, GlobalUnlock},
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            SystemInformation::GetTickCount,
            Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, GetKeyboardLayout, GetLastInputInfo, LASTINPUTINFO},
            WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible},
        },
    },
};

pub fn foreground() -> HWND {
    unsafe { GetForegroundWindow() }
}

pub fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The exe name without ".exe", like .NET's Process.ProcessName ("" when it can't be read).
pub fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        let file = path.rsplit('\\').next().unwrap_or("");
        match file.rfind('.') {
            Some(i) if file[i..].eq_ignore_ascii_case(".exe") => file[..i].to_string(),
            _ => file.to_string(),
        }
    }
}

/// One coherent read of the foreground window (hwnd, exe stem, title, keyboard layout, fullscreen)
/// so nova-sys doesn't repeat OpenProcess / QueryFullProcessImageNameW three times per tick.
pub struct FgSnapshot {
    pub hwnd: HWND,
    pub proc_name: String,
    pub title: String,
    pub layout: i64,
    pub fullscreen: bool,
}

fn monitor_bounds_match(h: HWND, r: &RECT) -> bool {
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left == m.left && r.top == m.top && r.right == m.right && r.bottom == m.bottom
    }
}

/// Reads the foreground window once per tick and reuses its PID/handle across fields.
pub fn foreground_snapshot() -> FgSnapshot {
    let h = foreground();
    if h.is_invalid() {
        return FgSnapshot { hwnd: HWND::default(), proc_name: String::new(), title: String::new(), layout: 0, fullscreen: false };
    }
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    let layout = unsafe { (GetKeyboardLayout(thread).0 as usize & 0xffff) as i64 };
    let proc_name = process_name(pid);
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);

    let mut fullscreen = false;
    let mut r = RECT::default();
    if unsafe { GetWindowRect(h, &mut r) }.is_ok() && monitor_bounds_match(h, &r) {
        fullscreen = may_hide_island(&class_of(h), &proc_name);
    }
    FgSnapshot { hwnd: h, proc_name, title, layout, fullscreen }
}

/// (process name, window title) of the window in front.
pub fn foreground_info() -> (String, String) {
    let h = foreground();
    if h.is_invalid() {
        return (String::new(), String::new());
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    (process_name(pid), String::from_utf16_lossy(&buf[..n.max(0) as usize]))
}

/// The window in front covers its whole monitor (the desktop and taskbar don't count).
pub fn is_fullscreen() -> bool {
    let h = foreground();
    if h.is_invalid() {
        return false;
    }
    let mut r = RECT::default();
    if unsafe { GetWindowRect(h, &mut r) }.is_err() || !monitor_bounds_match(h, &r) {
        return false;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    may_hide_island(&class_of(h), &process_name(pid))
}

/// Whether a window covering the whole screen counts as "full screen" (Hide in full screen). Not
/// for what only looks full screen but is the user at work, each a bug that came back once:
///   the desktop and the taskbar
///   File Explorer in full screen (F11): it hid the island, and every photo opened from it came
///     back to a hidden island (1.8.1)
///   the Snipping Tool's overlay (Win+Shift+S): the island hid, so no screenshot had it (1.8.3)
pub fn may_hide_island(class: &str, process: &str) -> bool {
    if matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "CabinetWClass" | "ExploreWClass") {
        return false;
    }
    let p = process.to_ascii_lowercase();
    !matches!(p.as_str(), "snippingtool" | "screenclippinghost" | "screensketch")
}

/// The keyboard layout (LANGID) of the window in front's thread.
pub fn keyboard_layout() -> i64 {
    unsafe {
        let h = foreground();
        let thread = if h.is_invalid() { 0 } else { GetWindowThreadProcessId(h, None) };
        (GetKeyboardLayout(thread).0 as usize & 0xffff) as i64
    }
}

pub fn caps_lock() -> bool {
    unsafe { (GetKeyState(0x14) & 1) != 0 }
}

pub fn clipboard_seq() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Password managers mark what they copy so history tools skip it: either the "Exclude..." format
/// is present, or CanIncludeInClipboardHistory is 0. Checked once per clipboard change.
pub struct Private {
    seq: Option<u32>,
    value: bool,
    // the last change whose answer is known, as reported; and tries on the one still open
    shown: Option<(u32, bool)>,
    tries: u32,
}

/// Ticks (500 ms each) to wait for a clipboard someone still holds, before calling it private.
const PRIVATE_TRIES: u32 = 4;

impl Private {
    pub fn new() -> Private {
        Private { seq: None, value: false, shown: None, tries: 0 }
    }

    /// Private or not, for the clipboard as it is (`seq`). None: it can't be read right now.
    pub fn check(&mut self, seq: u32) -> Option<bool> {
        if self.seq == Some(seq) {
            return Some(self.value);
        }
        let value = unsafe { Self::look() }?;
        self.seq = Some(seq);
        self.value = value;
        Some(value)
    }

    /// (sequence number, private) to report: a new change only once its answer is known, or after
    /// PRIVATE_TRIES ticks of a clipboard nobody can open (then private, as before). Before, a
    /// clipboard still held by the app that copied (the Snipping Tool, writing a screenshot) was
    /// called private at once, so the screenshot toast and its thumbnail never came.
    pub fn settled(&mut self, seq: u32) -> (u32, bool) {
        match self.check(seq) {
            Some(p) => {
                self.tries = 0;
                self.shown = Some((seq, p));
            }
            None => {
                self.tries += 1;
                if self.tries >= PRIVATE_TRIES || self.shown.is_none() {
                    self.tries = 0;
                    self.seq = Some(seq);
                    self.value = true;
                    self.shown = Some((seq, true));
                }
            }
        }
        self.shown.unwrap_or((seq, true))
    }

    /// None: the clipboard is held by someone else just now.
    unsafe fn look() -> Option<bool> {
        if IsClipboardFormatAvailable(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))).is_ok() {
            return Some(true);
        }
        let can = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if IsClipboardFormatAvailable(can).is_err() {
            return Some(false);
        }
        OpenClipboard(None).ok()?;
        let result = match GetClipboardData(can) {
            Ok(h) if !h.is_invalid() => {
                let g = windows::Win32::Foundation::HGLOBAL(h.0);
                let p = GlobalLock(g) as *const i32;
                let private = p.is_null() || *p == 0;
                let _ = GlobalUnlock(g);
                Some(private)
            }
            // not rendered yet: look again
            _ => None,
        };
        let _ = CloseClipboard();
        result
    }
}

/// Seconds since the last keyboard or mouse input anywhere.
pub fn idle_seconds() -> i64 {
    unsafe {
        let mut lii = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !GetLastInputInfo(&mut lii).as_bool() {
            return 0;
        }
        (GetTickCount().wrapping_sub(lii.dwTime) / 1000) as i64
    }
}

/// (battery percent or -1 without a battery, charger plugged in)
pub fn battery() -> (i64, bool) {
    unsafe {
        let mut s = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut s).is_err() {
            return (-1, true);
        }
        let pct = if (s.BatteryFlag & 128) == 0 && s.BatteryLifePercent <= 100 { s.BatteryLifePercent as i64 } else { -1 };
        (pct, s.ACLineStatus != 0)
    }
}

fn eq_exe_stem_ignore_ascii_case(wide: &[u16], stem: &[u8]) -> bool {
    let len = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    let slice = &wide[..len];
    let slice = if slice.len() >= 4
        && slice[slice.len() - 4..].iter().zip(b".exe").all(|(&w, &b)| w < 128 && (w as u8).to_ascii_lowercase() == b)
    {
        &slice[..slice.len() - 4]
    } else {
        slice
    };
    slice.len() == stem.len() && slice.iter().zip(stem).all(|(&w, &b)| w < 128 && (w as u8).to_ascii_lowercase() == b)
}

/// NVIDIA's capture helper, OBS or Streamlabs running (the island then doesn't hide from captures).
pub fn recorder_running() -> bool {
    const RECORDERS: &[&[u8]] = &[
        b"nvsphelper64",
        b"nvsphelper",
        b"obs64",
        b"obs32",
        b"obs",
        b"streamlabs obs",
        b"streamlabs desktop",
        b"bdcam",
        b"camrecorder",
    ];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            if RECORDERS.iter().any(|stem| eq_exe_stem_ignore_ascii_case(&e.szExeFile, stem)) {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(HANDLE(snap.0));
        found
    }
}

pub fn is_browser_proc(proc: &str) -> bool {
    const BROWSERS: &[&str] = &["chrome", "msedge", "brave", "opera", "vivaldi", "firefox"];
    BROWSERS.iter().any(|b| proc.eq_ignore_ascii_case(b))
}

fn is_main_browser_rect(r: &RECT) -> bool {
    (r.right - r.left) >= 320 && (r.bottom - r.top) >= 240
}

fn format_browser_rect(r: &RECT) -> String {
    format!("{{\"x\":{},\"y\":{},\"w\":{},\"h\":{}}}", r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn valid_browser_rect(h: HWND) -> Option<RECT> {
    if h.is_invalid() {
        return None;
    }
    let visible = unsafe { IsWindow(Some(h)).as_bool() && IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() };
    if !visible {
        return None;
    }
    let mut r = RECT::default();
    if unsafe { GetWindowRect(h, &mut r) }.is_err() || !is_main_browser_rect(&r) {
        return None;
    }
    Some(r)
}

/// Tracks the foreground browser window so browser popups/menus don't flap "browser line: on/off".
pub struct BrowserTracker {
    hwnd: HWND,
    rect: Option<RECT>,
    cached: String,
}

impl BrowserTracker {
    pub fn new() -> Self {
        Self { hwnd: HWND::default(), rect: None, cached: String::from("null") }
    }

    pub fn json(&mut self, fg: HWND, proc: &str, fullscreen: bool) -> &str {
        if fullscreen || !is_browser_proc(proc) {
            self.clear();
            return "null";
        }
        if let Some(r) = valid_browser_rect(fg) {
            if self.hwnd != fg || self.rect.is_none_or(|prev| prev != r) {
                self.hwnd = fg;
                self.rect = Some(r);
                self.cached = format_browser_rect(&r);
            }
            return &self.cached;
        }
        // Small browser popup/dropdown (w < 320 or h < 240): keep reporting the parent browser rect while open.
        if let Some(r) = valid_browser_rect(self.hwnd) {
            if self.rect.is_none_or(|prev| prev != r) {
                self.rect = Some(r);
                self.cached = format_browser_rect(&r);
            }
            return &self.cached;
        }
        self.clear();
        "null"
    }

    fn clear(&mut self) {
        self.hwnd = HWND::default();
        self.rect = None;
        if self.cached != "null" {
            self.cached.clear();
            self.cached.push_str("null");
        }
    }
}

/// The browser window in front, for "Stay in place": {"x","y","w","h"} or "null".
pub fn browser_json(proc: &str, fullscreen: bool) -> String {
    if fullscreen || !is_browser_proc(proc) {
        return "null".into();
    }
    valid_browser_rect(foreground()).map_or_else(|| "null".into(), |r| format_browser_rect(&r))
}

#[cfg(test)]
mod fullscreen_tests {
    use super::may_hide_island;

    // each case is a bug users met: these must keep the island on screen
    #[test]
    fn work_that_only_looks_full_screen_keeps_the_island() {
        assert!(!may_hide_island("CabinetWClass", "explorer"), "File Explorer in F11");
        assert!(!may_hide_island("XamlWindow", "SnippingTool"), "Win+Shift+S overlay (Windows 11)");
        assert!(!may_hide_island("ApplicationFrameWindow", "ScreenClippingHost"), "screen snip (Windows 10)");
        assert!(!may_hide_island("Progman", "explorer"), "the desktop");
        assert!(!may_hide_island("Shell_TrayWnd", "explorer"), "the taskbar");
    }

    #[test]
    fn real_full_screen_still_hides_it() {
        assert!(may_hide_island("WinUIDesktopWin32WindowClass", "Photos"), "a photo in full screen");
        assert!(may_hide_island("Chrome_WidgetWin_1", "chrome"), "a video in full screen");
        assert!(may_hide_island("UnityWndClass", "game"), "a game");
    }
}

#[cfg(test)]
mod perf_tests {
    use super::{eq_exe_stem_ignore_ascii_case, is_browser_proc, is_main_browser_rect};
    use windows::Win32::Foundation::RECT;

    #[test]
    fn recorder_match_without_allocating() {
        let obs: Vec<u16> = "OBS64.EXE\0".encode_utf16().collect();
        assert!(eq_exe_stem_ignore_ascii_case(&obs, b"obs64"));
        let chrome: Vec<u16> = "chrome.exe\0".encode_utf16().collect();
        assert!(!eq_exe_stem_ignore_ascii_case(&chrome, b"obs64"));
    }

    #[test]
    fn browser_rect_filters_tiny_popups() {
        assert!(is_browser_proc("Chrome"));
        assert!(!is_browser_proc("explorer"));
        assert!(!is_main_browser_rect(&RECT { left: 10, top: 10, right: 200, bottom: 90 }));
        assert!(is_main_browser_rect(&RECT { left: 0, top: 0, right: 1200, bottom: 800 }));
    }
}
