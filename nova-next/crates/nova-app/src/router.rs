// Routes page messages (the channels in contract/ipc.json) to the app. Channels not ported yet
// answer invokes with null and are counted, so each step can see what is still missing.
use std::collections::HashMap;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use nova_web::Msg;
use serde_json::{json, Value};

use crate::{screen::Rect, App};

thread_local! {
    static SEEN: RefCell<BTreeMap<String, u32>> = const { RefCell::new(BTreeMap::new()) };
}

/// The commands the island may send to the system bridge (nova.js MEDIA_CMD).
fn media_cmd_ok(c: &str) -> bool {
    let num = |s: &str| !s.is_empty() && s.split_once('.').map_or(s.bytes().all(|b| b.is_ascii_digit()), |(a, b)| !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|x| x.is_ascii_digit()));
    match c.split_once(' ') {
        None => matches!(c, "toggle" | "next" | "prev" | "mute"),
        Some(("seek", v)) => num(v),
        Some(("vol", v)) => (v.starts_with('0') || v.starts_with('1')) && (v.len() == 1 || v.as_bytes()[1] == b'.') && num(v),
        Some(("bright", v)) => (1..=3).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    }
}

/// Which page a message came from. Each page numbers its own requests, so an id carries the page:
/// id * 8 + page (1.8.3: room for the Tasks window and more).
#[derive(Clone, Copy, PartialEq)]
pub enum Src {
    Island = 0,
    Prefs = 1,
    Mini = 2,
    Radio = 3,
    Tasks = 4,
}

const SRC_SLOTS: u64 = 8;

thread_local! {
    /// Tests: every answer the Settings page got, to replay to 1.7 for the golden comparison.
    static RECORD: RefCell<Option<(HashMap<u64, String>, Vec<Value>)>> = const { RefCell::new(None) };
}

pub fn record_start() {
    RECORD.with(|r| *r.borrow_mut() = Some((HashMap::new(), Vec::new())));
}

pub fn record_take() -> Value {
    RECORD.with(|r| r.borrow_mut().take().map(|(_, v)| Value::Array(v)).unwrap_or(Value::Null))
}

/// The answer to request `id` (as handle() numbered it), to the page that asked.
pub fn answer(app: &Rc<RefCell<App>>, id: u64, v: Value) {
    if id % SRC_SLOTS == Src::Prefs as u64 {
        RECORD.with(|r| {
            if let Some((pending, out)) = r.borrow_mut().as_mut() {
                if let Some(c) = pending.remove(&id) {
                    out.push(json!({ "c": c, "v": v }));
                }
            }
        });
    }
    let page = match id % SRC_SLOTS {
        0 => app.borrow().page.clone(),
        1 => app.borrow().prefs.as_ref().map(|p| p.page.clone()),
        2 => crate::mini::page(),
        3 => crate::prefs::page(app, crate::prefs::Kind::Radio),
        4 => crate::prefs::page(app, crate::prefs::Kind::Tasks),
        _ => None,
    };
    if let Some(p) = page {
        p.resolve(id / SRC_SLOTS, v);
    }
}

/// generalState() in nova.js
pub fn general_state(app: &Rc<RefCell<App>>) -> Value {
    let s = app.borrow();
    json!({
        "shown": !s.core.user_hidden,
        "vertical": s.core.prefs.get("vertical").and_then(Value::as_bool).unwrap_or(false),
        "startWithWindows": crate::startup::starts_with_windows(),
        "version": env!("CARGO_PKG_VERSION"),
        "store": crate::startup::store(),
        "battery": s.glass.battery.0,
        "saver": s.glass.saver_on,
    })
}

pub fn notify_general(app: &Rc<RefCell<App>>) {
    let g = general_state(app);
    crate::prefs::send(app, "general", g);
}

