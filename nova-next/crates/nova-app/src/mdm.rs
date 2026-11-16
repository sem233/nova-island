//! MDM — video and high-quality MP3 stream downloader for Nova Island.
//! Derived from the user's custom mdm.js engine:
//! - In-memory probe cache (8 min TTL) so repeated probes and clicks return in 0ms
//! - Full format parsing for all video heights (4K, 2K, 1080p, 720p, 480p, 360p) + MP3 (320k)
//! - Accurate file size calculation via bitrates * duration or exact stream metadata
//! - High-speed parallel chunk downloading (-N 8, --hls-prefer-native)
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{Duration, Instant},
};
use serde_json::{json, Value};

struct ProbeCacheEntry {
    at: Instant,
    data: Value,
}

static PROBE_CACHE: std::sync::LazyLock<Mutex<HashMap<String, ProbeCacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn find_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    // 1. Beside running exe or in exe's tools subfolder
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

    // 2. in data_dir/tools
    let local = data_dir.join("tools").join("yt-dlp.exe");
    if local.is_file() {
        return Some(local);
    }

    // 3. in WinGet links
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local_app).join("Microsoft").join("WinGet").join("Links").join("yt-dlp.exe");
        if p.is_file() {
            return Some(p);
        }
    }

    // 4. in PATH
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
    // 1. Beside running exe or in tools
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

    // 2. In data_dir/tools
    let local = data_dir.join("tools").join("ffmpeg.exe");
    if local.is_file() {
        return Some(local);
    }

    // 3. In WinGet packages or Links
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

    // 4. in PATH
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

pub fn ensure_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    if let Some(p) = find_ytdlp(data_dir) {
        return Some(p);
    }

    let dest = data_dir.join("tools").join("yt-dlp.exe");
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Download standalone yt-dlp from GitHub releases
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
    let res = nova_core::http::request("GET", url, "User-Agent: NovaIsland/1.8\r\n", None, 60_000).ok()?;
    if res.ok() && !res.body.is_empty() {
        if fs::write(&dest, &res.body).is_ok() {
            return Some(dest);
        }
    }

    None
}

