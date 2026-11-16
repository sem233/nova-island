//! Downloads (downloads.js + browser-downloads.js).
//!   IDM Nova   live downloads through its local bridge (127.0.0.1:17890), polled fast while
//!              something moves, slowly while IDM Nova is idle or closed
//!   folders    downloads by browsers and other managers, seen from the folders they save to: a
//!              partial file (.crdownload, .part, ...) that grows is a download, its disappearance
//!              with a finished file left behind is the end. Only the folders themselves are watched
//!              (no subfolders) and no browser file is ever read.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use nova_core::http;
use serde_json::{json, Map, Value};
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        Storage::FileSystem::{FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE},
        System::Threading::{CreateEventW, SetEvent, WaitForMultipleObjects},
    },
};

use crate::ctx::{now_ms, Ctx, Wake};

const BASE: &str = "http://127.0.0.1:17890";
const FAST_MS: u64 = 700;
const IDLE_MS: u64 = 2500;
const OFFLINE_MS: u64 = 6000;
const MAX_THUMB: usize = 400 * 1024;
const MAX_THUMBS: usize = 30;

const TICK_MS: u64 = 1000;
const SHOW_AFTER_MS: f64 = 1500.0; // a download that ends sooner never shows as "downloading"
const STALLED_MS: f64 = 10.0 * 60.0 * 1000.0; // no growth for this long: shown as paused
const KEEP_DONE_MS: f64 = 60.0 * 60.0 * 1000.0;
const MAX_DONE: usize = 10;

struct MdmSource {
    items: Arc<dyn Fn() -> Vec<Value> + Send + Sync>,
    control: Arc<dyn Fn(&str, &str) + Send + Sync>,
}

static MDM_SOURCE: std::sync::LazyLock<Mutex<Option<MdmSource>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Connects built-in MDM video downloads (`v:*` items) into the unified Downloads feed.
pub fn register_mdm_source(
    items: impl Fn() -> Vec<Value> + Send + Sync + 'static,
    control: impl Fn(&str, &str) + Send + Sync + 'static,
) {
    *MDM_SOURCE.lock().unwrap() = Some(MdmSource {
        items: Arc::new(items),
        control: Arc::new(control),
    });
}

fn mdm_items() -> Vec<Value> {
    // Drop MDM_SOURCE lock before invoking callback so re-entrant publish calls cannot deadlock
    let f = MDM_SOURCE.lock().ok().and_then(|g| g.as_ref().map(|s| s.items.clone()));
    f.map_or_else(Vec::new, |cb| cb())
}

fn mdm_control(id: &str, action: &str) -> bool {
    let f = MDM_SOURCE.lock().ok().and_then(|g| g.as_ref().map(|s| s.control.clone()));
    if let Some(cb) = f {
        cb(id, action);
        true
    } else {
        false
    }
}

static EXPECTED_TOTALS: std::sync::LazyLock<Mutex<HashMap<String, u64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn set_expected_total(hint: &str, bytes: u64) {
    if bytes > 0 {
        if let Ok(mut map) = EXPECTED_TOTALS.lock() {
            map.insert(hint.to_lowercase(), bytes);
        }
    }
}

pub fn find_expected_total(name: &str) -> u64 {
    let lower = name.to_lowercase();
    if let Ok(map) = EXPECTED_TOTALS.lock() {
        for (hint, &sz) in map.iter() {
            if lower.contains(hint) || hint.contains(&lower) {
                return sz;
            }
        }
    }
    0
}

fn is_partial(p: &str) -> bool {
    let l = p.to_lowercase();
    [".crdownload", ".part", ".partial", ".download", ".opdownload", ".!ut", ".!qb"].iter().any(|e| l.ends_with(e))
}

/// "Movie.mp4.crdownload" -> "Movie.mp4"
fn final_path(p: &str) -> String {
    match p.rfind('.') {
        Some(i) if is_partial(p) => p[..i].to_string(),
        _ => p.to_string(),
    }
}

