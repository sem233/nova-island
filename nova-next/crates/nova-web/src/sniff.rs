//! Hidden WebView2 stream sniffer for JavaScript-rendered video pages (e.g. SPAs / HLS embeds).
//! Only extracts public playable streams when direct `yt-dlp` extraction fails on an HTTP(S) page;
//! keeps TLS verification intact and stops immediately on DRM, login walls, or CAPTCHA challenges.
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

use serde_json::Value;
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{w, Interface, HSTRING, PWSTR},
    Win32::{
        Foundation::*,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, TranslateMessage,
            MSG, PM_REMOVE, WINDOW_EX_STYLE, WS_POPUP,
        },
    },
};

use crate::environment;

static SNIFF_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SniffBlock {
    TlsError,
    AuthRequired,
    BotChallenge,
    DrmProtected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SniffResult {
    pub stream_url: Option<String>,
    pub page_title: Option<String>,
    pub blocked: Option<SniffBlock>,
    pub nav_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub url: String,
    pub content_type: String,
    pub status: Option<i32>,
    pub seq: usize,
}

/// Checks whether a URL looks like a streaming manifest or direct video stream.
pub fn is_stream_candidate_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return false;
    }
    if is_ignored_asset_or_ad(&lower) {
        return false;
    }
    let path_part = lower.split(['?', '#']).next().unwrap_or(&lower);
    path_part.ends_with(".m3u8")
        || path_part.contains(".m3u8/")
        || lower.contains(".m3u8?")
        || path_part.ends_with(".mpd")
        || path_part.contains(".mpd/")
        || lower.contains(".mpd?")
        || path_part.contains(".ism/manifest")
        || path_part.ends_with(".mp4")
}

/// Checks whether a response Content-Type header represents an HLS or DASH manifest or video stream.
pub fn is_stream_content_type(ct: &str) -> bool {
    let l = ct.to_ascii_lowercase();
    l.contains("mpegurl")
        || l.contains("dash+xml")
        || l.starts_with("video/mp4")
        || l.starts_with("video/webm")
}

fn is_ignored_asset_or_ad(lower_url: &str) -> bool {
    const AD_HOSTS: &[&str] = &[
        "doubleclick.net",
        "googlesyndication.com",
        "google-analytics.com",
        "googletagmanager.com",
        "facebook.com/tr",
        "clarity.ms",
        "mc.yandex.ru",
    ];
    if AD_HOSTS.iter().any(|h| lower_url.contains(h)) {
        return true;
    }
    let path = lower_url.split(['?', '#']).next().unwrap_or(lower_url);
    const STATIC_EXTS: &[&str] = &[
        ".js", ".mjs", ".css", ".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg", ".ico",
        ".woff", ".woff2", ".ttf", ".vtt", ".srt", ".ts",
    ];
    if STATIC_EXTS.iter().any(|ext| path.ends_with(ext)) {
        return true;
    }
    path.contains("favicon")
        || path.contains("/ads/")
        || path.contains("/preroll")
        || lower_url.contains("analytics")
}

/// Inspects headers/URL/body hints to detect anti-bot, auth, or DRM protection without bypassing.
pub fn detect_block_hint(status: i32, ct: &str, url: &str) -> Option<SniffBlock> {
    let u = url.to_ascii_lowercase();
    let c = ct.to_ascii_lowercase();
    if u.contains("widevine")
        || u.contains("playready")
        || u.contains("fairplay")
        || u.contains("drm") && (u.contains("license") || u.contains("key"))
    {
        return Some(SniffBlock::DrmProtected);
    }
    if u.contains("challenges.cloudflare.com")
        || u.contains("turnstile")
        || u.contains("hcaptcha.com")
        || u.contains("recaptcha")
    {
        return Some(SniffBlock::BotChallenge);
    }
    if c.contains("text/html") && (status == 401 || status == 403 || status == 429) {
        if status == 401 {
            return Some(SniffBlock::AuthRequired);
        }
        return Some(SniffBlock::BotChallenge);
    }
    None
}