/// Resolves URL from request, browser window, foreground window, or clipboard.
pub fn resolve_url(req: &Value) -> Option<String> {
    // 1. Direct url parameter
    if let Some(u) = req["url"].as_str().filter(|s| !s.trim().is_empty()) {
        let trimmed = u.trim();
        let full = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        };
        return Some(full);
    }

    // 2. SMTC app & title match
    let app_id = req["app"].as_str().unwrap_or("");
    if let Some(proc) = crate::mini::browser_proc(app_id) {
        let title = req["title"].as_str().unwrap_or("");
        let short: String = title.chars().take(200).collect();
        let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url(proc, &short)).unwrap_or_default();
        if let Some(u) = r["url"].as_str().filter(|s| !s.trim().is_empty()) {
            let trimmed = u.trim();
            let full = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                trimmed.to_string()
            } else {
                format!("https://{trimmed}")
            };
            return Some(full);
        }
    }

    // 3. Fallback: foreground window if it is a browser
    let (fg_proc, _) = nova_sys::win::foreground_info();
    let fg_proc_l = fg_proc.to_lowercase();
    if ["chrome", "msedge", "edge", "brave", "opera", "vivaldi"].iter().any(|b| fg_proc_l.contains(b)) {
        let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url(&fg_proc, "")).unwrap_or_default();
        if let Some(u) = r["url"].as_str().filter(|s| !s.trim().is_empty()) {
            let trimmed = u.trim();
            let full = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                trimmed.to_string()
            } else {
                format!("https://{trimmed}")
            };
            return Some(full);
        }
    }

    // 4. Fallback: check clipboard for any web link
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
/// Derived from mdm.js buildChoicesFromInfo with fast in-memory caching.
pub fn probe_video(req: &Value, data_dir: &Path) -> Value {
    let Some(url) = resolve_url(req) else {
        return json!({ "ok": false, "error": "no-url" });
    };

    // 1. Check in-memory probe cache (8 min TTL)
    {
        if let Ok(mut cache) = PROBE_CACHE.lock() {
            cache.retain(|_, entry| entry.at.elapsed() < Duration::from_secs(8 * 60));
            if let Some(entry) = cache.get(&url) {
                return entry.data.clone();
            }
        }
    }

    let ytdlp = match ensure_ytdlp(data_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "no-ytdlp" }),
    };

    // Run yt-dlp -J with speed-optimized flags from mdm.js
    let mut cmd = Command::new(&ytdlp);
    cmd.args([
        "--ignore-config",
        "--no-exec",
        "--no-update",
        "-J",
        "--no-playlist",
        "--no-warnings",
        "--socket-timeout",
        "15",
    ]);
    if let Some(ff) = find_ffmpeg(data_dir) {
        cmd.arg("--ffmpeg-location");
        cmd.arg(ff.to_string_lossy().as_ref());
    }
    cmd.arg(&url);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = match cmd.output() {
        Ok(out) if out.status.success() => out.stdout,
        _ => {
            // Direct .ts / .m3u8 fallback if yt-dlp failed
            if url.contains(".m3u8") || url.contains(".ts") {
                let fallback = json!({
                    "ok": true,
                    "url": url,
                    "title": req["title"].as_str().unwrap_or("Stream Video"),
                    "choices": [
                        { "id": "v-hls", "label": "HLS Stream", "tag": "HD", "height": 0, "size": 0, "kind": "video", "format_id": "best", "ext": "mp4" },
                        { "id": "mp3", "label": "MP3 Audio", "tag": "320k", "height": 0, "size": 0, "kind": "audio", "format_id": "bestaudio/best", "ext": "mp3" }
                    ]
                });
                if let Ok(mut cache) = PROBE_CACHE.lock() {
                    cache.insert(url.clone(), ProbeCacheEntry { at: Instant::now(), data: fallback.clone() });
                }
                return fallback;
            }
            return json!({ "ok": false, "error": "probe-failed" });
        }
    };

    let meta: Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => return json!({ "ok": false, "error": "parse-failed" }),
    };

    let title = meta["title"].as_str().unwrap_or("Video").to_string();
    let duration = meta["duration"].as_f64().unwrap_or(0.0);

    // Size estimator from mdm.js
    let est_size = |f: &Value| -> u64 {
        if let Some(exact) = f["filesize"].as_u64().or_else(|| f["filesize_approx"].as_u64()) {
            if exact > 0 {
                return exact;
            }
        }
        let rate = f["tbr"].as_f64().unwrap_or_else(|| {
            f["vbr"].as_f64().unwrap_or(0.0) + f["abr"].as_f64().unwrap_or(0.0)
        });
        if duration > 0.0 && rate > 0.0 {
            return (duration * rate * 125.0) as u64;
        }
        0
    };

    // Find best audio stream size (for DASH merging)
    let mut best_audio_size = if duration > 0.0 { (duration * 140.0 * 125.0) as u64 } else { 0 };
    if let Some(formats) = meta["formats"].as_array() {
        let mut audio_only: Vec<&Value> = formats.iter().filter(|f| {
            let ac = f["acodec"].as_str().unwrap_or("none");
            let vc = f["vcodec"].as_str().unwrap_or("none");
            ac != "none" && (vc == "none" || vc.is_empty())
        }).collect();
        audio_only.sort_by(|a, b| {
            let ra = a["abr"].as_f64().or_else(|| a["tbr"].as_f64()).unwrap_or(0.0);
            let rb = b["abr"].as_f64().or_else(|| b["tbr"].as_f64()).unwrap_or(0.0);
            rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(top_audio) = audio_only.first() {
            let sz = est_size(top_audio);
            if sz > 0 { best_audio_size = sz; }
        }
    }

    // Group video streams by height
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
            if f["has_drm"].as_bool() == Some(true) { continue; }
            let vc = f["vcodec"].as_str().unwrap_or("none");
            if vc == "none" || vc.is_empty() { continue; }
            let h = f["height"].as_u64().unwrap_or(0);
            if h == 0 { continue; }
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
    heights.sort_by(|a, b| b.cmp(a)); // Descending order: 2160, 1440, 1080, 720, 480...

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
            "format_id": format!("bestvideo[height={h}]+bestaudio/best[height={h}]/bestvideo[height<={h}]+bestaudio/best[height<={h}]/best"),
            "ext": "mp4"
        }));
    }

    // Generic fallback if no specific heights detected
    if choices.is_empty() {
        let fallback_size = if duration > 0.0 { (duration * 2.5 * 1024.0 * 1024.0 / 60.0).max(1.0) as u64 } else { 0 };
        choices.push(json!({
            "id": "v-best",
            "label": "Best Video",
            "tag": "HD",
            "height": 0,
            "size": fallback_size,
            "kind": "video",
            "format_id": "bestvideo+bestaudio/best",
            "ext": "mp4"
        }));
    }

    // Add MP3 (320 kbps)
    let mp3_size = if duration > 0.0 { (duration * 320.0 * 125.0) as u64 } else { best_audio_size };
    choices.push(json!({
        "id": "mp3",
        "label": "MP3 Audio",
        "tag": "320k",
        "height": 0,
        "size": mp3_size,
        "kind": "audio",
        "format_id": "bestaudio/best",
        "ext": "mp3"
    }));

    let result = json!({
        "ok": true,
        "url": url,
        "title": title,
        "choices": choices
    });

    // Save in cache
    if let Ok(mut cache) = PROBE_CACHE.lock() {
        cache.insert(url, ProbeCacheEntry { at: Instant::now(), data: result.clone() });
    }

    result
}

