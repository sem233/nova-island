//! MDM — video and high-quality MP3 stream downloader for Nova Island.
//! Native Rust port matching v1.7.6 mdm.js behavior:
//! - In-memory probe cache (8 min TTL) + in-flight probe deduplication (concurrent click shares probe)
//! - Passive URL resolution (`yt_url_opts(..., false)`) so background auto-probe never switches tabs
//! - Full format parsing for all heights (4K, 2K, 1080p..360p) + MP3 (320k) + TS/HLS
//! - `--js-runtimes` auto-detection (deno/node/bun) + accurate size estimation
//! - Full download lifecycle items (`v:1`, `v:2`, ...): live bytes, total, speed, ETA, thumbnail
//! - Live progress parsing (`NOVA_META`, `NOVA_PROG`, `NOVA_POST`, `NOVA_FILE`) + temp dir isolation
//! - Instant Pause / Resume (`--continue`) / Cancel (`taskkill /PID ... /T /F` + temp cleanup)
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use serde_json::{json, Value};

const PROBE_TTL_SECS: u64 = 8 * 60;
const KEEP_DONE_MS: u64 = 60 * 60 * 1000;
const MAX_ITEMS: usize = 25;
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

struct ProbeCacheEntry {
    at: Instant,
    data: Value,
}

static PROBE_CACHE: std::sync::LazyLock<Mutex<HashMap<String, ProbeCacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

type InFlightSlot = Arc<(Mutex<Option<Value>>, Condvar)>;
static IN_FLIGHT_PROBES: std::sync::LazyLock<Mutex<HashMap<String, InFlightSlot>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct MdmItem {
    id: String,
    url: String,
    referer: String,
    base_title: String,
    name: String,
    path: String,
    thumbnail: String,
    label: String,
    tag: String,
    selector: String,
    kind: String, // "video" | "audio"
    info_file: String,
    created_at: u64,
    downloaded: u64,
    total: u64,
    speed: f64,
    eta: Option<u64>,
    status: String, // "downloading" | "processing" | "paused" | "completed" | "error"
    stage: String,  // "starting" | ""
    error: String,
    finished_at: u64,
    stream_dl: u64,
    stream_total: u64,
    done_bytes: u64,
    canceling: bool,
    pausing: bool,
    pid: Option<u32>,
    ytdlp_exe: PathBuf,
    ffmpeg_exe: Option<PathBuf>,
    run_seq: u64,
}

struct DownloadsState {
    seq: u64,
    run_seq: u64,
    items: Vec<MdmItem>,
    last_notify: Instant,
}

static DOWNLOADS: std::sync::LazyLock<Mutex<DownloadsState>> = std::sync::LazyLock::new(|| {
    Mutex::new(DownloadsState {
        seq: 0,
        run_seq: 0,
        items: Vec::new(),
        last_notify: Instant::now() - Duration::from_secs(10),
    })
});

static ON_CHANGE: std::sync::LazyLock<Mutex<Option<Box<dyn Fn() + Send + Sync>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

static PROGRESS_CB: std::sync::LazyLock<Mutex<Option<Box<dyn Fn(Value) + Send + Sync>>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

pub fn set_progress_cb(f: impl Fn(Value) + Send + Sync + 'static) {
    *PROGRESS_CB.lock().unwrap() = Some(Box::new(f));
}

pub fn set_on_change(f: impl Fn() + Send + Sync + 'static) {
    *ON_CHANGE.lock().unwrap() = Some(Box::new(f));
}

fn emit_progress(val: Value) {
    if let Ok(cb) = PROGRESS_CB.lock() {
        if let Some(f) = cb.as_ref() {
            f(val);
        }
    }
}

fn notify(immediate: bool) {
    let fire = {
        let mut st = DOWNLOADS.lock().unwrap();
        if immediate || st.last_notify.elapsed() >= Duration::from_millis(250) {
            st.last_notify = Instant::now();
            true
        } else {
            false
        }
    };
    if fire {
        if let Ok(cb) = ON_CHANGE.lock() {
            if let Some(f) = cb.as_ref() {
                f();
            }
        }
    }
}

fn prune_locked(st: &mut DownloadsState) {
    let now = now_ms();
    st.items.retain(|d| d.finished_at == 0 || now.saturating_sub(d.finished_at) <= KEEP_DONE_MS);
    while st.items.len() > MAX_ITEMS {
        if let Some(pos) = st.items.iter().position(|d| !matches!(d.status.as_str(), "downloading" | "processing" | "queued")) {
            st.items.remove(pos);
        } else {
            break;
        }
    }
}

pub fn items() -> Vec<Value> {
    let mut st = DOWNLOADS.lock().unwrap();
    prune_locked(&mut st);
    st.items
        .iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "source": "mdm",
                "path": d.path,
                "thumbnail": d.thumbnail,
                "createdAt": d.created_at,
                "downloaded": d.downloaded,
                "total": d.total,
                "speed": d.speed,
                "eta": d.eta,
                "status": d.status,
                "stage": d.stage,
                "error": d.error,
            })
        })
        .collect()
}

