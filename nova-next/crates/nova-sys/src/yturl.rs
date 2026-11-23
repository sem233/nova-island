//! The URL of the browser tab playing a video (YtUrl.cs, `nova-helper yturl <process> <title>`),
//! for the mini player and MDM video downloader. Returns {"url":"...","how":"..."}.
//!   1) reads the address bar of each browser window (windows whose title matches first)
//!   2) if allowed (`allow_tab_switch`) and the playing tab isn't the active one, briefly selects
//!      it, reads the URL, then switches back to the tab that was active
//! Runs on its own single-threaded apartment, as UI Automation wants.
//! Uses Document-skipping BFS when searching Chrome/Edge UIA trees so heavy web pages are never
//! traversed (runs in milliseconds instead of hanging for seconds).
use std::{collections::{HashSet, VecDeque}, time::Duration};

use windows::{
    core::BSTR,
    Win32::{
        Foundation::CloseHandle,
        System::{
            Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED},
            Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS},
            Variant::VARIANT,
        },
        UI::Accessibility::*,
    },
};

fn is_browser_internal(u: &str) -> bool {
    let l = u.trim().to_lowercase();
    l.is_empty()
        || l.starts_with("chrome://")
        || l.starts_with("chrome-extension://")
        || l.starts_with("edge://")
        || l.starts_with("about:")
        || l.starts_with("brave://")
        || l.starts_with("vivaldi://")
        || l.starts_with("opera://")
}

/// Known video/streaming URL patterns (matched even when window title wasn't matched verbatim).
fn is_known_video(u: &str) -> bool {
    if is_browser_internal(u) {
        return false;
    }
    let l = u.trim().to_lowercase();
    l.contains("youtube.com")
        || l.contains("youtu.be")
        || l.contains("facebook.com")
        || l.contains("fb.watch")
        || l.contains("tiktok.com")
        || l.contains("instagram.com")
        || l.contains("twitter.com")
        || l.contains("x.com")
        || l.contains("vimeo.com")
        || l.contains("dailymotion.com")
        || l.contains("soundcloud.com")
        || l.contains("twitch.tv")
        || l.contains("kick.com")
        || l.contains("bilibili.com")
        || l.contains("vk.com")
        || l.contains("reddit.com")
        || l.contains("watch?v=")
        || l.contains("/shorts/")
        || l.contains("/reel/")
        || l.contains("/video")
        || l.contains(".m3u8")
        || l.contains(".mpd")
        || l.contains(".mp4")
        || l.contains(".ts")
}

/// Any valid web URL (used when window/tab title actually matches the playing media title).
fn is_web_url(u: &str) -> bool {
    if is_browser_internal(u) {
        return false;
    }
    let l = u.trim().to_lowercase();
    (l.contains('.') && !l.contains(' ')) || l.starts_with("http://") || l.starts_with("https://")
}

/// Whitespace runs to one space, trimmed, lower case.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn title_matches(haystack: &str, want: &str) -> bool {
    if want.is_empty() || haystack.is_empty() {
        return false;
    }
    let h = norm(haystack);
    let w = norm(want);
    if h.contains(&w) {
        return true;
    }
    // Browsers sometimes truncate long titles in the tab bar — match on a meaningful prefix (>= 16 chars)
    let prefix: String = w.chars().take(32).collect();
    prefix.chars().count() >= 16 && h.contains(&prefix)
}

fn json(url: &str, how: &str) -> String {
    let u = url.trim();
    let full = if u.is_empty() {
        String::new()
    } else if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("https://{u}")
    };
    format!("{{\"url\":\"{}\",\"how\":\"{how}\"}}", full.replace('\\', "\\\\").replace('"', "\\\""))
}

