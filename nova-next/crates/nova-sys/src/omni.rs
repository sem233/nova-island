//! The address bar ("omnibox") of a Chromium browser in front: where it is on screen, whether the
//! user is typing in it, and how long its text is (AddressBar.cs). The island sits in its right
//! third. The UI Automation search is slow-ish, so it is done once per browser window (retried
//! every 2 s if not found); after that only the element's properties are read each tick.
use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::HWND,
        System::{Com::{CoCreateInstance, CLSCTX_INPROC_SERVER}, Variant::VARIANT},
        UI::Accessibility::*,
    },
};

/// (1.8.7) Whether anything needs the address bar now: only "Address bar" in the browser, from
/// the island's default place (core.rs omni_allowed). Off, it isn't read at all.
pub static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub struct Omni {
    uia: Option<IUIAutomation>,
    hwnd: HWND,
    edit: Option<IUIAutomationElement>,
    next_search: Option<Instant>,
}

impl Omni {
    pub fn new() -> Omni {
        let uia = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        Omni { uia, hwnd: HWND::default(), edit: None, next_search: None }
    }

    /// (1.8.7) Nothing needs the address bar (the island isn't set to sit in it): what was found
    /// is let go, and the next ticks ask nothing of UI Automation.
    pub fn forget(&mut self) {
        self.hwnd = HWND::default();
        self.edit = None;
        self.next_search = None;
    }

    /// JSON for the status line, or "null" when no browser address bar is in front.
    pub fn json(&mut self, fg: HWND, proc: &str, fullscreen: bool) -> String {
        const BROWSERS: &[&str] = &["chrome", "msedge", "edge", "brave", "opera", "vivaldi", "arc"];
        let proc_l = proc.to_lowercase();
        if fullscreen || fg.is_invalid() || !BROWSERS.iter().any(|b| proc_l.contains(b)) || crate::win::class_of(fg) != "Chrome_WidgetWin_1" {
            self.hwnd = HWND::default();
            self.edit = None;
            return "null".into();
        }
        if fg != self.hwnd {
            self.hwnd = fg;
            self.edit = None;
            self.next_search = None;
        }
        if self.edit.is_none() {
            if self.next_search.is_some_and(|t| Instant::now() < t) {
                return "null".into();
            }
            self.next_search = Some(Instant::now() + Duration::from_secs(2));
            self.edit = self.search(fg);
            if self.edit.is_none() {
                return "null".into();
            }
        }
        match self.read() {
            Some(j) => j,
            None => {
                // the window changed under us (navigated, closed): search again
                self.edit = None;
                "null".into()
            }
        }
    }

    fn search(&self, fg: HWND) -> Option<IUIAutomationElement> {
        let uia = self.uia.as_ref()?;
        unsafe {
            let cond = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0)).ok()?;
            uia.ElementFromHandle(fg).ok()?.FindFirst(TreeScope_Descendants, &cond).ok()
        }
    }

    fn read(&mut self) -> Option<String> {
        let e = self.edit.as_ref()?;
        unsafe {
            let r = e.CurrentBoundingRectangle().ok()?;
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w <= 0 || h <= 0 {
                return Some("null".into());
            }
            if w < 200 || h < 16 {
                self.edit = None;
                return Some("null".into());
            }
            let focus = e.CurrentHasKeyboardFocus().ok()?.as_bool();
            let len = e
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentValue())
                .map(|v| v.len())
                .unwrap_or(0);
            Some(format!("{{\"x\":{},\"y\":{},\"w\":{w},\"h\":{h},\"focus\":{focus},\"len\":{len}}}", r.left, r.top))
        }
    }
}