pub fn active_downloads() -> Vec<Value> {
    let st = DOWNLOADS.lock().unwrap();
    st.items
        .iter()
        .filter(|d| matches!(d.status.as_str(), "downloading" | "processing" | "queued"))
        .map(|d| Value::String(d.id.clone()))
        .collect()
}

pub fn temp_dir() -> PathBuf {
    let d = std::env::temp_dir().join("nova-video-dl");
    let _ = fs::create_dir_all(&d);
    d
}

fn clean_stem(title: &str) -> String {
    let mut s = title.trim().to_string();
    if let Some(dot) = s.rfind('.') {
        let ext_len = s.len() - dot - 1;
        if (2..=4).contains(&ext_len) && s[dot + 1..].chars().all(|c| c.is_ascii_alphanumeric()) {
            s.truncate(dot);
        }
    }
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(140).collect();
    let trimmed = truncated.trim();
    if trimmed.is_empty() { "Video".to_string() } else { trimmed.to_string() }
}

fn clean_name(title: &str, ext: &str) -> String {
    format!("{}.{ext}", clean_stem(title))
}

fn kill_pid_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut tk = Command::new("taskkill");
        tk.args(["/PID", &pid.to_string(), "/T", "/F"]);
        tk.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let _ = tk.status();
    }
}

fn clean_temp_for(stem_src: &str) {
    let stem = clean_stem(stem_src).chars().take(40).collect::<String>().to_lowercase();
    if stem.is_empty() {
        return;
    }
    let td = temp_dir();
    if let Ok(entries) = fs::read_dir(&td) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.contains(&stem) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

pub fn cancel_download(id: &str) {
    control(id, "cancel");
}

pub fn control(id: &str, action: &str) {
    let mut kill_pid = None;
    let mut stem_to_clean = None;
    let mut resume_item = None;

    {
        let mut st = DOWNLOADS.lock().unwrap();
        let Some(idx) = st.items.iter().position(|d| d.id == id) else {
            return;
        };
        match action {
            "cancel" => {
                let item = st.items.remove(idx);
                kill_pid = item.pid;
                stem_to_clean = Some(item.base_title.clone());
            }
            "pause" => {
                let item = &mut st.items[idx];
                item.pausing = true;
                item.status = "paused".into();
                item.stage.clear();
                item.speed = 0.0;
                item.eta = None;
                kill_pid = item.pid;
                item.pid = None;
            }
            "resume" => {
                let item = &mut st.items[idx];
                if item.pid.is_some() || !matches!(item.status.as_str(), "paused" | "error") {
                    return;
                }
                resume_item = Some(item.id.clone());
            }
            "open" | "reveal" => {
                let p = st.items[idx].path.clone();
                if !p.is_empty() {
                    if action == "reveal" {
                        crate::files::reveal(&p);
                    } else {
                        crate::files::open(&p);
                    }
                }
                return;
            }
            _ => return,
        }
    }

    if let Some(pid) = kill_pid {
        std::thread::spawn(move || {
            kill_pid_tree(pid);
            if let Some(stem) = stem_to_clean {
                std::thread::sleep(Duration::from_millis(450));
                clean_temp_for(&stem);
            }
        });
    } else if let Some(stem) = stem_to_clean {
        clean_temp_for(&stem);
    }

    if let Some(id) = resume_item {
        spawn_download(id, false);
    }

    notify(true);
}

pub fn find_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p1 = dir.join("yt-dlp.exe");
            if p1.is_file() {
                return Some(p1);
            }
            let p2 = dir.join("tools").join("yt-dlp.exe");
            if p2.is_file() {
                return Some(p2);
            }
        }
    }
    let local = data_dir.join("tools").join("yt-dlp.exe");
    if local.is_file() {
        return Some(local);
    }
    if let Ok(app_data) = std::env::var("APPDATA") {
        let shared = PathBuf::from(&app_data).join("dynamic-nova").join("tools").join("yt-dlp.exe");
        if shared.is_file() {
            return Some(shared);
        }
    }
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local_app).join("Microsoft").join("WinGet").join("Links").join("yt-dlp.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join("yt-dlp.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn find_ffmpeg(data_dir: &Path) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p1 = dir.join("ffmpeg.exe");
            if p1.is_file() {
                return Some(p1);
            }
            let p2 = dir.join("tools").join("ffmpeg.exe");
            if p2.is_file() {
                return Some(p2);
            }
        }
    }
    let local = data_dir.join("tools").join("ffmpeg.exe");
    if local.is_file() {
        return Some(local);
    }
    if let Ok(app_data) = std::env::var("APPDATA") {
        let shared = PathBuf::from(&app_data).join("dynamic-nova").join("tools").join("ffmpeg.exe");
        if shared.is_file() {
            return Some(shared);
        }
    }
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let la = PathBuf::from(&local_app);
        let link = la.join("Microsoft").join("WinGet").join("Links").join("ffmpeg.exe");
        if link.is_file() {
            return Some(link);
        }
        let packages = la.join("Microsoft").join("WinGet").join("Packages");
        if packages.is_dir() {
            if let Ok(entries) = fs::read_dir(&packages) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    if name.contains("ffmpeg") {
                        let path = e.path();
                        let cand1 = path.join("bin").join("ffmpeg.exe");
                        if cand1.is_file() {
                            return Some(cand1);
                        }
                        if let Ok(subs) = fs::read_dir(&path) {
                            for sub in subs.flatten() {
                                let cand2 = sub.path().join("bin").join("ffmpeg.exe");
                                if cand2.is_file() {
                                    return Some(cand2);
                                }
                            }
                        }
                    }
                }
            }
        }
        for sub in &[
            "CapCut\\Apps\\4.6.0.1754",
            "Filee\\engines\\ffmpeg\\bin",
            "hermes\\tools\\ffmpeg-9.0.1-win32-x64\\bin",
        ] {
            let p = la.join(sub).join("ffmpeg.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let p = dir.join("ffmpeg.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn find_js_runtime() -> Option<String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        dirs.push(PathBuf::from(&pf).join("nodejs"));
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        let la = PathBuf::from(&la);
        dirs.push(la.join("Microsoft").join("WinGet").join("Links"));
        dirs.push(la.join("Programs").join("nodejs"));
    }
    if let Ok(up) = std::env::var("USERPROFILE") {
        let up = PathBuf::from(&up);
        dirs.push(up.join(".deno").join("bin"));
        dirs.push(up.join("scoop").join("shims"));
    }
    dirs.push(PathBuf::from("C:\\Program Files\\nodejs"));
    for (name, exe) in [("deno", "deno.exe"), ("node", "node.exe"), ("bun", "bun.exe")] {
        for d in &dirs {
            let p = d.join(exe);
            if p.is_file() {
                return Some(format!("{name}:{}", p.to_string_lossy()));
            }
        }
    }
    None
}