fn pids_of(proc: &str) -> HashSet<u32> {
    let mut out = HashSet::new();
    let want = proc.to_lowercase();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase();
            if name.strip_suffix(".exe").unwrap_or(&name) == want {
                out.insert(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(windows::Win32::Foundation::HANDLE(snap.0));
    }
    out
}

struct Uia {
    a: IUIAutomation,
}

impl Uia {
    fn all(&self, w: &IUIAutomationElement, scope: TreeScope, c: &IUIAutomationCondition) -> Vec<IUIAutomationElement> {
        unsafe {
            let Ok(arr) = w.FindAll(scope, c) else { return vec![] };
            (0..arr.Length().unwrap_or(0)).filter_map(|i| arr.GetElement(i).ok()).collect()
        }
    }

    /// Breadth-first search through the browser's own frame ONLY — skips `UIA_DocumentControlTypeId`
    /// so heavy web pages are never traversed. Much faster than TreeScope_Descendants.
    fn bfs_chrome<F>(&self, root: &IUIAutomationElement, max_nodes: usize, mut visitor: F)
    where
        F: FnMut(&IUIAutomationElement, i32) -> bool,
    {
        unsafe {
            let Ok(walker) = self.a.ControlViewWalker() else { return };
            let mut q = VecDeque::new();
            q.push_back(root.clone());
            let mut seen = 0;
            while let Some(el) = q.pop_front() {
                seen += 1;
                if seen > max_nodes {
                    break;
                }
                let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
                if ct == UIA_DocumentControlTypeId.0 {
                    continue;
                }
                if !visitor(&el, ct) {
                    break;
                }
                if let Ok(mut child) = walker.GetFirstChildElement(&el) {
                    loop {
                        let c_ct = child.CurrentControlType().map(|c| c.0).unwrap_or(0);
                        if c_ct != UIA_DocumentControlTypeId.0 {
                            q.push_back(child.clone());
                        }
                        match walker.GetNextSiblingElement(&child) {
                            Ok(next) => child = next,
                            Err(_) => break,
                        }
                    }
                }
            }
        }
    }

    fn url_of(&self, w: &IUIAutomationElement) -> String {
        let mut found = String::new();
        self.bfs_chrome(w, 450, |el, ct| {
            if ct == UIA_EditControlTypeId.0 {
                unsafe {
                    if let Ok(pat) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                        if let Ok(val) = pat.CurrentValue() {
                            let s = val.to_string();
                            let wide = el.CurrentBoundingRectangle().map(|r| r.right - r.left >= 180).unwrap_or(true);
                            if wide && !s.is_empty() {
                                found = s;
                                return false;
                            }
                        }
                    }
                }
            }
            true
        });
        found
    }

    fn tabs_of(&self, w: &IUIAutomationElement) -> Vec<IUIAutomationElement> {
        let mut out = Vec::new();
        self.bfs_chrome(w, 600, |el, ct| {
            if ct == UIA_TabItemControlTypeId.0 {
                out.push(el.clone());
            }
            true
        });
        out
    }
}

fn name(e: &IUIAutomationElement) -> String {
    unsafe { e.CurrentName().map(|b: BSTR| b.to_string()).unwrap_or_default() }
}

fn selected(tab: &IUIAutomationElement) -> bool {
    unsafe {
        tab.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
            .and_then(|p| p.CurrentIsSelected())
            .map(|b| b.as_bool())
            .unwrap_or(false)
    }
}

