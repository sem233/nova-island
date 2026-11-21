//! System status (system.js): CPU / RAM, configurable clipboard history (up to 50 items with
//! optional persistence and auto-expiry), weather, and the place (a city picked in Settings, else
//! the IP location, else the time zone's capital), shared with the prayer times.
use std::sync::{atomic::{AtomicU64, Ordering}, Arc, Mutex};

use nova_core::http;
use serde_json::{json, Value};

use crate::ctx::{now_ms, Ctx, Wake};

const MIN_CLIPS: usize = 5;
const MAX_CLIPS_CAP: usize = 50;
const DEFAULT_MAX_CLIPS: usize = 50;
// pinned clips (1.8.3): kept on top and on disk, never pushed out by new copies
const MAX_PINS: usize = 10;
const PINS_FILE: &str = "clips-pinned.json";
const HISTORY_FILE: &str = "clips-history.json";
const MAX_PERSIST_BYTES: usize = 64 * 1024;
const WEATHER_FRESH: f64 = 3.0 * 60.0 * 60.0 * 1000.0;
const DAY: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
// after a failure: 2, 5, then every 15 minutes
const RETRY_MIN: [u64; 3] = [2, 5, 15];

#[derive(Clone, Debug, PartialEq)]
struct ClipEntry {
    text: String,
    at: f64,
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn prune_clips(clips: &mut Vec<ClipEntry>, max_items: usize, expire_ms: Option<f64>, now: f64) -> bool {
    let before = clips.len();
    if let Some(exp) = expire_ms {
        clips.retain(|e| e.at.is_finite() && (now - e.at) < exp);
    }
    if clips.len() > max_items {
        clips.truncate(max_items);
    }
    clips.len() != before
}

/// JS Math.round
fn round(v: f64) -> f64 {
    nova_core::json::js_round(v)
}

struct State {
    clips: Vec<ClipEntry>,
    pins: Vec<String>,
    last_clip: String,
    smart: Option<Value>,
    weather: Option<Value>,
    manual: Option<Value>,
    location: Option<Value>,
    located_at: f64,
    weather_fails: usize,
}

pub struct System {
    ctx: Ctx,
    st: Mutex<State>,
    stats_every: AtomicU64,
    weather_wake: Wake,
    stats_wake: Wake,
    on_location: Mutex<Vec<Box<dyn Fn(Value) + Send>>>,
}

// First start with no internet and no location saved yet: the capital of the PC's time zone, so
// prayer times show at once. Marked approximate, never saved.
const CAPITALS: &[(&str, &str, f64, f64, &str)] = &[
    ("Africa/Cairo", "Cairo", 30.0444, 31.2357, "EG"), ("Asia/Riyadh", "Riyadh", 24.7136, 46.6753, "SA"),
    ("Asia/Dubai", "Dubai", 25.2048, 55.2708, "AE"), ("Asia/Kuwait", "Kuwait City", 29.3759, 47.9774, "KW"),
    ("Asia/Qatar", "Doha", 25.2854, 51.531, "QA"), ("Asia/Bahrain", "Manama", 26.2285, 50.586, "BH"),
    ("Asia/Muscat", "Muscat", 23.588, 58.3829, "OM"), ("Asia/Amman", "Amman", 31.9539, 35.9106, "JO"),
    ("Asia/Beirut", "Beirut", 33.8938, 35.5018, "LB"), ("Asia/Damascus", "Damascus", 33.5138, 36.2765, "SY"),
    ("Asia/Baghdad", "Baghdad", 33.3152, 44.3661, "IQ"), ("Asia/Aden", "Sana'a", 15.3694, 44.191, "YE"),
    ("Asia/Gaza", "Gaza", 31.5017, 34.4668, "PS"), ("Asia/Hebron", "Hebron", 31.5326, 35.0998, "PS"),
    ("Africa/Casablanca", "Rabat", 34.0209, -6.8416, "MA"), ("Africa/Algiers", "Algiers", 36.7538, 3.0588, "DZ"),
    ("Africa/Tunis", "Tunis", 36.8065, 10.1815, "TN"), ("Africa/Tripoli", "Tripoli", 32.8872, 13.1913, "LY"),
    ("Africa/Khartoum", "Khartoum", 15.5007, 32.5599, "SD"), ("Europe/Istanbul", "Istanbul", 41.0082, 28.9784, "TR"),
    ("Asia/Karachi", "Karachi", 24.8607, 67.0011, "PK"), ("Asia/Jakarta", "Jakarta", -6.2088, 106.8456, "ID"),
    ("Asia/Kuala_Lumpur", "Kuala Lumpur", 3.139, 101.6869, "MY"), ("Europe/London", "London", 51.5072, -0.1276, "GB"),
    ("Europe/Paris", "Paris", 48.8566, 2.3522, "FR"), ("America/New_York", "New York", 40.7128, -74.006, "US"),
];

fn capital_of_zone() -> Option<Value> {
    let zone = nova_prayer::icu::default_zone();
    CAPITALS.iter().find(|c| c.0 == zone).map(|c| json!({ "city": c.1, "lat": c.2, "lon": c.3, "country": c.4, "timezone": zone, "approx": true }))
}

/// IP geolocation with fallbacks (free services rate-limit often).
fn locate() -> Result<Value, String> {
    type Pick = fn(&Value) -> Option<Value>;
    let num = |v: &Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()));
    let services: [(&str, Pick); 3] = [
        ("https://ipwho.is/", |j| (j["success"] != false).then(|| json!({ "lat": j["latitude"], "lon": j["longitude"], "city": j["city"], "country": j["country_code"], "timezone": j["timezone"]["id"] }))),
        ("https://get.geojs.io/v1/ip/geo.json", |j| Some(json!({ "lat": j["latitude"].as_str().and_then(|s| s.parse::<f64>().ok()).map(Value::from).unwrap_or(j["latitude"].clone()), "lon": j["longitude"].as_str().and_then(|s| s.parse::<f64>().ok()).map(Value::from).unwrap_or(j["longitude"].clone()), "city": j["city"], "country": j["country_code"], "timezone": j["timezone"] }))),
        ("https://ipapi.co/json/", |j| Some(json!({ "lat": j["latitude"], "lon": j["longitude"], "city": j["city"], "country": j["country_code"], "timezone": j["timezone"] }))),
    ];
    for (url, pick) in services {
        if let Ok(j) = http::get_json(url, 10000) {
            if let Some(loc) = pick(&j) {
                if num(&loc["lat"]).is_some_and(f64::is_finite) && num(&loc["lon"]).is_some_and(f64::is_finite) {
                    // drop missing fields, like JSON.stringify drops undefined
                    let o: serde_json::Map<String, Value> = loc.as_object().unwrap().iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect();
                    return Ok(Value::Object(o));
                }
            }
        }
    }
    Err("no location".into())
}