pub fn ensure_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = find_ytdlp(data_dir) {
        return Some(p);
    }
    let dest = data_dir.join("tools").join("yt-dlp.exe");
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
    let res = nova_core::http::request("GET", url, "User-Agent: NovaIsland/1.8\r\n", None, 60_000).ok()?;
    if res.ok() && !res.body.is_empty() && fs::write(&dest, &res.body).is_ok() {
        return Some(dest);
    }
    None
}

fn ensure_https(u: &str) -> String {
    let t = u.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else {
        format!("https://{t}")
    }
}

/// Resolves URL from request, browser window, foreground window, or clipboard.
/// When `passive` is true (background auto-probe), NEVER switches tabs or falls back to clipboard.
pub fn resolve_url(req: &Value, passive: bool) -> Option<String> {
    // 1. Direct url parameter
    if let Some(u) = req["url"].as_str().filter(|s| !s.trim().is_empty()) {
        return Some(ensure_https(u));
    }

    // 2. SMTC app & title match
    let app_id = req["app"].as_str().unwrap_or("");
    if let Some(proc) = crate::mini::browser_proc(app_id) {
        let title = req["title"].as_str().unwrap_or("");
        let short: String = title.chars().take(200).collect();
        let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url_opts(proc, &short, !passive)).unwrap_or_default();
        if let Some(u) = r["url"].as_str().filter(|s| !s.trim().is_empty()) {
            return Some(ensure_https(u));
        }
    }

    if passive {
        return None;
    }

    // 3. Fallback (explicit click only): foreground window if it is a browser
    let (fg_proc, _) = nova_sys::win::foreground_info();
    let fg_proc_l = fg_proc.to_lowercase();
    if ["chrome", "msedge", "edge", "brave", "opera", "vivaldi"].iter().any(|b| fg_proc_l.contains(b)) {
        let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url_opts(&fg_proc, "", false)).unwrap_or_default();
        if let Some(u) = r["url"].as_str().filter(|s| !s.trim().is_empty()) {
            return Some(ensure_https(u));
        }
    }

    // 4. Fallback (explicit click only): clipboard web link
    let clip_text = nova_sys::clip::read_text();
    let trimmed = clip_text.trim();
    if (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
        && trimmed.contains('.')
        && !trimmed.contains(' ')
        && trimmed.len() >= 10
    {
        return Some(trimmed.to_string());
    }

    None
}