fn base_name(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn mtime_ms(m: &std::fs::Metadata) -> f64 {
    m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

fn files_in(dir: &str) -> Vec<String> {
    std::fs::read_dir(dir).map(|r| r.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).map(|e| e.path().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

fn same(a: &str, b: &str) -> bool {
    let n = |p: &str| p.trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase();
    n(a) == n(b)
}

/// Windows' Downloads folder, wherever the user moved it.
pub fn downloads_folder() -> String {
    unsafe {
        windows::Win32::UI::Shell::SHGetKnownFolderPath(&windows::Win32::UI::Shell::FOLDERID_Downloads, windows::Win32::UI::Shell::KNOWN_FOLDER_FLAG(0), None)
            .ok()
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
                s
            })
            .unwrap_or_default()
    }
}

struct Live {
    name: String,
    created_at: f64,
    first_seen: f64,
    size: f64,
    at: f64,
    grew_at: f64,
    speed: f64,
}

struct Folders {
    enabled: bool,
    extra: Vec<String>,
    live: HashMap<String, Live>,
    ignored: HashSet<String>,
    done: Vec<(Value, f64)>, // finished downloads, newest first, with when they finished
}

struct Nova {
    enabled: bool,
    items: Vec<Value>,
    online: bool,
    outdated: bool,
    token: Option<String>,
    thumbs: Vec<(String, String)>, // url -> data URL ("" while loading or failed), oldest first
    last: String,
}

pub struct Downloads {
    ctx: Ctx,
    nova: Mutex<Nova>,
    folders: Mutex<Folders>,
    poll_wake: Wake,
    rewatch: isize, // event handle: the folder list changed
}

impl Downloads {
    pub fn new(ctx: Ctx) -> Arc<Downloads> {
        let extra: Vec<String> = ctx.store().load("download-folders.json").and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().filter_map(|v| v.as_str().map(str::to_string)).filter(|p| Path::new(p).is_absolute()).collect();
        let (nova_on, folders_on) = (ctx.on("downloads"), ctx.on("folderDownloads"));
        let rewatch = unsafe { CreateEventW(None, false, false, None).map(|h| h.0 as isize).unwrap_or(0) };
        Arc::new(Downloads {
            ctx,
            nova: Mutex::new(Nova { enabled: nova_on, items: Vec::new(), online: false, outdated: false, token: None, thumbs: Vec::new(), last: String::new() }),
            folders: Mutex::new(Folders { enabled: folders_on, extra, live: HashMap::new(), ignored: HashSet::new(), done: Vec::new() }),
            poll_wake: Wake::new(),
            rewatch,
        })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            let next = me.poll();
            me.poll_wake.sleep(next);
        });
        let me = self.clone();
        std::thread::spawn(move || me.watch_loop());
    }

    // ---- IDM Nova ----
    /// IDM Nova only answers callers that know the secret it keeps in the user's profile.
    fn headers(&self, extra: &str) -> String {
        let mut n = self.nova.lock().unwrap();
        if n.token.is_none() {
            let file = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("app.idmnova.desktop").join("bridge.token");
            n.token = Some(std::fs::read_to_string(file).map(|t| t.trim().to_string()).unwrap_or_default());
        }
        let t = n.token.clone().unwrap_or_default();
        if t.is_empty() { extra.to_string() } else { format!("{extra}x-nova-token: {t}\r\n") }
    }

    /// A reinstall may have created a new secret: read it again next time.
    pub fn forget_token(&self) {
        self.nova.lock().unwrap().token = None;
    }

    fn exe_path() -> Option<String> {
        let cands = [
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("IDM Nova").join("idm-nova.exe"),
            PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("IDM Nova").join("idm-nova.exe"),
        ];
        cands.iter().find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
    }

    fn poll(self: &Arc<Self>) -> u64 {
        if !self.nova.lock().unwrap().enabled {
            return 60 * 60 * 1000;
        }
        let headers = self.headers("");
        let r = http::request("GET", &format!("{BASE}/activity"), &headers, None, 2000);
        let mut next = OFFLINE_MS;
        let result = r.and_then(|r| {
            if r.status == 403 {
                self.forget_token();
            }
            // an IDM Nova build from before /activity answers 404: running, but can't report
            let outdated = r.status == 404;
            let j: Value = if outdated { json!({}) } else { serde_json::from_slice(&r.body).map_err(|_| "not JSON".to_string())? };
            Ok((outdated, j["items"].as_array().cloned().unwrap_or_default()))
        });
        {
            let mut n = self.nova.lock().unwrap();
            match result {
                Ok((outdated, items)) => {
                    n.outdated = outdated;
                    n.items = items;
                    n.online = true;
                    // forget thumbnails of downloads that left the list, never keep more than MAX_THUMBS
                    let live: HashSet<String> = n.items.iter().filter_map(|d| d["thumbnail"].as_str().map(str::to_string)).collect();
                    n.thumbs.retain(|(u, _)| live.contains(u));
                    while n.thumbs.len() > MAX_THUMBS {
                        n.thumbs.remove(0);
                    }
                    let moving = n.items.iter().any(|d| matches!(d["status"].as_str(), Some("downloading" | "processing" | "queued")));
                    next = if moving { FAST_MS } else { IDLE_MS };
                    let urls: Vec<String> = n.items.iter().filter_map(|d| d["thumbnail"].as_str().map(str::to_string)).collect();
                    drop(n);
                    for u in urls {
                        self.load_thumb(u);
                    }
                }
                Err(_) => {
                    n.online = false;
                    n.outdated = false;
                    n.items.clear();
                }
            }
        }
        if !self.nova.lock().unwrap().enabled {
            return 60 * 60 * 1000; // switched off while the request was in flight
        }
        self.publish();
        next
    }

    /// The island only allows data: images, so remote thumbnails are inlined here (sent once).
    fn load_thumb(self: &Arc<Self>, url: String) {
        {
            let mut n = self.nova.lock().unwrap();
            if n.thumbs.iter().any(|(u, _)| *u == url) || !(url.starts_with("http://") || url.starts_with("https://")) {
                return;
            }
            n.thumbs.push((url.clone(), String::new()));
        }
        let me = self.clone();
        std::thread::spawn(move || {
            // (1.8.7) stopped at MAX_THUMB as it comes in, not checked after all of it is in memory
            let Ok(r) = http::request_progress("GET", &url, "", None, 8000, &mut |n, total| n <= MAX_THUMB && total <= MAX_THUMB) else { return };
            let ty = r.content_type.split(';').next().unwrap_or("").trim().to_lowercase();
            if r.ok() && ty.starts_with("image/") && r.body.len() <= MAX_THUMB {
                let data = format!("data:{ty};base64,{}", nova_sys::media::base64(&r.body));
                if let Some(t) = me.nova.lock().unwrap().thumbs.iter_mut().find(|(u, _)| *u == url) {
                    t.1 = data.clone();
                }
                me.ctx.send("dl-thumb", json!({ "url": url, "data": data }));
            }
        });
    }

    /// Built-in video downloads (MDM `v:*`), then IDM Nova's downloads, then download folders (each file listed once).
    fn all_items(&self) -> Vec<Value> {
        let mut out = mdm_items();
        let nova_items = self.nova.lock().unwrap().items.clone();
        let mut names: HashSet<String> = out.iter().filter_map(|d| d["name"].as_str().map(str::to_string)).collect();
        for d in nova_items {
            if let Some(n) = d["name"].as_str() {
                if !names.insert(n.to_string()) {
                    continue;
                }
            }
            out.push(d);
        }
        let paths: HashSet<String> = out.iter().filter_map(|d| d["path"].as_str().filter(|p| !p.is_empty()).map(|p| p.to_lowercase())).collect();
        out.extend(self.folder_items().into_iter().filter(|d| {
            let n = d["name"].as_str().unwrap_or("");
            let p = d["path"].as_str().unwrap_or("").to_lowercase();
            !names.contains(n) && (p.is_empty() || !paths.contains(&p))
        }));
        out
    }

    fn payload(&self) -> Value {
        let (online, outdated) = {
            let n = self.nova.lock().unwrap();
            (n.online, n.outdated)
        };
        json!({ "online": online, "outdated": outdated, "canLaunch": !online && Self::exe_path().is_some(), "items": self.all_items() })
    }

    /// Triggers an immediate update (used by built-in MDM downloads) and loads any new thumbnails.
    pub fn publish_now(self: &Arc<Self>) {
        let urls: Vec<String> = mdm_items()
            .into_iter()
            .filter_map(|d| d["thumbnail"].as_str().map(str::to_string))
            .collect();
        for u in urls {
            self.load_thumb(u);
        }
        self.publish();
    }

    /// For a freshly loaded island: the list plus every thumbnail already fetched.
    pub fn snapshot(&self) -> Value {
        let mut p = self.payload();
        let thumbs: Map<String, Value> = self.nova.lock().unwrap().thumbs.iter().filter(|(_, d)| !d.is_empty()).map(|(u, d)| (u.clone(), json!(d))).collect();
        p["thumbs"] = Value::Object(thumbs);
        p
    }

    fn publish(&self) {
        let p = self.payload();
        let s = p.to_string();
        let mut n = self.nova.lock().unwrap();
        if s == n.last {
            return;
        }
        n.last = s;
        drop(n);
        self.ctx.send("downloads", p);
    }

    pub fn control(self: &Arc<Self>, id: &str, action: &str) {
        let d = self.all_items().into_iter().find(|x| x["id"].as_str() == Some(id));
        match action {
            "open" | "reveal" => {
                // only paths MDM, IDM Nova or the folder watch itself reported
                if let Some(p) = d.as_ref().and_then(|d| d["path"].as_str()).filter(|p| !p.is_empty()) {
                    crate::shell::open_or_reveal(p, action == "reveal");
                }
                return;
            }
            "launch" => {
                if let Some(exe) = Self::exe_path() {
                    let _ = std::process::Command::new(exe).spawn();
                }
                return;
            }
            _ => {}
        }
        if id.starts_with("v:") {
            if mdm_control(id, action) {
                self.publish_now();
            }
            return;
        }
        match action {
            "cancel" => {
                if let Some(p) = id.strip_prefix("f:") {
                    let _ = std::fs::remove_file(p);
                    let mut f = self.folders.lock().unwrap();
                    f.live.remove(p);
                    f.ignored.insert(p.to_string());
                    drop(f);
                    self.publish();
                }
            }
            "pause" | "resume" | "show" if d.as_ref().is_none_or(|d| d["source"] != "folder") => {
                let body = json!({ "id": if id.is_empty() { Value::Null } else { json!(id) }, "action": action }).to_string();
                let headers = self.headers("content-type: application/json\r\n");
                let _ = http::request("POST", &format!("{BASE}/control"), &headers, Some(body.as_bytes()), 3000);
                self.poll_wake.wake();
            }
            _ => {}
        }
    }

    /// Smart clipboard "download": hand the link to IDM Nova (video sites as media). Err when IDM
    /// Nova didn't take it (the caller opens the link instead).
    pub fn add_url(&self, url: &str, media: bool) -> Result<(), String> {
        let body = json!({ "url": url, "media": media }).to_string();
        let headers = self.headers("content-type: application/json\r\n");
        let r = http::request("POST", &format!("{BASE}/add"), &headers, Some(body.as_bytes()), 4000)?;
        if r.status == 403 {
            self.forget_token();
        }
        if !r.ok() {
            return Err(format!("IDM Nova answered {}", r.status));
        }
        Ok(())
    }

    /// Turned off in Settings: stop asking IDM Nova and clear the island.
    pub fn set_enabled(&self, on: bool) {
        {
            let mut n = self.nova.lock().unwrap();
            if n.enabled == on {
                return;
            }
            n.enabled = on;
            n.online = false;
            n.outdated = false;
            n.items.clear();
        }
        self.publish();
        self.poll_wake.wake();
    }

    // ---- download folders ----
    fn folder_list_paths(&self) -> Vec<String> {
        let f = self.folders.lock().unwrap();
        let mut out: Vec<String> = Vec::new();
        for p in std::iter::once(downloads_folder()).chain(f.extra.iter().cloned()) {
            if !p.is_empty() && !out.iter().any(|x| same(x, &p)) {
                out.push(p);
            }
        }
        out
    }

    pub fn folder_list(&self) -> Value {
        Value::Array(self.folder_list_paths().iter().enumerate().map(|(i, p)| json!({ "path": p, "fixed": i == 0 })).collect())
    }

    pub fn add_folder(&self, p: &str) -> Value {
        if Path::new(p).is_absolute() && Path::new(p).is_dir() && !self.folder_list_paths().iter().any(|x| same(x, p)) {
            let extra = {
                let mut f = self.folders.lock().unwrap();
                f.extra.push(p.to_string());
                f.extra.clone()
            };
            let _ = self.ctx.store().save("download-folders.json", &json!(extra));
            self.rewatch();
        }
        self.folder_list()
    }

    pub fn remove_folder(&self, p: &str) -> Value {
        let extra = {
            let mut f = self.folders.lock().unwrap();
            f.extra.retain(|x| !same(x, p));
            f.live.retain(|k, _| !same(&Path::new(k).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(), p));
            f.extra.clone()
        };
        let _ = self.ctx.store().save("download-folders.json", &json!(extra));
        self.rewatch();
        self.publish();
        self.folder_list()
    }

    pub fn set_folders(&self, on: bool) {
        {
            let mut f = self.folders.lock().unwrap();
            if f.enabled == on {
                return;
            }
            f.enabled = on;
            f.live.clear();
            f.done.clear();
        }
        self.rewatch();
        self.publish();
    }

    fn rewatch(&self) {
        unsafe {
            let _ = SetEvent(HANDLE(self.rewatch as *mut _));
        }
    }

    /// The folders' downloads in the same shape as IDM Nova's.
    fn folder_items(&self) -> Vec<Value> {
        let f = self.folders.lock().unwrap();
        if !f.enabled {
            return Vec::new();
        }
        let now = now_ms();
        let mut live: Vec<(&String, &Live)> = f.live.iter().filter(|(_, d)| now - d.first_seen >= SHOW_AFTER_MS).collect();
        live.sort_by(|a, b| a.1.first_seen.partial_cmp(&b.1.first_seen).unwrap());
        let mut out: Vec<Value> = live
            .iter()
            .map(|(p, d)| {
                let total = find_expected_total(&d.name);
                let eta = if total > d.size as u64 && d.speed > 0.0 {
                    Some(((total - d.size as u64) as f64 / d.speed).round() as u64)
                } else {
                    None
                };
                json!({
                    "id": format!("f:{p}"), "name": d.name, "source": "folder", "createdAt": d.created_at, "downloaded": d.size, "total": total, "eta": eta,
                    "status": if now - d.grew_at > STALLED_MS { "paused" } else { "downloading" }, "speed": if now - d.grew_at > 3000.0 { 0.0 } else { d.speed },
                })
            })
            .collect();
        out.extend(f.done.iter().map(|(d, _)| d.clone()));
        out
    }

    fn finish(f: &mut Folders, partial: &str, d: Live) {
        let dir = Path::new(partial).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let mut file = final_path(partial);
        let mut size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        // Chrome's "Unconfirmed 123.crdownload" gets its real name only at the end: the newest
        // file that appeared in the folder in the last few seconds
        if size == 0 && base_name(partial).to_lowercase().starts_with("unconfirmed ") {
            let now = now_ms();
            let fresh = files_in(&dir).into_iter().filter(|p| !is_partial(p)).filter_map(|p| std::fs::metadata(&p).ok().map(|m| (p, m))).filter(|(_, m)| m.len() > 0 && now - mtime_ms(m) < 5000.0).max_by(|a, b| mtime_ms(&a.1).partial_cmp(&mtime_ms(&b.1)).unwrap());
            if let Some((p, m)) = fresh {
                file = p;
                size = m.len();
            }
        }
        if size == 0 {
            return; // cancelled
        }
        let item = json!({ "id": format!("f:{file}"), "name": base_name(&file), "source": "folder", "createdAt": d.created_at, "downloaded": size, "total": size, "eta": null, "status": "completed", "speed": 0, "path": file });
        f.done.retain(|(x, _)| x["path"].as_str() != Some(file.as_str()));
        f.done.insert(0, (item, now_ms()));
        f.done.truncate(MAX_DONE);
    }

    /// One look at the folders; true while a download is in progress (look again in a second).
    fn scan(&self) -> bool {
        let now = now_ms();
        let dirs = self.folder_list_paths();
        let mut f = self.folders.lock().unwrap();
        if !f.enabled {
            return false;
        }
        let mut seen = HashSet::new();
        for dir in &dirs {
            for p in files_in(dir) {
                if !is_partial(&p) || f.ignored.contains(&p) {
                    continue;
                }
                seen.insert(p.clone());
                let Ok(m) = std::fs::metadata(&p) else { continue };
                let size = m.len() as f64;
                let d = f.live.entry(p.clone()).or_insert_with(|| Live { name: base_name(&final_path(&p)), created_at: now, first_seen: now, size, at: now, grew_at: now, speed: 0.0 });
                let dt = (now - d.at) / 1000.0;
                if dt >= 0.5 {
                    let rate = ((size - d.size) / dt).max(0.0);
                    d.speed = if d.speed != 0.0 { d.speed * 0.6 + rate * 0.4 } else { rate };
                    if size > d.size {
                        d.grew_at = now;
                    }
                    d.size = size;
                    d.at = now;
                }
            }
        }
        let gone: Vec<String> = f.live.keys().filter(|k| !seen.contains(*k)).cloned().collect();
        for p in gone {
            if let Some(d) = f.live.remove(&p) {
                Self::finish(&mut f, &p, d);
            }
        }
        f.done.retain(|(_, at)| now - at < KEEP_DONE_MS);
        let busy = !f.live.is_empty();
        drop(f);
        self.publish();
        busy
    }

    /// Waits on change notifications for the folders (and the "folder list changed" event).
    fn watch_loop(&self) {
        loop {
            // partial files already there are old (a browser closed mid-download), not news
            let dirs = if self.folders.lock().unwrap().enabled { self.folder_list_paths() } else { Vec::new() };
            {
                let mut f = self.folders.lock().unwrap();
                for d in &dirs {
                    for p in files_in(d) {
                        if is_partial(&p) {
                            f.ignored.insert(p);
                        }
                    }
                }
            }
            let mut handles: Vec<HANDLE> = vec![HANDLE(self.rewatch as *mut _)];
            for d in &dirs {
                if let Ok(h) = unsafe { FindFirstChangeNotificationW(&HSTRING::from(d.as_str()), false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE) } {
                    handles.push(h);
                }
            }
            let mut busy = false;
            loop {
                let r = unsafe { WaitForMultipleObjects(&handles, false, if busy { TICK_MS as u32 } else { u32::MAX }) };
                if r == WAIT_OBJECT_0 {
                    break; // the folder list changed: watch again
                }
                let i = (r.0 - WAIT_OBJECT_0.0) as usize;
                if i < handles.len() {
                    unsafe {
                        let _ = FindNextChangeNotification(handles[i]);
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                busy = self.scan();
            }
            for h in &handles[1..] {
                unsafe {
                    let _ = FindCloseChangeNotification(*h);
                }
            }

        }
    }
}
