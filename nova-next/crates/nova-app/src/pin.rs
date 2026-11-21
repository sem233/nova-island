//! Native Win32 floating 1:1 screenshot pin overlays (0% idle CPU, zero WebView2).
//! Captures triggered via Windows Snipping Tool (PrtScn / Win+Shift+S / ms-screenclip:)
//! appear immediately at their exact captured physical pixel size, pinned above other apps.
use std::{
    cell::RefCell,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use windows::{
    core::{w, HSTRING},
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND},
            Gdi::{
                BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC,
                DeleteObject, EndPaint, FrameRect, GetMonitorInfoW, InvalidateRect, MonitorFromPoint,
                SelectObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
                MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, SRCCOPY,
            },
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL,
                VK_ESCAPE, VK_LBUTTON, VK_SNAPSHOT,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::*,
        },
    },
};

const CLASS_NAME: windows::core::PCWSTR = w!("NovaIslandPinShot");
const MAX_OPEN_PINS: usize = 12;
const WM_MOUSELEAVE: u32 = 0x02A3;

const MENU_PIN: u32 = 1;
const MENU_COPY: u32 = 2;
const MENU_SAVE: u32 = 3;
const MENU_CLOSE: u32 = 4;
const MENU_CLOSE_ALL: u32 = 5;

/// Clipboard sequence number produced by our own "Copy image" action (never re-pin it).
static IGNORE_CLIP_SEQ: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static CLASS_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OPEN_PINS: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

/// Returns true once if `seq` came from a pin overlay's own "Copy image" action.
pub fn take_ignored_clip_seq(seq: u32) -> bool {
    let cur = IGNORE_CLIP_SEQ.load(Ordering::Relaxed);
    cur != 0 && cur == seq && IGNORE_CLIP_SEQ.compare_exchange(cur, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

/// Launches Windows' built-in region snipping overlay (`ms-screenclip:`).
pub fn launch_snip() {
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from("ms-screenclip:"), None, None, SW_SHOWNORMAL);
    }
}

/// Tracks PrtScn presses, Windows Snipping Tool activity, and region drag coordinates
/// so captured snips spawn in-place at 1:1 size while ordinary browser image copies are left alone.
pub struct SnipTracker {
    armed_until: Option<Instant>,
    fg_snip_at: Option<Instant>,
    dragging: bool,
    drag_start: POINT,
    last_drag: Option<(RECT, Instant)>,
}

impl SnipTracker {
    pub fn new() -> Self {
        // Consume any stale VK_SNAPSHOT press from before Nova Island started.
        unsafe {
            let _ = GetAsyncKeyState(VK_SNAPSHOT.0 as i32);
        }
        Self {
            armed_until: None,
            fg_snip_at: None,
            dragging: false,
            drag_start: POINT::default(),
            last_drag: None,
        }
    }

    pub fn arm(&mut self) {
        self.armed_until = Some(Instant::now() + Duration::from_secs(60));
        self.dragging = false;
    }

    pub fn on_fg_proc(&mut self, proc_name: &str) {
        if is_snip_process(proc_name) {
            let now = Instant::now();
            self.fg_snip_at = Some(now);
            let until = now + Duration::from_secs(20);
            if self.armed_until.is_none_or(|t| t < until) {
                self.armed_until = Some(until);
            }
        }
    }