/// Probes a video URL and returns available qualities.
/// Deduplicates concurrent probes for the same URL and caches for 8 minutes.
pub fn probe_video(req: &Value, data_dir: &Path) -> Value {
    let passive = req["passive"].as_bool().unwrap_or(false);
    let Some(url) = resolve_url(req, passive) else {
        return json!({ "ok": false, "error": "no-url" });
    };

    // 1. Check in-memory probe cache (8 min TTL)
    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.retain(|_, entry| entry.at.elapsed() < Duration::from_secs(PROBE_TTL_SECS));
        if let Some(entry) = cache.get(&url) {
            return entry.data.clone();
        }
    }

    // 2. Join an in-flight probe for the same URL instead of launching a second yt-dlp
    let (slot, is_owner) = {
        let mut map = IN_FLIGHT_PROBES.lock().unwrap();
        if let Some(existing) = map.get(&url) {
            (existing.clone(), false)
        } else {
            let fresh = Arc::new((Mutex::new(None), Condvar::new()));
            map.insert(url.clone(), fresh.clone());
            (fresh, true)
        }
    };

    if !is_owner {
        let (lock, cv) = &*slot;
        let mut guard = lock.lock().unwrap();
        while guard.is_none() {
            guard = cv.wait(guard).unwrap();
        }
        return guard.as_ref().unwrap().clone();
    }

    let result = run_probe_uncached(&url, req, data_dir);

    // Publish to waiting callers & remove from in-flight map
    {
        let mut map = IN_FLIGHT_PROBES.lock().unwrap();
        map.remove(&url);
    }
    {
        let (lock, cv) = &*slot;
        *lock.lock().unwrap() = Some(result.clone());
        cv.notify_all();
    }

    result
}

fn run_probe_cmd(ytdlp: &Path, url: &str, ff: Option<&Path>, js: Option<&str>) -> Option<Vec<u8>> {
    let mut cmd = Command::new(ytdlp);
    cmd.args([
        "--ignore-config",
        "--no-exec",
        "--no-update",
        "-J",
        "--no-playlist",
        "--no-warnings",
        "--encoding",
        "utf-8",
        "--socket-timeout",
        "8",
        "--no-check-certificates",
        "--extractor-retries",
        "1",
        "--user-agent",
        UA,
        "--referer",
        url,
    ]);
    if url.contains("youtube.com") || url.contains("youtu.be") {
        cmd.args(["--extractor-args", "youtube:player_client=default,-tv"]);
    }
    if let Some(js) = js {
        cmd.args(["--js-runtimes", js]);
    }
    if let Some(ff) = ff {
        cmd.args(["--ffmpeg-location", ff.to_string_lossy().as_ref()]);
    }
    cmd.args(["--", url]);
    cmd.env("PYTHONIOENCODING", "utf-8");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd.output().ok().filter(|o| o.status.success() && !o.stdout.is_empty()).map(|o| o.stdout)
}

