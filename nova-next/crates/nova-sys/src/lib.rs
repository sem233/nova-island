//! The system bridge (helper/NovaHelper.cs), inside the app instead of a separate process.
//! One JSON line about every 500 ms: media session (SMTC, or VLC), volume, brightness, full screen,
//! the window in front, keyboard layout, Caps Lock, clipboard, idle, battery, recorders, the
//! browser's address bar. Commands (the same text as the C# helper's stdin):
//!   toggle | next | prev | seek <sec> | vol <0-1> | mute | bright <0-100>
//!   | pauseapp <appId> | playapp <appId> | seekapp <appId> <sec>
//! A line goes out right after a command, so the island sees the effect at once.
pub mod apps;
pub mod appwin;
pub mod audio;
pub mod clip;
pub mod brightness;
pub mod icons;
pub mod json;
pub mod media;
pub mod notify;
pub mod omni;
pub mod players;
pub mod tabs;
pub mod taskbar;
pub mod sysinfo;
pub mod vlc;
pub mod win;
pub mod yturl;

use std::{
    sync::mpsc::{channel, RecvTimeoutError, Sender},
    time::{Duration, Instant},
};

use json::Line;
use windows::Win32::{
    System::Com::{CoInitializeEx, COINIT_MULTITHREADED},
    UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
};

struct State {
    mgr: Option<media::Manager>,
    vlc: vlc::Vlc,
    vlc_shown: bool,
    // AIMP, Winamp, MPC-HC (players.rs), shown the same way as VLC
    players: players::Players,
    player_shown: bool,
    bright: i64,
}

fn number(s: &str) -> Option<f64> {
    // the C# patterns: \d+(\.\d+)?
    let ok = !s.is_empty() && s.split_once('.').map_or(s.bytes().all(|b| b.is_ascii_digit()), |(a, b)| !a.is_empty() && !b.is_empty() && (a.to_owned() + b).bytes().all(|c| c.is_ascii_digit()));
    if ok { s.parse().ok() } else { None }
}

fn handle(st: &mut State, cmd: &str, emit: &dyn Fn(String)) {
    // Apps tab: focus an app's open window, or list which pinned apps are running
    const TAB: char = '\u{9}';
    if let Some(rest) = cmd.strip_prefix("focusapp").and_then(|r| r.strip_prefix(TAB)) {
        let mut f = rest.split(TAB);
        let (id, exe) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
        let result = if id.is_empty() { "none" } else { appwin::focus(id, exe) };
        emit(Line::new().str("focusapp", id).str("result", result).done());
        return;
    }
    if let Some(rest) = cmd.strip_prefix("runningapps").and_then(|r| r.strip_prefix(TAB)) {
        let f: Vec<&str> = rest.split(TAB).collect();
        let apps: Vec<(String, String)> = f.chunks(2).map(|c| (c[0].to_string(), c.get(1).unwrap_or(&"").to_string())).collect();
        let on: Vec<String> = appwin::running(&apps).iter().map(|id| json::quote(id)).collect();
        emit(format!("{{\"running\":[{}]}}", on.join(",")));
        return;
    }
    let parts: Vec<&str> = cmd.split(' ').collect();
    match parts.as_slice() {
        [op @ ("pauseapp" | "playapp"), id] if !id.is_empty() => {
            if let Some(s) = media::find(st.mgr.as_ref(), id) {
                let _ = if *op == "pauseapp" { media::wait(s.TryPauseAsync(), 3000) } else { media::wait(s.TryPlayAsync(), 3000) };
            }
        }
        ["seekapp", id, sec] => {
            if let (Some(s), Some(sec)) = (media::find(st.mgr.as_ref(), id), number(sec)) {
                media::wait(s.TryChangePlaybackPositionAsync(media::ticks(sec)), 3000);
            }
        }
        ["vol", v] => {
            if let Some(v) = number(v) {
                let v = v.clamp(0.0, 1.0) as f32;
                if audio::set_volume(v).is_ok() && v > 0.0 {
                    let _ = audio::set_mute(false);
                }
            }
        }
        ["mute"] => {
            if let Ok(m) = audio::muted() {
                let _ = audio::set_mute(!m);
            }
        }
        ["bright", v] if v.bytes().all(|b| b.is_ascii_digit()) && !v.is_empty() => {
            st.bright = v.parse::<i64>().unwrap_or(0).min(100);
            brightness::set(st.bright);
        }
        [c @ ("toggle" | "next" | "prev")] if st.vlc_shown => st.vlc.command(c),
        [c @ ("toggle" | "next" | "prev")] if st.player_shown => st.players.command(c),
        ["seek", sec] if st.player_shown => {
            if let Some(sec) = number(sec) {
                st.players.seek(sec);
            }
        }
        _ => {
            let Some(s) = media::current(st.mgr.as_ref()) else { return };
            match parts.as_slice() {
                ["seek", sec] => {
                    if let Some(sec) = number(sec) {
                        media::wait(s.TryChangePlaybackPositionAsync(media::ticks(sec)), 3000);
                    }
                }
                ["toggle"] => {
                    media::wait(s.TryTogglePlayPauseAsync(), 3000);
                }
                ["next"] => {
                    media::wait(s.TrySkipNextAsync(), 3000);
                }
                ["prev"] => {
                    media::wait(s.TrySkipPreviousAsync(), 3000);
                }
                _ => {}
            }
        }
    }
}

