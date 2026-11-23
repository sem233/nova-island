//! Which parts of the island are on (features.js). Edited in the Settings window, applied live by the
//! island. Same keys, defaults, ranges and choices, in the same order, so features.json is the same file.
use serde_json::{json, Map, Value};

use crate::{json::js_round, settings::Store};

/// (key, default), in features.js order: the order features.json is written in.
pub fn defaults() -> Map<String, Value> {
    let list: &[(&str, Value)] = &[
        // expanded tabs
        ("tabMedia", json!(true)),
        ("tabApps", json!(true)),
        ("tabShelf", json!(true)),
        ("tabPrayer", json!(true)),
        ("tabTimer", json!(true)),
        ("tabSystem", json!(true)),
        // idle pill slides
        ("slideClock", json!(true)),
        ("slideStats", json!(true)),
        ("slidePrayer", json!(true)),
        ("slideNet", json!(false)), // (1.8.7) ↓ ↑ and the ping on the closed island
        // live activities
        ("mediaPill", json!(true)),
        ("mediaTitle", json!(true)),
        ("trackPeek", json!(true)),
        // notifications
        ("kbdToast", json!(true)),
        ("capsToast", json!(true)),
        ("hudVolume", json!(true)),
        ("hudBrightness", json!(true)),
        ("battery", json!(true)),
        ("prayerAlerts", json!(true)),
        ("prayerSoonMin", json!(10)),      // minutes before each prayer to alert (0 = off)
        ("prayerSoonSound", json!("bell")), // pre-prayer alert chime
        ("iqamaAlert", json!(true)),       // remind before / at iqama
        ("iqamaDelayMin", json!(15)),      // minutes after adhan until iqama
        ("iqamaBeforeMin", json!(2)),      // alert N minutes before iqama (0 = at iqama)
        ("iqamaSound", json!("crystal")),  // iqama alert chime
        ("asrHanafi", json!(false)),       // Asr by the Hanafi shadow rule
        ("adhanSound", json!(false)),      // (1.8.7) the adhan plays with the prayer alert (off until turned on)
        ("adhanPreset", json!("")),        // selected online adhan preset id (or "")
        ("adhanFajrPreset", json!("")),    // selected online Fajr adhan preset id (or "")
        ("hijriAdjust", json!(0)),         // days added to the Hijri date (local moon sighting)
        // IDM Nova
        ("downloads", json!(true)),
        ("dlToasts", json!(true)),
        ("folderDownloads", json!(true)), // browser downloads, seen in the download folders
        // window
        ("hideFullscreen", json!(true)),
        ("autoUpdate", json!(true)), // download new versions from GitHub and offer a restart on the island
        // take the right third of the browser's address bar
        ("addrIsland", json!(false)),
        ("browserLine", json!(false)),
        ("taskbarStrip", json!(false)), // (1.8.7) the Nova strip on the taskbar (display only)
        // performance (the battery saver turns all three on while the battery is low)
        ("noBlur", json!(false)), // solid island, no screen capture at all
        ("noAnimations", json!(false)),
        ("glassFreeze", json!(false)), // capture the screen only while the island is open
        ("batterySaver", json!(false)),
        ("batterySaverAt", json!(20)), // percent
        ("news", json!(true)),         // plugins, updates and news from the Nova team (news.json on GitHub)
        ("calculator", json!(true)),   // compact calculator panel in the expanded island topbar
        ("theme", json!("madar")),     // the island's look (themes.css; 1.8.7: Madar for new users)
        ("prefsLang", json!("en")),    // the Settings window in English or Arabic (1.8: "en" | "ar")
        ("clock24", json!(false)),     // times in 24 hours on the island and the widgets (1.8; off: 12 hours)
        // System / Clipboard tab
        ("weather", json!(false)),          // optional weather fetch (off by default to save resources)
        ("clipboardHistory", json!(true)),
        ("clipPersist", json!(true)),       // save clipboard history to disk until it expires
        ("clipMaxItems", json!(50)),        // max unpinned clipboard history items (5..=50)
        ("clipExpireMinutes", json!(60)),   // auto-clear clipboard history after N minutes (0 = never)
        // copied image / screenshot -> sleek toast with Pin button; optional auto-pin & auto-shelf
        ("screenshotToast", json!(true)),
        ("shotPinOverlay", json!(true)),    // show Pin-on-screen button on screenshot toast
        ("shotAutoPin", json!(false)),      // automatically float captured screenshots without clicking
        ("shotAutoShelf", json!(true)),
        ("shotShelfMinutes", json!(20)),
        // mic / camera in use
        ("privacyDots", json!(true)),
        // other apps' notifications on the island (1.8; they hold messages, so off until asked)
        ("appNotify", json!(false)),
        // Apps tab: apps with a window open come first (1.8)
        ("appsRunningFirst", json!(true)),
        // companion
        ("netToasts", json!(true)),
        ("slowNetToast", json!(true)), // (1.8.7) "Internet is slow" when the ping stays over 200 ms for a minute
        ("welcomeBack", json!(true)),
        ("morningBrief", json!(true)),
        ("wellbeing", json!(true)),
        ("smartClipboard", json!(true)),
        ("novaEyes", json!(true)),
        ("novaPrayer", json!(true)), // Nova in her prayer cap at the adhan
        ("novaFocus", json!(true)),  // Nova with her headband at focus start / end
        ("water", json!(false)),     // the Drop reminds you to drink (off until turned on: 1.8.1)
        ("waterEvery", json!(60)),   // minutes at the computer between glasses
        ("waterGoal", json!(8)),     // glasses a day (~2 L)
        ("celebrate", json!(true)),
        // upcoming meetings (iCal link in calendar.json)
        ("meetings", json!(true)),
        // reminders: dhikr, du'a, Qur'an
        ("reminders", json!(true)),
        ("remindAyah", json!(true)),
        ("remindDhikr", json!(true)),
        ("remindDua", json!(true)),
        ("remindEvery", json!(5)), // minutes
        ("remindFor", json!(30)),  // seconds on screen
        // quiet hours for reminders (minutes after midnight; may wrap past midnight)
        ("quietHours", json!(false)),
        ("quietFrom", json!(23 * 60)),
        ("quietTo", json!(7 * 60)),
        // focus sessions (Pomodoro)
        ("focusMin", json!(25)),
        ("breakMin", json!(5)),
        ("longBreakMin", json!(15)),
    ];
    list.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// Number settings and their allowed range.
fn range(key: &str) -> Option<(f64, f64)> {
    Some(match key {
        "remindEvery" => (1.0, 240.0),
        "remindFor" => (5.0, 300.0),
        "quietFrom" | "quietTo" => (0.0, 1439.0),
        "focusMin" => (5.0, 120.0),
        "breakMin" => (1.0, 60.0),
        "longBreakMin" => (5.0, 60.0),
        "hijriAdjust" => (-2.0, 2.0),
        "prayerSoonMin" => (0.0, 60.0),
        "iqamaDelayMin" => (5.0, 45.0),
        "iqamaBeforeMin" => (0.0, 15.0),
        "waterEvery" => (15.0, 240.0),
        "waterGoal" => (4.0, 16.0),
        "shotShelfMinutes" => (5.0, 1440.0),
        "clipMaxItems" => (5.0, 50.0),
        "clipExpireMinutes" => (0.0, 10080.0),
        "batterySaverAt" => (10.0, 100.0),
        _ => return None,
    })
}

/// Settings that are one of a fixed list.
pub const THEMES: &[&str] = &["original", "bakka", "outline", "midnight", "forest", "snow", "cat", "galaxy", "aurora", "ocean", "nody", "lava", "retro", "madar"];
/// Themes without glass: no screen capture behind them.
pub const SOLID_THEMES: &[&str] = &["bakka", "midnight", "forest", "cat", "galaxy", "aurora", "ocean", "nody", "lava", "retro", "madar"];
/// Built-in synthesized prayer & iqama alert chimes (zero audio-file RAM overhead).
pub const PRAYER_SOUNDS: &[&str] = &["silent", "bell", "chime", "crystal", "takbir", "pulse"];
/// Online Adhan presets (empty = default/none, "custom" = user file).
pub const ADHAN_PRESETS: &[&str] = &["", "custom", "makkah", "alafasy", "abdulbasit", "refaat", "minshawi", "qatami", "fajr_malek"];

/// Keeps only known keys with values of the right type (numbers rounded and clamped to their range).
pub fn pick(obj: &Value, defaults: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let Some(obj) = obj.as_object() else { return out };
    for k in crate::json::js_key_order(obj) {
        let v = &obj[k];
        if !defaults.contains_key(k) {
            continue;
        }
        if k == "theme" {
            if v.as_str().is_some_and(|s| THEMES.contains(&s)) {
                out.insert(k.clone(), v.clone());
            }
        } else if k == "prefsLang" {
            if v.as_str().is_some_and(|s| s == "en" || s == "ar") {
                out.insert(k.clone(), v.clone());
            }
        } else if k == "prayerSoonSound" || k == "iqamaSound" {
            if v.as_str().is_some_and(|s| PRAYER_SOUNDS.contains(&s)) {
                out.insert(k.clone(), v.clone());
            }
        } else if k == "adhanPreset" || k == "adhanFajrPreset" {
            if v.as_str().is_some_and(|s| ADHAN_PRESETS.contains(&s)) {
                out.insert(k.clone(), v.clone());
            }
        } else if let Some((lo, hi)) = range(k) {
            if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                out.insert(k.clone(), json!(js_round(n).max(lo).min(hi)));
            }
        } else if v.is_boolean() {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// (1.8.7) The companion's features: no longer in Settings, always on.
pub const FIXED_ON: &[&str] = &["netToasts", "welcomeBack", "morningBrief", "wellbeing", "smartClipboard", "novaEyes", "novaPrayer", "novaFocus", "celebrate"];

/// What 1.8.7 settles whatever was saved: the companion on, the taskbar strip off (coming soon), and
/// Address bar (off Settings) as the thin line.
fn settle(values: &mut Map<String, Value>) {
    for k in FIXED_ON {
        values.insert(k.to_string(), json!(true));
    }
    // the taskbar strip: "Coming soon" in Settings, off until it ships
    values.insert("taskbarStrip".into(), json!(false));
}

pub struct Features {
    store: Store,
    defaults: Map<String, Value>,
    values: Map<String, Value>,
}

impl Features {
    /// Defaults with features.json on top (only what pick() keeps).
    pub fn load(store: Store) -> Features {
        let defaults = defaults();
        let mut values = defaults.clone();
        let saved = store.load("features.json").unwrap_or(json!({}));
        for (k, v) in pick(&saved, &defaults) {
            values.insert(k, v);
        }
        settle(&mut values);
        Features { store, defaults, values }
    }

    pub fn get(&self) -> Value {
        Value::Object(self.values.clone())
    }

    pub fn bool(&self, key: &str) -> bool {
        self.values.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    /// Applies a patch; saves and returns true when anything valid was in it.
    pub fn set(&mut self, patch: &Value) -> bool {
        let clean = pick(patch, &self.defaults);
        if clean.is_empty() {
            return false;
        }
        for (k, v) in clean {
            self.values.insert(k, v);
        }
        settle(&mut self.values);
        let _ = self.store.save("features.json", &Value::Object(self.values.clone()));
        true
    }

    pub fn reset(&mut self) {
        self.values = self.defaults.clone();
        let _ = self.store.save("features.json", &Value::Object(self.values.clone()));
    }
}

#[cfg(test)]
mod v1815_tests {
    use super::*;

    #[test]
    fn v1815_defaults_and_validation() {
        let defs = defaults();
        assert_eq!(defs.get("weather"), Some(&json!(false)));
        assert_eq!(defs.get("shotAutoPin"), Some(&json!(false)));
        assert_eq!(defs.get("shotPinOverlay"), Some(&json!(true)));
        assert_eq!(defs.get("prayerSoonMin"), Some(&json!(10)));
        assert_eq!(defs.get("prayerSoonSound"), Some(&json!("bell")));
        assert_eq!(defs.get("iqamaAlert"), Some(&json!(true)));
        assert_eq!(defs.get("iqamaDelayMin"), Some(&json!(15)));
        assert_eq!(defs.get("iqamaBeforeMin"), Some(&json!(2)));
        assert_eq!(defs.get("iqamaSound"), Some(&json!("crystal")));

        let cleaned = pick(&json!({
            "weather": true,
            "prayerSoonMin": 999,
            "prayerSoonSound": "takbir",
            "iqamaSound": "invalid_sound",
            "iqamaDelayMin": 2,
            "iqamaBeforeMin": 25,
            "adhanPreset": "makkah",
            "adhanFajrPreset": "evil_url",
        }), &defs);
        assert_eq!(cleaned.get("weather"), Some(&json!(true)));
        assert_eq!(cleaned.get("prayerSoonMin").and_then(Value::as_f64), Some(60.0));
        assert_eq!(cleaned.get("prayerSoonSound"), Some(&json!("takbir")));
        assert!(!cleaned.contains_key("iqamaSound"));
        assert_eq!(cleaned.get("iqamaDelayMin").and_then(Value::as_f64), Some(5.0));
        assert_eq!(cleaned.get("iqamaBeforeMin").and_then(Value::as_f64), Some(15.0));
        assert_eq!(cleaned.get("adhanPreset"), Some(&json!("makkah")));
        assert!(!cleaned.contains_key("adhanFajrPreset"));
    }
}
