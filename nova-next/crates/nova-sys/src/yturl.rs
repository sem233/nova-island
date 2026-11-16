//! The URL of the browser tab playing a video (YtUrl.cs, `nova-helper yturl <process> <title>`),
//! for the mini player. Returns {"url":"...","how":"..."}.
//!   1) reads the address bar of each browser window (windows whose title matches first)
//!   2) if the playing tab isn't the active one, briefly selects it, reads the URL, then switches
//!      back to the tab that was active
//! Runs on its own single-threaded apartment, as UI Automation wants.
use std::{collections::HashSet, time::Duration};

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

fn is_video(u: &str) -> bool {
    let l = u.to_lowercase();
    let l = l.trim();
    if l.is_empty() || l.starts_with("chrome://") || l.starts_with("edge://") || l.starts_with("about:") || l.starts_with("brave://") {
        return false;
    }
    l.contains("youtube.com")
        || l.contains("youtu.be")
        || l.contains("facebook.com")
        || l.contains("tiktok.com")
        || l.contains("instagram.com")
        || l.contains("twitter.com")
        || l.contains("x.com")
        || l.contains("vimeo.com")
        || l.contains("dailymotion.com")
        || l.contains("soundcloud.com")
        || l.contains("watch")
        || l.contains("video")
        || l.contains(".")
        || l.starts_with("http")
}

/// Whitespace runs to one space, trimmed, lower case.
fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
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
    fn cond_type(&self, t: UIA_CONTROLTYPE_ID) -> Option<IUIAutomationCondition> {
        unsafe { self.a.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(t.0)).ok() }
    }
    fn url_of(&self, w: &IUIAutomationElement) -> String {
        unsafe {
            let Some(c) = self.cond_type(UIA_EditControlTypeId) else { return String::new() };
            w.FindFirst(TreeScope_Descendants, &c)
                .and_then(|e| e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId))
                .and_then(|p| p.CurrentValue())
                .map(|b| b.to_string())
                .unwrap_or_default()
        }
    }
    fn all(&self, w: &IUIAutomationElement, scope: TreeScope, c: &IUIAutomationCondition) -> Vec<IUIAutomationElement> {
        unsafe {
            let Ok(arr) = w.FindAll(scope, c) else { return vec![] };
            (0..arr.Length().unwrap_or(0)).filter_map(|i| arr.GetElement(i).ok()).collect()
        }
    }
}

fn name(e: &IUIAutomationElement) -> String {
    unsafe { e.CurrentName().map(|b: BSTR| b.to_string()).unwrap_or_default() }
}

fn selected(tab: &IUIAutomationElement) -> bool {
    unsafe { tab.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).and_then(|p| p.CurrentIsSelected()).map(|b| b.as_bool()).unwrap_or(false) }
}

fn find(proc: &str, title: &str) -> String {
    let want = norm(title);
    let pids = pids_of(proc);
    if pids.is_empty() {
        return json("", "no-browser");
    }
    let Ok(a) = (unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }) else { return json("", "not-found") };
    let u = Uia { a };
    let Ok(root) = (unsafe { u.a.GetRootElement() }) else { return json("", "not-found") };
    let Ok(cls) = (unsafe { u.a.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(BSTR::from("Chrome_WidgetWin_1"))) }) else { return json("", "not-found") };
    let mut windows: Vec<(IUIAutomationElement, String)> = u
        .all(&root, TreeScope_Children, &cls)
        .into_iter()
        .filter(|w| unsafe { w.CurrentProcessId() }.is_ok_and(|p| pids.contains(&(p as u32))))
        .map(|w| {
            let n = name(&w);
            (w, n)
        })
        .filter(|(_, n)| !n.is_empty())
        .collect();
    // windows whose title contains the video title first (a stable sort, like OrderBy)
    windows.sort_by_key(|(_, n)| if !want.is_empty() && norm(n).contains(&want) { 0 } else { 1 });

    for (w, n) in &windows {
        let url = u.url_of(w);
        if is_video(&url) && (want.is_empty() || norm(n).contains(&want) || windows.len() == 1) {
            return json(&url, "address-bar");
        }
    }
    // not the active tab: select it, read, restore
    if !want.is_empty() {
        let Some(tab_cond) = u.cond_type(UIA_TabItemControlTypeId) else { return json("", "not-found") };
        for (w, _) in &windows {
            let tabs = u.all(w, TreeScope_Descendants, &tab_cond);
            let Some(target) = tabs.iter().find(|t| norm(&name(t)).contains(&want)) else { continue };
            let prev = tabs.iter().find(|t| selected(t));
            let select = |t: &IUIAutomationElement| unsafe { t.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId).and_then(|p| p.Select()) };
            if select(target).is_err() {
                continue;
            }
            std::thread::sleep(Duration::from_millis(250));
            let url = u.url_of(w);
            if let Some(prev) = prev {
                let same = unsafe { u.a.CompareElements(prev, target) }.map(|b| b.as_bool()).unwrap_or(false);
                if !same {
                    let _ = select(prev);
                }
            }
            if is_video(&url) {
                return json(&url, "tab");
            }
        }
    }
    // last resort: any window showing a video
    for (w, _) in &windows {
        let url = u.url_of(w);
        if is_video(&url) {
            return json(&url, "any");
        }
    }
    json("", "not-found")
}

/// {"url","how"} for the video `title` playing in browser `proc` ("chrome", "msedge", ...).
pub fn yt_url(proc: &str, title: &str) -> String {
    let (proc, title) = (proc.to_string(), title.to_string());
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        find(&proc, &title)
    })
    .join()
    .unwrap_or_else(|_| json("", "not-found"))
}
