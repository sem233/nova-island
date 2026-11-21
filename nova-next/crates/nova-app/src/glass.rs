// The glass backdrop (nova.js): the screen area under the island, captured small for the renderer
// to blur. Re-captured often while the island is open or changing shape, rarely while it sits
// collapsed, not at all while hidden; an unchanged picture is not re-sent. The window is left out
// of captures (SetWindowDisplayAffinity) only while the glass needs it: a glass theme, glass on, no
// screen recorder (NVIDIA reads the request as protected video and stops Instant Replay).
// The battery saver and the performance switches live here too, as in nova.js.
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use webview2_com::{CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::*};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HWND, RECT},
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::{GetWindowRect, SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE},
    },
};

pub const CAPTURE_ACTIVE_MS: u32 = 700;
pub const CAPTURE_IDLE_MS: u32 = 10_000;
pub const CAPTURE_FROZEN_MS: u32 = 60 * 1000; // "Freeze glass while closed"

pub struct Glass {
    pub busy_until: Instant,
    pub screen_locked: bool,
    pub recording: bool,
    last_key: String,
    protected_now: Option<bool>,
    // the battery saver
    pub battery: (i64, bool),
    pub saver_on: bool,
    perf_key: String,
    buffers: Vec<ICoreWebView2SharedBuffer>,
    buffer_size: u64,
    turn: usize,
    /// the last frame
    pub last: Option<(Vec<u8>, i32, i32)>,
}

pub struct Perf {
    pub no_blur: bool,
    pub no_animations: bool,
    pub glass_freeze: bool,
}

impl Perf {
    pub fn slow(&self) -> bool {
        self.no_blur || self.no_animations || self.glass_freeze
    }
}

impl Glass {
    pub fn new() -> Glass {
        Glass { busy_until: Instant::now(), screen_locked: false, recording: false, last_key: String::new(), protected_now: None, battery: (-1, true), saver_on: false, perf_key: String::new(), buffers: Vec::new(), buffer_size: 0, turn: 0, last: None }
    }

    /// perf(): the user's switches, or all three while the battery saver is on
    pub fn perf(&self, f: &nova_core::features::Features) -> Perf {
        Perf { no_blur: f.bool("noBlur") || self.saver_on, no_animations: f.bool("noAnimations") || self.saver_on, glass_freeze: f.bool("glassFreeze") || self.saver_on }
    }

    /// islandFeatures(): features plus the performance switches and the saver
    pub fn island_features(&self, f: &nova_core::features::Features) -> Value {
        let p = self.perf(f);
        let mut v = f.get();
        v["noBlur"] = json!(p.no_blur);
        v["noAnimations"] = json!(p.no_animations);
        v["glassFreeze"] = json!(p.glass_freeze);
        v["saver"] = json!(self.saver_on);
        v
    }

    fn glass_wanted(&self, f: &nova_core::features::Features) -> bool {
        let theme = f.get()["theme"].as_str().unwrap_or("original").to_string();
        !self.perf(f).no_blur && !nova_core::features::SOLID_THEMES.contains(&theme.as_str())
    }

    pub fn glass_live(&self, f: &nova_core::features::Features) -> bool {
        self.glass_wanted(f) && !self.recording
    }