fn candidate_score(c: &Candidate) -> i32 {
    let l = c.url.to_ascii_lowercase();
    if is_ignored_asset_or_ad(&l) {
        return i32::MIN;
    }
    if let Some(st) = c.status {
        if st >= 400 || st < 200 {
            return i32::MIN;
        }
    }
    let path = l.split(['?', '#']).next().unwrap_or(&l);
    let ct = c.content_type.to_ascii_lowercase();
    let is_hls = ct.contains("mpegurl") || path.contains(".m3u8") || l.contains(".m3u8?");
    let is_dash = ct.contains("dash+xml") || path.contains(".mpd") || l.contains(".mpd?") || path.contains(".ism/manifest");
    let is_mp4 = ct.starts_with("video/mp4") || ct.starts_with("video/webm") || path.ends_with(".mp4");

    let mut score = if is_hls {
        1000
    } else if is_dash {
        900
    } else if is_mp4 {
        300
    } else {
        return i32::MIN;
    };

    if c.status.is_some_and(|s| (200..300).contains(&s)) {
        score += 300;
    }
    if is_hls || is_dash {
        if path.contains("master") || path.contains("playlist") || path.contains("manifest") {
            score += 250;
        }
        if path.contains("index-v")
            || path.contains("audio-")
            || path.contains("360p")
            || path.contains("480p")
            || path.contains("720p")
            || path.contains("1080p")
            || path.contains("chunk")
        {
            score -= 120;
        }
    } else if path.contains("preview") || path.contains("trailer") || path.contains("thumb") {
        score -= 150;
    }
    score
}

/// Picks the best stream manifest/media URL from observed network traffic.
pub fn pick_best_manifest(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .filter_map(|c| {
            let s = candidate_score(c);
            (s > i32::MIN / 2).then_some((s, c))
        })
        .max_by(|(sa, ca), (sb, cb)| sa.cmp(sb).then_with(|| cb.seq.cmp(&ca.seq)))
        .map(|(_, c)| c)
}

