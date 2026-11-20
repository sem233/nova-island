//! News from the Nova team (news.js): news.json in the GitHub repository, read two minutes after
//! start, every 15 minutes and when the PC wakes up or gets back online. Every item is signed with
//! AmerTech's key; an item shows once, one a day at most, and links only to Nova's own pages.
//! Nothing is sent from the PC: the file is only read.
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::ctx::{now_ms, Ctx, Wake};

const URL: &str = "https://raw.githubusercontent.com/NOVA-TECH-KIT/nova-island/main/news.json";
const FIRST_MS: u64 = 2 * 60 * 1000;
const EVERY_MS: u64 = 15 * 60 * 1000;
const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

/// The app's version, for an item's minVersion / maxVersion.
pub const VERSION: &str = "1.8.12";

/// nova://plugins/<id>, nova://settings/<page>, or Nova's own pages on the web
pub fn link_ok(l: &str) -> bool {
    let word = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if let Some(rest) = l.strip_prefix("nova://plugins/").or(l.strip_prefix("nova://settings/")) {
        return word(rest);
    }
    if let Some(rest) = l.strip_prefix("https://github.com/NOVA-TECH-KIT/") {
        return rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"_./#?=&-".contains(&b));
    }
    if l == "https://ko-fi.com/binamer" {
        return true;
    }
    if let Some(rest) = l.strip_prefix("https://ipn.eg/S/binamerbm/") {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'/');
    }
    false
}

fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| -> Vec<f64> { s.split('.').map(|p| p.parse::<f64>().unwrap_or(f64::NAN)).collect() };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..3 {
        let (x, y) = (pa.get(i).copied().unwrap_or(0.0), pb.get(i).copied().unwrap_or(0.0));
        let (x, y) = (if x.is_nan() { 0.0 } else { x }, if y.is_nan() { 0.0 } else { y });
        if x != y {
            return x > y;
        }
    }
    false
}

fn today() -> String {
    let zone = nova_prayer::icu::default_zone();
    nova_prayer::day_in(&zone, now_ms()).map(|d| format!("{}-{:02}-{:02}", d.y, d.m, d.d)).unwrap_or_default()
}

pub fn valid(item: &Value) -> bool {
    let Some(o) = item.as_object() else { return false };
    let body: Map<String, Value> = o.iter().filter(|(k, _)| *k != "sig").map(|(k, v)| (k.clone(), v.clone())).collect();
    if !nova_sign::verify(&Value::Object(body.clone()), o.get("sig").and_then(Value::as_str)) {
        return false;
    }
    let id = body.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() || id.chars().count() > 60 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return false;
    }
    if !matches!(body.get("type").and_then(Value::as_str), Some("plugin" | "update" | "news")) || body.get("title").and_then(Value::as_str).is_none_or(str::is_empty) {
        return false;
    }
    let truthy = |k: &str| body.get(k).is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false)) && v.as_str() != Some(""));
    let text = |k: &str| body.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    if truthy("link") && !link_ok(&text("link")) {
        return false;
    }
    if truthy("minVersion") && newer(&text("minVersion"), VERSION) {
        return false;
    }
    if truthy("maxVersion") && newer(VERSION, &text("maxVersion")) {
        return false;
    }
    let t = today();
    if truthy("start") && t < text("start") {
        return false;
    }
    if truthy("end") && t > text("end") {
        return false;
    }
    true
}

fn cut(s: &str, n: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&u)
}

struct State {
    enabled: bool,
    items: Vec<Value>,
    shown: Vec<String>,
    dismissed: Vec<String>,
    read: Vec<String>,
    last_shown: f64,
    failed_once: String,
}

pub struct News {
    ctx: Ctx,
    st: Mutex<State>,
    wake: Wake,
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

impl News {
    pub fn new(ctx: Ctx) -> Arc<News> {
        let seen = ctx.store().load("news-seen.json").unwrap_or(json!({}));
        let on = ctx.on("news");
        Arc::new(News {
            ctx,
            st: Mutex::new(State { enabled: on, items: Vec::new(), shown: ids(&seen["shown"]), dismissed: ids(&seen["dismissed"]), read: ids(&seen["read"]), last_shown: seen["lastShown"].as_f64().unwrap_or(0.0), failed_once: String::new() }),
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
                let woken = me.wake.sleep(if first { FIRST_MS } else { EVERY_MS });
                first = false;
                let _ = woken;
                me.read_now();
            }
        });
    }

    fn save(&self, st: &State) {
        let _ = self.ctx.store().save("news-seen.json", &json!({ "shown": st.shown, "dismissed": st.dismissed, "read": st.read, "lastShown": st.last_shown }));
    }