    /// updateProtection(): true when a capture should follow soon (150 ms)
    pub fn update_protection(&mut self, hwnd: HWND, f: &nova_core::features::Features) -> bool {
        let on = self.glass_live(f);
        if self.protected_now == Some(on) {
            return false;
        }
        self.protected_now = Some(on);
        // (1.8.4) never left out of captures for good: only for the moment the glass takes its
        // own picture (capture()). Before, a glass theme kept the island out of every screenshot
        // and recording but NVIDIA's and OBS's (it was missing from Snipping Tool screenshots,
        // Xbox Game Bar and other recorders).
        unsafe {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE);
        }
        if on {
            self.last_key.clear();
        }
        on
    }

    /// evalSaver(): Some(saver state) when it changed
    pub fn eval_saver(&mut self, f: &nova_core::features::Features) -> Option<bool> {
        let (pct, ac) = self.battery;
        let mut on = self.saver_on;
        if !f.bool("batterySaver") || pct < 0 || ac {
            on = false;
        } else if pct <= f.get()["batterySaverAt"].as_i64().unwrap_or(20) {
            on = true;
        } else if pct >= f.get()["batterySaverAt"].as_i64().unwrap_or(20) + 10 {
            on = false;
        }
        if on == self.saver_on {
            return None;
        }
        self.saver_on = on;
        nova_core::log::info(&format!("battery saver {} ({pct}%)", if on { "on" } else { "off" }));
        Some(on)
    }

    /// applyPerf(): true when the switches changed (features go out again, capture in 300 ms)
    pub fn apply_perf(&mut self, f: &nova_core::features::Features) -> bool {
        let p = self.perf(f);
        let key = format!("{}{}{}", p.no_blur, p.no_animations, p.glass_freeze);
        if key == self.perf_key {
            return false;
        }
        self.perf_key = key;
        self.last_key.clear();
        true
    }

    /// The delay before the next capture (scheduleCapture's choice).
    pub fn next_delay(&self, active: bool, f: &nova_core::features::Features) -> u32 {
        if active {
            CAPTURE_ACTIVE_MS
        } else if self.perf(f).glass_freeze {
            CAPTURE_FROZEN_MS
        } else if Instant::now() < self.busy_until {
            CAPTURE_ACTIVE_MS
        } else {
            CAPTURE_IDLE_MS
        }
    }

    pub fn shape_changed(&mut self) {
        self.busy_until = Instant::now() + Duration::from_secs(2);
    }

    /// captureBackdrop(): the area under the window, downscaled like Electron's thumbnail crop,
    /// sent to the page unless it's unchanged. `scale` 1/3 (1/4 with the performance switches on).
    pub fn capture(&mut self, hwnd: HWND, display: &crate::screen::Display, dip_w: f64, dip_h: f64, slow: bool, env: &ICoreWebView2Environment, webview: &ICoreWebView2) {
        let scale = if slow { 0.25 } else { 1.0 / 3.0 };
        // Electron: thumbnail of the whole display, k = its width / the display's DIP width
        let k = (display.bounds.width * scale).round() / display.bounds.width;
        let (cw, ch) = ((dip_w * k).round() as i32, (dip_h * k).round() as i32);
        if cw <= 0 || ch <= 0 {
            return;
        }
        let mut r = RECT::default();
        unsafe {
            if GetWindowRect(hwnd, &mut r).is_err() {
                return;
            }
        }
        let origin = display.to_phys(display.bounds.x, display.bounds.y);
        // out of captures for one screen frame: Windows leaves the island out of the next frame
        // it draws (DwmFlush waits for it), the area behind is copied, and it's back at once
        let grabbed = unsafe {
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            let _ = windows::Win32::Graphics::Dwm::DwmFlush();
            let g = grab_box(r, origin, k / display.scale, cw, ch);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_NONE);
            g
        };
        let Some(bgra) = grabbed else { return };
        // cheap fingerprint: sample the small bitmap instead of hashing all of it
        let mut key = bgra.len().to_string();
        for i in (0..bgra.len()).step_by(997) {
            key.push_str(&radix36(bgra[i]));
        }
        if key == self.last_key {
            return;
        }
        self.last_key = key;
        self.last = Some((bgra.clone(), cw, ch));
        self.post(env, webview, &bgra, cw, ch);
    }

    /// The frame to the page as shared memory (two buffers used in turn); the bridge hands the
    /// renderer {w, h, bgra} like Electron's.
    fn post(&mut self, env: &ICoreWebView2Environment, webview: &ICoreWebView2, bgra: &[u8], w: i32, h: i32) {
        let size = bgra.len() as u64;
        unsafe {
            if self.buffer_size != size || self.buffers.is_empty() {
                let Ok(env12) = env.cast::<ICoreWebView2Environment12>() else { return };
                self.buffers = [env12.CreateSharedBuffer(size), env12.CreateSharedBuffer(size)].into_iter().flatten().collect();
                self.buffer_size = size;
            }
            let Some(buf) = self.buffers.get(self.turn % self.buffers.len().max(1)).cloned() else { return };
            self.turn += 1;
            let mut p: *mut u8 = std::ptr::null_mut();
            if buf.Buffer(&mut p).is_err() || p.is_null() {
                return;
            }
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), p, bgra.len());
            let Ok(wv17) = webview.cast::<ICoreWebView2_17>() else { return };
            let meta = CoTaskMemPWSTR::from(format!("{{\"c\":\"backdrop\",\"w\":{w},\"h\":{h}}}").as_str());
            let _ = wv17.PostSharedBufferToScript(&buf, COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, *meta.as_ref().as_pcwstr());
        }
    }
}

fn radix36(b: u8) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if b < 36 {
        return (D[b as usize] as char).to_string();
    }
    let (hi, lo) = (b / 36, b % 36);
    format!("{}{}", D[hi as usize] as char, D[lo as usize] as char)
}