fn run_probe_uncached(url: &str, req: &Value, data_dir: &Path) -> Value {
    let ytdlp = match ensure_ytdlp(data_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "no-ytdlp" }),
    };

    let ff = find_ffmpeg(data_dir);
    // Try without overriding --js-runtimes first (~3.9s vs ~7s); retry with explicit JS runtime only if needed
    let raw_out = run_probe_cmd(&ytdlp, url, ff.as_deref(), None).or_else(|| {
        find_js_runtime().and_then(|js| run_probe_cmd(&ytdlp, url, ff.as_deref(), Some(&js)))
    });

    let output = match raw_out {
        Some(out) => out,
        None => {
            if url.contains(".m3u8") || url.contains(".ts") {
                let fallback = json!({
                    "ok": true,
                    "url": url,
                    "title": req["title"].as_str().unwrap_or("Stream Video"),
                    "thumbnail": "",
                    "infoFile": "",
                    "choices": [
                        { "id": "v-hls", "label": "HLS", "tag": "HD", "height": 0, "size": 0, "kind": "video", "format_id": "best", "ext": "mp4" },
                        { "id": "mp3", "label": "MP3", "tag": "320k", "height": 0, "size": 0, "kind": "audio", "format_id": "bestaudio/best", "ext": "mp3" }
                    ]
                });
                if let Ok(mut cache) = PROBE_CACHE.lock() {
                    cache.insert(url.to_string(), ProbeCacheEntry { at: Instant::now(), data: fallback.clone() });
                }
                return fallback;
            }
            return json!({ "ok": false, "error": "probe-failed" });
        }
    };

    // Save raw JSON to temp dir so starting download can use `--load-info-json` without re-probing!
    static INFO_SEQ: AtomicU64 = AtomicU64::new(1);
    let info_file = temp_dir().join(format!("info-{}-{}.json", std::process::id(), INFO_SEQ.fetch_add(1, Ordering::Relaxed)));
    let info_file_str = if fs::write(&info_file, &output).is_ok() {
        info_file.to_string_lossy().into_owned()
    } else {
        String::new()
    };

    let meta: Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => return json!({ "ok": false, "error": "parse-failed" }),
    };

    let title = meta["title"].as_str().unwrap_or_else(|| req["title"].as_str().unwrap_or("Video")).to_string();
    let thumb = meta["thumbnail"].as_str().filter(|s| s.starts_with("http://") || s.starts_with("https://")).unwrap_or("").to_string();
    let duration = meta["duration"].as_f64().unwrap_or(0.0);

    let est_size = |f: &Value| -> u64 {
        if let Some(exact) = f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_u64()) {
            if exact > 0 {
                return exact;
            }
        }
        let rate = f["tbr"].as_f64().unwrap_or_else(|| f["vbr"].as_f64().unwrap_or(0.0) + f["abr"].as_f64().unwrap_or(0.0));
        if duration > 0.0 && rate > 0.0 {
            return (duration * rate * 125.0) as u64;
        }
        0
    };

    let mut best_audio_size = if duration > 0.0 { (duration * 140.0 * 125.0) as u64 } else { 0 };
    if let Some(formats) = meta["formats"].as_array() {
        let mut audio_only: Vec<&Value> = formats
            .iter()
            .filter(|f| {
                let ac = f["acodec"].as_str().unwrap_or("none");
                let vc = f["vcodec"].as_str().unwrap_or("none");
                ac != "none" && (vc == "none" || vc.is_empty())
            })
            .collect();
        audio_only.sort_by(|a, b| {
            let ra = a["abr"].as_f64().or_else(|| a["tbr"].as_f64()).unwrap_or(0.0);
            let rb = b["abr"].as_f64().or_else(|| b["tbr"].as_f64()).unwrap_or(0.0);
            rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(top_audio) = audio_only.first() {
            let sz = est_size(top_audio);
            if sz > 0 {
                best_audio_size = sz;
            }
        }
    }

    struct StreamInfo {
        #[allow(dead_code)]
        height: u64,
        fps: u64,
        size: u64,
        score: f64,
    }
    let mut by_height: HashMap<u64, StreamInfo> = HashMap::new();
    if let Some(formats) = meta["formats"].as_array() {
        for f in formats {
            if f["has_drm"].as_bool() == Some(true) {
                continue;
            }
            let vc = f["vcodec"].as_str().unwrap_or("none");
            if vc == "none" || vc.is_empty() {
                continue;
            }
            let h = f["height"].as_u64().unwrap_or(0);
            if h == 0 {
                continue;
            }
            let has_audio = f["acodec"].as_str().map(|a| a != "none").unwrap_or(false);
            let v_size = est_size(f);
            let total_size = if v_size > 0 {
                if has_audio { v_size } else { v_size + best_audio_size }
            } else {
                0
            };
            let fps = f["fps"].as_u64().unwrap_or(0);
            let tbr = f["tbr"].as_f64().or_else(|| f["vbr"].as_f64()).unwrap_or(0.0);
            let score = (if total_size > 0 { 1e9 } else { 0.0 }) + (fps as f64) * 1e5 + tbr;
            match by_height.get(&h) {
                Some(prev) if prev.score >= score => {}
                _ => {
                    by_height.insert(h, StreamInfo { height: h, fps, size: total_size, score });
                }
            }
        }
    }

    let mut choices = Vec::new();
    let mut heights: Vec<u64> = by_height.keys().cloned().collect();
    heights.sort_by(|a, b| b.cmp(a));

    for h in heights {
        let v = &by_height[&h];
        let label = if h >= 2160 { "4K" } else if h >= 1440 { "2K" } else { &format!("{h}p") };
        let tag = if v.fps > 30 {
            format!("{}fps", v.fps)
        } else if h >= 2160 {
            "2160p".to_string()
        } else if h >= 1440 {
            "1440p".to_string()
        } else if h >= 720 {
            "HD".to_string()
        } else {
            "".to_string()
        };
        choices.push(json!({
            "id": format!("v-{h}"),
            "label": label,
            "tag": tag,
            "height": h,
            "size": v.size,
            "kind": "video",
            "format_id": format!("bv*[height={h}]+ba/b[height={h}]/bv*[height<={h}]+ba/b[height<={h}]/b"),
            "ext": "mp4"
        }));
    }

    if choices.is_empty() {
        let fallback_size = if duration > 0.0 { (duration * 2.5 * 1024.0 * 1024.0 / 60.0).max(1.0) as u64 } else { 0 };
        choices.push(json!({
            "id": "v-best",
            "label": "Best HD",
            "tag": "MP4",
            "height": 0,
            "size": fallback_size,
            "kind": "video",
            "format_id": "bv*+ba/b",
            "ext": "mp4"
        }));
    }

    let mp3_size = if duration > 0.0 { (duration * 320.0 * 125.0) as u64 } else { best_audio_size };
    choices.push(json!({
        "id": "mp3",
        "label": "MP3",
        "tag": "",
        "height": 0,
        "size": mp3_size,
        "kind": "audio",
        "format_id": "ba/b",
        "ext": "mp3"
    }));

    let result = json!({
        "ok": true,
        "url": url,
        "title": title,
        "thumbnail": thumb,
        "infoFile": info_file_str,
        "choices": choices
    });

    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.insert(url.to_string(), ProbeCacheEntry { at: Instant::now(), data: result.clone() });
    }

    result
}