fn find(proc: &str, title: &str, allow_tab_switch: bool) -> String {
    let want = norm(title);
    let pids = pids_of(proc);
    if pids.is_empty() {
        return json("", "no-browser");
    }
    let Ok(a) = (unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }) else {
        return json("", "not-found");
    };
    if let Ok(u2) = windows::core::Interface::cast::<IUIAutomation2>(&a) {
        unsafe {
            let _ = u2.SetConnectionTimeout(800);
            let _ = u2.SetTransactionTimeout(800);
        }
    }
    let u = Uia { a };
    let Ok(root) = (unsafe { u.a.GetRootElement() }) else { return json("", "not-found") };
    let Ok(cls) = (unsafe { u.a.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(BSTR::from("Chrome_WidgetWin_1"))) }) else {
        return json("", "not-found");
    };
    let fg_hwnd = crate::win::foreground().0 as isize;
    let mut windows: Vec<(IUIAutomationElement, String, bool)> = u
        .all(&root, TreeScope_Children, &cls)
        .into_iter()
        .filter(|w| unsafe { w.CurrentProcessId() }.is_ok_and(|p| pids.contains(&(p as u32))))
        .map(|w| {
            let n = name(&w);
            let hwnd = unsafe { w.CurrentNativeWindowHandle().map(|h| h.0 as isize).unwrap_or(0) };
            (w, n, hwnd != 0 && hwnd == fg_hwnd)
        })
        .filter(|(_, n, _)| !n.is_empty())
        .collect();
    // windows whose title matches first; break ties in favor of the foreground browser window
    windows.sort_by_key(|(_, n, is_fg)| (if title_matches(n, &want) { 0 } else { 1 }, if *is_fg { 0 } else { 1 }));

    // 1. Check active windows whose title matches (or when no title filter was requested)
    for (w, n, _) in &windows {
        let matched = title_matches(n, &want);
        if matched || want.is_empty() {
            let url = u.url_of(w);
            if is_web_url(&url) {
                return json(&url, "address-bar");
            }
        }
    }

    // 1b. If the user is actively viewing a video page in the foreground browser window (e.g.
    // navigated to a second video while SMTC still reports the previous video's title), prefer
    // the live foreground video URL over jumping to a stale background tab.
    if let Some((w, _, _)) = windows.iter().find(|(_, _, is_fg)| *is_fg) {
        let url = u.url_of(w);
        let l = url.to_ascii_lowercase();
        if is_known_video(&url) || (is_web_url(&url) && (l.contains("/watch") || l.contains("/embed") || l.contains("/episode") || l.contains("/movie"))) {
            return json(&url, "foreground-video");
        }
    }

    // 2. If allowed, check background tabs matching title BEFORE falling back to an unrelated active tab
    if allow_tab_switch && !want.is_empty() {
        for (w, _, _) in &windows {
            let tabs = u.tabs_of(w);
            let Some(target) = tabs.iter().find(|t| title_matches(&name(t), &want)) else { continue };
            let prev = tabs.iter().find(|t| selected(t));
            let select = |t: &IUIAutomationElement| unsafe {
                t.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).and_then(|p| p.Select())
            };
            if select(target).is_err() {
                continue;
            }
            std::thread::sleep(Duration::from_millis(200));
            let url = u.url_of(w);
            if let Some(prev) = prev {
                let same = unsafe { u.a.CompareElements(prev, target) }.map(|b| b.as_bool()).unwrap_or(false);
                if !same {
                    let _ = select(prev);
                }
            }
            if is_web_url(&url) {
                return json(&url, "tab");
            }
        }
    }

    // 3. Check if an active window is already on a known video site (e.g. TikTok/Reels where tab title differs from media)
    for (w, _, _) in &windows {
        let url = u.url_of(w);
        if is_known_video(&url) {
            return json(&url, "address-bar");
        }
    }

    // 4. Final active-tab fallback ONLY if allow_tab_switch (user explicitly clicked) & only 1 window
    if allow_tab_switch && windows.len() == 1 {
        let url = u.url_of(&windows[0].0);
        if is_web_url(&url) {
            return json(&url, "any");
        }
    }

    json("", "not-found")
}

/// {"url","how"} for the video `title` playing in browser `proc` ("chrome", "msedge", ...).
pub fn yt_url(proc: &str, title: &str) -> String {
    yt_url_opts(proc, title, true)
}

/// Variant letting passive callers skip tab-switching side effects.
pub fn yt_url_opts(proc: &str, title: &str, allow_tab_switch: bool) -> String {
    let (proc, title) = (proc.to_string(), title.to_string());
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        find(&proc, &title, allow_tab_switch)
    })
    .join()
    .unwrap_or_else(|_| json("", "not-found"))
}