fn is_tls_web_error(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> bool {
    matches!(
        status,
        COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_COMMON_NAME_IS_INCORRECT
            | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_EXPIRED
            | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_IS_INVALID
            | COREWEBVIEW2_WEB_ERROR_STATUS_CERTIFICATE_REVOKED
            | COREWEBVIEW2_WEB_ERROR_STATUS_CLIENT_CERTIFICATE_CONTAINS_ERRORS
    )
}

#[derive(Default)]
struct SniffState {
    candidates: Vec<Candidate>,
    next_seq: usize,
    nav_done: bool,
    nav_failed: bool,
    blocked: Option<SniffBlock>,
    page_title: Option<String>,
    first_manifest_at: Option<Instant>,
}

impl SniffState {
    fn record_url(&mut self, url: String, content_type: String, status: Option<i32>) {
        if let Some(existing) = self.candidates.iter_mut().find(|c| c.url == url) {
            if !content_type.is_empty() {
                existing.content_type = content_type;
            }
            if status.is_some() {
                existing.status = status;
            }
            if self.first_manifest_at.is_none() && candidate_score(existing) >= 900 {
                self.first_manifest_at = Some(Instant::now());
            }
            return;
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let cand = Candidate { url, content_type, status, seq };
        if candidate_score(&cand) > i32::MIN / 2 {
            if self.first_manifest_at.is_none() && candidate_score(&cand) >= 900 {
                self.first_manifest_at = Some(Instant::now());
            }
            self.candidates.push(cand);
        }
    }

    fn has_verified_master(&self) -> bool {
        self.candidates.iter().any(|c| candidate_score(c) >= 1400)
    }
}

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn inspect_dom_and_title(webview: &ICoreWebView2, state: &Rc<RefCell<SniffState>>) {
    const JS: &str = r#"(() => {
  const title = (document.title || '').trim();
  const body = (document.body && document.body.innerText ? document.body.innerText.slice(0, 2000) : '').toLowerCase();
  const html = document.documentElement ? document.documentElement.innerHTML.slice(0, 4000).toLowerCase() : '';
  const hasCaptcha = !!(document.querySelector('.cf-turnstile, .g-recaptcha, .h-captcha, iframe[src*="challenges.cloudflare.com"], iframe[src*="recaptcha"], iframe[src*="hcaptcha"]')
    || /just a moment|verify you are human|checking your browser before accessing/i.test(title + ' ' + body));
  const hasAuth = !!(document.querySelector('input[type="password"]') && !document.querySelector('video')
    || /sign in to watch|members only|subscription required|login required/i.test(body));
  const hasDrm = /widevine|playready|fairplay|encrypted media/i.test(html) && !document.querySelector('video[src]');
  let clicked = false;
  if (!hasCaptcha && !hasAuth) {
    const sels = ['video', '.vjs-big-play-button', '.plyr__control--overlaid', '.jw-display-icon-container', 'button[aria-label*="play" i]', '[class*="play-btn"]', '[class*="playBtn"]'];
    for (const s of sels) {
      const el = document.querySelector(s);
      if (el) {
        try { if (typeof el.play === 'function') el.play().catch(() => {}); } catch {}
        try { el.click(); clicked = true; break; } catch {}
      }
    }
  }
  return { title, hasCaptcha, hasAuth, hasDrm, clicked };
})()"#;
    let wv = webview.clone();
    let st = state.clone();
    let js = HSTRING::from(JS);
    unsafe {
        let _ = wv.ExecuteScript(
            &js,
            &ExecuteScriptCompletedHandler::create(Box::new(move |code, result| {
                if code.is_ok() {
                    if let Ok(v) = serde_json::from_str::<Value>(&result) {
                        let mut s = st.borrow_mut();
                        if let Some(t) = v["title"].as_str().map(str::trim).filter(|t| !t.is_empty()) {
                            s.page_title = Some(t.to_string());
                        }
                        if s.blocked.is_none() && s.candidates.is_empty() {
                            if v["hasCaptcha"].as_bool() == Some(true) {
                                s.blocked = Some(SniffBlock::BotChallenge);
                            } else if v["hasAuth"].as_bool() == Some(true) {
                                s.blocked = Some(SniffBlock::AuthRequired);
                            } else if v["hasDrm"].as_bool() == Some(true) {
                                s.blocked = Some(SniffBlock::DrmProtected);
                            }
                        }
                    }
                }
                Ok(())
            })),
        );
    }
}

/// Loads `url` in a hidden, audio-muted WebView2 window and listens for HLS/DASH streams.
/// Safe to call from any thread (spawns its own STA COM thread). TLS verification stays active.
pub fn sniff_stream(url: &str, data_dir: &str, timeout: Duration) -> SniffResult {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return SniffResult::default();
    }
    let _guard = SNIFF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let url = url.to_string();
    let data_dir = data_dir.to_string();
    let (tx, rx) = mpsc::channel();
    let Ok(handle) = std::thread::Builder::new().name("nova-sniff".into()).spawn(move || {
        let res = sniff_on_sta(&url, &data_dir, timeout);
        let _ = tx.send(res);
    }) else {
        return SniffResult::default();
    };
    let res = rx.recv().unwrap_or_default();
    let _ = handle.join();
    res
}

