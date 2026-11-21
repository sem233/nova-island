//! Where the tabs of the browser in front are on screen (new in 1.8): "Stay in place" lets clicks
//! through and dims the island only while a tab is under it. The tab strip is found once per
//! browser window, in the browser's own frame only (never inside a web page); then each read asks
//! for the tabs' rectangles in one cached call.
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use windows::Win32::{
    Foundation::HWND,
    System::{Com::{CoCreateInstance, CLSCTX_INPROC_SERVER}, Variant::VARIANT},
    UI::Accessibility::*,
};

/// (1.8.7) Whether anything needs the tabs now: only "Stay in place" does (the thin line goes by
/// the browser window's place, the address bar by the omnibox). Off, nothing is asked of UI
/// Automation at all.
pub static WANTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub struct Tabs {
    uia: Option<IUIAutomation>,
    hwnd: HWND,
    strip: Option<IUIAutomationElement>,
    next_search: Option<Instant>,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs::new()
    }
}

impl Tabs {
    pub fn new() -> Tabs {
        let uia: Option<IUIAutomation> = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() };
        // a busy browser answers within a second or not at all
        if let Some(u2) = uia.as_ref().and_then(|u| windows::core::Interface::cast::<IUIAutomation2>(u).ok()) {
            unsafe {
                let _ = u2.SetConnectionTimeout(1000);
                let _ = u2.SetTransactionTimeout(1000);
            }
        }
        Tabs { uia, hwnd: HWND::default(), strip: None, next_search: None }
    }

    /// Reads the tabs once a second on a thread of its own, so a slow or hung browser never holds
    /// up the status line; the status line takes the latest answer.
    pub fn spawn() -> Arc<Mutex<String>> {
        let latest = Arc::new(Mutex::new(String::from("null")));
        let out = latest.clone();
        std::thread::Builder::new()
            .name("nova-tabs".into())
            .spawn(move || {
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                let mut tabs = Tabs::new();
                loop {
                    if !WANTED.load(std::sync::atomic::Ordering::Relaxed) {
                        tabs.hwnd = HWND::default();
                        tabs.strip = None;
                        *out.lock().unwrap() = "null".into();
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                    let fg = crate::win::foreground_snapshot();
                    let j = tabs.json(fg.hwnd, &fg.proc_name, fg.fullscreen);
                    *out.lock().unwrap() = j;
                    std::thread::sleep(Duration::from_secs(1));
                }
            })
            .expect("nova-tabs thread");
        latest
    }

    /// JSON for the status line: [[x, y, w, h], ...] in screen pixels, or "null" when no browser
    /// with tabs is in front.
    pub fn json(&mut self, fg: HWND, proc: &str, fullscreen: bool) -> String {
        const BROWSERS: &[&str] = &["chrome", "msedge", "brave", "opera", "vivaldi", "firefox"];
        if fullscreen || fg.is_invalid() || !BROWSERS.contains(&proc.to_lowercase().as_str()) {
            self.hwnd = HWND::default();
            self.strip = None;
            return "null".into();
        }
        if fg != self.hwnd {
            self.hwnd = fg;
            self.strip = None;
            self.next_search = None;
        }
        if self.strip.is_none() {
            if self.next_search.is_some_and(|t| Instant::now() < t) {
                return "null".into();
            }
            self.next_search = Some(Instant::now() + Duration::from_secs(2));
            self.strip = self.search(fg);
        }
        match self.read() {
            Some(j) => j,
            None => {
                // the strip went away (window closed, browser restarted its UI): search again
                self.strip = None;
                "null".into()
            }
        }
    }

    /// The browser's own tab strip: a Tab control in the browser's frame, looked for breadth-first
    /// without ever going into a web page (Document), so a page's own tabs (YouTube's Home /
    /// Videos / Shorts...) are never taken for the browser's, and the page isn't walked at all.
    fn search(&self, fg: HWND) -> Option<IUIAutomationElement> {
        let uia = self.uia.as_ref()?;
        unsafe {
            let walker = uia.ControlViewWalker().ok()?;
            let mut queue = std::collections::VecDeque::from([(uia.ElementFromHandle(fg).ok()?, 0u32)]);
            let mut seen = 0;
            while let Some((e, depth)) = queue.pop_front() {
                seen += 1;
                if seen > 600 {
                    return None;
                }
                let ct = e.CurrentControlType().unwrap_or_default();
                if ct == UIA_TabControlTypeId {
                    return Some(e);
                }
                if ct == UIA_DocumentControlTypeId || depth >= 14 {
                    continue;
                }
                let mut child = walker.GetFirstChildElement(&e).ok();
                while let Some(c) = child {
                    child = walker.GetNextSiblingElement(&c).ok();
                    queue.push_back((c, depth + 1));
                }
            }
            None
        }
    }

    fn read(&self) -> Option<String> {
        let (uia, strip) = (self.uia.as_ref()?, self.strip.as_ref()?);
        unsafe {
            let cond = uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_TabItemControlTypeId.0)).ok()?;
            let cache = uia.CreateCacheRequest().ok()?;
            cache.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
            // grouped tabs (and Edge's) sit a level deeper than the strip itself
            let items = strip.FindAllBuildCache(TreeScope_Descendants, &cond, &cache).ok()?;
            let mut out = Vec::new();
            for i in 0..items.Length().ok()? {
                let r = items.GetElement(i).ok()?.CachedBoundingRectangle().ok()?;
                if let Some([x, y, w, h]) = filter_tab_rect(r.left, r.top, r.right, r.bottom) {
                    out.push(format!("[{x},{y},{w},{h}]"));
                }
            }
            Some(format!("[{}]", out.join(",")))
        }
    }
}

/// Filters out empty rects and oversized non-tab UIA elements (`> 120px` tall).
pub fn filter_tab_rect(left: i32, top: i32, right: i32, bottom: i32) -> Option<[i32; 4]> {
    let w = right.saturating_sub(left);
    let h = bottom.saturating_sub(top);
    (w > 0 && h > 0 && h <= 120).then_some([left, top, w, h])
}

#[cfg(test)]
mod tests {
    use super::filter_tab_rect;

    #[test]
    fn tab_rect_sanity_filters_empty_and_tall() {
        assert_eq!(filter_tab_rect(100, 0, 280, 42), Some([100, 0, 180, 42]));
        assert_eq!(filter_tab_rect(100, 0, 280, 120), Some([100, 0, 180, 120]));
        assert_eq!(filter_tab_rect(100, 0, 280, 121), None);
        assert_eq!(filter_tab_rect(200, 0, 200, 40), None);
        assert_eq!(filter_tab_rect(100, 50, 200, 40), None);
    }
}
