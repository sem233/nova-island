//! MDM — video and high-quality MP3 stream downloader for Nova Island.
//! Uses yt-dlp to probe video qualities (1080p, 720p, 480p, MP3 320k) and download directly into the Downloads folder.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use serde_json::{json, Value};

pub fn find_ytdlp(data_dir: &Path) -> Option<PathBuf> {
    // 1. in data_dir/tools
    let local = data_dir.join("tools").join("yt-dlp.exe");
    if local.is_file() {
        return Some(local);
    }

    // 2. in WinGet links
    if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(&local_app).join("Microsoft").join("WinGet").join("Links").join("yt-dlp.exe");
        if p.is_file() {
            return Some(p);
        }
    }

    // 3. in PATH
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

/// Resolves URL from request or browser window via UI Automation.
pub fn resolve_url(req: &Value) -> Option<String> {
    if let Some(u) = req["url"].as_str().filter(|s| s.starts_with("http://") || s.starts_with("https://")) {
        return Some(u.trim().to_string());
    }

    let app_id = req["app"].as_str().unwrap_or("");
    let proc = crate::mini::browser_proc(app_id)?;
    let title = req["title"].as_str().unwrap_or("");
    let short: String = title.chars().take(200).collect();
    let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url(proc, &short)).unwrap_or_default();
    r["url"].as_str().filter(|s| s.starts_with("http://") || s.starts_with("https://")).map(|s| s.to_string())
}

/// Probes a video URL and returns available qualities.
pub fn probe_video(req: &Value, data_dir: &Path) -> Value {
    let Some(url) = resolve_url(req) else {
        return json!({ "ok": false, "error": "no-url" });
    };

    let ytdlp = match ensure_ytdlp(data_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "no-ytdlp" }),
    };

    // Run yt-dlp -J --no-playlist
    let mut cmd = Command::new(&ytdlp);
    cmd.args(["-J", "--no-playlist", "--socket-timeout", "15", &url]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = match cmd.output() {
        Ok(out) if out.status.success() => out.stdout,
        _ => return json!({ "ok": false, "error": "probe-failed" }),
    };

    let meta: Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => return json!({ "ok": false, "error": "parse-failed" }),
    };

    let title = meta["title"].as_str().unwrap_or("Video").to_string();
    let duration = meta["duration"].as_f64().unwrap_or(0.0);

    // Build format choices
    let mut choices = Vec::new();

    // Check what heights are available
    let mut has_1080 = false;
    let mut has_720 = false;
    let mut has_480 = false;

    if let Some(formats) = meta["formats"].as_array() {
        for f in formats {
            if let Some(h) = f["height"].as_u64() {
                if h >= 1080 { has_1080 = true; }
                if h >= 720 { has_720 = true; }
                if h >= 480 { has_480 = true; }
            }
        }
    }

    // Estimate file sizes from duration if available (~3MB/min for 1080p, ~1.8MB/min for 720p, ~0.8MB/min for 480p, ~2.4MB/min for 320k)
    let dur_min = (duration / 60.0).max(1.0);

    if has_1080 {
        choices.push(json!({
            "label": "1080p FHD",
            "kind": "video",
            "size": (dur_min * 2.8 * 1024.0 * 1024.0) as u64,
            "note": "1080p · MP4",
            "tag": "FHD",
            "format_id": "bestvideo[height<=1080]+bestaudio/best[height<=1080]/best",
            "ext": "mp4"
        }));
    }

    if has_720 || !has_1080 {
        choices.push(json!({
            "label": "720p HD",
            "kind": "video",
            "size": (dur_min * 1.6 * 1024.0 * 1024.0) as u64,
            "note": "720p · MP4",
            "tag": "HD",
            "format_id": "bestvideo[height<=720]+bestaudio/best[height<=720]/best",
            "ext": "mp4"
        }));
    }

    if has_480 {
        choices.push(json!({
            "label": "480p SD",
            "kind": "video",
            "size": (dur_min * 0.9 * 1024.0 * 1024.0) as u64,
            "note": "480p · MP4",
            "tag": "480p",
            "format_id": "bestvideo[height<=480]+bestaudio/best[height<=480]/best",
            "ext": "mp4"
        }));
    }

    // High Quality MP3 320k option
    choices.push(json!({
        "label": "MP3 Audio (320k)",
        "kind": "audio",
        "size": (dur_min * 2.4 * 1024.0 * 1024.0) as u64,
        "note": "320 kbps · MP3",
        "tag": "320k",
        "format_id": "bestaudio/best",
        "ext": "mp3"
    }));

    json!({
        "ok": true,
        "url": url,
        "title": title,
        "choices": choices
    })
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
    let template = downloads_dir.join("%(title)s.%(ext)s").to_string_lossy().into_owned();

    let mut args = Vec::new();
    args.push("--no-playlist".to_string());
    args.push("-o".to_string());
    args.push(template);

    if kind == "audio" {
        args.push("-x".to_string());
        args.push("--audio-format".to_string());
        args.push("mp3".to_string());
        args.push("--audio-quality".to_string());
        args.push("0".to_string());
        args.push("--embed-thumbnail".to_string());
        args.push("--add-metadata".to_string());
    } else {
        args.push("-f".to_string());
        args.push(format_id.to_string());
        args.push("--merge-output-format".to_string());
        args.push("mp4".to_string());
    }

    args.push(url);

    // Spawn yt-dlp in the background so it doesn't block
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