    fn card(st: &State, i: &Value) -> Value {
        let id = i["id"].as_str().unwrap_or("");
        let body = i["body"].as_str().map(str::to_string).unwrap_or_else(|| if i["body"].is_null() { String::new() } else { i["body"].to_string() });
        json!({ "id": id, "type": i["type"], "title": cut(i["title"].as_str().unwrap_or(""), 120), "body": cut(&body, 240), "link": i["link"].as_str().unwrap_or(""), "unread": !st.read.iter().any(|r| r == id) })
    }

    /// The bell's list (shown, not closed) and at most one new item a day.
    fn publish(&self) {
        let st = self.st.lock().unwrap();
        let id = |i: &Value| i["id"].as_str().unwrap_or("").to_string();
        let cards: Vec<Value> = st.items.iter().filter(|i| st.shown.contains(&id(i)) && !st.dismissed.contains(&id(i))).map(|i| Self::card(&st, i)).collect();
        let next = if now_ms() - st.last_shown >= DAY_MS { st.items.iter().find(|i| !st.shown.contains(&id(i))).map(|i| Self::card(&st, i)) } else { None };
        drop(st);
        self.ctx.send("news", json!({ "cards": cards, "next": next }));
    }

    fn read_now(&self) {
        let r = nova_core::http::get(URL, 15000).and_then(|r| {
            if !r.ok() {
                return Err(format!("HTTP {}", r.status));
            }
            serde_json::from_slice::<Value>(&r.body).map_err(|e| e.to_string())
        });
        match r {
            Ok(list) => {
                let all = list.as_array().cloned().unwrap_or_default();
                let items: Vec<Value> = all.iter().filter(|i| valid(i)).cloned().collect();
                if all.len() > items.len() {
                    nova_core::log::info(&format!("[news] {} item(s) skipped (not signed or not for this version)", all.len() - items.len()));
                }
                let mut st = self.st.lock().unwrap();
                st.items = items;
                st.failed_once.clear();
                drop(st);
                self.publish();
            }
            Err(msg) => {
                // one line per kind of failure, none while offline
                let mut st = self.st.lock().unwrap();
                let offline = ["fetch failed", "timeout", "network unavailable"].iter().any(|w| msg.contains(w));
                if !offline && msg != st.failed_once {
                    st.failed_once = msg.clone();
                    nova_core::log::info(&format!("[news] can't read news.json: {msg}"));
                }
            }
        }
    }

    pub fn set_enabled(&self, on: bool) {
        let mut st = self.st.lock().unwrap();
        st.enabled = on;
        if !on {
            st.items.clear();
            drop(st);
            self.ctx.send("news", json!({ "cards": [], "next": null }));
        }
        self.wake.wake();
    }

    pub fn retry_soon(self: &Arc<Self>) {
        if self.st.lock().unwrap().enabled {
            let me = self.clone();
            std::thread::spawn(move || me.read_now());
        }
    }

    fn known(st: &State, id: &str) -> bool {
        st.items.iter().any(|i| i["id"].as_str() == Some(id))
    }

    fn push200(list: &mut Vec<String>, id: &str) {
        list.push(id.to_string());
        let n = list.len();
        if n > 200 {
            list.drain(..n - 200);
        }
    }

    /// The island showed it (once the island was free).
    pub fn shown(&self, id: &str) {
        let mut st = self.st.lock().unwrap();
        if !Self::known(&st, id) || st.shown.iter().any(|s| s == id) {
            return;
        }
        Self::push200(&mut st.shown, id);
        st.last_shown = now_ms();
        self.save(&st);
        drop(st);
        self.publish();
    }

    /// The user closed its card.
    pub fn dismiss(&self, id: &str) {
        let mut st = self.st.lock().unwrap();
        if !Self::known(&st, id) || st.dismissed.iter().any(|s| s == id) {
            return;
        }
        Self::push200(&mut st.dismissed, id);
        self.save(&st);
        drop(st);
        self.publish();
    }

    /// The bell's list was opened, or an item's link.
    pub fn mark_read(&self, ids: &[String]) {
        let mut st = self.st.lock().unwrap();
        let add: Vec<String> = ids.iter().filter(|id| Self::known(&st, id) && !st.read.contains(id)).cloned().collect();
        if add.is_empty() {
            return;
        }
        for a in add {
            Self::push200(&mut st.read, &a);
        }
        self.save(&st);
        drop(st);
        self.publish();
    }

    /// The link of an item the app itself read and checked, never one from the page.
    pub fn link_of(&self, id: &str) -> String {
        self.st.lock().unwrap().items.iter().find(|i| i["id"].as_str() == Some(id)).and_then(|i| i["link"].as_str()).unwrap_or("").to_string()
    }
}