/// Starts downloading the selected choice in the background.
pub fn download_video(req: &Value, data_dir: &Path) -> Value {
    let Some(url) = resolve_url(req) else {
        return json!({ "ok": false, "error": "no-url" });
    };

    let ytdlp = match ensure_ytdlp(data_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "no-ytdlp" }),
    };

    let choice = &req["choice"];
    let kind = choice["kind"].as_str().unwrap_or("video");
    let format_id = choice["format_id"].as_str().unwrap_or("best");
    let label = choice["label"].as_str().unwrap_or("video").to_string();

    let downloads_dir = PathBuf::from(nova_services::downloads::downloads_folder());
    let template = downloads_dir.join("%(title).150B [%(resolution)s].%(ext)s").to_string_lossy().into_owned();

    let mut args = Vec::new();
    args.push("--ignore-config".to_string());
    args.push("--no-exec".to_string());
    args.push("--no-update".to_string());
    args.push("--no-playlist".to_string());
    args.push("--continue".to_string());
    args.push("--windows-filenames".to_string());
    args.push("-N".to_string());
    args.push("8".to_string());
    args.push("--hls-prefer-native".to_string());
    args.push("-o".to_string());
    args.push(template);
    args.push("--user-agent".to_string());
    args.push("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string());
    args.push("--referer".to_string());
    args.push(url.clone());

    let ffmpeg = find_ffmpeg(data_dir);
    if let Some(ff) = &ffmpeg {
        args.push("--ffmpeg-location".to_string());
        args.push(ff.to_string_lossy().into_owned());
    }

    if kind == "audio" {
        if ffmpeg.is_some() {
            args.push("-x".to_string());
            args.push("--audio-format".to_string());
            args.push("mp3".to_string());
            args.push("--audio-quality".to_string());
            args.push("0".to_string());
        } else {
            args.push("-f".to_string());
            args.push("bestaudio[ext=m4a]/bestaudio/best".to_string());
        }
    } else {
        args.push("-f".to_string());
        args.push(format!("{format_id}/best"));
        if ffmpeg.is_some() {
            args.push("--merge-output-format".to_string());
            args.push("mp4/mkv".to_string());
        }
    }

    args.push(url);

    // Spawn yt-dlp in the background so it doesn't block the UI thread
    std::thread::spawn(move || {
        let mut cmd = Command::new(&ytdlp);
        cmd.args(&args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let _ = cmd.status();
    });

    json!({ "ok": true, "label": label })
}
