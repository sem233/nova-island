// The island window: 640x320 DIP, transparent, topmost, no taskbar button, never activated by a
// click (Electron: frame:false, transparent, alwaysOnTop 'screen-saver', skipTaskbar, focusable:false).
// Click-through by default, like setIgnoreMouseEvents(true) at start (nova.js).
use std::cell::Cell;

use windows::{
    core::w,
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
};

use crate::{core::{WIN_H, WIN_W}, screen};

pub const CLASS: windows::core::PCWSTR = w!("NovaIsland");

/// Creates the (hidden) window; `wndproc` handles its messages.
pub fn create(wndproc: WNDPROC) -> windows::core::Result<HWND> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        let cls = WNDCLASSW { lpfnWndProc: wndproc, hInstance: HINSTANCE(inst.0), lpszClassName: CLASS, hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() };
        RegisterClassW(&cls);
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_NOREDIRECTIONBITMAP | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            CLASS,
            w!("Nova Island"),
            WS_POPUP,
            0,
            0,
            WIN_W as i32,
            WIN_H as i32,
            None,
            None,
            Some(HINSTANCE(inst.0)),
            None,
        )
    }
}

/// The window as nova.js sees it: a position in DIPs, a 640x320 DIP resting size.
pub struct Window {
    pub hwnd: HWND,
    pos: Cell<(f64, f64)>,
    size: Cell<(f64, f64)>,
    ignore_mouse: Cell<bool>,
    #[cfg(test)]
    test_visible: Cell<bool>,
}

impl Window {
    pub fn new(hwnd: HWND) -> Window {
        Window {
            hwnd,
            pos: Cell::new((f64::NAN, f64::NAN)),
            size: Cell::new((WIN_W, WIN_H)),
            ignore_mouse: Cell::new(true),
            #[cfg(test)]
            test_visible: Cell::new(true),
        }
    }

    pub fn pos(&self) -> (f64, f64) {
        self.pos.get()
    }

    pub fn size(&self) -> (f64, f64) {
        self.size.get()
    }

    /// Moves the window (DIPs); its pixel size follows the scale of the display it lands on.
    pub fn set_pos(&self, x: f64, y: f64) {
        self.set_rect(x, y, WIN_W, WIN_H);
    }

    /// Moves and sizes the window in DIPs using the display's DPI scale.
    pub fn set_rect(&self, x: f64, y: f64, w_dip: f64, h_dip: f64) {
        if self.pos.get() == (x, y) && self.size.get() == (w_dip, h_dip) {
            return;
        }
        self.pos.set((x, y));
        self.size.set((w_dip, h_dip));
        if self.hwnd.0.is_null() {
            return;
        }
        let d = screen::nearest(x + w_dip / 2.0, y + h_dip / 2.0);
        let (px, py) = d.to_phys(x, y);
        let (w, h) = ((w_dip * d.scale).round() as i32, (h_dip * d.scale).round() as i32);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), px, py, w, h, SWP_NOACTIVATE);
        }
    }

    pub fn visible(&self) -> bool {
        #[cfg(test)]
        if self.hwnd.0.is_null() {
            return self.test_visible.get();
        }
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn show_inactive(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// Called every system tick: when another app comes to the front, the island goes back on top.
    /// A full-screen viewer (Photos) can open above the island or take its topmost place, and
    /// the island stayed under every window until a restart. Our own windows (menus, Settings,
    /// the mini player) are left alone.
    pub fn keep_on_top(&self) {
        thread_local!(static LAST: Cell<isize> = const { Cell::new(0) });
        unsafe {
            let fg = GetForegroundWindow();
            if LAST.with(|l| l.replace(fg.0 as isize)) == fg.0 as isize || !self.visible() {
                return;
            }
            let (mut ours, mut theirs) = (0u32, 0u32);
            GetWindowThreadProcessId(self.hwnd, Some(&mut ours));
            GetWindowThreadProcessId(fg, Some(&mut theirs));
            if ours == theirs {
                return;
            }
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    fn ex_style(&self, add: WINDOW_EX_STYLE, remove: WINDOW_EX_STYLE) {
        if self.hwnd.0.is_null() {
            return;
        }
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) as u32;
            let next = (ex | add.0) & !remove.0;
            if next != ex {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next as isize);
            }
        }
    }

    /// setIgnoreMouseEvents: clicks fall through to the window below
    pub fn set_ignore_mouse(&self, on: bool) {
        self.ignore_mouse.set(on);
        if on {
            self.ex_style(WS_EX_TRANSPARENT | WS_EX_LAYERED, WINDOW_EX_STYLE(0));
        } else {
            self.ex_style(WINDOW_EX_STYLE(0), WS_EX_TRANSPARENT | WS_EX_LAYERED);
        }
    }

    pub fn ignore_mouse(&self) -> bool {
        self.ignore_mouse.get()
    }

    /// setFocusable
    pub fn set_focusable(&self, on: bool) {
        if on {
            self.ex_style(WINDOW_EX_STYLE(0), WS_EX_NOACTIVATE);
        } else {
            self.ex_style(WS_EX_NOACTIVATE, WINDOW_EX_STYLE(0));
        }
    }

    pub fn focus(&self) {
        if self.hwnd.0.is_null() {
            return;
        }
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
        }
    }
}