    /// Lightweight poll run inside TIMER_CURSOR. Returns true only while a snip is actively underway.
    pub fn poll_input(&mut self) -> bool {
        let now = Instant::now();
        unsafe {
            if (GetAsyncKeyState(VK_SNAPSHOT.0 as i32) as u16 & 0x8001) != 0 {
                self.arm();
            }
        }
        let armed = self.armed_until.is_some_and(|t| now < t);
        if !armed {
            self.armed_until = None;
            self.dragging = false;
            return false;
        }
        unsafe {
            if (GetAsyncKeyState(VK_ESCAPE.0 as i32) as u16 & 0x8001) != 0 {
                self.armed_until = None;
                self.dragging = false;
                return false;
            }
            let down = (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0;
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if down && !self.dragging {
                self.dragging = true;
                self.drag_start = pt;
            } else if !down && self.dragging {
                self.dragging = false;
                let r = RECT {
                    left: self.drag_start.x.min(pt.x),
                    top: self.drag_start.y.min(pt.y),
                    right: self.drag_start.x.max(pt.x),
                    bottom: self.drag_start.y.max(pt.y),
                };
                if (r.right - r.left) >= 8 && (r.bottom - r.top) >= 8 {
                    self.last_drag = Some((r, now));
                }
            }
        }
        self.dragging || self.fg_snip_at.is_some_and(|t| now.duration_since(t) < Duration::from_secs(3))
    }

    /// Called when the clipboard sequence changes. Returns `(was_snip, optional_drag_rect)`.
    pub fn consume_on_clip(&mut self) -> (bool, Option<RECT>) {
        let now = Instant::now();
        let armed = self.armed_until.is_some_and(|t| now < t);
        let fg_recent = self.fg_snip_at.is_some_and(|t| now.duration_since(t) < Duration::from_secs(10));
        let drag = self.last_drag.filter(|(_, t)| now.duration_since(*t) < Duration::from_secs(8)).map(|(r, _)| r);
        if armed || fg_recent {
            self.armed_until = None;
            self.dragging = false;
            self.last_drag = None;
            (true, drag)
        } else {
            (false, None)
        }
    }
}

pub fn is_snip_process(proc_name: &str) -> bool {
    matches!(
        proc_name.to_ascii_lowercase().as_str(),
        "screenclippinghost" | "snippingtool" | "screensketch"
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HoverHit {
    None,
    Pin,
    Close,
}

/// Computes the exact 1:1 physical pixel rectangle (or proportionally fitted if larger than the
/// monitor's work area), placed right at `drag_hint` when its dimensions match the image, else near cursor.
pub fn place_pin_rect(img_w: u32, img_h: u32, work: RECT, drag_hint: Option<RECT>, cursor: POINT) -> (i32, i32, i32, i32) {
    let work_w = (work.right - work.left).max(64);
    let work_h = (work.bottom - work.top).max(64);
    let iw = img_w.max(1) as i32;
    let ih = img_h.max(1) as i32;

    let (win_w, win_h) = if iw <= work_w && ih <= work_h {
        (iw.max(16), ih.max(16))
    } else {
        let scale = ((work_w as f64 * 0.92) / iw as f64).min((work_h as f64 * 0.92) / ih as f64);
        (
            (iw as f64 * scale).round().clamp(64.0, work_w as f64) as i32,
            (ih as f64 * scale).round().clamp(64.0, work_h as f64) as i32,
        )
    };

    let (mut x, mut y) = match drag_hint {
        Some(d) => {
            let dw = (d.right - d.left).abs();
            let dh = (d.bottom - d.top).abs();
            if (dw - iw).abs() <= 40 && (dh - ih).abs() <= 40 {
                (d.left.min(d.right), d.top.min(d.bottom))
            } else {
                (cursor.x - win_w / 2, cursor.y - win_h / 2)
            }
        }
        None => (cursor.x - win_w / 2, cursor.y - win_h / 2),
    };

    let max_x = (work.right - win_w).max(work.left);
    let max_y = (work.bottom - win_h).max(work.top);
    x = x.clamp(work.left, max_x);
    y = y.clamp(work.top, max_y);
    (x, y, win_w, win_h)
}

/// Top-right hover pill layout: `(pill_rect, pin_btn_rect, close_btn_rect)`.
/// Returns None only for ultra-tiny snips (< 44×24 px) where double-click / Esc / right-click still work.
pub fn control_rects(win_w: i32, win_h: i32, dpi: u32) -> Option<(RECT, RECT, RECT)> {
    if win_w < 44 || win_h < 24 {
        return None;
    }
    let sc = (dpi as f32 / 96.0).clamp(1.0, 2.5);
    let max_btn = ((win_h - 10).min((win_w - 16) / 2)).max(16);
    let btn = ((24.0 * sc).round() as i32).min(max_btn);
    let gap = ((4.0 * sc).round() as i32).min(4);
    let inner = ((3.0 * sc).round() as i32).min(4);
    let margin = ((6.0 * sc).round() as i32).min((win_w - (btn * 2 + gap + inner * 2)) / 2).max(2);

    let pill_w = btn * 2 + gap + inner * 2;
    let pill_h = btn + inner * 2;
    let pill = RECT {
        left: (win_w - margin - pill_w).max(1),
        top: margin.max(1),
        right: (win_w - margin).min(win_w - 1),
        bottom: (margin + pill_h).min(win_h - 1),
    };
    let close = RECT {
        left: pill.right - inner - btn,
        top: pill.top + inner,
        right: pill.right - inner,
        bottom: pill.top + inner + btn,
    };
    let pin = RECT {
        left: close.left - gap - btn,
        top: close.top,
        right: close.left - gap,
        bottom: close.bottom,
    };
    Some((pill, pin, close))
}

pub fn hit_test_controls(win_w: i32, win_h: i32, dpi: u32, x: i32, y: i32) -> HoverHit {
    let Some((_, pin, close)) = control_rects(win_w, win_h, dpi) else {
        return HoverHit::None;
    };
    if x >= close.left && x < close.right && y >= close.top && y < close.bottom {
        HoverHit::Close
    } else if x >= pin.left && x < pin.right && y >= pin.top && y < pin.bottom {
        HoverHit::Pin
    } else {
        HoverHit::None
    }
}

/// Bilinear resize of top-down 32-bit BGRA pixels (used only when an image exceeds the monitor work area).
fn resize_bgra(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dw as usize) * (dh as usize) * 4];
    let sx_ratio = sw as f32 / dw as f32;
    let sy_ratio = sh as f32 / dh as f32;
    for y in 0..dh {
        let fy = (y as f32 + 0.5) * sy_ratio - 0.5;
        let y0 = fy.floor().clamp(0.0, (sh - 1) as f32) as usize;
        let y1 = (y0 + 1).min((sh - 1) as usize);
        let wy = (fy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..dw {
            let fx = (x as f32 + 0.5) * sx_ratio - 0.5;
            let x0 = fx.floor().clamp(0.0, (sw - 1) as f32) as usize;
            let x1 = (x0 + 1).min((sw - 1) as usize);
            let wx = (fx - x0 as f32).clamp(0.0, 1.0);

            let i00 = (y0 * sw as usize + x0) * 4;
            let i10 = (y0 * sw as usize + x1) * 4;
            let i01 = (y1 * sw as usize + x0) * 4;
            let i11 = (y1 * sw as usize + x1) * 4;
            let o = (y as usize * dw as usize + x as usize) * 4;
            for c in 0..4 {
                let top = src[i00 + c] as f32 * (1.0 - wx) + src[i10 + c] as f32 * wx;
                let bot = src[i01 + c] as f32 * (1.0 - wx) + src[i11 + c] as f32 * wx;
                dst[o + c] = (top * (1.0 - wy) + bot * wy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    dst
}

/// Creates a top-down 32-bit BGRA GDI HBITMAP from `bgra`.
unsafe fn make_dib_bitmap(w: i32, h: i32, bgra: &[u8]) -> Option<HBITMAP> {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    if bits.is_null() {
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        return None;
    }
    let len = (w as usize) * (h as usize) * 4;
    if bgra.len() >= len {
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, len);
    }
    Some(bmp)
}

struct PinState {
    orig_w: u32,
    orig_h: u32,
    orig_px: Vec<u8>,
    win_w: i32,
    win_h: i32,
    /// Display pixels matching `(win_w, win_h)` when downscaled; None when 1:1 (reuses `orig_px`).
    scaled_px: Option<Vec<u8>>,
    bmp: HBITMAP,
    pinned: bool,
    hovered: bool,
    hit: HoverHit,
    tracking_leave: bool,
    arabic: bool,
}

impl PinState {
    fn display_px(&self) -> &[u8] {
        self.scaled_px.as_deref().unwrap_or(&self.orig_px)
    }
}

impl Drop for PinState {
    fn drop(&mut self) {
        if !self.bmp.0.is_null() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            }
            self.bmp = HBITMAP::default();
        }
    }
}

fn ensure_class() {
    if CLASS_READY.with(|c| c.get()) {
        return;
    }
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW | CS_DBLCLKS,
            lpfnWndProc: Some(pin_wndproc),
            hInstance: HINSTANCE(inst.0),
            hCursor: LoadCursorW(None, IDC_SIZEALL).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
    CLASS_READY.with(|c| c.set(true));
}

/// Spawns a native floating pinned screenshot at exact 1:1 physical pixel size.
/// Must be called on the UI thread.
pub fn spawn(w: u32, h: u32, px: Vec<u8>, drag_hint: Option<RECT>, arabic: bool) -> Option<HWND> {
    if w == 0 || h == 0 || px.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    ensure_class();

    // Keep open pin count bounded so spamming screenshots never bloats RAM.
    OPEN_PINS.with(|list| {
        while list.borrow().len() >= MAX_OPEN_PINS {
            let oldest = list.borrow_mut().remove(0);
            unsafe {
                let h = HWND(oldest as *mut _);
                if IsWindow(Some(h)).as_bool() {
                    let _ = DestroyWindow(h);
                }
            }
        }
    });

    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }
    let anchor = drag_hint.map_or(cursor, |d| POINT { x: (d.left + d.right) / 2, y: (d.top + d.bottom) / 2 });
    let work = unsafe {
        let mon = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            mi.rcWork
        } else {
            RECT { left: 0, top: 0, right: 1920, bottom: 1040 }
        }
    };

    let (x, y, win_w, win_h) = place_pin_rect(w, h, work, drag_hint, cursor);
    let scaled_px = if win_w as u32 != w || win_h as u32 != h {
        Some(resize_bgra(&px, w, h, win_w as u32, win_h as u32))
    } else {
        None
    };
    let disp_ref = scaled_px.as_deref().unwrap_or(&px);
    let bmp = unsafe { make_dib_bitmap(win_w, win_h, disp_ref)? };

    let state = Box::new(PinState {
        orig_w: w,
        orig_h: h,
        orig_px: px,
        win_w,
        win_h,
        scaled_px,
        bmp,
        pinned: true,
        hovered: false,
        hit: HoverHit::None,
        tracking_leave: false,
        arabic,
    });
    let raw_state = Box::into_raw(state);

    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("Nova Pin"),
            WS_POPUP,
            x,
            y,
            win_w,
            win_h,
            None,
            None,
            Some(HINSTANCE(inst.0)),
            Some(raw_state as *const _),
        )
    };

    let Ok(hwnd) = hwnd else {
        unsafe { drop(Box::from_raw(raw_state)) };
        return None;
    };

    unsafe {
        // Keep exact sharp corners so 1:1 edges aren't clipped away by Windows 11 rounding.
        let pref = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, win_w, win_h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }

    OPEN_PINS.with(|list| list.borrow_mut().push(hwnd.0 as isize));
    Some(hwnd)
}