/// Starts the bridge on its own thread. `emit` gets every line; send commands on the returned
/// channel. The thread ends when the channel is dropped.
pub fn start(emit: impl Fn(String) + Send + 'static) -> Sender<String> {
    start_every(emit, 500)
}

/// start() with another tick (the leak test runs it 10x faster).
pub fn start_every(emit: impl Fn(String) + Send + 'static, tick_ms: u64) -> Sender<String> {
    let (tx, rx) = channel::<String>();
    std::thread::Builder::new()
        .name("nova-sys".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            }
            let mgr = media::manager();
            if mgr.is_none() {
                emit("{\"error\":\"smtc-unavailable\"}".into());
            }
            let bright = brightness::get();
            let mut st = State { mgr, vlc: vlc::Vlc::new(), vlc_shown: false, players: players::Players::new(), player_shown: false, bright };
            let bright_ok = bright >= 0;
            let mut omni = omni::Omni::new();
            let tabs = tabs::Tabs::spawn();
            let mut private = win::Private::new();
            let mut browser = win::BrowserTracker::new();
            let (mut last_key, mut prev_thumb): (Option<String>, String) = (None, String::new());
            let (mut thumb_ok, mut thumb_tries, mut tick) = (false, 0, 0u64);
            let mut rec = false;
            let mut next = Instant::now();
            loop {
                // commands as they come; report right after one
                let mut acted = false;
                loop {
                    let wait = if acted || Instant::now() >= next { Duration::ZERO } else { next - Instant::now() };
                    match rx.recv_timeout(wait) {
                        Ok(cmd) => {
                            handle(&mut st, cmd.trim(), &emit);
                            acted = true;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if !acted && Instant::now() < next {
                    continue;
                }
                next = Instant::now() + Duration::from_millis(tick_ms);
                tick += 1;

                let mut o = Line::new();
                let fg = win::foreground_snapshot();
                let session = media::current_for(st.mgr.as_ref(), &fg.proc_name);
                // VLC (no media session): shown while it plays and the session isn't playing,
                // or paused with no session at all
                // (1.8.3) a player that was shown and is now paused stays shown until something
                // else plays, so pausing it doesn't hand the island (and its buttons) to a paused
                // browser video
                let mut vlc_playing = false;
                let (vlc_was, player_was) = (st.vlc_shown, st.player_shown);
                st.vlc_shown = false;
                if st.vlc.find() {
                    vlc_playing = st.vlc.playing();
                    let session_playing = session.as_ref().is_some_and(media::playing);
                    st.vlc_shown = if vlc_playing { !session_playing } else { session.is_none() || (vlc_was && !session_playing) };
                }
                // AIMP, Winamp, MPC-HC: the same rule, when VLC isn't the one shown
                let mut player = None;
                st.player_shown = false;
                if !st.vlc_shown {
                    if let Some(now) = st.players.find() {
                        let session_playing = session.as_ref().is_some_and(media::playing);
                        st.player_shown = if now.playing { !session_playing } else { session.is_none() || (player_was && !session_playing) };
                        player = Some(now);
                    }
                    if !st.player_shown {
                        st.players.clear();
                    }
                }
                if let (true, Some(p)) = (st.player_shown, player.as_ref()) {
                    o = o.bool("has", true).str("title", &p.title).str("artist", &p.artist).str("album", "").str("app", p.app).bool("playing", p.playing).num("pos", json::round(p.pos, 2)).num("dur", json::round(p.dur, 2));
                    let key = format!("{}|{}|{}", p.title, p.artist, p.app);
                    if last_key.as_deref() != Some(key.as_str()) {
                        last_key = Some(key);
                        o = o.str("thumb", "");
                    }
                    thumb_ok = true;
                } else if st.vlc_shown {
                    o = o.bool("has", true).str("title", &st.vlc.title).str("artist", "").str("album", "").str("app", "vlc.exe").bool("playing", vlc_playing).int("pos", 0).int("dur", 0);
                    let key = format!("{}||vlc.exe", st.vlc.title);
                    if last_key.as_deref() != Some(key.as_str()) {
                        last_key = Some(key);
                        o = o.str("thumb", "");
                    }
                    thumb_ok = true;
                } else if let Some(s) = session.as_ref() {
                    let i = media::info(s);
                    o = o.bool("has", true).str("title", &i.title).str("artist", &i.artist).str("album", &i.album).str("app", &i.app).bool("playing", i.playing).num("pos", json::round(i.pos, 2)).num("dur", json::round(i.dur, 2));
                    // browsers publish the thumbnail after the title, so keep retrying for ~10 s;
                    // include duration so consecutive short videos with a generic site title still refresh
                    let dur_k = if i.dur > 0.0 { i.dur.round() as i64 } else { 0 };
                    let key = format!("{}|{}|{}|{}", i.title, i.artist, i.app, dur_k);
                    if last_key.as_deref() != Some(key.as_str()) {
                        last_key = Some(key);
                        thumb_ok = false;
                        thumb_tries = 0;
                        o = o.str("thumb", "");
                    }
                    if !thumb_ok && thumb_tries < 20 {
                        thumb_tries += 1;
                        let t = media::thumb(i.props.as_ref());
                        // identical to the previous track's thumbnail is usually stale: wait a little longer
                        if !t.is_empty() && (t != prev_thumb || thumb_tries >= 8) {
                            o = o.str("thumb", &t);
                            prev_thumb = t;
                            thumb_ok = true;
                        }
                    }
                } else {
                    o = o.bool("has", false);
                    last_key = None;
                }

                if let (Ok(v), Ok(m)) = (audio::volume(), audio::muted()) {
                    o = o.num("vol", json::round(v as f64, 3)).bool("muted", m);
                }
                if bright_ok && tick % 4 == 0 {
                    st.bright = brightness::get();
                }
                o = o.int("bright", if bright_ok { st.bright } else { -1 });
                o = o.bool("fs", fg.fullscreen).int("kl", fg.layout).bool("caps", win::caps_lock());
                let seq = win::clipboard_seq();
                // a change is reported once it's known whether it's private: the app copying it may
                // still hold the clipboard (the Snipping Tool does), and then it waits a tick
                let (seq, private) = private.settled(seq);
                o = o.num("clip", seq as f64).bool("clipPriv", private).int("idle", win::idle_seconds());
                let (pct, ac) = win::battery();
                o = o.int("bat", pct).bool("ac", ac);
                if tick % 10 == 1 {
                    rec = win::recorder_running();
                }
                o = o.bool("rec", rec);
                o = o.str("fgProc", &fg.proc_name).str("fgTitle", &fg.title);
                // the address bar only while the island may sit in it (1.8.7)
                let omni_json = if omni::WANTED.load(std::sync::atomic::Ordering::Relaxed) {
                    omni.json(fg.hwnd, &fg.proc_name, fg.fullscreen)
                } else {
                    omni.forget();
                    "null".into()
                };
                o = o.raw("omni", &omni_json);
                o = o.raw("browser", browser.json(fg.hwnd, &fg.proc_name, fg.fullscreen));
                // the browser's tabs (new in 1.8, for "Stay in place"), read on their own thread
                let tabs_json = tabs.lock().unwrap().clone();
                o = o.raw("tabs", &tabs_json);
                emit(o.done());
            }
        })
        .expect("nova-sys thread");
    tx
}