/// Starts downloading the selected choice and creates a tracked `v:N` download item.
pub fn download_video(req: &Value, data_dir: &Path) -> Value {
    let Some(url) = resolve_url(req, false) else {
        return json!({ "ok": false, "error": "no-url" });
    };

    let ytdlp = match ensure_ytdlp(data_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "no-ytdlp" }),
    };

    let choice = &req["choice"];
    let kind = choice["kind"].as_str().unwrap_or("video").to_string();
    let format_id = choice["format_id"].as_str().unwrap_or("bv*+ba/b").to_string();
    let raw_label = choice["label"].as_str().unwrap_or("HD").to_string();
    let raw_tag = choice["tag"].as_str().unwrap_or("").to_string();
    let label = if raw_tag.is_empty() { raw_label.clone() } else { format!("{raw_label} {raw_tag}") };
    let est_total = choice["size"].as_u64().unwrap_or(0);
    let ext = if kind == "audio" { "mp3" } else { "mp4" };

    // Look up cached metadata (thumbnail + infoFile + exact title) for instant card population
    let cached = PROBE_CACHE.lock().ok().and_then(|c| c.get(&url).map(|e| e.data.clone()));
    let base_title = req["titleHint"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| cached.as_ref().and_then(|c| c["title"].as_str()).filter(|s| !s.trim().is_empty()))
        .or_else(|| req["title"].as_str().filter(|s| !s.trim().is_empty()))
        .unwrap_or("Video")
        .to_string();
    let thumbnail = req["thumbnail"]
        .as_str()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .or_else(|| cached.as_ref().and_then(|c| c["thumbnail"].as_str()).filter(|s| s.starts_with("http://") || s.starts_with("https://")))
        .unwrap_or("")
        .to_string();
    let info_file = cached
        .as_ref()
        .and_then(|c| c["infoFile"].as_str())
        .filter(|p| Path::new(p).is_file())
        .unwrap_or("")
        .to_string();

    let id = {
        let mut st = DOWNLOADS.lock().unwrap();
        st.seq += 1;
        let id = format!("v:{}", st.seq);
        st.items.insert(0, MdmItem {
            id: id.clone(),
            url: url.clone(),
            referer: url.clone(),
            base_title: base_title.clone(),
            name: clean_name(&format!("{base_title} [{raw_label}]"), ext),
            path: String::new(),
            thumbnail,
            label: label.clone(),
            tag: raw_label.clone(),
            selector: format_id,
            kind,
            info_file,
            created_at: now_ms(),
            downloaded: 0,
            total: est_total,
            speed: 0.0,
            eta: None,
            status: "downloading".into(),
            stage: "starting".into(),
            error: String::new(),
            finished_at: 0,
            stream_dl: 0,
            stream_total: 0,
            done_bytes: 0,
            canceling: false,
            pausing: false,
            pid: None,
            ytdlp_exe: ytdlp,
            ffmpeg_exe: find_ffmpeg(data_dir),
            run_seq: 0,
        });
        prune_locked(&mut st);
        id
    };

    notify(true);
    spawn_download(id.clone(), true);
    json!({ "ok": true, "id": id, "label": label })
}