/// Destroys all currently open floating pin overlays and frees their GDI bitmaps.
pub fn close_all() {
    let handles: Vec<isize> = OPEN_PINS.with(|list| std::mem::take(&mut *list.borrow_mut()));
    for raw in handles {
        unsafe {
            let h = HWND(raw as *mut _);
            if IsWindow(Some(h)).as_bool() {
                let _ = DestroyWindow(h);
            }
        }
    }
}

pub fn open_count() -> usize {
    OPEN_PINS.with(|list| list.borrow().len())
}

// ---- Pure-Rust anti-aliased hover pill rendering (tiny ~68×36 patch only when hovered) ----

fn blend_px(dst: &mut [u8], r: u8, g: u8, b: u8, a: f32) {
    let a = a.clamp(0.0, 1.0);
    if a <= 0.001 {
        return;
    }
    let inv = 1.0 - a;
    dst[0] = (b as f32 * a + dst[0] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    dst[1] = (g as f32 * a + dst[1] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    dst[2] = (r as f32 * a + dst[2] as f32 * inv).round().clamp(0.0, 255.0) as u8;
    dst[3] = 255;
}

fn rounded_rect_cov(px: f32, py: f32, x0: f32, y0: f32, x1: f32, y1: f32, rad: f32) -> f32 {
    let cx = (x0 + x1) * 0.5;
    let cy = (y0 + y1) * 0.5;
    let hx = ((x1 - x0) * 0.5 - rad).max(0.0);
    let hy = ((y1 - y0) * 0.5 - rad).max(0.0);
    let dx = (px - cx).abs() - hx;
    let dy = (py - cy).abs() - hy;
    let dist = (dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0)) - rad;
    (0.5 - dist).clamp(0.0, 1.0)
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let len2 = vx * vx + vy * vy;
    if len2 <= 1e-6 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * vx + (py - ay) * vy) / len2).clamp(0.0, 1.0);
    (px - (ax + t * vx)).hypot(py - (ay + t * vy))
}

