//! VLC 3 doesn't publish a Windows media session, so it is read directly (Vlc.cs):
//!   title    the main window's title ("<now playing> - VLC media player")
//!   playing  VLC's own audio session is active (VLC stops its audio stream on pause) and made a
//!            sound in the last 10 s (quiet passages stay "playing")
//!   controls WM_APPCOMMAND play/pause, next, previous to the main window
//! No position, duration or artwork: VLC 3 doesn't expose them without its web interface.
use std::time::{Duration, Instant};

use windows::{
    core::{Interface, BOOL},
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Media::Audio::{eMultimedia, eRender, AudioSessionStateActive, Endpoints::IAudioMeterInformation, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator},
        System::Com::{CoCreateInstance, CLSCTX_ALL},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindow, PostMessageW, WM_APPCOMMAND},
    },
};

const SUFFIX: &str = " - VLC media player";
const IDLE_TITLE: &str = "VLC media player";
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

pub struct Vlc {
    pub window: HWND,
    pub title: String,
    pid: u32,
    heard_at: Option<Instant>,
    next_scan: Option<Instant>,
}

struct Search {
    found: HWND,
    title: String,
    pid: u32,
}

unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
    let s = &mut *(l.0 as *mut Search);
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(h, &mut buf) as usize;
    if n <= SUFFIX.len() {
        return true.into();
    }
    let t = String::from_utf16_lossy(&buf[..n]);
    let Some(title) = t.strip_suffix(SUFFIX) else { return true.into() };
    let mut p = 0u32;
    GetWindowThreadProcessId(h, Some(&mut p));
    if p != s.pid {
        if !crate::win::process_name(p).eq_ignore_ascii_case("vlc") {
            return true.into();
        }
        s.pid = p;
    }
    s.found = h;
    s.title = title.trim().to_string();
    false.into()
}

impl Vlc {
    pub fn new() -> Vlc {
        Vlc { window: HWND::default(), title: String::new(), pid: 0, heard_at: None, next_scan: None }
    }

    /// Fills window/title; false when VLC isn't running or has nothing open. Hidden windows count:
    /// VLC minimized to the tray hides its main window.
    pub fn find(&mut self) -> bool {
        if !self.window.is_invalid() && unsafe { IsWindow(Some(self.window)).as_bool() } {
            let mut buf = [0u16; 512];
            let n = unsafe { GetWindowTextW(self.window, &mut buf) } as usize;
            let t = String::from_utf16_lossy(&buf[..n]);
            if let Some(title) = t.strip_suffix(SUFFIX) {
                self.title = title.trim().to_string();
                self.next_scan = None;
                return !self.title.is_empty();
            }
            if t.eq_ignore_ascii_case(IDLE_TITLE) {
                self.title.clear();
                return false;
            }
        }
        let now = Instant::now();
        if self.next_scan.is_some_and(|at| now < at) {
            return false;
        }
        let mut s = Search { found: HWND::default(), title: String::new(), pid: self.pid };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut s as *mut Search as isize));
        }
        self.pid = s.pid;
        self.window = s.found;
        self.title = s.title;
        self.next_scan = if self.window.is_invalid() { Some(now + SCAN_INTERVAL) } else { None };
        !self.window.is_invalid() && !self.title.is_empty()
    }

    pub fn playing(&mut self) -> bool {
        let (mut active, mut peak) = (false, 0f32);
        let look = || -> windows::core::Result<(bool, f32)> {
            let (mut active, mut peak) = (false, 0f32);
            unsafe {
                let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let dev = en.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
                let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None)?;
                let list = mgr.GetSessionEnumerator()?;
                for i in 0..list.GetCount()? {
                    let Ok(s) = list.GetSession(i) else { continue };
                    let Ok(c) = s.cast::<IAudioSessionControl2>() else { continue };
                    if c.GetProcessId().ok() != Some(self.pid) {
                        continue;
                    }
                    if c.GetState().ok() == Some(AudioSessionStateActive) {
                        active = true;
                    }
                    if let Ok(m) = s.cast::<IAudioMeterInformation>() {
                        if let Ok(v) = m.GetPeakValue() {
                            peak = peak.max(v);
                        }
                    }
                }
            }
            Ok((active, peak))
        };
        if let Ok((a, p)) = look() {
            active = a;
            peak = p;
        }
        if active && peak > 0.0005 {
            self.heard_at = Some(Instant::now());
        }
        active && self.heard_at.is_some_and(|t| t.elapsed() < Duration::from_secs(10))
    }

    /// toggle | next | prev
    pub fn command(&self, cmd: &str) {
        let app: isize = match cmd {
            "toggle" => 14,
            "next" => 11,
            "prev" => 12,
            _ => return,
        };
        if self.window.is_invalid() {
            return;
        }
        unsafe {
            let _ = PostMessageW(Some(self.window), WM_APPCOMMAND, WPARAM(self.window.0 as usize), LPARAM(app << 16));
        }
    }
}