fn handle_progress_line(id: &str, run_seq: u64, raw: &str, last_err: &Mutex<String>) {
    let line = raw.trim();
    if line.is_empty() {
        return;
    }

    if let Some(rest) = line.strip_prefix("NOVA_META\t") {
        let cols: Vec<&str> = rest.split('\t').collect();
        let title = cols.first().copied().unwrap_or("");
        let thumb = cols.get(1).copied().unwrap_or("");
        let size = cols.get(5).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let ext_s = cols.get(6).copied().unwrap_or("");
        let mut st = DOWNLOADS.lock().unwrap();
        if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == run_seq) {
            let e = if item.kind == "audio" { "mp3" } else if !ext_s.is_empty() && ext_s != "NA" { ext_s } else { "mp4" };
            if !title.is_empty() && title != "NA" {
                item.base_title = title.to_string();
                let tag = if item.tag.is_empty() { "HD" } else { item.tag.as_str() };
                item.name = clean_name(&format!("{title} [{tag}]"), e);
            }
            if thumb.starts_with("http://") || thumb.starts_with("https://") {
                item.thumbnail = thumb.to_string();
            }
            if size > item.total && item.kind != "audio" {
                item.total = size;
            }
            item.stage.clear();
        }
        drop(st);
        notify(true);
        return;
    }

    if let Some(rest) = line.strip_prefix("NOVA_PROG\t") {
        let cols: Vec<&str> = rest.split('\t').collect();
        let dl = cols.first().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0).max(0.0) as u64;
        let tot = cols.get(1).and_then(|s| s.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .or_else(|| cols.get(2).and_then(|s| s.parse::<u64>().ok()))
            .unwrap_or(0);
        let spd = cols.get(3).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0);
        let eta = cols.get(4).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).map(|v| v.round() as u64);

        let mut pct_event = None;
        {
            let mut st = DOWNLOADS.lock().unwrap();
            if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == run_seq) {
                // Detect stream reset (video -> audio)
                if item.stream_dl > 1024 && (dl as f64) < (item.stream_dl as f64) * 0.3 {
                    item.done_bytes += if item.stream_total > 0 { item.stream_total } else { item.stream_dl };
                    item.stream_dl = 0;
                    item.stream_total = 0;
                }
                if dl > item.stream_dl {
                    item.stream_dl = dl;
                }
                if tot > item.stream_total {
                    item.stream_total = tot;
                }
                item.downloaded = item.done_bytes + item.stream_dl;
                let combined = item.done_bytes + item.stream_total;
                if combined > item.total {
                    item.total = combined;
                }
                if item.total > 0 && item.downloaded > item.total {
                    item.total = item.downloaded;
                }
                item.speed = spd;
                item.eta = eta;
                item.stage.clear();
                if item.status != "processing" && !item.pausing && !item.canceling {
                    item.status = "downloading".into();
                }
                if item.total > 0 {
                    pct_event = Some(((item.downloaded as f64 / item.total as f64) * 100.0).min(100.0));
                }
            }
        }
        notify(false);
        if let Some(p) = pct_event {
            emit_progress(json!({ "id": id, "pct": p }));
        }
        return;
    }

    if line.starts_with("NOVA_POST\t")
        || line.starts_with("[Merger]")
        || line.starts_with("[ExtractAudio]")
        || line.starts_with("[Fixup")
    {
        {
            let mut st = DOWNLOADS.lock().unwrap();
            if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == run_seq) {
                if !item.pausing && !item.canceling {
                    item.status = "processing".into();
                    item.stage.clear();
                    item.speed = 0.0;
                    item.eta = None;
                }
            }
        }
        notify(true);
        return;
    }

    if let Some(file) = line.strip_prefix("NOVA_FILE\t") {
        let f = file.trim();
        if !f.is_empty() {
            let mut st = DOWNLOADS.lock().unwrap();
            if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == run_seq) {
                item.path = f.to_string();
                if let Some(bn) = Path::new(f).file_name() {
                    item.name = bn.to_string_lossy().into_owned();
                }
            }
        }
        return;
    }

    if let Some(err) = line.strip_prefix("ERROR:") {
        let cleaned = err.trim();
        let after_bracket = if cleaned.starts_with('[') {
            cleaned.find("]: ").map(|i| cleaned[i + 3..].trim()).unwrap_or(cleaned)
        } else {
            cleaned
        };
        let short: String = after_bracket.chars().take(120).collect();
        *last_err.lock().unwrap() = short;
    }
}