fn paint_hover_pill(hdc: windows::Win32::Graphics::Gdi::HDC, st: &PinState, dpi: u32) {
    let Some((pill, pin_r, close_r)) = control_rects(st.win_w, st.win_h, dpi) else {
        return;
    };
    let pw = (pill.right - pill.left).max(1);
    let ph = (pill.bottom - pill.top).max(1);
    let mut buf = vec![0u8; (pw as usize) * (ph as usize) * 4];
    let src = st.display_px();
    let stride = (st.win_w as usize) * 4;
    for y in 0..ph as usize {
        let sy = (pill.top as usize + y).min((st.win_h - 1) as usize);
        let sx = pill.left as usize;
        let s_off = sy * stride + sx * 4;
        let d_off = y * (pw as usize) * 4;
        let copy_len = (pw as usize * 4).min(src.len().saturating_sub(s_off));
        buf[d_off..d_off + copy_len].copy_from_slice(&src[s_off..s_off + copy_len]);
    }

    let rad_pill = (ph as f32 * 0.5).min(12.0);
    let pin_x0 = (pin_r.left - pill.left) as f32;
    let pin_y0 = (pin_r.top - pill.top) as f32;
    let pin_x1 = (pin_r.right - pill.left) as f32;
    let pin_y1 = (pin_r.bottom - pill.top) as f32;
    let pin_rad = ((pin_x1 - pin_x0) * 0.35).min(8.0);

    let cls_x0 = (close_r.left - pill.left) as f32;
    let cls_y0 = (close_r.top - pill.top) as f32;
    let cls_x1 = (close_r.right - pill.left) as f32;
    let cls_y1 = (close_r.bottom - pill.top) as f32;
    let cls_rad = ((cls_x1 - cls_x0) * 0.35).min(8.0);

    let pin_cx = (pin_x0 + pin_x1) * 0.5;
    let pin_cy = (pin_y0 + pin_y1) * 0.5;
    let pin_s = (pin_x1 - pin_x0) * 0.27;

    let cls_cx = (cls_x0 + cls_x1) * 0.5;
    let cls_cy = (cls_y0 + cls_y1) * 0.5;
    let cls_s = (cls_x1 - cls_x0) * 0.21;
    let stroke = ((pin_x1 - pin_x0) * 0.075).clamp(1.2, 2.2);

    for y in 0..ph as usize {
        let py = y as f32 + 0.5;
        for x in 0..pw as usize {
            let px = x as f32 + 0.5;
            let o = (y * pw as usize + x) * 4;
            let p_cov = rounded_rect_cov(px, py, 0.0, 0.0, pw as f32, ph as f32, rad_pill);
            if p_cov <= 0.0 {
                continue;
            }
            // Translucent dark pill background
            blend_px(&mut buf[o..o + 4], 15, 18, 24, 0.80 * p_cov);

            // Pin button fill
            let b_pin = rounded_rect_cov(px, py, pin_x0, pin_y0, pin_x1, pin_y1, pin_rad);
            if b_pin > 0.0 {
                if st.pinned {
                    let a = if st.hit == HoverHit::Pin { 0.96 } else { 0.82 };
                    blend_px(&mut buf[o..o + 4], 59, 130, 246, a * b_pin);
                } else if st.hit == HoverHit::Pin {
                    blend_px(&mut buf[o..o + 4], 255, 255, 255, 0.20 * b_pin);
                }
            }

            // Close button fill
            let b_cls = rounded_rect_cov(px, py, cls_x0, cls_y0, cls_x1, cls_y1, cls_rad);
            if b_cls > 0.0 && st.hit == HoverHit::Close {
                blend_px(&mut buf[o..o + 4], 239, 68, 68, 0.92 * b_cls);
            }

            // Pushpin icon (tilted clean minimal geometry: head bar + body + needle)
            let d_pin = seg_dist(px, py, pin_cx - pin_s * 0.75, pin_cy - pin_s * 0.75, pin_cx + pin_s * 0.75, pin_cy - pin_s * 0.75)
                .min(seg_dist(px, py, pin_cx, pin_cy - pin_s * 0.75, pin_cx, pin_cy + pin_s * 0.25))
                .min(seg_dist(px, py, pin_cx - pin_s * 0.95, pin_cy + pin_s * 0.25, pin_cx + pin_s * 0.95, pin_cy + pin_s * 0.25))
                .min(seg_dist(px, py, pin_cx, pin_cy + pin_s * 0.25, pin_cx, pin_cy + pin_s * 1.1));
            let pin_ico = (stroke * 0.55 + 0.5 - d_pin).clamp(0.0, 1.0);
            if pin_ico > 0.0 {
                let alpha = if st.pinned || st.hit == HoverHit::Pin { 0.98 } else { 0.75 };
                blend_px(&mut buf[o..o + 4], 255, 255, 255, alpha * pin_ico);
            }

            // Close X icon
            let d_x = seg_dist(px, py, cls_cx - cls_s, cls_cy - cls_s, cls_cx + cls_s, cls_cy + cls_s)
                .min(seg_dist(px, py, cls_cx + cls_s, cls_cy - cls_s, cls_cx - cls_s, cls_cy + cls_s));
            let x_ico = (stroke * 0.55 + 0.5 - d_x).clamp(0.0, 1.0);
            if x_ico > 0.0 {
                let alpha = if st.hit == HoverHit::Close { 1.0 } else { 0.85 };
                blend_px(&mut buf[o..o + 4], 255, 255, 255, alpha * x_ico);
            }
        }
    }

    unsafe {
        if let Some(patch_bmp) = make_dib_bitmap(pw, ph, &buf) {
            let mem = CreateCompatibleDC(Some(hdc));
            let prev = SelectObject(mem, HGDIOBJ(patch_bmp.0));
            let _ = BitBlt(hdc, pill.left, pill.top, pw, ph, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, prev);
            let _ = DeleteDC(mem);
            let _ = DeleteObject(HGDIOBJ(patch_bmp.0));
        }
    }
}

