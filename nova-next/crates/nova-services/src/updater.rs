//! Updates from the GitHub releases (updater.js, electron-updater): latest.yml is checked a minute
//! after start and every 6 hours, a newer version is downloaded in the background and checked
//! against its SHA-512, then offered on the island ("Update ready · Restart"). An update that is
//! never accepted is installed when the app quits. Only the installed app updates; the Microsoft
//! Store version is updated by the Store.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde_json::json;
use sha2::{Digest, Sha512};

use crate::ctx::{now_ms, Ctx, Wake};

pub const FEED: &str = "https://github.com/NOVA-TECH-KIT/nova-island/releases/latest/download/";
pub const FORK_FEED: &str = "https://github.com/sem233/nova-island/releases/latest/download/";
const FIRST_MS: u64 = 60 * 1000;
const EVERY_MS: u64 = 6 * 60 * 60 * 1000;
const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// What latest.yml says about the newest release.
#[derive(Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub path: String,
    pub sha512: String,
}

/// The top-level `version`, `path` and `sha512` of latest.yml (electron-builder writes them flat).
pub fn parse_latest(yml: &str) -> Option<Release> {
    let mut get = HashMap::new();
    for line in yml.lines() {
        if line.starts_with([' ', '-', '#']) {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            get.insert(k.trim().to_string(), v.trim().trim_matches(['\'', '"']).to_string());
        }
    }
    let r = Release { version: get.remove("version")?, path: get.remove("path")?, sha512: get.remove("sha512")? };
    (!r.version.is_empty() && !r.path.is_empty() && !r.sha512.is_empty()).then_some(r)
}

/// semver: a.b.c newer than the running version (a pre-release is older than its release)
pub fn newer(a: &str, b: &str) -> bool {
    let split = |s: &str| {
        // build metadata (+...) has no say in the order
        let s = s.split_once('+').map_or(s, |(v, _)| v);
        let (core, pre) = s.split_once('-').unwrap_or((s, ""));
        let n: Vec<u64> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        ((n.first().copied().unwrap_or(0), n.get(1).copied().unwrap_or(0), n.get(2).copied().unwrap_or(0)), pre.to_string())
    };
    let ((ca, pa), (cb, pb)) = (split(a), split(b));
    if ca != cb {
        return ca > cb;
    }
    match (pa.is_empty(), pb.is_empty()) {
        (true, false) => true,
        (false, true) | (true, true) => false,
        (false, false) => pre_newer(&pa, &pb),
    }
}

/// (1.8.7) SemVer's pre-release order: part by part, numbers as numbers (beta.10 after beta.2),
/// a number before a word, and more parts after fewer when the rest is the same.
fn pre_newer(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != std::cmp::Ordering::Equal {
                    return o == std::cmp::Ordering::Greater;
                }
            }
            (Some(_), None) => return true,
            (None, _) => return false,
        }
    }
}

fn sha512_b64(bytes: &[u8]) -> String {
    nova_sys::media::base64(&Sha512::digest(bytes))
}

/// A file name from latest.yml, kept to a plain name in the pending folder.
fn safe_name(p: &str) -> Option<String> {
    let name = p.rsplit(['/', '\\']).next()?;
    (!name.is_empty() && name.ends_with(".exe") && !name.contains("..")).then(|| name.to_string())
}

struct State {
    enabled: bool,
    ready: Option<(String, PathBuf, String)>, // version, file, its sha512 (latest.yml's)
    installing: bool,
    check_failed: bool,
    logged: HashMap<String, f64>,
}

pub struct Updater {
    ctx: Ctx,
    feed: String,
    pending: PathBuf,
    first_ms: u64,
    st: Mutex<State>,
    wake: Wake,
}

impl Updater {
    /// `feed`: the folder holding latest.yml and the installers; `pending`: where downloads wait.
    pub fn new(ctx: Ctx, feed: &str, pending: PathBuf, first_ms: Option<u64>) -> Arc<Updater> {
        let on = ctx.on("autoUpdate");
        Arc::new(Updater {
            ctx,
            feed: feed.to_string(),
            pending,
            first_ms: first_ms.unwrap_or(FIRST_MS),
            st: Mutex::new(State { enabled: on, ready: None, installing: false, check_failed: false, logged: HashMap::new() }),
            wake: Wake::new(),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            let mut first = true;
            loop {
                if !me.st.lock().unwrap().enabled {
                    me.wake.sleep(60 * 60 * 1000);
                    first = true;
                    continue;
                }
                // turned on (again): the first check a minute later, like setEnabled
                if me.wake.sleep(if first { me.first_ms } else { EVERY_MS }) {
                    first = true;
                    continue;
                }
                first = false;
                me.check();
            }
        });
    }

