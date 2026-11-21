// Quran Radio's window (new in 1.8): what it shows and what it asks for. The island does the
// playing (views/radio.js); this passes the window's choice to the island and the island's state
// back to the window. Favourites live in radio.json in the data folder.
use std::{cell::RefCell, rc::Rc, sync::OnceLock};

use serde_json::{json, Value};

use crate::{prefs::Kind, App};

/// The 114 surah names, in Mushaf order (mp3quran's list).
pub fn suwar() -> Value {
    static NAMES: OnceLock<Value> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let v: Value = serde_json::from_str(include_str!("../../../tools/radio/source/suwar.json")).unwrap_or_default();
            let mut list: Vec<(i64, String)> = v["suwar"].as_array().into_iter().flatten().filter_map(|s| Some((s["id"].as_i64()?, s["name"].as_str()?.trim().to_string()))).collect();
            list.sort();
            Value::Array(list.into_iter().map(|(_, n)| json!(n)).collect())
        })
        .clone()
}

/// The radio's stations, if it is installed and on.
fn stations(a: &Rc<RefCell<App>>) -> Value {
    let active = a.borrow().plugins.active();
    active.as_array().and_then(|l| l.iter().find(|p| p["kind"] == "radio")).map(|p| p["stations"].clone()).unwrap_or(json!([]))
}

fn favorites(a: &Rc<RefCell<App>>) -> Value {
    a.borrow().core.store.load("radio.json").map(|v| v["favorites"].clone()).filter(Value::is_array).unwrap_or(json!([]))
}

/// Everything the window shows when it opens.
pub fn data(a: &Rc<RefCell<App>>) -> Value {
    json!({ "stations": stations(a), "suwar": suwar(), "now": a.borrow().radio_now.clone(), "favorites": favorites(a) })
}

fn to_window(a: &Rc<RefCell<App>>, state: Value) {
    crate::prefs::send_kind(a, Kind::Radio, "radio-state", state);
}

/// The island's radio changed (station, surah, playing).
pub fn now(a: &Rc<RefCell<App>>, v: Value) {
    let playing = v["playing"] == true;
    a.borrow_mut().radio_now = v.clone();
    to_window(a, json!({ "now": v }));
    if playing {
        crate::ensure_page_awake(a);
    } else {
        crate::maybe_schedule_suspend(a);
    }
}

/// The plugin was switched on / off, removed or updated.
pub fn plugins_changed(a: &Rc<RefCell<App>>) {
    let s = stations(a);
    to_window(a, json!({ "stations": s }));
}

/// Messages from the window (radio/preload.js).
pub fn channel(a: &Rc<RefCell<App>>, channel: &str, args: &[Value]) -> Option<Value> {
    let arg0 = args.first().cloned().unwrap_or(Value::Null);
    match channel {
        "radio-data" => return Some(data(a)),
        // the island plays it
        "radio-play" => {
            crate::ensure_page_awake(a);
            let station = arg0["station"].as_str().filter(|s| !s.is_empty() && s.len() <= 80).map(str::to_string);
            let surah = arg0["surah"].as_i64().filter(|n| (1..=114).contains(n));
            let page = a.borrow().page.clone();
            if let (Some(st), Some(p)) = (station, page) {
                p.event("radio-play", json!({ "station": st, "surah": surah }));
            }
        }
        "radio-fav" => {
            let (Some(id), on) = (arg0.as_str(), args.get(1) == Some(&json!(true))) else { return Some(Value::Null) };
            let mut list: Vec<Value> = favorites(a).as_array().cloned().unwrap_or_default();
            list.retain(|x| x.as_str() != Some(id));
            if on && id.len() <= 80 {
                list.push(json!(id));
            }
            let _ = a.borrow().core.store.save("radio.json", &json!({ "favorites": list }));
            to_window(a, json!({ "favorites": list }));
        }
        "radio-window" => match arg0.as_str() {
            Some("close") => crate::prefs::close_kind(Kind::Radio),
            Some("minimize") => crate::prefs::minimize_kind(Kind::Radio),
            _ => {}
        },
        _ => return None,
    }
    Some(Value::Null)
}