/// Settings > Support: the page names a link, the app holds the addresses.
fn support_link(key: &str) -> Option<&'static str> {
    Some(match key {
        "kofi" => "https://ko-fi.com/binamer",
        "instapay" => "https://ipn.eg/S/binamerbm/instapay/38txJO",
        "github" => "https://github.com/NOVA-TECH-KIT/nova-island",
        "facebook" => "https://www.facebook.com/nova4kit/",
        "telegram" => "https://t.me/nova4apps",
        "qrnova" => "https://github.com/NOVA-TECH-KIT/qr-nova/releases/latest",
        "qrnova-src" => "https://github.com/NOVA-TECH-KIT/qr-nova",
        "startupapps" => "ms-settings:startupapps",
        _ => return None,
    })
}

/// Runs `f` on a worker thread and answers invoke `id` with its result.
fn later(app: &Rc<RefCell<App>>, id: Option<u64>, f: impl FnOnce() -> Value + Send + 'static) {
    let bus = app.borrow().bus.clone();
    std::thread::spawn(move || {
        let v = f();
        if let Some(id) = id {
            bus.resolve(id, v);
        }
    });
}

/// Settings-only channels (prefs/preload.js). None when the channel isn't one of them.
fn prefs_channel(app: &Rc<RefCell<App>>, channel: &str, args: &[Value], id: Option<u64>) -> Option<Option<Value>> {
    let arg0 = args.first().cloned().unwrap_or(Value::Null);
    let v = match channel {
        "features-get" => app.borrow().core.features.get(),
        "features-reset" => {
            let before = app.borrow().core.features.get();
            app.borrow_mut().core.features.reset();
            let f = {
                let s = app.borrow();
                s.glass.island_features(&s.core.features)
            };
            (app.borrow().core.send)("features", f);
            crate::after_change(app);
            crate::features_changed(app, &before);
            Value::Null
        }
        "general-get" => general_state(app),
        "general-set" => {
            let on = !matches!(args.get(1), None | Some(Value::Null) | Some(Value::Bool(false)));
            match arg0.as_str() {
                Some("shown") => {
                    let mut s = app.borrow_mut();
                    s.core.user_hidden = !on;
                    s.core.update_visibility();
                }
                Some("vertical") => app.borrow_mut().core.set_vertical(on),
                Some("startWithWindows") => crate::startup::set_start_with_windows(on),
                Some("resetPosition") => app.borrow_mut().core.reset_position(),
                _ => {}
            }
            notify_general(app);
            Value::Null
        }
        "prefs-window" => {
            match arg0.as_str() {
                Some("close") => crate::prefs::close(),
                Some("minimize") => crate::prefs::minimize(),
                _ => {}
            }
            Value::Null
        }
        "reminder-preview" => {
            (app.borrow().core.send)("reminder-now", Value::Null);
            Value::Null
        }
        // the system bridge lives inside the app now: it's running whenever the app is
        "bridge-status" => json!({ "state": "ok", "detail": "", "engine": "built-in" }),
        "open-log" => {
            if let Some(p) = nova_core::log::path().filter(|p| p.exists()) {
                crate::files::open(&p.to_string_lossy());
            }
            Value::Null
        }
        // Settings › Widgets: what there is, what's set, and changes (applied at once)
        "widgets-get" => crate::widgets::get_with_previews(),
        "widgets-do" => crate::widgets::act(&args.first().cloned().unwrap_or(Value::Null)),
        // Settings' update bar (1.8): what's waiting, and "Update now" (the island counts down,
        // as its own card does, then installs)
        "update-state" => json!({ "ready": app.borrow().updater.ready_version() }),
        // Settings › General › Check for updates (1.8.6): the answer comes as "update-check"
        "update-check" => {
            app.borrow().updater.check_now();
            Value::Null
        }
        "update-now" => {
            if let Some(p) = app.borrow().page.clone() {
                p.event("update-now", Value::Null);
            }
            Value::Null
        }
        "copy-report" => {
            let report = crate::report(app);
            app.borrow().system.write_quiet(&report);
            json!(true)
        }
        "open-support" => {
            if let Some(url) = arg0.as_str().and_then(support_link) {
                crate::bus::open_external(url);
            }
            Value::Null
        }
        "dl-folder-add" => {
            let hwnd = app.borrow().prefs.as_ref().map(|p| p.hwnd);
            match crate::files::pick(hwnd, "Add a download folder", true, None) {
                Some(p) => app.borrow().downloads.add_folder(&p),
                None => app.borrow().downloads.folder_list(),
            }
        }
        // (1.8.7) the adhan's sound (adhan.rs): which file, choose one, back to Nova's own
        "adhan-get" => crate::adhan::info(),
        "adhan-pick" => {
            let hwnd = app.borrow().prefs.as_ref().map(|p| p.hwnd);
            let fajr = arg0 == "fajr";
            let r = match crate::files::pick(hwnd, if fajr { "Choose the Fajr adhan" } else { "Choose the adhan" }, false, Some(("Audio", "*.mp3;*.m4a;*.ogg;*.opus;*.wav;*.aac"))) {
                Some(p) => crate::adhan::set_file(fajr, &p),
                None => json!({ "cancel": true }),
            };
            if r.get("ok").is_some() {
                if let Some(p) = app.borrow().page.clone() {
                    p.event("adhan-file", Value::Null);
                }
            }
            r
        }
        "adhan-reset" => {
            crate::adhan::reset(arg0 == "fajr");
            if let Some(p) = app.borrow().page.clone() {
                p.event("adhan-file", Value::Null);
            }
            crate::adhan::info()
        }
        "plugin-install-file" => {
            let hwnd = app.borrow().prefs.as_ref().map(|p| p.hwnd);
            match crate::files::pick(hwnd, "Install a plugin", false, Some(("Nova Island plugin", "*.nvp"))) {
                Some(p) => app.borrow().plugins.install_file(&p),
                None => Value::Null,
            }
        }
        _ => return None,
    };
    let _ = id;
    Some(Some(v))
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

pub fn handle(app: &Rc<RefCell<App>>, msg: Msg, src: Src) {
    let (kind, channel, args, id) = match msg {
        Msg::Send { channel, args } => ("send", channel, args, None),
        Msg::Invoke { id, channel, args } => ("invoke", channel, args, Some(id * SRC_SLOTS + src as u64)),
    };
    if src == Src::Radio {
        if let (Some(v), Some(id)) = (crate::radio::channel(app, &channel, &args), id) {
            answer(app, id, v);
        }
        return;
    }
    if src == Src::Mini {
        crate::mini::channel(app, &channel, &args);
        return;
    }
    if src == Src::Tasks {
        if let Some(v) = crate::tasks::channel(app, &channel, &args) {
            if let Some(id) = id {
                answer(app, id, v);
            }
        }
        return;
    }
    if let (Src::Prefs, Some(id)) = (src, id) {
        RECORD.with(|r| {
            if let Some((pending, _)) = r.borrow_mut().as_mut() {
                pending.insert(id, channel.clone());
            }
        });
    }
    if src == Src::Prefs {
        if let Some(Some(v)) = prefs_channel(app, &channel, &args, id) {
            if let Some(id) = id {
                answer(app, id, v);
            }
            return;
        }
    }
    let arg0 = args.first().cloned().unwrap_or(Value::Null);
    let hwnd = app.borrow().core.win.hwnd;
    let mut handled = true;
    let reply: Value = match channel.as_str() {
        "drag-start" => {
            if app.borrow_mut().core.drag_start() {
                crate::drag_timer(hwnd, true);
            }
            Value::Null
        }
        "drag-end" => {
            crate::drag_timer(hwnd, false);
            app.borrow_mut().core.drag_end();
            // refresh the glass for the new spot right away
            crate::capture_now(app);
            Value::Null
        }
        "island-rect" => {
            let r = Rect { x: arg0["x"].as_f64().unwrap_or(0.0), y: arg0["y"].as_f64().unwrap_or(0.0), width: arg0["width"].as_f64().unwrap_or(0.0), height: arg0["height"].as_f64().unwrap_or(0.0) };
            let old = app.borrow().core.island_rect;
            app.borrow_mut().core.set_island_rect(r);
            if r.width != old.width || r.height != old.height {
                app.borrow_mut().glass.shape_changed();
                crate::schedule_capture(hwnd, 120);
            }
            Value::Null
        }
        "focus-input" => {
            let on = arg0.as_bool().unwrap_or(false);
            app.borrow_mut().core.focus_input(on);
            // the window has focus now; the page inside it (WebView2's own window) needs it too, or
            // the typing never reaches the search box
            if on {
                if let Some(p) = app.borrow().page.clone() {
                    unsafe {
                        let _ = p.controller.MoveFocus(webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                    }
                }
            }
            Value::Null
        }
        "clickable" => {
            app.borrow_mut().core.set_clickable(arg0.as_bool().unwrap_or(false));
            Value::Null
        }
        "pinned" => {
            app.borrow_mut().core.set_pinned(arg0.as_bool().unwrap_or(false));
            Value::Null
        }
        "context-menu" => {
            crate::menu(hwnd);
            Value::Null
        }
        "media-cmd" => {
            if let Some(c) = arg0.as_str().filter(|c| media_cmd_ok(c)) {
                let _ = app.borrow().sys.send(c.to_string());
            }
            Value::Null
        }
        "island-features" => {
            let s = app.borrow();
            s.glass.island_features(&s.core.features)
        }
        "features-set" => {
            let before = app.borrow().core.features.get();
            let changed = app.borrow_mut().core.features.set(&arg0);
            if changed {
                let f = {
                    let s = app.borrow();
                    s.glass.island_features(&s.core.features)
                };
                (app.borrow().core.send)("features", f);
                {
                    let mut s = app.borrow_mut();
                    s.core.update_ghost();
                    s.core.update_visibility();
                }
                crate::after_change(app);
                crate::features_changed(app, &before);
            }
            Value::Null
        }
        "system-snapshot" => app.borrow().system.snapshot(),
        // the reminder card's copy button (1.8.6): its text to the clipboard, kept out of
        // Nova's own clipboard history
        // (1.8.7) a verse / dhikr / du'a card for the taskbar strip (the strip is on)
        "strip-card" => {
            crate::taskbar::card(&arg0);
            Value::Null
        }
        "copy-text" => {
            if let Some(t) = args.first().and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() < 20_000) {
                app.borrow().system.write_quiet(t);
            }
            Value::Null
        }
        "clip-copy" => {
            if let Some(i) = arg0.as_u64() {
                app.borrow().system.copy_clip(i as usize);
            }
            Value::Null
        }
        "clip-clear" => {
            app.borrow().system.clear_clips();
            Value::Null
        }
        // clipboard (1.8.3): the whole text of a clip, and pinning one
        "clip-full" => arg0.as_u64().map(|i| app.borrow().system.clip_full(i as usize)).unwrap_or(Value::Null),
        "clip-pin" => {
            if let Some(i) = arg0.as_u64() {
                let on = args.get(1).and_then(Value::as_bool).unwrap_or(true);
                app.borrow().system.pin_clip(i as usize, on);
            }
            Value::Null
        }
        // --- QuickClips (clips.js) ---
        "clips-list" => app.borrow().clips.list(),
        "clips-add" => app.borrow().clips.add(arg0.as_str().unwrap_or("")),
        "clips-add-clipboard" => app.borrow().clips.add_from_clipboard(),
        "clips-remove" => app.borrow().clips.remove(arg0.as_str().unwrap_or("")),
        "clips-pin" => {
            let pinned = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            app.borrow().clips.pin(arg0.as_str().unwrap_or(""), pinned)
        }
        "clips-edit" => {
            let text = args.get(1).and_then(Value::as_str).unwrap_or("");
            app.borrow().clips.edit(arg0.as_str().unwrap_or(""), text)
        }
        "clips-clear" => {
            let keep_pinned = arg0.as_bool().unwrap_or(true);
            app.borrow().clips.clear(keep_pinned)
        }
        "clips-copy" => {
            app.borrow().clips.copy(arg0.as_str().unwrap_or(""));
            Value::Null
        }
        // --- Video Downloader (MDM) ---
        "video-probe" => {
            let data_dir = app.borrow().data_dir.clone();
            let req = arg0.clone();
            later(app, id, move || crate::mdm::probe_video(&req, &data_dir));
            return;
        }
        "video-download" => {
            let data_dir = app.borrow().data_dir.clone();
            let req = arg0.clone();
            later(app, id, move || crate::mdm::download_video(&req, &data_dir));
            return;
        }
        // smart clipboard: act on what the app itself recognised, never on data from the page
        "clip-action" => {
            let s = app.borrow().system.last_smart();
            if let Some(s) = s {
                match (s["type"].as_str(), arg0.as_str()) {
                    (Some("url"), Some("open")) => crate::bus::open_web_link(s["url"].as_str().unwrap_or("")),
                    (Some("math"), Some("copy")) => app.borrow().system.write_quiet(s["result"].as_str().unwrap_or("")),
                    // to IDM Nova (video sites as media); if it doesn't take it, open the link
                    (Some("url"), Some("download")) => {
                        let dl = app.borrow().downloads.clone();
                        let (url, host) = (s["url"].as_str().unwrap_or("").to_string(), s["host"].as_str().unwrap_or("").to_lowercase());
                        const VIDEO_HOSTS: &[&str] = &["youtube.com", "youtu.be", "instagram.com", "tiktok.com", "x.com", "twitter.com", "facebook.com", "fb.watch", "vimeo.com", "reddit.com", "soundcloud.com", "dailymotion.com"];
                        let media = VIDEO_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")));
                        std::thread::spawn(move || {
                            if dl.add_url(&url, media).is_err() {
                                crate::bus::open_web_link(&url);
                            }
                        });
                    }
                    _ => {}
                }
            }
            Value::Null
        }
        "prayer-snapshot" => app.borrow().prayer.snapshot(),
        "net-get" => app.borrow().network.snapshot(),
        // --- apps (icons are made on a worker thread) ---
        "apps-list" => {
            let a = app.borrow().apps.clone();
            later(app, id, move || a.list());
            return;
        }
        "apps-all" => {
            let a = app.borrow().apps.clone();
            later(app, id, move || a.all());
            return;
        }
        "apps-running" => {
            let a = app.borrow().apps.clone();
            later(app, id, move || a.running());
            return;
        }
        "apps-add" | "apps-remove" | "apps-reorder" => {
            let a = app.borrow().apps.clone();
            let ch = channel.clone();
            std::thread::spawn(move || match ch.as_str() {
                "apps-add" => a.add(arg0.as_str().unwrap_or("")),
                "apps-remove" => a.remove(arg0.as_str().unwrap_or("")),
                _ => a.reorder(&arg0),
            });
            Value::Null
        }
        "apps-launch" => {
            let a = app.borrow().apps.clone();
            let fresh = args.get(1).and_then(Value::as_bool).unwrap_or(false);
            std::thread::spawn(move || a.launch(arg0.as_str().unwrap_or(""), fresh));
            Value::Null
        }
        // --- news ---
        "news-shown" | "news-dismiss" | "news-open" | "news-read" => {
            let n = app.borrow().news.clone();
            let id = arg0.as_str().unwrap_or("").to_string();
            match channel.as_str() {
                "news-shown" => n.shown(&id),
                "news-dismiss" => n.dismiss(&id),
                "news-read" => n.mark_read(&strings(&arg0)),
                _ => {
                    n.mark_read(&[id.clone()]);
                    crate::open_nova_link(&n.link_of(&id));
                }
            }
            Value::Null
        }
        // --- calendar ---
        "cal-get" => app.borrow().calendar.snapshot(),
        "cal-set" => app.borrow().calendar.set_url(arg0.as_str().unwrap_or("")),
        "open-meeting" => {
            let l = arg0.as_str().unwrap_or("");
            if nova_services::calendar::service::is_join_link(l) {
                crate::bus::open_external(l);
            }
            Value::Null
        }
        // --- plugins (the island's own controls; the Settings page comes with step 7) ---
        "plugins-active" => app.borrow().plugins.active(),
        "plugins-list" => {
            let p = app.borrow().plugins.clone();
            later(app, id, move || {
                p.refresh(false);
                p.list()
            });
            return;
        }
        "plugin-install" => {
            let p = app.borrow().plugins.clone();
            let pid = arg0.as_str().unwrap_or("").to_string();
            later(app, id, move || Value::Bool(p.install(&pid)));
            return;
        }
        "plugin-remove" => {
            app.borrow().plugins.remove(arg0.as_str().unwrap_or(""));
            Value::Null
        }
        "plugin-enable" => {
            app.borrow().plugins.set_enabled(arg0.as_str().unwrap_or(""), args.get(1).and_then(Value::as_bool).unwrap_or(false));
            Value::Null
        }
        "plugin-setting" => {
            let (key, value) = (args.get(1).and_then(Value::as_str).unwrap_or("").to_string(), args.get(2).cloned().unwrap_or(Value::Null));
            app.borrow().plugins.set_setting(arg0.as_str().unwrap_or(""), &key, &value);
            Value::Null
        }
        // --- downloads ---
        "dl-snapshot" => {
            let d = app.borrow().downloads.clone();
            later(app, id, move || d.snapshot());
            return;
        }
        "dl-control" => {
            let d = app.borrow().downloads.clone();
            let action = args.get(1).and_then(Value::as_str).unwrap_or("").to_string();
            std::thread::spawn(move || d.control(arg0.as_str().unwrap_or(""), &action));
            Value::Null
        }
        "dl-folders" => app.borrow().downloads.folder_list(),
        "dl-folder-remove" => app.borrow().downloads.remove_folder(arg0.as_str().unwrap_or("")),
        "shot-to-shelf" => {
            app.borrow().shots.to_shelf(0.0);
            Value::Null
        }
        // --- shelf ---
        "shelf-list" => {
            let s = app.borrow().shelf.clone();
            later(app, id, move || s.list());
            return;
        }
        "shelf-add" => {
            let s = app.borrow().shelf.clone();
            let paths = strings(&arg0);
            std::thread::spawn(move || s.add(&paths, 0.0));
            Value::Null
        }
        "shelf-remove" => {
            app.borrow().shelf.remove(arg0.as_str().unwrap_or(""));
            Value::Null
        }
        "shelf-clear" => {
            app.borrow().shelf.clear();
            Value::Null
        }
        "shelf-open" | "shelf-reveal" | "shelf-drag" => {
            let p = arg0.as_str().unwrap_or("").to_string();
            if app.borrow().shelf.has(&p) {
                match channel.as_str() {
                    "shelf-open" => crate::files::open(&p),
                    "shelf-reveal" => crate::files::reveal(&p),
                    _ => crate::files::drag_out(hwnd, &p),
                }
            }
            Value::Null
        }
        "loc-get" => app.borrow().system.location_info(),
        "loc-set" => {
            let system = app.borrow().system.clone();
            std::thread::spawn(move || system.set_location(&arg0));
            Value::Null
        }
        "loc-search" => {
            // answered later from a worker thread (a network call)
            let (system, bus) = (app.borrow().system.clone(), app.borrow().bus.clone());
            if let Some(id) = id {
                let q = arg0.as_str().unwrap_or("").to_string();
                std::thread::spawn(move || bus.resolve(id, system.search_city(&q).unwrap_or(Value::Null)));
            }
            return;
        }
        // pop a YouTube video out of the browser: pause the tab, find its address, play it here
        "mini-open" => {
            let app_id = arg0["app"].as_str().unwrap_or("").to_string();
            let proc = crate::mini::browser_proc(&app_id);
            let Some(proc) = proc.filter(|_| crate::mini::app_id_ok(&app_id)) else {
                if let Some(id) = id {
                    answer(app, id, json!({ "ok": false, "error": "not-browser" }));
                }
                return;
            };
            let pos = arg0["pos"].as_f64().filter(|p| p.is_finite()).unwrap_or(0.0).max(0.0);
            let was_playing = match &arg0["playing"] {
                Value::Null => false,
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
                Value::String(s) => !s.is_empty(),
                _ => true,
            };
            let title = match &arg0["title"] {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
            // pause first so the tab stops exactly where the mini player will pick up
            let sys = app.borrow().sys.clone();
            let _ = sys.send(format!("pauseapp {app_id}"));
            let bus = app.borrow().bus.clone();
            std::thread::spawn(move || {
                let short: String = title.chars().take(200).collect();
                let r: Value = serde_json::from_str(&nova_sys::yturl::yt_url(proc, &short)).unwrap_or_default();
                let reply = match crate::mini::youtube_id(r["url"].as_str().unwrap_or("")) {
                    None => {
                        if was_playing {
                            let _ = sys.send(format!("playapp {app_id}"));
                        }
                        json!({ "ok": false, "error": "no-url" })
                    }
                    Some(vid) => {
                        let s = crate::mini::Session { id: vid, t: pos, app_id, title };
                        bus.ui(move |a| crate::mini::open(a, s));
                        json!({ "ok": true })
                    }
                };
                if let Some(id) = id {
                    bus.resolve(id, reply);
                }
            });
            return;
        }
        // "Update ready · Restart": the installer runs silently and opens the app again
        "update-install" => {
            if app.borrow().updater.install() {
                unsafe {
                    let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
                }
            }
            Value::Null
        }
        // Quran Radio (1.8): its window, and the island's state for it
        "radio-open" => {
            crate::prefs::open_kind(crate::prefs::Kind::Radio, None);
            Value::Null
        }
        // Tasks (1.8.3): the island's list, finishing one, a reminder snoozed, the window
        "tasks-list" => app.borrow().tasks.list(),
        "tasks-done" => {
            if let Some(id) = arg0.as_str() {
                let done = args.get(1).and_then(Value::as_bool).unwrap_or(true);
                app.borrow().tasks.update(id, &json!({ "done": done }));
            }
            Value::Null
        }
        "tasks-snooze" => {
            if let (Some(id), Some(m)) = (arg0.as_str(), args.get(1).and_then(Value::as_i64)) {
                app.borrow().tasks.snooze(id, m);
            }
            Value::Null
        }
        "tasks-open" => {
            if crate::tasks::active(app) {
                crate::prefs::open_kind(crate::prefs::Kind::Tasks, None);
            }
            Value::Null
        }
        "radio-now" => {
            crate::radio::now(app, arg0.clone());
            Value::Null
        }
        "radio-suwar" => crate::radio::suwar(),
        // another app's notification on the island: open that app
        "app-notify-open" => {
            if let Some(id) = arg0.as_u64().and_then(|i| u32::try_from(i).ok()) {
                let n = app.borrow().app_notify.clone();
                std::thread::spawn(move || n.open(id));
            }
            Value::Null
        }
        "open-settings" => {
            crate::prefs::open(None);
            Value::Null
        }
        "island-ready" => {
            nova_core::log::info("island ready");
            crate::set_page_alive(true);
            let mut s = app.borrow_mut();
            s.core.relayout();
            s.core.sync_dim();
            Value::Null
        }
        // --- Video Downloader (MDM) — cancel a running download, list active IDs ---
        "video-cancel" => {
            crate::mdm::cancel_download(arg0.as_str().unwrap_or(""));
            Value::Null
        }
        "video-dl-active" => {
            Value::Array(crate::mdm::active_downloads())
        }
        _ => {
            handled = false;
            Value::Null
        }
    };
    if !handled {
        SEEN.with(|s| *s.borrow_mut().entry(format!("{kind}:{channel}")).or_default() += 1);
    }
    if let Some(id) = id {
        answer(app, id, reply);
    }
}

/// The channels the page used that aren't ported yet (printed on exit).
pub fn print_summary() {
    SEEN.with(|s| println!("not ported yet: {}", serde_json::to_string(&*s.borrow()).unwrap_or_default()));
}