    /// One short line per kind of failure, at most once a day; none while offline.
    fn log_error(&self, what: &str, msg: &str, record_auto: bool) {
        let mut st = self.st.lock().unwrap();
        if record_auto {
            st.check_failed = true;
        }
        let offline = ["fetch failed", "timeout", "network unavailable", "12007", "12029", "12002"].iter().any(|w| msg.contains(w));
        if offline {
            return;
        }
        let key = format!("{what}: {msg}");
        if now_ms() - st.logged.get(&key).copied().unwrap_or(0.0) < DAY_MS {
            return;
        }
        st.logged.insert(key.clone(), now_ms());
        drop(st);
        nova_core::log::info(&format!("update {key}"));
    }

    fn check(&self) {
        let feed = self.feed.clone();
        let _ = self.check_feed(&feed, true);
    }

    fn check_feed(&self, feed: &str, record_auto: bool) -> bool {
        // a ready update is offered again on every check until it is installed
        let ready = self.st.lock().unwrap().ready.as_ref().map(|r| r.0.clone());
        if let Some(v) = ready {
            self.ctx.send("update", json!({ "version": v }));
            return false;
        }
        if record_auto {
            self.st.lock().unwrap().check_failed = false;
        }
        let yml = match nova_core::http::get(&format!("{feed}latest.yml"), 30000) {
            Ok(r) if r.ok() => r.text(),
            Ok(r) => {
                let tail = if r.status == 404 { " (latest.yml not found)" } else { "" };
                self.log_error("check failed", &format!("HTTP {}{tail}", r.status), record_auto);
                return true;
            }
            Err(e) => {
                self.log_error("check failed", &e, record_auto);
                return true;
            }
        };
        let Some(rel) = parse_latest(&yml) else {
            self.log_error("check failed", "latest.yml can't be read", record_auto);
            return true;
        };
        if !newer(&rel.version, crate::news::VERSION) {
            return false;
        }
        let Some(name) = safe_name(&rel.path) else {
            self.log_error("check failed", "bad file name in latest.yml", record_auto);
            return true;
        };
        let file = self.pending.join(&name);
        // already downloaded earlier (the app was closed before installing it)
        let have = std::fs::read(&file).ok().filter(|b| sha512_b64(b) == rel.sha512);
        if have.is_none() {
            nova_core::log::info(&format!("update {} available, downloading", rel.version));
            let body = match nova_core::http::get(&format!("{feed}{}", nova_core::http::encode_component(&name)), 10 * 60 * 1000) {
                Ok(r) if r.ok() => r.body,
                Ok(r) => {
                    self.log_error("download failed", &format!("HTTP {}", r.status), record_auto);
                    return true;
                }
                Err(e) => {
                    self.log_error("download failed", &e, record_auto);
                    return true;
                }
            };
            if sha512_b64(&body) != rel.sha512 {
                self.log_error("download failed", "sha512 checksum mismatch", record_auto);
                return true;
            }
            let _ = std::fs::remove_dir_all(&self.pending);
            if let Err(e) = std::fs::create_dir_all(&self.pending).and_then(|_| std::fs::write(&file, &body)) {
                self.log_error("download failed", &e.to_string(), record_auto);
                return true;
            }
        }
        nova_core::log::info(&format!("update {} ready", rel.version));
        self.st.lock().unwrap().ready = Some((rel.version.clone(), file, rel.sha512.clone()));
        self.ctx.send("update", json!({ "version": rel.version }));
        false
    }

    /// Settings' "Check now": checks the original repo (`fork=false`) or user's fork (`fork=true`).
    /// Answers on `"update-check"` with `{ state, version, source }`.
    pub fn check_now(self: &Arc<Self>) {
        self.check_now_from(false);
    }

    pub fn check_now_from(self: &Arc<Self>, fork: bool) {
        let me = self.clone();
        std::thread::spawn(move || {
            let feed = if fork { FORK_FEED.to_string() } else { me.feed.clone() };
            let failed = me.check_feed(&feed, !fork);
            let st = me.st.lock().unwrap();
            let (state, version) = match (&st.ready, failed) {
                (Some(r), _) => ("ready", r.0.clone()),
                (None, true) => ("failed", String::new()),
                (None, false) => ("latest", crate::news::VERSION.to_string()),
            };
            drop(st);
            let source = if fork { "fork" } else { "official" };
            me.ctx.send("update-check", json!({ "state": state, "version": version, "source": source }));
        });
    }

    /// The version downloaded and waiting to be installed, if any (Settings' update bar).
    pub fn ready_version(&self) -> Option<String> {
        self.st.lock().unwrap().ready.as_ref().map(|r| r.0.clone())
    }