fn spawn_download(id: String, use_preloaded: bool) {
    let snap = {
        let mut st = DOWNLOADS.lock().unwrap();
        st.run_seq += 1;
        let rseq = st.run_seq;
        let Some(item) = st.items.iter_mut().find(|d| d.id == id) else {
            return;
        };
        item.run_seq = rseq;
        item.status = "downloading".into();
        item.stage = "starting".into();
        item.error.clear();
        item.speed = 0.0;
        item.eta = None;
        item.pausing = false;
        item.canceling = false;
        item.stream_dl = 0;
        item.stream_total = 0;
        item.done_bytes = 0;
        item.clone()
    };

    std::thread::spawn(move || {
        let out_dir = PathBuf::from(nova_services::downloads::downloads_folder());
        let tmp_dir = temp_dir();
        let _ = fs::create_dir_all(&out_dir);

        let is_audio = snap.kind == "audio";
        let has_ffmpeg = snap.ffmpeg_exe.is_some();
        let format_sel = if is_audio {
            "ba/b".to_string()
        } else if has_ffmpeg {
            snap.selector.clone()
        } else {
            "b".to_string()
        };
        let has_info = use_preloaded && !snap.info_file.is_empty() && Path::new(&snap.info_file).is_file();
        let file_tag = if snap.tag.is_empty() { "HD" } else { snap.tag.as_str() };

        let mut args = vec![
            "--ignore-config".into(),
            "--no-exec".into(),
            "--no-update".into(),
            "--no-playlist".into(),
            "--no-simulate".into(),
            "--continue".into(),
            "--windows-filenames".into(),
            "--progress".into(),
            "--newline".into(),
            "--encoding".into(),
            "utf-8".into(),
            "--user-agent".into(),
            UA.into(),
            "--referer".into(),
            snap.referer.clone(),
            "-N".into(),
            "8".into(),
            "--hls-prefer-native".into(),
            "-f".into(),
            format_sel,
        ];
        if let Some(js) = find_js_runtime() {
            args.push("--js-runtimes".into());
            args.push(js);
        }
        if let Some(ff) = &snap.ffmpeg_exe {
            args.push("--ffmpeg-location".into());
            args.push(ff.to_string_lossy().into_owned());
        }
        if is_audio {
            if has_ffmpeg {
                args.extend(["-x", "--audio-format", "mp3", "--audio-quality", "0"].map(String::from));
            }
        } else if has_ffmpeg {
            args.extend(["--merge-output-format", "mp4/mkv"].map(String::from));
        }
        args.extend([
            "-P".into(),
            format!("home:{}", out_dir.to_string_lossy()),
            "-P".into(),
            format!("temp:{}", tmp_dir.to_string_lossy()),
            "-o".into(),
            format!("%(title).150B [{file_tag}].%(ext)s"),
            "--print".into(),
            "before_dl:NOVA_META\t%(title)s\t%(thumbnail)s\t%(resolution)s\t%(height)s\t%(fps)s\t%(filesize,filesize_approx)s\t%(ext)s".into(),
            "--print".into(),
            "after_move:NOVA_FILE\t%(filepath)s".into(),
            "--progress-template".into(),
            "download:NOVA_PROG\t%(progress.downloaded_bytes)s\t%(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t%(progress.speed)s\t%(progress.eta)s".into(),
            "--progress-template".into(),
            "postprocess:NOVA_POST\t%(progress.status)s".into(),
        ]);
        if has_info {
            args.push("--load-info-json".into());
            args.push(snap.info_file.clone());
        } else {
            args.push("--".into());
            args.push(snap.url.clone());
        }

        let mut cmd = Command::new(&snap.ytdlp_exe);
        cmd.args(&args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.env("PYTHONIOENCODING", "utf-8");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let mut st = DOWNLOADS.lock().unwrap();
                if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == snap.run_seq) {
                    item.status = "error".into();
                    item.stage.clear();
                    item.error = e.to_string();
                }
                drop(st);
                notify(true);
                return;
            }
        };

        let pid = child.id();
        {
            let mut st = DOWNLOADS.lock().unwrap();
            if let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == snap.run_seq) {
                if item.canceling || item.pausing {
                    if item.pausing {
                        item.status = "paused".into();
                        item.stage.clear();
                        item.speed = 0.0;
                        item.eta = None;
                    }
                    drop(st);
                    kill_pid_tree(pid);
                    notify(true);
                    return;
                }
                item.pid = Some(pid);
            } else {
                // Cancelled before spawn completed
                drop(st);
                kill_pid_tree(pid);
                return;
            }
        }

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let last_err = Arc::new(Mutex::new(String::new()));

        let err_reader = stderr_pipe.map(|pipe| {
            let id_c = id.clone();
            let err_c = last_err.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(pipe);
                for line in reader.lines().map_while(Result::ok) {
                    handle_progress_line(&id_c, snap.run_seq, &line, &err_c);
                }
            })
        });

        if let Some(pipe) = stdout_pipe {
            let reader = BufReader::new(pipe);
            for line in reader.lines().map_while(Result::ok) {
                handle_progress_line(&id, snap.run_seq, &line, &last_err);
            }
        }
        if let Some(t) = err_reader {
            let _ = t.join();
        }

        let success = child.wait().map(|s| s.success()).unwrap_or(false);

        let mut retry_direct = false;
        let mut completed = false;
        {
            let mut st = DOWNLOADS.lock().unwrap();
            let Some(item) = st.items.iter_mut().find(|d| d.id == id && d.run_seq == snap.run_seq) else {
                // Removed by cancel
                return;
            };
            item.pid = None;
            if item.canceling {
                return;
            }
            if item.pausing {
                item.status = "paused".into();
                item.stage.clear();
                item.speed = 0.0;
                item.eta = None;
            } else if !success && has_info {
                // Expired info JSON — retry directly with URL once
                item.info_file.clear();
                retry_direct = true;
            } else if success {
                if !item.path.is_empty() {
                    if let Ok(meta) = fs::metadata(&item.path) {
                        if meta.len() > 0 {
                            item.downloaded = meta.len();
                            item.total = meta.len();
                        }
                    }
                }
                if item.total == 0 && item.downloaded > 0 {
                    item.total = item.downloaded;
                } else if item.total > 0 && item.downloaded > 0 {
                    item.downloaded = item.total;
                }
                item.status = "completed".into();
                item.stage.clear();
                item.speed = 0.0;
                item.eta = None;
                item.finished_at = now_ms();
                completed = true;
            } else {
                item.status = "error".into();
                item.stage.clear();
                item.speed = 0.0;
                item.eta = None;
                let msg = last_err.lock().unwrap().clone();
                item.error = if msg.is_empty() { "Could not download this video".into() } else { msg };
            }
        }

        if retry_direct {
            spawn_download(id, false);
            return;
        }
        notify(true);
        if completed {
            emit_progress(json!({ "id": id, "done": true }));
        }
    });
}