unsafe fn get_state(hwnd: HWND) -> Option<&'static mut PinState> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

fn toggle_pin(hwnd: HWND, st: &mut PinState) {
    st.pinned = !st.pinned;
    let insert = if st.pinned { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe {
        let _ = SetWindowPos(hwnd, Some(insert), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn copy_pin_image(st: &PinState) {
    if nova_sys::clip::write_image(st.orig_w, st.orig_h, &st.orig_px) {
        let seq = nova_sys::win::clipboard_seq();
        IGNORE_CLIP_SEQ.store(seq, Ordering::Relaxed);
    }
}

fn save_pin_image(hwnd: HWND, st: &PinState) {
    let title = if st.arabic { "حفظ اللقطة" } else { "Save screenshot" };
    let filter_name = if st.arabic { "صورة PNG" } else { "PNG image" };
    let default_name = "screenshot.png";
    let Some(path) = crate::files::save_file(Some(hwnd), title, default_name, (filter_name, "*.png"), "png") else {
        return;
    };
    if let Some(png) = nova_sys::icons::bgra_png(st.orig_w, st.orig_h, &st.orig_px, None) {
        let _ = std::fs::write(path, png);
    }
}

fn show_context_menu(hwnd: HWND, st: &mut PinState) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let check = if st.pinned { MF_CHECKED } else { MF_UNCHECKED };
        let (lbl_pin, lbl_copy, lbl_save, lbl_close, lbl_all) = if st.arabic {
            ("تثبيت فوق التطبيقات\tP", "نسخ الصورة\tCtrl+C", "حفظ كصورة PNG…\tCtrl+S", "إغلاق\tEsc", "إغلاق الكل")
        } else {
            ("Always on top\tP", "Copy image\tCtrl+C", "Save as PNG…\tCtrl+S", "Close\tEsc", "Close all")
        };
        let _ = AppendMenuW(menu, MF_STRING | check, MENU_PIN as usize, &HSTRING::from(lbl_pin));
        let _ = AppendMenuW(menu, MF_STRING, MENU_COPY as usize, &HSTRING::from(lbl_copy));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SAVE as usize, &HSTRING::from(lbl_save));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_CLOSE as usize, &HSTRING::from(lbl_close));
        if open_count() > 1 {
            let _ = AppendMenuW(menu, MF_STRING, MENU_CLOSE_ALL as usize, &HSTRING::from(lbl_all));
        }
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match id {
            MENU_PIN => toggle_pin(hwnd, st),
            MENU_COPY => copy_pin_image(st),
            MENU_SAVE => save_pin_image(hwnd, st),
            MENU_CLOSE => {
                let _ = DestroyWindow(hwnd);
            }
            MENU_CLOSE_ALL => close_all(),
            _ => {}
        }
    }
}