    pub fn set_enabled(&self, on: bool) {
        self.st.lock().unwrap().enabled = on;
        self.wake.wake();
    }

    /// After sleep or a lost connection: a check that failed is tried again a minute later.
    pub fn retry_soon(self: &Arc<Self>) {
        let st = self.st.lock().unwrap();
        if st.enabled && st.check_failed {
            drop(st);
            let me = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(60 * 1000));
                me.check();
            });
        }
    }

    /// Runs the downloaded installer: silent, as an update, and (`reopen`) starting the app after.
    /// The caller quits the app. False when there is nothing to install.
    fn run_installer(&self, reopen: bool) -> bool {
        let mut st = self.st.lock().unwrap();
        let Some((v, file, sha)) = st.ready.clone() else { return false };
        if st.installing {
            return false;
        }
        st.installing = true;
        drop(st);
        // (1.8.7) the file is checked again just before it runs: it waited in a folder the user
        // can write to since it was downloaded. Gone or changed: downloaded again at the next check.
        if std::fs::read(&file).ok().map(|b| sha512_b64(&b)).as_deref() != Some(sha.as_str()) {
            nova_core::log::error(&format!("update {v}: the downloaded file is gone or changed"));
            let mut st = self.st.lock().unwrap();
            st.ready = None;
            st.installing = false;
            drop(st);
            self.ctx.send("update", json!({ "version": null, "failed": true }));
            self.wake.wake();
            return false;
        }
        let mut args = vec!["--updated", "/S"];
        if reopen {
            nova_core::log::info(&format!("update {v} installing, will reopen"));
            args.push("--force-run");
        } else {
            nova_core::log::info(&format!("update {v} installing on quit"));
        }
        match std::process::Command::new(&file).args(&args).spawn() {
            Ok(_) => true,
            Err(e) => {
                // (1.8.7) blocked (an antivirus) or gone: "Update now" works again, and the pages
                // stop waiting for a restart that isn't coming
                nova_core::log::error(&format!("update {v} didn't start: {e}"));
                self.st.lock().unwrap().installing = false;
                self.ctx.send("update", json!({ "version": v, "failed": true }));
                false
            }
        }
    }

    /// "Restart" on the island.
    pub fn install(&self) -> bool {
        self.run_installer(true)
    }

    /// The app is quitting: a ready update goes in now (autoInstallOnAppQuit), while updates are on.
    pub fn install_on_quit(&self) {
        if self.st.lock().unwrap().enabled {
            self.run_installer(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_yml() {
        let y = "version: 1.9.1\nfiles:\n  - url: Nova-Island-Setup-1.9.1.exe\n    sha512: abc==\n    size: 5\npath: Nova-Island-Setup-1.9.1.exe\nsha512: abc==\nreleaseDate: '2026-11-03T00:00:00.000Z'\n";
        assert_eq!(parse_latest(y), Some(Release { version: "1.9.1".into(), path: "Nova-Island-Setup-1.9.1.exe".into(), sha512: "abc==".into() }));
        assert_eq!(parse_latest("files:\n  - url: x\n"), None);
    }

    #[test]
    fn versions() {
        assert!(newer("1.9.1", "1.9.0") && newer("2.1.0", "1.9.9") && newer("10.0.0", "9.9.9"));
        assert!(!newer("1.9.0", "1.9.0") && !newer("1.7.0", "1.9.0"));
        assert!(newer("1.9.0", "1.9.0-beta.1") && !newer("1.9.0-beta.1", "1.9.0"));
        // (1.8.7) numbers in a pre-release compare as numbers; build metadata is left out
        assert!(newer("1.9.0-beta.10", "1.9.0-beta.2") && !newer("1.9.0-beta.2", "1.9.0-beta.10"));
        assert!(newer("1.9.0-rc.1", "1.9.0-beta.9") && newer("1.9.0-beta.1.1", "1.9.0-beta.1") && newer("1.9.0-beta", "1.9.0-1"));
        assert!(!newer("1.9.0+build.5", "1.9.0+build.4") && !newer("1.9.0-beta.1", "1.9.0-beta.1"));
        assert_eq!(safe_name("Nova-Island-Setup-1.9.1.exe").as_deref(), Some("Nova-Island-Setup-1.9.1.exe"));
        assert_eq!(safe_name("x/../evil.bat"), None);
    }

    #[test]
    fn feeds_stay_separate() {
        assert_eq!(FEED, "https://github.com/NOVA-TECH-KIT/nova-island/releases/latest/download/");
        assert_eq!(FORK_FEED, "https://github.com/sem233/nova-island/releases/latest/download/");
    }
}