fn clip_max_from(ctx: &Ctx) -> usize {
    ctx.feature("clipMaxItems")
        .as_f64()
        .filter(|n| n.is_finite())
        .map(|n| (n.round() as isize).clamp(MIN_CLIPS as isize, MAX_CLIPS_CAP as isize) as usize)
        .unwrap_or(DEFAULT_MAX_CLIPS)
}

fn clip_expire_ms_from(ctx: &Ctx) -> Option<f64> {
    let mins = ctx.feature("clipExpireMinutes").as_f64().filter(|n| n.is_finite()).unwrap_or(60.0);
    if mins <= 0.0 {
        None
    } else {
        Some(mins.clamp(1.0, 10080.0) * 60_000.0)
    }
}

fn clip_persist_from(ctx: &Ctx) -> bool {
    ctx.on("clipboardHistory") && ctx.on("clipPersist")
}

fn load_history(store: &nova_core::settings::Store, pins: &[String], max_items: usize, expire_ms: Option<f64>, now: f64) -> (Vec<ClipEntry>, bool) {
    let Some(raw) = store.load(HISTORY_FILE) else {
        return (Vec::new(), false);
    };
    let Some(arr) = raw.as_array() else {
        return (Vec::new(), true);
    };
    let mut clips = Vec::new();
    let mut changed = false;
    for v in arr {
        let entry = if let Some(o) = v.as_object() {
            let text = o.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            let at = o.get("at").and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(now);
            ClipEntry { text, at }
        } else if let Some(s) = v.as_str() {
            changed = true;
            ClipEntry { text: s.to_string(), at: now }
        } else {
            changed = true;
            continue;
        };
        if entry.text.trim_matches(crate::smartclip::js_space).is_empty()
            || pins.contains(&entry.text)
            || clips.iter().any(|c: &ClipEntry| c.text == entry.text)
        {
            changed = true;
            continue;
        }
        clips.push(entry);
    }
    clips.sort_by(|a, b| b.at.total_cmp(&a.at));
    if prune_clips(&mut clips, max_items, expire_ms, now) {
        changed = true;
    }
    (clips, changed)
}

