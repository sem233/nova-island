//! Windows media sessions (SMTC): what is playing, its position, artwork, and the transport
//! commands (NovaHelper.cs). WinRT calls are awaited the way the C# helper does it: polling the
//! operation's status, giving up (and cancelling) after a timeout.
use std::time::{Duration, Instant};

use windows_future::{AsyncStatus, IAsyncInfo, IAsyncOperation};
use windows::{
    core::{Interface, RuntimeType, HSTRING},
    Media::Control::*,
    Storage::Streams::DataReader,
    Win32::System::SystemInformation::GetSystemTimeAsFileTime,
};

/// The result of a WinRT operation, or None after `ms` (the operation is then cancelled).
pub fn wait<T: RuntimeType + 'static>(op: windows::core::Result<IAsyncOperation<T>>, ms: u64) -> Option<T> {
    let op = op.ok()?;
    let until = Instant::now() + Duration::from_millis(ms);
    loop {
        match op.Status().ok()? {
            AsyncStatus::Started => {
                if Instant::now() > until {
                    if let Ok(info) = op.cast::<IAsyncInfo>() {
                        let _ = info.Cancel();
                    }
                    return None;
                }
                std::thread::sleep(Duration::from_millis(4));
            }
            AsyncStatus::Completed => return op.GetResults().ok(),
            _ => return None,
        }
    }
}

pub type Manager = GlobalSystemMediaTransportControlsSessionManager;
pub type Session = GlobalSystemMediaTransportControlsSession;

pub fn manager() -> Option<Manager> {
    wait(Manager::RequestAsync(), 8000)
}

pub fn current(mgr: Option<&Manager>) -> Option<Session> {
    current_for(mgr, "")
}

/// Checks whether an SMTC `SourceAppUserModelId` belongs to the foreground process name.
fn app_matches_fg(app_id: &str, fg_proc: &str) -> bool {
    if fg_proc.is_empty() || app_id.is_empty() {
        return false;
    }
    let a = app_id.to_ascii_lowercase();
    let f = fg_proc.to_ascii_lowercase();
    a.contains(&f)
        || (f == "msedge" && a.contains("edge"))
        || (f == "firefox" && a == "308046b0af4a39cb")
}

/// Selects the best active media session across all open SMTC sessions:
/// 1) A currently `Playing` session ALWAYS beats a stale `Paused`/`Stopped` session.
/// 2) Among tied sessions, prefers one matching the foreground process (`fg_proc`),
///    then Windows' `GetCurrentSession()`, then the most recently updated timeline.
pub fn current_for(mgr: Option<&Manager>, fg_proc: &str) -> Option<Session> {
    let mgr = mgr?;
    let cur = mgr.GetCurrentSession().ok();
    let cur_id = cur.as_ref().and_then(|s| s.SourceAppUserModelId().ok()).map(|h| h.to_string_lossy());
    let Ok(sessions) = mgr.GetSessions() else {
        return cur;
    };
    let n = sessions.Size().unwrap_or(0);
    if n <= 1 {
        return cur.or_else(|| (n == 1).then(|| sessions.GetAt(0).ok()).flatten());
    }
    let mut best: Option<((u8, u8, u8, i64), Session)> = None;
    for i in 0..n {
        let Ok(s) = sessions.GetAt(i) else { continue };
        let is_play = u8::from(playing(&s));
        let app_id = s.SourceAppUserModelId().map(|h| h.to_string_lossy()).unwrap_or_default();
        let is_fg = u8::from(app_matches_fg(&app_id, fg_proc));
        let is_cur = u8::from(cur_id.as_deref() == Some(app_id.as_str()));
        let updated = s
            .GetTimelineProperties()
            .and_then(|tl| tl.LastUpdatedTime())
            .map(|t| t.UniversalTime)
            .unwrap_or(0);
        let rank = (is_play, is_fg, is_cur, updated);
        if best.as_ref().is_none_or(|(b, _)| rank > *b) {
            best = Some((rank, s));
        }
    }
    best.map(|(_, s)| s).or(cur)
}

/// The session of one app (its AppUserModelId), for the mini player hand-off.
pub fn find(mgr: Option<&Manager>, app_id: &str) -> Option<Session> {
    let sessions = mgr?.GetSessions().ok()?;
    let want = HSTRING::from(app_id);
    (0..sessions.Size().ok()?).filter_map(|i| sessions.GetAt(i).ok()).find(|s| s.SourceAppUserModelId().ok().as_ref() == Some(&want))
}

pub fn playing(s: &Session) -> bool {
    s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|p| p == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing).unwrap_or(false)
}

/// 100 ns units since 1601, now.
fn now_ticks() -> i64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

pub struct Info {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    pub pos: f64,
    pub dur: f64,
    pub props: Option<GlobalSystemMediaTransportControlsSessionMediaProperties>,
}

/// Title, artist, position (moved on by the time since the app last reported it while playing).
pub fn info(s: &Session) -> Info {
    let props = wait(s.TryGetMediaPropertiesAsync(), 3000);
    let playing = playing(s);
    let (mut pos, mut dur) = (0.0, 0.0);
    if let Ok(tl) = s.GetTimelineProperties() {
        pos = tl.Position().map(|t| t.Duration as f64 / 1e7).unwrap_or(0.0);
        let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
        let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
        dur = (end - start) as f64 / 1e7;
        if playing && dur > 0.0 {
            if let Ok(last) = tl.LastUpdatedTime() {
                pos += (now_ticks() - last.UniversalTime) as f64 / 1e7;
            }
            if pos > dur {
                pos = dur;
            }
        }
    }
    let text = |f: windows::core::Result<HSTRING>| f.map(|h| h.to_string_lossy()).unwrap_or_default();
    Info {
        title: props.as_ref().map(|p| text(p.Title())).unwrap_or_default(),
        artist: props.as_ref().map(|p| text(p.Artist())).unwrap_or_default(),
        album: props.as_ref().map(|p| text(p.AlbumTitle())).unwrap_or_default(),
        app: text(s.SourceAppUserModelId()),
        playing,
        pos,
        dur,
        props,
    }
}

/// The artwork as base64, "" when there is none (yet).
pub fn thumb(props: Option<&GlobalSystemMediaTransportControlsSessionMediaProperties>) -> String {
    let read = || -> Option<String> {
        let reference = props?.Thumbnail().ok()?;
        let stream = wait(reference.OpenReadAsync(), 3000)?;
        let size = stream.Size().ok()?;
        // (1.8.7) the size comes from the playing app: one far too big (or wrong) made the
        // allocation fail, and Rust ends the app there without a panic or a log line
        if size == 0 || size > 4 * 1024 * 1024 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
        let n = wait(reader.LoadAsync(size as u32).and_then(|op| op.cast::<IAsyncOperation<u32>>()), 3000)?;
        if n == 0 {
            return None;
        }
        let mut bytes = vec![0u8; n as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        let _ = reader.Close();
        Some(base64(&bytes))
    };
    read().unwrap_or_default()
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

/// Seconds to the 100 ns ticks SMTC wants.
pub fn ticks(sec: f64) -> i64 {
    (sec * 10_000_000.0) as i64
}