fn sniff_on_sta(url: &str, data_dir: &str, timeout: Duration) -> SniffResult {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    struct ComDrop;
    impl Drop for ComDrop {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _com = ComDrop;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("NovaSniff"),
            WS_POPUP,
            -32000,
            -32000,
            1280,
            720,
            None,
            None,
            None,
            None,
        )
    };
    let Ok(hwnd) = hwnd else {
        return SniffResult::default();
    };
    struct HwndDrop(HWND);
    impl Drop for HwndDrop {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    let _hwnd_drop = HwndDrop(hwnd);

    let Ok(env) = environment(data_dir, "--mute-audio") else {
        return SniffResult::default();
    };
    let (tx, rx) = mpsc::channel();
    let env2 = env.clone();
    let create_res = CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            env2.CreateCoreWebView2Controller(hwnd, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |code, ctrl| {
            code?;
            tx.send(ctrl.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                .ok();
            Ok(())
        }),
    );
    let Ok(controller) = (create_res.is_ok()).then(|| rx.recv().ok()).flatten().unwrap_or(Err(windows::core::Error::from(E_POINTER))) else {
        return SniffResult::default();
    };
    let Ok(webview) = (unsafe { controller.CoreWebView2() }) else {
        unsafe {
            let _ = controller.Close();
        }
        return SniffResult::default();
    };

    unsafe {
        let _ = controller.SetBounds(RECT { left: 0, top: 0, right: 1280, bottom: 720 });
        // Keep WebView visibility active on the unshown offscreen HWND so lazy JS/media players
        // do not throttle or suspend stream initialization under PageVisibilityState=hidden.
        let _ = controller.SetIsVisible(true);
        if let Ok(settings) = webview.Settings() {
            let _ = settings.SetAreDefaultContextMenusEnabled(false);
            let _ = settings.SetAreDevToolsEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
        }
        let _ = webview.AddWebResourceRequestedFilter(&HSTRING::from("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
    }

    let state = Rc::new(RefCell::new(SniffState::default()));
    let mut tok = 0i64;

    unsafe {
        let _ = webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut tok,
        );

        let st_req = state.clone();
        let _ = webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let req = args.Request()?;
                let mut uri_p = PWSTR::null();
                req.Uri(&mut uri_p)?;
                let uri = CoTaskMemPWSTR::from(uri_p).to_string();
                let mut st = st_req.borrow_mut();
                if detect_block_hint(0, "", &uri) == Some(SniffBlock::DrmProtected) {
                    if st.blocked.is_none() && st.candidates.is_empty() {
                        st.blocked = Some(SniffBlock::DrmProtected);
                    }
                }
                if is_stream_candidate_url(&uri) {
                    st.record_url(uri, String::new(), None);
                }
                Ok(())
            })),
            &mut tok,
        );

        if let Ok(wv2) = webview.cast::<ICoreWebView2_2>() {
            let st_res = state.clone();
            let _ = wv2.add_WebResourceResponseReceived(
                &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let req = args.Request()?;
                    let mut uri_p = PWSTR::null();
                    req.Uri(&mut uri_p)?;
                    let uri = CoTaskMemPWSTR::from(uri_p).to_string();
                    let resp = args.Response()?;
                    let mut status = 0i32;
                    let _ = resp.StatusCode(&mut status);
                    let ct = resp
                        .Headers()
                        .ok()
                        .and_then(|h| {
                            let mut v = PWSTR::null();
                            h.GetHeader(&HSTRING::from("content-type"), &mut v)
                                .ok()
                                .map(|_| CoTaskMemPWSTR::from(v).to_string())
                        })
                        .unwrap_or_default();
                    let mut st = st_res.borrow_mut();
                    if st.blocked.is_none() && st.candidates.is_empty() {
                        if let Some(b) = detect_block_hint(status, &ct, &uri) {
                            st.blocked = Some(b);
                        }
                    }
                    if is_stream_content_type(&ct) || is_stream_candidate_url(&uri) {
                        st.record_url(uri, ct, Some(status));
                    }
                    Ok(())
                })),
                &mut tok,
            );
        }

        let st_nav = state.clone();
        let target_url = url.to_string();
        let _ = webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |wv, args| {
                let Some(args) = args else { return Ok(()) };
                let mut ok = windows::core::BOOL::default();
                let _ = args.IsSuccess(&mut ok);
                let mut err_status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                let _ = args.WebErrorStatus(&mut err_status);
                {
                    let mut st = st_nav.borrow_mut();
                    st.nav_done = true;
                    if !ok.as_bool() {
                        if is_tls_web_error(err_status) {
                            st.blocked = Some(SniffBlock::TlsError);
                        } else if st.candidates.is_empty() {
                            st.nav_failed = true;
                        }
                    }
                }
                if ok.as_bool() {
                    if let Some(wv) = wv.as_ref() {
                        inspect_dom_and_title(wv, &st_nav);
                    }
                } else {
                    let _ = &target_url;
                }
                Ok(())
            })),
            &mut tok,
        );

        if webview.Navigate(&HSTRING::from(url)).is_err() {
            let _ = controller.Close();
            return SniffResult::default();
        }
    }

    let started = Instant::now();
    let mut last_probe_js = Instant::now();
    loop {
        pump_messages();
        let now = Instant::now();
        {
            let st = state.borrow();
            if st.blocked.is_some() && st.candidates.is_empty() {
                break;
            }
            if st.nav_failed && st.candidates.is_empty() && now.duration_since(started) > Duration::from_secs(3) {
                break;
            }
            if let Some(t) = st.first_manifest_at {
                let wait_after_manifest = if st.has_verified_master() && st.page_title.is_some() {
                    Duration::from_millis(650)
                } else {
                    Duration::from_millis(2000)
                };
                if now.duration_since(t) >= wait_after_manifest {
                    break;
                }
            }
            if now.duration_since(started) >= timeout {
                break;
            }
        }
        if now.duration_since(last_probe_js) >= Duration::from_millis(900) {
            last_probe_js = now;
            inspect_dom_and_title(&webview, &state);
        }
        std::thread::sleep(Duration::from_millis(30));
    }

    // One last message pump so any pending ExecuteScript title callback settles.
    pump_messages();

    let st = state.borrow();
    let best = pick_best_manifest(&st.candidates).map(|c| c.url.clone());
    let out = SniffResult {
        stream_url: best,
        page_title: st.page_title.clone(),
        blocked: st.blocked.clone(),
        nav_failed: st.nav_failed,
    };
    drop(st);

    unsafe {
        let _ = controller.Close();
    }
    drop(webview);
    drop(controller);
    pump_messages();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_master_hls_over_variants_and_static_assets() {
        let candidates = vec![
            Candidate { url: "https://cdn.example.com/assets/init.png".into(), content_type: "image/png".into(), status: Some(200), seq: 0 },
            Candidate { url: "https://lightspeed.st/hls/abc/720p/index.m3u8".into(), content_type: "application/vnd.apple.mpegurl".into(), status: Some(200), seq: 1 },
            Candidate { url: "https://lightspeed.st/hls/abc/1/master.m3u8".into(), content_type: "application/vnd.apple.mpegurl".into(), status: Some(200), seq: 2 },
            Candidate { url: "https://ads.doubleclick.net/ad/master.m3u8".into(), content_type: "application/vnd.apple.mpegurl".into(), status: Some(200), seq: 3 },
        ];
        let picked = pick_best_manifest(&candidates).expect("should pick master manifest");
        assert_eq!(picked.url, "https://lightspeed.st/hls/abc/1/master.m3u8");
    }

    #[test]
    fn rejects_error_responses_and_detects_manifest_by_content_type() {
        let candidates = vec![
            Candidate { url: "https://cdn.example.com/dead/master.m3u8".into(), content_type: "application/vnd.apple.mpegurl".into(), status: Some(403), seq: 0 },
            Candidate { url: "https://cdn.example.com/token/stream?id=99".into(), content_type: "application/vnd.apple.mpegurl; charset=utf-8".into(), status: Some(200), seq: 1 },
            Candidate { url: "https://cdn.example.com/trailer.mp4".into(), content_type: "video/mp4".into(), status: Some(200), seq: 2 },
        ];
        let picked = pick_best_manifest(&candidates).expect("should pick content-type HLS");
        assert_eq!(picked.url, "https://cdn.example.com/token/stream?id=99");
    }

    #[test]
    fn detects_bot_auth_and_drm_blocks() {
        assert_eq!(detect_block_hint(403, "text/html", "https://site.com/"), Some(SniffBlock::BotChallenge));
        assert_eq!(detect_block_hint(401, "text/html", "https://site.com/"), Some(SniffBlock::AuthRequired));
        assert_eq!(detect_block_hint(0, "", "https://challenges.cloudflare.com/turnstile/v0"), Some(SniffBlock::BotChallenge));
        assert_eq!(detect_block_hint(200, "application/octet-stream", "https://drm.example.com/widevine/license"), Some(SniffBlock::DrmProtected));
        assert_eq!(detect_block_hint(200, "application/vnd.apple.mpegurl", "https://lightspeed.st/hls/master.m3u8"), None);
    }
}