/// The screen under `r`, downscaled like Electron's thumbnail: each small pixel is the average of
/// its exact area on the whole display's grid (a 1/k box), then cropped. `origin` is the display's
/// physical top-left and `k` the thumbnail's scale, so the grid lines fall where Electron's do.
fn grab_box(r: RECT, origin: (i32, i32), k: f64, cw: i32, ch: i32) -> Option<Vec<u8>> {
    // the crop's first small pixel on the display's grid (Electron rounds the crop origin)
    let gx = ((r.left - origin.0) as f64 * k).round() as i32;
    let gy = ((r.top - origin.1) as f64 * k).round() as i32;
    let inv = 1.0 / k;
    // the source span those small pixels cover, physical, plus a pixel of margin
    let sx0 = origin.0 + (gx as f64 * inv).floor() as i32;
    let sy0 = origin.1 + (gy as f64 * inv).floor() as i32;
    let sw = ((cw as f64 + 1.0) * inv).ceil() as i32 + 2;
    let sh = ((ch as f64 + 1.0) * inv).ceil() as i32 + 2;
    let src = unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, sw, sh);
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, sw, sh, Some(screen), sx0, sy0, SRCCOPY).is_ok();
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: sw, biHeight: -sh, biPlanes: 1, biBitCount: 32, ..Default::default() };
        let mut px = vec![0u8; (sw * sh * 4) as usize];
        GetDIBits(mem, bmp, 0, sh as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !ok {
            return None;
        }
        px
    };
    // Chromium scales the thumbnail with libyuv's bilinear filter: each small pixel samples the
    // display at its centre, (i + 0.5) / k - 0.5 (at 1/3 that is exactly the middle pixel of each 3x3
    // block). Measured against Electron's own capture: identical.
    let px = |x: i32, y: i32, c: usize| -> f64 {
        let (x, y) = (x.clamp(0, sw - 1), y.clamp(0, sh - 1));
        src[((y * sw + x) * 4) as usize + c] as f64
    };
    let mut out = vec![0u8; (cw * ch * 4) as usize];
    for j in 0..ch {
        let fy = (gy + j) as f64 * inv + 0.5 * inv - 0.5 - (sy0 - origin.1) as f64;
        let (y0, ay) = (fy.floor() as i32, fy - fy.floor());
        for i in 0..cw {
            let fx = (gx + i) as f64 * inv + 0.5 * inv - 0.5 - (sx0 - origin.0) as f64;
            let (x0, ax) = (fx.floor() as i32, fx - fx.floor());
            let o = ((j * cw + i) * 4) as usize;
            for c in 0..3 {
                let top = px(x0, y0, c) * (1.0 - ax) + px(x0 + 1, y0, c) * ax;
                let bottom = px(x0, y0 + 1, c) * (1.0 - ax) + px(x0 + 1, y0 + 1, c) * ax;
                out[o + c] = (top * (1.0 - ay) + bottom * ay).round() as u8;
            }
            out[o + 3] = 255;
        }
    }
    Some(out)
}

/// The screen under `r` (the island window is left out by its display affinity), scaled to cw x ch.
#[allow(dead_code)]
fn grab(r: RECT, cw: i32, ch: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let small = CreateCompatibleBitmap(screen, cw, ch);
        let old = SelectObject(mem, small.into());
        SetStretchBltMode(mem, HALFTONE);
        let _ = SetBrushOrgEx(mem, 0, 0, None);
        let ok = StretchBlt(mem, 0, 0, cw, ch, Some(screen), r.left, r.top, r.right - r.left, r.bottom - r.top, SRCCOPY).as_bool();
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: cw, biHeight: -ch, biPlanes: 1, biBitCount: 32, ..Default::default() };
        let mut px = vec![0u8; (cw * ch * 4) as usize];
        GetDIBits(mem, small, 0, ch as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(small.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        // opaque, like Electron's bitmap
        for a in px.iter_mut().skip(3).step_by(4) {
            *a = 255;
        }
        ok.then_some(px)
    }
}

/// `--glass-test x,y out.bgra`: the 640x320 DIP area at (x, y) captured the way the glass does it
/// (no window shown), saved as "w h\n" + raw BGRA, to compare with Electron's desktopCapturer crop.
pub fn test_capture(x: f64, y: f64, out: &str) -> Result<(), String> {
    use crate::{core::{WIN_H, WIN_W}, screen};
    let display = screen::matching(screen::Rect { x, y, width: WIN_W, height: WIN_H });
    let k = (display.bounds.width / 3.0).round() / display.bounds.width;
    let (cw, ch) = ((WIN_W * k).round() as i32, (WIN_H * k).round() as i32);
    let (px, py) = display.to_phys(x, y);
    let r = RECT { left: px, top: py, right: px + (WIN_W * display.scale).round() as i32, bottom: py + (WIN_H * display.scale).round() as i32 };
    let origin = display.to_phys(display.bounds.x, display.bounds.y);
    let bgra = grab_box(r, origin, k / display.scale, cw, ch).ok_or("capture failed")?;
    let mut file = format!("{cw} {ch}\n").into_bytes();
    file.extend_from_slice(&bgra);
    std::fs::write(out, file).map_err(|e| e.to_string())
}