extern "system" fn pin_wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(l.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                return LRESULT(1);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                if let Some(st) = get_state(hwnd) {
                    let mem = CreateCompatibleDC(Some(hdc));
                    let prev = SelectObject(mem, HGDIOBJ(st.bmp.0));
                    let _ = BitBlt(hdc, 0, 0, st.win_w, st.win_h, Some(mem), 0, 0, SRCCOPY);
                    SelectObject(mem, prev);
                    let _ = DeleteDC(mem);

                    if st.hovered {
                        let dpi = GetDpiForWindow(hwnd).max(96);
                        paint_hover_pill(hdc, st, dpi);
                    }

                    // Crisp 1px inset hairline border (COLORREF is 0x00BBGGRR)
                    let col = if st.pinned { COLORREF(0x00F6823B) } else { COLORREF(0x008A8A92) };
                    let brush = CreateSolidBrush(col);
                    let rect = RECT { left: 0, top: 0, right: st.win_w, bottom: st.win_h };
                    let _ = FrameRect(hdc, &rect, brush);
                    let _ = DeleteObject(HGDIOBJ(brush.0));
                }
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                if let Some(st) = get_state(hwnd) {
                    if !st.tracking_leave {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut tme);
                        st.tracking_leave = true;
                    }
                    let x = (l.0 & 0xFFFF) as i16 as i32;
                    let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    let hit = hit_test_controls(st.win_w, st.win_h, dpi, x, y);
                    if !st.hovered || st.hit != hit {
                        st.hovered = true;
                        st.hit = hit;
                        let cur = if hit == HoverHit::None { IDC_SIZEALL } else { IDC_HAND };
                        if let Ok(hc) = LoadCursorW(None, cur) {
                            SetCursor(Some(hc));
                        }
                        if let Some((pill, _, _)) = control_rects(st.win_w, st.win_h, dpi) {
                            let _ = InvalidateRect(Some(hwnd), Some(&pill), false);
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_SETCURSOR => {
                if let Some(st) = get_state(hwnd) {
                    let cur = if st.hit == HoverHit::None { IDC_SIZEALL } else { IDC_HAND };
                    if let Ok(hc) = LoadCursorW(None, cur) {
                        SetCursor(Some(hc));
                        return LRESULT(1);
                    }
                }
            }
            WM_MOUSELEAVE => {
                if let Some(st) = get_state(hwnd) {
                    st.tracking_leave = false;
                    if st.hovered {
                        st.hovered = false;
                        st.hit = HoverHit::None;
                        let dpi = GetDpiForWindow(hwnd).max(96);
                        if let Some((pill, _, _)) = control_rects(st.win_w, st.win_h, dpi) {
                            let _ = InvalidateRect(Some(hwnd), Some(&pill), false);
                        } else {
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
                return LRESULT(0);
            }
            WM_LBUTTONDOWN => {
                if let Some(st) = get_state(hwnd) {
                    let x = (l.0 & 0xFFFF) as i16 as i32;
                    let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    match hit_test_controls(st.win_w, st.win_h, dpi, x, y) {
                        HoverHit::Close => {
                            let _ = DestroyWindow(hwnd);
                            return LRESULT(0);
                        }
                        HoverHit::Pin => {
                            toggle_pin(hwnd, st);
                            return LRESULT(0);
                        }
                        HoverHit::None => {
                            let _ = SetForegroundWindow(hwnd);
                            let _ = windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                            let _ = SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
                            return LRESULT(0);
                        }
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                if let Some(st) = get_state(hwnd) {
                    let x = (l.0 & 0xFFFF) as i16 as i32;
                    let y = ((l.0 >> 16) & 0xFFFF) as i16 as i32;
                    let dpi = GetDpiForWindow(hwnd).max(96);
                    if hit_test_controls(st.win_w, st.win_h, dpi, x, y) != HoverHit::Pin {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                return LRESULT(0);
            }
            WM_NCLBUTTONDBLCLK => {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            WM_MBUTTONUP => {
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            WM_RBUTTONUP => {
                if let Some(st) = get_state(hwnd) {
                    show_context_menu(hwnd, st);
                }
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                if let Some(st) = get_state(hwnd) {
                    let vk = w.0 as u16;
                    let ctrl = (GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
                    if vk == VK_ESCAPE.0 {
                        let _ = DestroyWindow(hwnd);
                        return LRESULT(0);
                    } else if vk == 'P' as u16 {
                        toggle_pin(hwnd, st);
                        return LRESULT(0);
                    } else if ctrl && vk == 'C' as u16 {
                        copy_pin_image(st);
                        return LRESULT(0);
                    } else if ctrl && vk == 'S' as u16 {
                        save_pin_image(hwnd, st);
                        return LRESULT(0);
                    }
                }
            }
            WM_NCDESTROY => {
                OPEN_PINS.with(|list| list.borrow_mut().retain(|&h| h != hwnd.0 as isize));
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut PinState;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                return LRESULT(0);
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_1_to_1_placement_at_drag_rect() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let drag = RECT { left: 250, top: 180, right: 650, bottom: 480 };
        let (x, y, w, h) = place_pin_rect(400, 300, work, Some(drag), POINT { x: 900, y: 500 });
        assert_eq!((x, y, w, h), (250, 180, 400, 300));
    }

    #[test]
    fn mismatched_drag_falls_back_to_cursor_and_clamps_to_work_area() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let stale_drag = RECT { left: 10, top: 10, right: 110, bottom: 110 };
        // Cursor near bottom-right corner -> clamped inside work area, size remains exact 1:1
        let (x, y, w, h) = place_pin_rect(500, 300, work, Some(stale_drag), POINT { x: 1900, y: 1020 });
        assert_eq!((w, h), (500, 300));
        assert_eq!((x, y), (1420, 740));
    }

    #[test]
    fn oversized_fullscreen_capture_scales_proportionally_inside_work_area() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        let (x, y, w, h) = place_pin_rect(2560, 1440, work, None, POINT { x: 960, y: 520 });
        assert!(w <= 1920 && h <= 1040);
        assert!(x >= 0 && y >= 0 && x + w <= 1920 && y + h <= 1040);
        let ratio = w as f64 / h as f64;
        assert!((ratio - (2560.0 / 1440.0)).abs() < 0.03);
    }

    #[test]
    fn hover_controls_hit_test_and_tiny_guard() {
        assert!(control_rects(30, 20, 96).is_none());
        let (_, pin, close) = control_rects(320, 200, 96).expect("pill fits");
        assert_eq!(hit_test_controls(320, 200, 96, (close.left + close.right) / 2, (close.top + close.bottom) / 2), HoverHit::Close);
        assert_eq!(hit_test_controls(320, 200, 96, (pin.left + pin.right) / 2, (pin.top + pin.bottom) / 2), HoverHit::Pin);
        assert_eq!(hit_test_controls(320, 200, 96, 10, 10), HoverHit::None);
    }

    #[test]
    fn snip_process_detection_is_case_insensitive() {
        assert!(is_snip_process("ScreenClippingHost"));
        assert!(is_snip_process("snippingtool"));
        assert!(is_snip_process("ScreenSketch"));
        assert!(!is_snip_process("chrome"));
    }
}