impl System {
    pub fn new(ctx: Ctx) -> Arc<System> {
        let store = ctx.store();
        let now = now_ms();
        let weather = store.load("weather-cache.json").filter(|s| now - s["at"].as_f64().unwrap_or(0.0) < WEATHER_FRESH).map(|s| s["weather"].clone());
        let pins: Vec<String> = store.load(PINS_FILE).and_then(|v| v.as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).take(MAX_PINS).collect())).unwrap_or_default();
        let (clips, history_changed) = if clip_persist_from(&ctx) {
            load_history(&store, &pins, clip_max_from(&ctx), clip_expire_ms_from(&ctx), now)
        } else {
            (Vec::new(), false)
        };
        let st = State {
            clips,
            pins,
            last_clip: nova_sys::clip::read_text(),
            smart: None,
            weather,
            manual: store.load("location.json").filter(|v| !v.is_null()),
            location: store.load("location-cache.json").filter(|v| !v.is_null()),
            located_at: 0.0,
            weather_fails: 0,
        };
        let sys = Arc::new(System { ctx, st: Mutex::new(st), stats_every: AtomicU64::new(2000), weather_wake: Wake::new(), stats_wake: Wake::new(), on_location: Mutex::new(Vec::new()) });
        if history_changed {
            let clips_copy = sys.st.lock().unwrap().clips.clone();
            sys.persist_history(&clips_copy);
        }
        sys
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || me.stats_loop());
        if let Some(w) = self.st.lock().unwrap().weather.clone() {
            self.ctx.send("weather", w); // the saved one, until a fresh one arrives
        }
        let me = self.clone();
        std::thread::spawn(move || me.weather_loop());
    }

    // ---- CPU / RAM ----
    fn stats_loop(&self) {
        let mut last = nova_sys::sysinfo::cpu_times();
        let mut net = (crate::netspeed::counters(), std::time::Instant::now());
        loop {
            self.stats_wake.sleep(self.stats_every.load(Ordering::Relaxed));
            // (1.8.7) bytes a second down and up since the last reading
            let now_net = (crate::netspeed::counters(), std::time::Instant::now());
            let secs = now_net.1.duration_since(net.1).as_secs_f64().max(0.001);
            let rx = (now_net.0 .0.saturating_sub(net.0 .0) as f64 / secs).round();
            let tx = (now_net.0 .1.saturating_sub(net.0 .1) as f64 / secs).round();
            net = now_net;
            let ping = crate::netspeed::PING_MS.load(Ordering::Relaxed);
            let now = nova_sys::sysinfo::cpu_times();
            let d_total = now.1.saturating_sub(last.1);
            let cpu = if d_total > 0 { 1.0 - now.0.saturating_sub(last.0) as f64 / d_total as f64 } else { 0.0 };
            last = now;
            let (total, free) = nova_sys::sysinfo::memory();
            let used = total - free;
            self.ctx.send("stats", json!({ "cpu": cpu, "ram": used / total, "ramUsed": used, "ramTotal": total, "rx": rx, "tx": tx, "ping": if ping < 0 { Value::Null } else { json!(ping) } }));
            // O(1) oldest-clip expiry check — zero extra threads or timers
            if let Some(exp) = clip_expire_ms_from(&self.ctx) {
                let now_ts = now_ms();
                let expired = self.st.lock().unwrap().clips.last().is_some_and(|e| now_ts - e.at >= exp);
                if expired {
                    self.prune_expired_at(now_ts);
                }
            }
        }
    }

    /// CPU / RAM every 2 s, every 5 s with the performance options (or the battery saver) on
    pub fn set_stats_every(&self, ms: u64) {
        if self.stats_every.swap(ms, Ordering::Relaxed) != ms {
            self.stats_wake.wake();
        }
    }

    // ---- clipboard (configurable count up to 50 + expiry + optional disk persistence) ----
    fn persist_history(&self, clips: &[ClipEntry]) {
        if clip_persist_from(&self.ctx) {
            let saved: Vec<Value> = clips
                .iter()
                .map(|e| json!({ "text": truncate_utf8(&e.text, MAX_PERSIST_BYTES), "at": e.at }))
                .collect();
            let _ = self.ctx.store().save(HISTORY_FILE, &Value::Array(saved));
        } else {
            let _ = std::fs::remove_file(self.ctx.dir.join(HISTORY_FILE));
        }
    }

    fn prune_expired_at(&self, now: f64) -> bool {
        let max_items = clip_max_from(&self.ctx);
        let expire_ms = clip_expire_ms_from(&self.ctx);
        let mut st = self.st.lock().unwrap();
        if !prune_clips(&mut st.clips, max_items, expire_ms, now) {
            return false;
        }
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        self.ctx.send("clipboard", list);
        true
    }

    /// Applies updated clipboard history settings (enable/disable, persistence, limit, expiry) immediately.
    pub fn clip_settings_changed(&self) {
        let now = now_ms();
        if !self.ctx.on("clipboardHistory") {
            let mut st = self.st.lock().unwrap();
            let had = !st.clips.is_empty();
            st.clips.clear();
            let list = Self::listing(&st);
            drop(st);
            let _ = std::fs::remove_file(self.ctx.dir.join(HISTORY_FILE));
            if had {
                self.ctx.send("clipboard", list);
            }
            return;
        }
        let max_items = clip_max_from(&self.ctx);
        let expire_ms = clip_expire_ms_from(&self.ctx);
        let mut st = self.st.lock().unwrap();
        let changed = prune_clips(&mut st.clips, max_items, expire_ms, now);
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        if changed {
            self.ctx.send("clipboard", list);
        }
    }

    /// A new copy (the bridge's clipboard counter moved). Private copies are never read.
    pub fn clipboard_changed(&self, is_private: bool) {
        if is_private {
            return;
        }
        let text = nova_sys::clip::read_text();
        self.record_clip_at(text, now_ms());
    }

    /// Records `text` into smart clipboard & history at timestamp `now` (ms since epoch).
    pub fn record_clip_at(&self, text: String, now: f64) {
        let max_items = clip_max_from(&self.ctx);
        let expire_ms = clip_expire_ms_from(&self.ctx);
        let mut st = self.st.lock().unwrap();
        if text.is_empty() || text == st.last_clip {
            return;
        }
        st.last_clip = text.clone();
        if text.trim_matches(crate::smartclip::js_space).is_empty() {
            return;
        }
        if self.ctx.on("smartClipboard") {
            st.smart = crate::smartclip::classify(&text).ok().flatten();
            if let Some(s) = st.smart.clone() {
                self.ctx.send("clip-smart", s);
            }
        }
        if !self.ctx.on("clipboardHistory") {
            return;
        }
        if st.pins.contains(&text) {
            return; // already on top, pinned
        }
        st.clips.retain(|c| c.text != text);
        st.clips.insert(0, ClipEntry { text, at: now });
        prune_clips(&mut st.clips, max_items, expire_ms, now);
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        self.ctx.send("clipboard", list);
    }

    /// The list the page shows: the pinned clips first, then the recent ones.
    fn entries(st: &State) -> Vec<(String, bool)> {
        st.pins.iter().map(|p| (p.clone(), true)).chain(st.clips.iter().filter(|c| !st.pins.contains(&c.text)).map(|c| (c.text.clone(), false))).collect()
    }

    /// Each clip as the page gets it: the first 300 characters (UTF-16 units, as c.slice(0, 300)),
    /// whether it's pinned, and whether there's more to see in the full view.
    fn listing(st: &State) -> Value {
        Value::Array(
            Self::entries(st)
                .into_iter()
                .map(|(c, pinned)| {
                    let u: Vec<u16> = c.encode_utf16().collect();
                    let short = String::from_utf16_lossy(&u[..u.len().min(300)]);
                    let long = u.len() > 300 || c.contains('\n');
                    json!({ "text": short, "pinned": pinned, "long": long, "chars": c.chars().count() })
                })
                .collect(),
        )
    }

    /// The whole text of clip `i` (the full view).
    pub fn clip_full(&self, i: usize) -> Value {
        self.prune_expired_at(now_ms());
        let st = self.st.lock().unwrap();
        Self::entries(&st).get(i).map(|(c, _)| Value::String(c.clone())).unwrap_or(Value::Null)
    }

    /// Pins or unpins clip `i`. Pins are saved on this PC (the user chose to keep them).
    pub fn pin_clip(&self, i: usize, on: bool) {
        let now = now_ms();
        let max_items = clip_max_from(&self.ctx);
        let expire_ms = clip_expire_ms_from(&self.ctx);
        let mut st = self.st.lock().unwrap();
        prune_clips(&mut st.clips, max_items, expire_ms, now);
        let Some((text, pinned)) = Self::entries(&st).get(i).cloned() else { return };
        if on && !pinned {
            st.pins.insert(0, text.clone());
            st.pins.truncate(MAX_PINS);
            st.clips.retain(|c| c.text != text);
        } else if !on && pinned {
            st.pins.retain(|c| *c != text);
            st.clips.retain(|c| c.text != text);
            st.clips.insert(0, ClipEntry { text, at: now });
            prune_clips(&mut st.clips, max_items, expire_ms, now);
        } else {
            return;
        }
        let _ = self.ctx.store().save(PINS_FILE, &json!(st.pins));
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        self.ctx.send("clipboard", list);
    }

    /// What the last copy was recognised as (acted on by the app, never on data from the page).
    pub fn last_smart(&self) -> Option<Value> {
        self.st.lock().unwrap().smart.clone()
    }

    /// Writing to the clipboard ourselves must not be announced again.
    pub fn write_quiet(&self, text: &str) {
        self.st.lock().unwrap().last_clip = text.to_string();
        nova_sys::clip::write_text(text);
    }

    pub fn copy_clip(&self, i: usize) {
        let now = now_ms();
        let max_items = clip_max_from(&self.ctx);
        let expire_ms = clip_expire_ms_from(&self.ctx);
        let mut st = self.st.lock().unwrap();
        prune_clips(&mut st.clips, max_items, expire_ms, now);
        let Some((text, pinned)) = Self::entries(&st).get(i).cloned() else { return };
        st.last_clip = text.clone();
        nova_sys::clip::write_text(&text);
        if !pinned {
            st.clips.retain(|c| c.text != text);
            st.clips.insert(0, ClipEntry { text, at: now });
            prune_clips(&mut st.clips, max_items, expire_ms, now);
        }
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        self.ctx.send("clipboard", list);
    }

    /// Clears the history; pinned clips stay until they're unpinned.
    pub fn clear_clips(&self) {
        let mut st = self.st.lock().unwrap();
        st.clips.clear();
        let clips_copy = st.clips.clone();
        let list = Self::listing(&st);
        drop(st);
        self.persist_history(&clips_copy);
        self.ctx.send("clipboard", list);
    }

    pub fn snapshot(&self) -> Value {
        self.prune_expired_at(now_ms());
        let st = self.st.lock().unwrap();
        json!({ "clips": Self::listing(&st), "weather": st.weather.clone().unwrap_or(Value::Null) })
    }

    // ---- the place ----
    /// A city picked in Settings wins; otherwise the IP location, looked up again once a day,
    /// falling back to the last one found (or the time zone's capital).
    pub fn get_location(&self) -> Result<Value, String> {
        let st = self.st.lock().unwrap();
        if let Some(m) = st.manual.clone() {
            return Ok(json!({ "lat": m["lat"], "lon": m["lon"], "city": m["name"], "country": m["countryCode"].as_str().unwrap_or(""), "timezone": m["timezone"] }));
        }
        let stale = st.location.as_ref().is_none_or(|l| l["approx"] == true) || now_ms() - st.located_at > DAY;
        drop(st);
        if stale {
            match locate() {
                Ok(loc) => {
                    let mut st = self.st.lock().unwrap();
                    st.location = Some(loc.clone());
                    st.located_at = now_ms();
                    let _ = self.ctx.store().save("location-cache.json", &loc);
                }
                Err(e) => {
                    let mut st = self.st.lock().unwrap();
                    if st.location.is_none() {
                        st.location = capital_of_zone();
                    }
                    if st.location.is_none() {
                        return Err(e);
                    }
                    st.located_at = now_ms() - DAY + 30.0 * 60.0 * 1000.0; // try again in 30 minutes
                }
            }
        }
        Ok(self.st.lock().unwrap().location.clone().unwrap())
    }

    /// After sleep or a lost connection: weather (and a guessed location) right away.
    pub fn refresh_now(&self) {
        let mut st = self.st.lock().unwrap();
        if st.location.as_ref().is_some_and(|l| l["approx"] == true) {
            st.located_at = 0.0;
        }
        st.weather_fails = 0;
        drop(st);
        self.weather_wake.wake();
    }

    pub fn location_info(&self) -> Value {
        let st = self.st.lock().unwrap();
        match &st.manual {
            Some(m) => {
                let mut v = json!({ "mode": "manual" });
                for (k, x) in m.as_object().unwrap() {
                    v[k] = x.clone();
                }
                v
            }
            None => json!({ "mode": "auto", "name": st.location.as_ref().and_then(|l| l["city"].as_str()).unwrap_or("") }),
        }
    }

    /// City search for Settings (Open-Meteo geocoding, no key). None when the search failed.
    pub fn search_city(&self, query: &str) -> Option<Value> {
        let q: String = query.trim().chars().take(80).collect();
        if q.chars().count() < 2 {
            return Some(json!([]));
        }
        let url = format!("https://geocoding-api.open-meteo.com/v1/search?count=6&language=ar&format=json&name={}", http::encode_component(&q));
        let j = http::get_json(&url, 10000).ok()?;
        let list: Vec<Value> = j["results"].as_array().cloned().unwrap_or_default().iter().map(|r| json!({
            "name": r["name"], "country": r["country"].as_str().unwrap_or(""), "countryCode": r["country_code"].as_str().unwrap_or(""),
            "admin": r["admin1"].as_str().unwrap_or(""), "lat": r["latitude"], "lon": r["longitude"], "timezone": r["timezone"].as_str().unwrap_or(""),
        })).collect();
        Some(Value::Array(list))
    }

    /// null = back to automatic
    pub fn set_location(&self, loc: &Value) {
        let ok = loc["lat"].as_f64().is_some_and(f64::is_finite) && loc["lon"].as_f64().is_some_and(f64::is_finite) && loc["name"].is_string();
        let cut = |v: &Value, n: usize| -> String { v.as_str().map(|s| s.chars().take(n).collect()).unwrap_or_default() };
        let manual = ok.then(|| {
            let cc = loc["countryCode"].as_str().filter(|c| c.len() == 2 && c.bytes().all(|b| b.is_ascii_alphabetic())).map(|c| c.to_uppercase()).unwrap_or_default();
            json!({ "name": cut(&loc["name"], 80), "country": cut(&loc["country"], 80), "countryCode": cc, "lat": loc["lat"], "lon": loc["lon"], "timezone": cut(&loc["timezone"], 60) })
        });
        let _ = self.ctx.store().save("location.json", manual.as_ref().unwrap_or(&Value::Null));
        {
            let mut st = self.st.lock().unwrap();
            st.manual = manual;
            st.location = None;
        }
        self.weather_wake.wake();
        let info = self.location_info();
        for f in self.on_location.lock().unwrap().iter() {
            f(info.clone());
        }
    }

    pub fn on_location_change(&self, f: impl Fn(Value) + Send + 'static) {
        self.on_location.lock().unwrap().push(Box::new(f));
    }

    // ---- weather (Open-Meteo, location from IP) ----
    fn fetch_weather(&self) -> Result<(), String> {
        let loc = self.get_location()?;
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code,is_day&daily=temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=1",
            nova_core::json::number(loc["lat"].as_f64().unwrap_or(0.0)),
            nova_core::json::number(loc["lon"].as_f64().unwrap_or(0.0))
        );
        let w = http::get_json(&url, 10000)?;
        let c = &w["current"];
        let weather = json!({
            "city": loc["city"].as_str().unwrap_or(""),
            "temp": round(c["temperature_2m"].as_f64().ok_or("bad weather")?),
            "code": c["weather_code"],
            "isDay": c["is_day"].as_f64().unwrap_or(0.0) != 0.0,
            "hi": round(w["daily"]["temperature_2m_max"][0].as_f64().unwrap_or(0.0)),
            "lo": round(w["daily"]["temperature_2m_min"][0].as_f64().unwrap_or(0.0)),
        });
        let _ = self.ctx.store().save("weather-cache.json", &json!({ "at": now_ms(), "weather": weather }));
        self.st.lock().unwrap().weather = Some(weather.clone());
        self.ctx.send("weather", weather);
        Ok(())
    }

    fn weather_loop(&self) {
        loop {
            let wait = match self.fetch_weather() {
                Ok(()) => {
                    let mut st = self.st.lock().unwrap();
                    if st.weather_fails > 0 {
                        nova_core::log::warn("[weather] back");
                    }
                    st.weather_fails = 0;
                    20 * 60 * 1000
                }
                Err(e) => {
                    let mut st = self.st.lock().unwrap();
                    if st.weather_fails == 0 {
                        nova_core::log::warn(&format!("[weather] {e} (retrying quietly)"));
                    }
                    let m = RETRY_MIN[st.weather_fails.min(RETRY_MIN.len() - 1)];
                    st.weather_fails += 1;
                    m * 60 * 1000
                }
            };
            self.weather_wake.sleep(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nova-sys-clip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::create_dir_all(&d);
        d
    }

    fn make_ctx(dir: PathBuf, feats: Value) -> (Ctx, Arc<Mutex<Vec<(String, Value)>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let emit: crate::ctx::Emit = Arc::new(move |ch, v| {
            log2.lock().unwrap().push((ch.to_string(), v));
        });
        let ctx = Ctx { emit, dir, features: Arc::new(Mutex::new(feats)) };
        (ctx, log)
    }

    #[test]
    fn stores_up_to_50_items_and_shrinks_live() {
        let dir = test_dir("limit50");
        let (ctx, _) = make_ctx(dir.clone(), json!({
            "clipboardHistory": true,
            "clipPersist": true,
            "clipMaxItems": 50,
            "clipExpireMinutes": 0,
            "smartClipboard": false,
        }));
        let sys = System::new(ctx.clone());
        let base = now_ms();
        for i in 1..=60 {
            sys.record_clip_at(format!("msg-{i}"), base + i as f64 * 100.0);
        }
        let snap = sys.snapshot();
        let clips = snap["clips"].as_array().unwrap();
        assert_eq!(clips.len(), 50);
        assert_eq!(clips[0]["text"], "msg-60");
        assert_eq!(clips[49]["text"], "msg-11");

        // Shrink limit live to 15
        ctx.features.lock().unwrap()["clipMaxItems"] = json!(15);
        sys.clip_settings_changed();
        let clips2 = sys.snapshot()["clips"].as_array().unwrap().clone();
        assert_eq!(clips2.len(), 15);
        assert_eq!(clips2[0]["text"], "msg-60");
        assert_eq!(clips2[14]["text"], "msg-46");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expires_old_items_and_keeps_pins_and_never_mode() {
        let dir = test_dir("expire");
        let (ctx, _) = make_ctx(dir.clone(), json!({
            "clipboardHistory": true,
            "clipPersist": true,
            "clipMaxItems": 50,
            "clipExpireMinutes": 15,
            "smartClipboard": false,
        }));
        let sys = System::new(ctx.clone());
        let t0 = now_ms();
        // Insert an item 20 min ago, another 5 min ago, and pin an old item
        sys.record_clip_at("pinned-old".into(), t0 - 30.0 * 60_000.0);
        // Pin directly via state so timestamp is old
        {
            let mut st = sys.st.lock().unwrap();
            st.pins.push("pinned-old".into());
            st.clips.clear();
            st.clips.push(ClipEntry { text: "fresh".into(), at: t0 - 5.0 * 60_000.0 });
            st.clips.push(ClipEntry { text: "expired".into(), at: t0 - 20.0 * 60_000.0 });
        }
        assert!(sys.prune_expired_at(t0));
        let clips = sys.snapshot()["clips"].as_array().unwrap().clone();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0]["text"], "pinned-old");
        assert_eq!(clips[0]["pinned"], true);
        assert_eq!(clips[1]["text"], "fresh");
        assert_eq!(clips[1]["pinned"], false);

        // Mode 0 (Never) must never drop old items
        ctx.features.lock().unwrap()["clipExpireMinutes"] = json!(0);
        {
            let mut st = sys.st.lock().unwrap();
            st.clips.push(ClipEntry { text: "ancient".into(), at: t0 - 30.0 * DAY });
        }
        assert!(!sys.prune_expired_at(t0));
        let clips_never = sys.snapshot()["clips"].as_array().unwrap().clone();
        assert_eq!(clips_never.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_across_restart_and_deletes_when_disabled() {
        let dir = test_dir("persist");
        let feats = json!({
            "clipboardHistory": true,
            "clipPersist": true,
            "clipMaxItems": 25,
            "clipExpireMinutes": 60,
            "smartClipboard": false,
        });
        let (ctx1, _) = make_ctx(dir.clone(), feats.clone());
        let sys1 = System::new(ctx1);
        let now = now_ms();
        sys1.record_clip_at("أول رسالة عربية".into(), now - 10_000.0);
        sys1.record_clip_at("second clip".into(), now - 5_000.0);
        assert!(dir.join(HISTORY_FILE).exists());

        // Inject an expired item into the saved file to verify load-time pruning
        let mut raw: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(dir.join(HISTORY_FILE)).unwrap()).unwrap();
        raw.push(json!({ "text": "too-old", "at": now - 120.0 * 60_000.0 }));
        std::fs::write(dir.join(HISTORY_FILE), serde_json::to_string(&raw).unwrap()).unwrap();

        // Reload in a fresh System instance
        let (ctx2, _) = make_ctx(dir.clone(), feats);
        let sys2 = System::new(ctx2.clone());
        let clips = sys2.snapshot()["clips"].as_array().unwrap().clone();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0]["text"], "second clip");
        assert_eq!(clips[1]["text"], "أول رسالة عربية");

        // Turning off clipPersist must remove clips-history.json immediately
        ctx2.features.lock().unwrap()["clipPersist"] = json!(false);
        sys2.clip_settings_changed();
        assert!(!dir.join(HISTORY_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncate_utf8_preserves_arabic_boundaries() {
        let s = "مرحبا بالعالم 🚀";
        for n in 0..=s.len() + 2 {
            let t = truncate_utf8(s, n);
            assert!(t.len() <= n);
            assert!(s.starts_with(&t));
        }
    }
}

