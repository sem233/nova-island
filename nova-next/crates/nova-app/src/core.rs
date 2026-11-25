// The island window's behaviour (nova.js): where it sits (an "anchor"), docking to the top edge,
// vertical mode on the left or right edge, dragging, the browser address-bar slot, "Stay in place"
// over a browser, hover with intent, Nova's eyes, and showing / hiding. Ported function by function;
// the names and numbers are nova.js's own.
use std::time::Instant;

use nova_core::{features::Features, settings::Store};
use serde_json::{json, Map, Value};

use crate::{
    island::Window,
    screen::{self, Rect},
};

pub const WIN_W: f64 = 640.0;
pub const WIN_H: f64 = 320.0;
pub const CALC_WIN_H: f64 = 440.0;
/// Short dwell so touching the pill or Chrome thin line opens almost instantly while ignoring a fast sweep.
const HOVER_INTENT_MS: u128 = 45;
/// Tiny jitter guard (~2 polls) so leaving the open island collapses immediately without 1px edge flicker.
const LEAVE_GRACE_MS: u128 = 35;
const THIN_LINE_W: f64 = 96.0;
const THIN_LINE_H: f64 = 6.0;
/// (1.8.7) the thin line stays this long after it last looked over the browser's tabs
const LINE_KEEP: std::time::Duration = std::time::Duration::from_millis(1500);
/// how much of the browser window's top is its tab strip, DIPs (a maximized window starts 8px
/// above the screen)
const TAB_BAND: f64 = 56.0;
// The island's position is an "anchor": the top-center of the collapsed pill, in screen
// coordinates. The window (always fully on screen) follows it, and the renderer places the
// island inside the window so it expands toward the free space near screen edges.
const PILL_HALF_W: f64 = 120.0; // half of the widest collapsed pill (keep in sync with style.css)
const PILL_H: f64 = 34.0;
const FLOAT_TOP: f64 = 8.0; // gap above a floating island inside the window
const SNAP_PX: f64 = 28.0; // distance from the top edge that docks the island
const CENTER_SNAP_PX: f64 = 40.0; // horizontal magnet to the screen center while docked
// In vertical mode the anchor is the middle of the pill on the left or right screen edge.
const V_PILL_HALF_H: f64 = 124.0;
// (1.8.7) Nova's eyes' reach: this close to the island's edge (DIPs). It was 260 from its middle,
// a wide band over the browser's tabs and address bar, so a cursor resting there for a while
// brought the eyes up with nobody near the island.
const NEAR_EDGE_PX: f64 = 100.0;
const FAR_PX: f64 = 400.0;

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// JS Math.round
fn round(v: f64) -> f64 {
    nova_core::json::js_round(v)
}

#[derive(Clone, Copy)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

struct Drag {
    ox: f64,
    oy: f64,
}

/// The page message that dims the island (handled by the app, not sent to the page).
pub const DIM: &str = "nova-dim";

pub struct Core {
    pub win: Window,
    pub send: Box<dyn Fn(&str, Value)>,
    pub store: Store,
    pub features: Features,
    pub prefs: Map<String, Value>,
    anchor: Option<Point>,
    last_layout: String,
    omni: Option<Value>,
    omni_key: String,
    drag: Option<Drag>,
    drag_from: Option<Point>,
    pub user_hidden: bool,
    pub fullscreen_hidden: bool,
    tab_rects: Vec<Rect>,
    /// (1.8.7) "Thin line" in the browser: over the tabs the island is a line along the top, and
    /// the pill it was, on the screen (what "over the tabs" is measured with while it's a line)
    line: bool,
    line_rect: Rect,
    /// since when it no longer looks over the tabs while it's a line (it waits LINE_KEEP first)
    line_off_since: Option<Instant>,
    /// the browser in front, its window (DIPs): the line's "over the tabs" is the top of it
    browser_rect: Option<Rect>,
    ghost: bool,
    ghost_open: bool,
    dimmed: bool,
    clickable: bool,
    pinned: bool,
    calc_open: bool,
    calc_closing: bool,
    pub timer_active: bool,
    pub calc_rect: Rect,
    pub island_rect: Rect,
    /// The pill at rest (not opened, no notification): what "over a tab" is decided on, so the
    /// opened island covering a tab doesn't close itself.
    rest_rect: Rect,
    /// A new size of the pill at rest, taken once it has held still (not mid-animation).
    pending_rest: Option<(Rect, Instant)>,
    inside: bool,
    active: bool,
    entered_at: Instant,
    left_at: Option<Instant>,
    suppress_until_leave: bool,
    near_sent: Option<(f64, f64)>,
    pub cursor_far: bool,
}

impl Core {
    pub fn new(win: Window, send: Box<dyn Fn(&str, Value)>, store: Store, features: Features) -> Core {
        let mut prefs: Map<String, Value> = json!({ "ax": null, "ay": null, "docked": false, "vertical": false, "side": "right" }).as_object().unwrap().clone();
        if let Some(Value::Object(saved)) = store.load("settings.json") {
            for (k, v) in saved {
                prefs.insert(k, v);
            }
        }
        Core {
            win, send, store, features, prefs,
            anchor: None, last_layout: String::new(), omni: None, omni_key: String::new(), drag: None, drag_from: None,
            user_hidden: false, fullscreen_hidden: false, tab_rects: Vec::new(), line: false, line_rect: Rect::default(), line_off_since: None, browser_rect: None, ghost: false, ghost_open: false, dimmed: false, clickable: false, pinned: false,
            calc_open: false, calc_closing: false, timer_active: false, calc_rect: Rect::default(),
            island_rect: Rect::default(), rest_rect: Rect::default(), pending_rest: None, inside: false, active: false, entered_at: Instant::now(), left_at: None, suppress_until_leave: false,
            near_sent: None, cursor_far: false,
        }
    }

    fn pref_bool(&self, k: &str) -> bool {
        self.prefs.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    fn vertical(&self) -> bool {
        self.pref_bool("vertical")
    }
    fn docked(&self) -> bool {
        self.pref_bool("docked")
    }
    fn side(&self) -> String {
        self.prefs.get("side").and_then(Value::as_str).unwrap_or("right").to_string()
    }

    fn default_anchor() -> Point {
        let b = screen::primary().bounds;
        Point { x: b.x + b.width / 2.0, y: b.y + FLOAT_TOP }
    }

    // --- the address bar slot ---
    // While a Chromium browser is in front the island takes the right third of its address bar.
    // Only from the island's default place: never while vertical or after the user put it somewhere.
    pub fn omni_allowed(&self) -> bool {
        self.features.bool("addrIsland") && !self.vertical()
    }

    pub fn on_omni(&mut self, o: &Value) {
        let next = if o.is_object() && self.omni_allowed() { Some(o.clone()) } else { None };
        let key = next.as_ref().map(|n| format!("{},{},{},{},{}", n["x"], n["y"], n["w"], n["h"], n["focus"])).unwrap_or_default();
        if key == self.omni_key {
            return;
        }
        self.omni_key = key;
        self.omni = next;
        self.layout();
    }

    fn emit_layout(&mut self, msg: Value) {
        let key = msg.to_string();
        if key != self.last_layout {
            self.last_layout = key;
            (self.send)("anchor", msg);
        }
    }

    fn win_h_for(&self, bounds_h: f64) -> f64 {
        if self.calc_open { CALC_WIN_H.min(bounds_h.max(WIN_H)) } else { WIN_H }
    }

    fn layout_address_bar(&mut self) {
        let o = self.omni.clone().unwrap();
        // the helper reports physical pixels; place windows in DIPs
        let r = screen::rect_to_dip(o["x"].as_i64().unwrap_or(0) as i32, o["y"].as_i64().unwrap_or(0) as i32, o["w"].as_i64().unwrap_or(0) as i32, o["h"].as_i64().unwrap_or(0) as i32);
        let b = screen::matching(r).bounds;
        let wh = self.win_h_for(b.height);
        let third = r.width / 3.0;
        let sh = round(30f64.min(22f64.max(r.height + 6.0)));
        let sw = round((third - 12.0).min(380.0));
        let cx = r.x + r.width - third / 2.0; // centre of the right third
        let top = round(r.y + r.height / 2.0 - sh / 2.0);
        let wx = round(clamp(cx - WIN_W / 2.0, b.x, b.x + b.width - WIN_W));
        let wy = round(clamp(top - FLOAT_TOP, b.y, (b.y + b.height - wh).max(b.y)));
        self.win.set_rect(wx, wy, WIN_W, wh);
        let msg = json!({ "x": round(cx - wx), "y": top - wy, "docked": false, "vertical": false, "side": self.side(), "addr": if o["focus"] == true { "line" } else { "slim" }, "sw": sw, "sh": sh });
        self.emit_layout(msg);
    }

    /// The page has loaded: it gets the pill's place again (nova.js did-finish-load: what was
    /// sent before it was listening is lost, and a vertical island drew itself flat at the edge).
    pub fn relayout(&mut self) {
        self.last_layout.clear();
        self.layout();
    }

    // Clamp the anchor to the display, position the window around it and tell the renderer
    // where the pill sits inside the window.
    pub fn layout(&mut self) {
        let Some(mut a) = self.anchor else { return };
        // not while the user drags the island somewhere else
        if self.omni.is_some() && self.drag.is_none() && self.omni_allowed() {
            self.layout_address_bar();
            return;
        }
        let d = screen::nearest(round(a.x), round(a.y));
        let (b, wa) = (d.bounds, d.work_area);
        let wh = self.win_h_for(wa.height);
        let (wx, wy);
        if self.vertical() {
            a.x = if self.side() == "left" { b.x } else { b.x + b.width };
            a.y = clamp(a.y, wa.y + V_PILL_HALF_H, wa.y + wa.height - V_PILL_HALF_H);
            wx = if self.side() == "left" { b.x } else { b.x + b.width - WIN_W };
            wy = round(clamp(a.y - WIN_H / 2.0, wa.y, (wa.y + wa.height - wh).max(wa.y)));
        } else {
            a.x = clamp(a.x, b.x + PILL_HALF_W, b.x + b.width - PILL_HALF_W);
            a.y = if self.docked() { b.y } else { clamp(a.y, b.y, wa.y + wa.height - PILL_H - 6.0) };
            wx = round(clamp(a.x - WIN_W / 2.0, b.x, b.x + b.width - WIN_W));
            wy = round(if self.docked() { b.y } else { clamp(a.y - FLOAT_TOP, b.y, (wa.y + wa.height - wh).max(b.y)) });
        }
        // (1.8.7) the place the user set stays as it is: only where it shows is kept inside the
        // display. Writing the kept-inside place back made it the island's own: started before a
        // second display was there (Windows' start, waking from sleep), the island stayed on the
        // first display when the second came, and the next save lost its place for good.
        // Dragging sets the place itself (follow_cursor), so it's always on a display then.
        // over the browser's tabs: the thin line, flush with the screen's top edge wherever the
        // island sits (renderer.js draws it as in the address bar; opened, it comes down from
        // there as a docked island does). The island's own place stays as it was.
        if self.line && !self.vertical() {
            self.win.set_rect(wx, b.y, WIN_W, wh);
            self.emit_layout(json!({ "x": round(a.x - wx), "y": 0, "docked": true, "vertical": false, "side": self.side(), "addr": "line" }));
            return;
        }
        self.win.set_rect(wx, wy, WIN_W, wh);
        let msg = json!({ "x": round(a.x - wx), "y": round(a.y - wy), "docked": !self.vertical() && self.docked(), "vertical": self.vertical(), "side": self.side() });
        self.emit_layout(msg);
    }

    /// (1.8.7) Settings › In the browser › Thin line.
    fn browser_line(&self) -> bool {
        self.features.bool("browserLine") && !self.features.bool("addrIsland") && !self.vertical()
    }

    /// Exact window-local geometry of the collapsed Chrome "Thin line" bar (96x6 at top edge).
    fn thin_line_rect(&self) -> Rect {
        let (wx, _) = self.win.pos();
        let ax = self.anchor.map(|a| a.x).unwrap_or(wx + WIN_W / 2.0);
        let local_x = clamp(round(ax - wx) - THIN_LINE_W / 2.0, 4.0, WIN_W - THIN_LINE_W - 4.0);
        Rect { x: local_x, y: 0.0, width: THIN_LINE_W, height: THIN_LINE_H }
    }

    /// Snaps `island_rect` immediately to the collapsed target rect on close or thin-line entry so
    /// a shrinking CSS transition can never leave an oversized hit-box over Chrome tabs.
    fn snap_collapsed_rect(&mut self) {
        if self.line && !self.vertical() {
            self.island_rect = self.thin_line_rect();
        } else if self.rest_rect.width > 0.0 && self.rest_rect.height > 0.0 {
            self.island_rect = self.rest_rect;
        }
    }

    /// The island is the thin line now (the glass isn't captured for it).
    pub fn is_line(&self) -> bool {
        self.line && !self.active
    }

    /// Thin line: the island becomes a line while it sits over a tab of the browser in front.
    /// Nothing is dimmed and nothing waits: the line lets the clicks through, and opens the
    /// island when the cursor rests on it.
    fn update_line(&mut self) {
        let want = if self.browser_line() {
            // while it's a line its own size and place are the line's: "over the tabs" is the pill
            // it was, where it was (line_rect, on the screen)
            let r = if self.line {
                self.line_rect
            } else {
                let (wx, wy) = self.win.pos();
                Rect { x: wx + self.rest_rect.x, y: wy + self.rest_rect.y, ..self.rest_rect }
            };
            // the top of the browser in front: its tab strip (no need to read the tabs one by one
            // for this, UI Automation every second; the window's place is free)
            let band = self.browser_rect.map(|b| Rect { x: b.x, y: b.y, width: b.width, height: TAB_BAND });
            r.width > 0.0 && band.is_some_and(|t| r.x < t.x + t.width && r.x + r.width > t.x && r.y < t.y + t.height && r.y + r.height > t.y)
        } else {
            false
        };
        // a read of the tabs that comes back empty (the browser busy, a tooltip or menu in front
        // for a moment), or the island over the gap between two tabs, isn't the island leaving
        // them: it stays a line until it has looked off the tabs for LINE_KEEP
        if want || !self.browser_line() {
            self.line_off_since = None;
        } else if self.line {
            let since = *self.line_off_since.get_or_insert_with(Instant::now);
            if since.elapsed() < LINE_KEEP {
                return;
            }
            self.line_off_since = None;
        }
        if want != self.line {
            if want {
                let (wx, wy) = self.win.pos();
                self.line_rect = Rect { x: wx + self.rest_rect.x, y: wy + self.rest_rect.y, ..self.rest_rect };
            }
            self.line = want;
            nova_core::log::info(if want { "browser line: on (over the tabs)" } else { "browser line: off" });
            self.layout();
            if !self.active && !self.clickable && !self.pinned && !self.ghost_open {
                self.snap_collapsed_rect();
            }
        }
    }

    pub fn place_window(&mut self) {
        let (ax, ay) = (self.prefs.get("ax").and_then(Value::as_f64), self.prefs.get("ay").and_then(Value::as_f64));
        self.anchor = Some(match (ax, ay) {
            (Some(x), Some(y)) => Point { x, y },
            _ => Self::default_anchor(),
        });
        // settings from before "placed" existed: judge by where the island is
        if !self.prefs.get("placed").is_some_and(Value::is_boolean) {
            let placed = !self.at_default_place();
            self.prefs.insert("placed".into(), json!(placed));
        }
        self.layout();
    }

    // the island's own spot: horizontal, at the top, centered on its display
    fn at_default_place(&self) -> bool {
        let Some(a) = self.anchor else { return false };
        if self.vertical() {
            return false;
        }
        let b = screen::nearest(round(a.x), round(a.y)).bounds;
        (a.x - (b.x + b.width / 2.0)).abs() < 2.0 && a.y - b.y <= FLOAT_TOP + 2.0
    }

    fn save_position(&mut self) {
        if let Some(a) = self.anchor {
            self.prefs.insert("ax".into(), json!(round(a.x)));
            self.prefs.insert("ay".into(), json!(round(a.y)));
        }
        let _ = self.store.save("settings.json", &Value::Object(self.prefs.clone()));
    }

    pub fn reset_position(&mut self) {
        for k in ["docked", "vertical", "placed"] {
            self.prefs.insert(k.into(), json!(false));
        }
        self.anchor = Some(Self::default_anchor());
        self.layout();
        self.save_position();
    }

    // Switch between the horizontal island and a vertical one docked to the nearest side.
    pub fn set_vertical(&mut self, on: bool) {
        let Some(a) = self.anchor else { return };
        if on == self.vertical() {
            return;
        }
        let b = screen::nearest(round(a.x), round(a.y)).bounds;
        if on {
            let side = if a.x < b.x + b.width / 2.0 { "left" } else { "right" };
            self.prefs.insert("side".into(), json!(side));
            self.anchor = Some(Point { x: a.x, y: a.y + V_PILL_HALF_H });
        } else {
            self.prefs.insert("docked".into(), json!(false));
            let x = if self.side() == "left" { b.x + 260.0 } else { b.x + b.width - 260.0 };
            self.anchor = Some(Point { x, y: a.y - V_PILL_HALF_H });
        }
        self.prefs.insert("vertical".into(), json!(on));
        // leaving the address bar at once (it would otherwise stay until the bar itself changes)
        if on {
            self.omni = None;
            self.omni_key.clear();
        }
        self.layout();
        self.save_position();
    }

    // --- dragging ---
    pub fn drag_start(&mut self) -> bool {
        let Some(a) = self.anchor else { return false };
        if self.drag.is_some() {
            return false;
        }
        let (px, py) = screen::cursor();
        self.drag_from = Some(a);
        // grab the (shrinking) pill near where it was pressed so it stays under the cursor
        let ox = clamp(px - a.x, -PILL_HALF_W + 16.0, PILL_HALF_W - 16.0);
        let oy = if self.vertical() { clamp(py - a.y, -V_PILL_HALF_H + 20.0, V_PILL_HALF_H - 20.0) } else { clamp(py - a.y, 4.0, PILL_H - 6.0) };
        self.drag = Some(Drag { ox, oy });
        true
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// True when the island is expanded, pinned open, or showing a clickable card.
    pub fn is_open(&self) -> bool {
        self.active || self.clickable || self.pinned
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn follow_cursor(&mut self) {
        let Some(d) = self.drag.as_ref() else { return };
        let (ox, oy) = (d.ox, d.oy);
        let (px, py) = screen::cursor();
        let b = screen::nearest(px, py).bounds;
        if self.vertical() {
            // slides along the edge; crossing the middle of the screen moves it to the other side
            let side = if px < b.x + b.width / 2.0 { "left" } else { "right" };
            self.prefs.insert("side".into(), json!(side));
            self.anchor = Some(Point { x: px, y: py - oy });
            self.layout();
            return;
        }
        let mut next = Point { x: px - ox, y: py - oy };
        let docked = next.y - b.y < SNAP_PX;
        if docked {
            next.y = b.y;
            let center = b.x + b.width / 2.0;
            if (next.x - center).abs() < CENTER_SNAP_PX {
                next.x = center;
            }
        }
        self.prefs.insert("docked".into(), json!(docked));
        self.anchor = Some(next);
        self.layout();
    }

    pub fn drag_end(&mut self) {
        if self.drag.take().is_none() {
            return;
        }
        // a real move decides whether the island stays where the user put it (no address-bar jump)
        if let (Some(from), Some(a)) = (self.drag_from, self.anchor) {
            if (a.x - from.x).hypot(a.y - from.y) > 20.0 {
                let placed = !self.at_default_place();
                self.prefs.insert("placed".into(), json!(placed));
            }
        }
        self.drag_from = None;
        self.save_position();
        // don't pop the island open right after a move; re-arm when the cursor leaves
        self.suppress_until_leave = true;
        if self.active {
            self.set_active(false);
        } else {
            (self.send)("hover", json!(false));
            // dropped onto the tabs, or off them
            self.update_ghost();
        }
    }

    pub fn update_visibility(&mut self) {
        let show = !self.user_hidden && !(self.fullscreen_hidden && self.features.bool("hideFullscreen"));
        if show && !self.win.visible() {
            self.win.show_inactive();
        }
        if !show && self.win.visible() {
            self.win.hide();
        }
        (self.send)("visibility", json!(show));
    }

    // --- "Stay in place" in the browser (Address bar off) ---
    // 1.7 let clicks through whenever the island was over the browser window; 1.8 only while a
    // tab is under it (the tabs of the browser in front, from nova-sys), and dims it too.
    /// The browser in front, its window (nova-sys "browser", physical pixels), or null.
    pub fn on_browser(&mut self, b: &Value) {
        let next = b.as_object().map(|o| {
            let n = |k: &str| o.get(k).and_then(Value::as_i64).unwrap_or(0) as i32;
            screen::rect_to_dip(n("x"), n("y"), n("w"), n("h"))
        });
        let changed = next != self.browser_rect;
        self.browser_rect = next;
        if self.browser_rect.is_none() && !self.tab_rects.is_empty() {
            self.tab_rects.clear();
            self.update_ghost();
            return;
        }
        if changed && !self.active && !self.clickable {
            self.update_line();
        }
    }

    pub fn on_tabs(&mut self, t: &Value) {
        let px = |v: &Value, i: usize| v.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
        let next: Vec<Rect> = t.as_array().map(|a| a.iter().map(|r| screen::rect_to_dip(px(r, 0), px(r, 1), px(r, 2), px(r, 3))).collect()).unwrap_or_default();
        if next != self.tab_rects {
            self.tab_rects = next;
            self.update_ghost();
        }
    }

    pub fn update_ghost(&mut self) {
        let mut over = false;
        // opened (by the mouse or a notification) or pinned: stays opaque and interactive until closed
        if self.active || self.clickable || self.pinned {
            if self.ghost {
                self.ghost = false;
            }
            self.update_dim();
            return;
        }
        self.update_line();
        if !self.features.bool("addrIsland") && !self.browser_line() && self.rest_rect.width > 0.0 {
            let (wx, wy) = self.win.pos();
            if !wx.is_nan() && !wy.is_nan() {
                let (x, y, r) = (wx + self.rest_rect.x, wy + self.rest_rect.y, self.rest_rect);
                over = self.tab_rects.iter().any(|t| x < t.x + t.width && x + r.width > t.x && y < t.y + t.height && y + r.height > t.y);
            }
        }
        if !over {
            self.ghost_open = false;
        }
        let on = over && !self.ghost_open;
        if on != self.ghost {
            self.ghost = on;
            if on {
                self.set_active(false);
            }
        }
        self.update_dim();
    }

    /// Gentle dim while the idle pill lets clicks through to a browser tab underneath; active,
    /// pinned, toggled, or clickable states always display at full opacity.
    fn update_dim(&mut self) {
        let dim = self.ghost && !self.clickable && !self.active && !self.pinned && !self.ghost_open;
        if dim != self.dimmed {
            self.dimmed = dim;
            (self.send)(DIM, json!(dim));
        }
    }

    /// Re-sends the current dim state after a page reload (`island-ready`).
    pub fn sync_dim(&self) {
        (self.send)(DIM, json!(self.dimmed));
    }

    pub fn set_clickable(&mut self, on: bool) {
        self.clickable = on;
        if on {
            self.ghost = false;
            if self.inside && !self.suppress_until_leave {
                self.set_active(true);
            }
            self.update_dim();
            return;
        }
        // Interactive toast/banner closed: return to click-through only when cursor is outside
        // and the island isn't pinned or toggled open.
        if !self.inside && !self.ghost_open && !self.pinned {
            self.set_active(false);
            self.update_ghost();
        } else {
            self.update_dim();
        }
    }

    pub fn set_pinned(&mut self, on: bool) {
        self.pinned = on;
        if on {
            self.ghost_open = true;
            self.ghost = false;
            if self.inside && !self.suppress_until_leave {
                self.set_active(true);
            }
            self.update_dim();
        } else {
            self.ghost_open = false;
            if !self.inside && !self.clickable {
                self.set_active(false);
            }
            self.update_ghost();
        }
    }

    /// Ctrl+Alt+I
    pub fn toggle(&mut self) {
        if !self.pinned {
            self.ghost_open = true;
            self.ghost = false;
            if self.inside && !self.suppress_until_leave {
                self.set_active(true);
            }
            self.update_dim();
        }
        (self.send)("toggle", Value::Null);
    }

    pub fn set_island_rect(&mut self, r: Rect) {
        let is_expanded_rect = r.width > 410.0 && r.height > 70.0;
        let idle_closed = !self.active && !self.clickable && !self.pinned && !self.ghost_open;
        if idle_closed && self.line && (r.width > THIN_LINE_W + 14.0 || r.height > THIN_LINE_H + 10.0) {
            // Ignore stale/mid-shrink DOM rects while collapsed to the Chrome thin line.
            self.island_rect = self.thin_line_rect();
            return;
        }
        if idle_closed && is_expanded_rect {
            self.snap_collapsed_rect();
            self.pending_rest = None;
            return;
        }
        self.island_rect = r;
        if idle_closed && !is_expanded_rect {
            self.pending_rest = Some((r, Instant::now()));
        } else if self.pinned || self.ghost_open || is_expanded_rect {
            self.pending_rest = None;
        }
    }

    fn set_active(&mut self, on: bool) {
        if on == self.active {
            return;
        }
        self.active = on;
        if on {
            self.left_at = None;
        }
        self.win.set_ignore_mouse(!on);
        if !on {
            self.win.set_focusable(false);
            if !self.clickable && !self.pinned && !self.ghost_open {
                self.snap_collapsed_rect();
            }
        } else {
            self.ghost = false;
        }
        (self.send)("hover", json!(on));
        if on {
            self.update_dim();
        } else {
            self.update_ghost();
        }
    }

    /// The cursor is out of the eyes' reach: the page starts counting again
    /// the next time it comes near.
    fn clear_near(&mut self) {
        if self.near_sent.take().is_some() {
            (self.send)("near", Value::Null);
        }
    }

    // Nova's eyes follow the cursor while it is near the island (sent only on real movement).
    // `gap`: how far the cursor is from the island's edge (0 on it).
    fn send_near(&mut self, x: f64, y: f64, gap: f64) {
        // (1.8.7) not while the island is open or the cursor is on it: after it closes, the wait
        // starts over (the eyes came at once then, the cursor never having left)
        if !self.features.bool("novaEyes") || self.island_rect.width == 0.0 || self.active || self.inside || gap > NEAR_EDGE_PX {
            self.clear_near();
            return;
        }
        let r = self.island_rect;
        let dx = round(x - (r.x + r.width / 2.0));
        let dy = round(y - (r.y + r.height / 2.0));
        if let Some((sx, sy)) = self.near_sent {
            if (sx - dx).abs() < 3.0 && (sy - dy).abs() < 3.0 {
                return;
            }
        }
        self.near_sent = Some((dx, dy));
        (self.send)("near", json!({ "dx": dx, "dy": dy }));
    }

    // --- hover detection with intent ---
    // The island only becomes interactive after the cursor rests on it, so a quick sweep to a
    // browser tab underneath never gets caught.
    pub fn poll_cursor(&mut self) {
        let (px, py) = screen::cursor();
        let down = unsafe {
            (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
                windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON.0 as i32,
            ) as u16
                & 0x8000)
                != 0
        };
        self.poll_cursor_state(px, py, Instant::now(), down);
    }

    pub fn poll_cursor_at(&mut self, px: f64, py: f64, now_ts: Instant) {
        self.poll_cursor_state(px, py, now_ts, false);
    }

    pub fn poll_cursor_state(&mut self, px: f64, py: f64, now_ts: Instant, mouse_down: bool) {
        // the line waiting to go: looked at again until it's back over the tabs or LINE_KEEP is up
        if self.line_off_since.is_some() && !self.active {
            self.update_line();
        }
        // the pill's size at rest, once it has held still (never an expanded panel rect)
        if let Some((r, t)) = self.pending_rest {
            let is_expanded_rect = r.width > 410.0 && r.height > 70.0;
            if self.active || self.clickable || self.pinned || self.ghost_open || is_expanded_rect {
                self.pending_rest = None;
            } else if now_ts.saturating_duration_since(t).as_millis() >= 550 {
                self.pending_rest = None;
                self.rest_rect = r;
                self.update_ghost();
            }
        }
        if !self.win.visible() || self.drag.is_some() {
            // (1.8.7) hidden or moved: a "near" from before would bring the eyes at once later
            self.clear_near();
            return;
        }
        let (bx, by) = self.win.pos();
        let (x, y) = (px - bx, py - by);
        let r = self.island_rect;
        let fx = (r.x - x).max(0.0).max(x - r.x - r.width);
        let fy = (r.y - y).max(0.0).max(y - r.y - r.height);
        self.cursor_far = !self.calc_active() && fx.hypot(fy) > FAR_PX;
        self.send_near(x, y, fx.hypot(fy));
        // Tight hit-testing when collapsed (especially the 96x6 Chrome thin line, so approaching
        // browser tabs 1 cm away never triggers it); comfortable edge margin while actively open.
        let (pad_x, pad_y) = if self.active {
            (8.0, 8.0)
        } else if self.line {
            (1.0, 2.0)
        } else {
            (2.0, 2.0)
        };
        let min_x = if self.vertical() && self.side() == "left" { -6.0 } else { r.x - pad_x };
        let max_x = if self.vertical() && self.side() == "right" { WIN_W + 6.0 } else { r.x + r.width + pad_x };
        let min_y = if !self.vertical() && (self.docked() || self.line || r.y <= 14.0) { -6.0 } else { r.y - pad_y };
        let max_y = r.y + r.height + pad_y;
        let in_island = r.width > 0.0 && r.height > 0.0 && x >= min_x && x <= max_x && y >= min_y && y <= max_y;
        let c = self.calc_rect;
        let in_calc = self.calc_active() && c.width > 0.0 && c.height > 0.0 && x >= c.x - 4.0 && x <= c.x + c.width + 4.0 && y >= c.y - 4.0 && y <= c.y + c.height + 4.0;
        let now = in_island || in_calc;
        if now && !self.inside {
            self.entered_at = now_ts;
        }
        self.inside = now;
        if self.calc_active() {
            self.left_at = None;
            self.win.set_ignore_mouse(!now);
            if now && !self.active && !self.suppress_until_leave {
                self.set_active(true);
            }
            return;
        }
        let instant_wake = self.clickable || self.pinned || self.ghost_open;
        if !now {
            self.suppress_until_leave = false;
            if self.active {
                if mouse_down {
                    // Holding a slider / scrollbar near the border must never collapse the island.
                    self.left_at = None;
                    return;
                }
                let since = *self.left_at.get_or_insert(now_ts);
                if now_ts.saturating_duration_since(since).as_millis() >= LEAVE_GRACE_MS {
                    self.left_at = None;
                    self.set_active(false);
                }
            } else {
                self.left_at = None;
            }
        } else {
            self.left_at = None;
            if !self.active && !self.suppress_until_leave && (instant_wake || (!self.ghost && now_ts.saturating_duration_since(self.entered_at).as_millis() >= HOVER_INTENT_MS)) {
                self.set_active(true);
            }
        }
    }

    pub fn set_calc_open(&mut self, on: bool) -> bool {
        if on == self.calc_open && (!on || !self.calc_closing) {
            return false;
        }
        self.calc_open = on;
        self.calc_closing = false;
        if !on {
            self.calc_rect = Rect::default();
        }
        self.layout();
        if !on && !self.win.hwnd.0.is_null() {
            self.poll_cursor();
        }
        true
    }

    pub fn set_calc_rect(&mut self, r: Option<Rect>) {
        self.calc_rect = r.unwrap_or_default();
    }

    pub fn calc_active(&self) -> bool {
        self.calc_open && !self.calc_closing
    }

    pub fn inside(&self) -> bool {
        self.inside
    }

    pub fn prepare_calc_blur(&mut self) -> bool {
        if !self.calc_active() {
            return false;
        }
        self.calc_closing = true;
        self.win.set_focusable(false);
        self.win.set_ignore_mouse(true);
        true
    }

    /// Faded over a browser's tabs (Stay in place), as the page is told by DIM.
    pub fn dimmed(&self) -> bool {
        self.dimmed
    }

    /// The cursor is within Nova's eyes' reach (they show in the page then).
    pub fn cursor_near(&self) -> bool {
        self.near_sent.is_some()
    }

    /// How long until the next cursor check (nova.js cursorLoop).
    pub fn next_poll_ms(&self) -> u32 {
        if !self.win.visible() {
            600
        } else if self.calc_active() || !self.cursor_far {
            20
        } else {
            80
        }
    }

    /// Text inputs (app search) need keyboard focus; the window is otherwise non-focusable.
    pub fn focus_input(&mut self, on: bool) {
        self.win.set_focusable(on);
        if on {
            self.win.focus();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use windows::Win32::Foundation::HWND;

    use super::*;

    fn test_core() -> (Core, Rc<RefCell<Vec<(String, Value)>>>) {
        let dir = std::env::temp_dir().join(format!("nova-core-test-{}", std::process::id()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let ev = events.clone();
        let send = Box::new(move |ch: &str, val: Value| {
            ev.borrow_mut().push((ch.to_string(), val));
        });
        let win = Window::new(HWND::default());
        let mut core = Core::new(win, send, Store::new(dir.clone()), Features::load(Store::new(dir)));
        let b = screen::primary().bounds;
        let wx = b.x + (b.width - WIN_W) / 2.0;
        let wy = b.y;
        core.win.set_pos(wx, wy);
        let pill = Rect { x: 202.0, y: 8.0, width: 236.0, height: 32.0 };
        core.island_rect = pill;
        core.rest_rect = pill;
        (core, events)
    }

    fn overlapping_tab_json(core: &Core) -> Value {
        let (wx, wy) = core.win.pos();
        let dip_x = wx + core.rest_rect.x - 10.0;
        let dip_y = wy + core.rest_rect.y - 4.0;
        let dip_w = core.rest_rect.width + 20.0;
        let dip_h = core.rest_rect.height + 8.0;
        let d = screen::nearest(dip_x, dip_y);
        let (px, py) = d.to_phys(dip_x, dip_y);
        let pw = (dip_w * d.scale).round() as i32;
        let ph = (dip_h * d.scale).round() as i32;
        json!([[px, py, pw, ph]])
    }

    fn pill_center(core: &Core) -> (f64, f64) {
        let (wx, wy) = core.win.pos();
        (wx + core.island_rect.x + core.island_rect.width / 2.0, wy + core.island_rect.y + core.island_rect.height / 2.0)
    }

    #[test]
    fn tab_overlap_enables_ghost_and_dim_and_preserves_click_through() {
        let (mut core, events) = test_core();
        let tabs = overlapping_tab_json(&core);
        core.on_tabs(&tabs);

        assert!(core.ghost, "ghost should turn on over tabs");
        assert!(core.dimmed(), "idle pill should dim over tabs");
        assert!(core.win.ignore_mouse(), "idle ghost pill must remain click-through");
        assert!(events.borrow().iter().any(|(c, v)| c == DIM && v == &json!(true)));

        let (cx, cy) = pill_center(&core);
        let t0 = Instant::now();
        core.poll_cursor_at(cx, cy, t0);
        core.poll_cursor_at(cx, cy, t0 + Duration::from_millis(400));

        assert!(!core.active(), "hovering idle ghost pill over tabs must stay click-through");
        assert!(core.win.ignore_mouse());
        assert!(core.dimmed());
    }

    #[test]
    fn clickable_prompt_is_opaque_and_clickable_without_hover_delay() {
        let (mut core, _) = test_core();
        core.on_tabs(&overlapping_tab_json(&core));
        assert!(core.dimmed());
        assert!(core.win.ignore_mouse());

        core.set_clickable(true);
        assert!(!core.dimmed(), "clickable notification over tabs must not appear dimmed");

        let (cx, cy) = pill_center(&core);
        let t0 = Instant::now();
        // Immediate entry (0ms elapsed < HOVER_INTENT_MS) must unlock clicks immediately.
        core.poll_cursor_at(cx, cy, t0);
        assert!(core.active(), "clickable island must capture fast clicks without 280ms delay");
        assert!(!core.win.ignore_mouse());
        assert!(!core.dimmed());
    }

    #[test]
    fn clearing_clickable_while_cursor_inside_keeps_interaction_until_leave() {
        let (mut core, _) = test_core();
        core.on_tabs(&overlapping_tab_json(&core));
        core.set_clickable(true);

        let (cx, cy) = pill_center(&core);
        let t0 = Instant::now();
        core.poll_cursor_at(cx, cy, t0);
        assert!(core.active());

        // Interactive toast expires while the user still hovers the island.
        core.set_clickable(false);
        assert!(core.active(), "clearing clickable under cursor must not drop clicks mid-interaction");
        assert!(!core.win.ignore_mouse());
        assert!(!core.dimmed(), "active island must remain opaque while cursor is inside");

        // Moving the cursor off cleanly resumes click-through over tabs after leave grace.
        let (wx, wy) = core.win.pos();
        core.poll_cursor_at(wx - 50.0, wy + 250.0, t0 + Duration::from_millis(50));
        core.poll_cursor_at(wx - 50.0, wy + 250.0, t0 + Duration::from_millis(320));
        assert!(!core.active());
        assert!(core.win.ignore_mouse());
        assert!(core.dimmed());
    }

    #[test]
    fn pinned_and_toggled_over_tabs_stays_opaque_and_responsive() {
        let (mut core, _) = test_core();
        core.on_tabs(&overlapping_tab_json(&core));
        assert!(core.dimmed());

        core.toggle();
        assert!(!core.dimmed(), "Ctrl+Alt+I toggle must clear dim over tabs");
        core.set_pinned(true);
        assert!(!core.dimmed());

        let (cx, cy) = pill_center(&core);
        core.poll_cursor_at(cx, cy, Instant::now());
        assert!(core.active(), "pinned island must respond immediately to cursor entry");
        assert!(!core.win.ignore_mouse());
        assert!(!core.dimmed());

        let (wx, wy) = core.win.pos();
        let t_out = Instant::now();
        core.poll_cursor_at(wx - 40.0, wy + 250.0, t_out);
        core.poll_cursor_at(wx - 40.0, wy + 250.0, t_out + Duration::from_millis(300));
        core.set_pinned(false);
        assert!(!core.active());
        assert!(core.win.ignore_mouse());
        assert!(core.dimmed(), "unpinning outside cursor returns cleanly to click-through");
    }

    #[test]
    fn leaving_browser_clears_stale_tabs_and_dim() {
        let (mut core, _) = test_core();
        let (wx, _wy) = core.win.pos();
        core.on_browser(&json!({ "x": wx as i32, "y": 0, "w": 1200, "h": 800 }));
        core.on_tabs(&overlapping_tab_json(&core));
        assert!(core.dimmed());

        core.on_browser(&Value::Null);
        assert!(!core.dimmed(), "leaving browser must clear stale tab dimming immediately");
        assert!(!core.ghost);

        let (cx, cy) = pill_center(&core);
        let t0 = Instant::now();
        core.poll_cursor_at(cx, cy, t0);
        core.poll_cursor_at(cx, cy, t0 + Duration::from_millis(320));
        assert!(core.active(), "island must open normally on hover once browser is gone");
        assert!(!core.win.ignore_mouse());
    }

    #[test]
    fn expanded_rect_never_poisons_rest_rect() {
        let (mut core, _) = test_core();
        assert_eq!(core.rest_rect.width, 236.0);

        core.set_island_rect(Rect { x: 46.0, y: 8.0, width: 548.0, height: 232.0 });
        let (wx, wy) = core.win.pos();
        core.poll_cursor_at(wx - 50.0, wy + 250.0, Instant::now() + Duration::from_millis(650));

        assert_eq!(core.rest_rect.width, 236.0, "expanded panel dimensions must never overwrite rest_rect");
    }

    #[test]
    fn calculator_extends_height_keeps_hover_and_restores_on_close() {
        let (mut core, _) = test_core();
        core.place_window();
        assert_eq!(core.win.size(), (WIN_W, WIN_H));

        assert!(core.set_calc_open(true));
        assert!(core.calc_active());
        assert_eq!(core.win.size().0, WIN_W);
        assert!(core.win.size().1 > WIN_H);

        let calc_r = Rect { x: 300.0, y: 52.0, width: 260.0, height: 340.0 };
        core.set_calc_rect(Some(calc_r));
        core.set_pinned(true);

        let (wx, wy) = core.win.pos();
        let t0 = Instant::now();
        // Cursor over calculator panel below expanded island keeps interaction active
        core.poll_cursor_at(wx + 420.0, wy + 360.0, t0);
        assert!(core.active());
        assert!(!core.win.ignore_mouse(), "cursor inside calculator panel must be clickable");

        // Moving cursor off both keeps island active while typing, while passing outside clicks
        core.poll_cursor_at(wx - 40.0, wy + 360.0, t0 + Duration::from_millis(30));
        assert!(core.active(), "typing with cursor outside calc must not collapse island");
        assert!(core.win.ignore_mouse(), "outside cursor lets clicks pass to window below");

        // Closing calculator restores resting 640x320 size and normal collapse
        assert!(core.set_calc_open(false));
        assert!(!core.calc_active());
        assert_eq!(core.win.size(), (WIN_W, WIN_H));
        core.set_pinned(false);
        core.poll_cursor_at(wx - 40.0, wy + 250.0, t0 + Duration::from_millis(60));
        assert!(!core.active(), "island must collapse normally after calculator closes");
        assert!(core.win.ignore_mouse());
    }

    #[test]
    fn calculator_blur_closes_cleanly_and_allows_reopen() {
        let (mut core, _) = test_core();
        core.place_window();
        assert!(core.set_calc_open(true));
        assert!(core.calc_active());
        assert_eq!(core.win.size(), (WIN_W, CALC_WIN_H));

        // First external blur prepares close without losing expanded window size mid-animation
        assert!(core.prepare_calc_blur());
        assert!(!core.calc_active(), "blur must deactivate active state immediately");
        assert!(!core.prepare_calc_blur(), "second blur while closing must be ignored");
        assert!(core.win.ignore_mouse());
        assert_eq!(core.win.size(), (WIN_W, CALC_WIN_H));

        // Re-opening mid-close animation cancels closing state cleanly
        assert!(core.set_calc_open(true));
        assert!(core.calc_active());

        // Finishing close restores resting height
        assert!(core.set_calc_open(false));
        assert!(!core.calc_active());
        assert_eq!(core.win.size(), (WIN_W, WIN_H));
    }

    #[test]
    fn brief_edge_exit_and_top_edge_do_not_collapse_active_island() {
        let (mut core, _) = test_core();
        let (cx, cy) = pill_center(&core);
        let t0 = Instant::now();
        core.poll_cursor_at(cx, cy, t0);
        core.poll_cursor_at(cx, cy, t0 + Duration::from_millis(320));
        assert!(core.active());

        // Pushing cursor flush to the top screen edge (y = 0) stays inside
        core.poll_cursor_at(cx, 0.0, t0 + Duration::from_millis(360));
        assert!(core.active());
        assert!(core.inside());

        // Stepping slightly past the edge for 20ms (< LEAVE_GRACE_MS) must NOT collapse
        let (wx, wy) = core.win.pos();
        core.poll_cursor_at(wx - 30.0, wy + 200.0, t0 + Duration::from_millis(400));
        core.poll_cursor_at(wx - 30.0, wy + 200.0, t0 + Duration::from_millis(420));
        assert!(core.active(), "brief 1-frame slip past edge must not collapse open island");

        // Holding left mouse button down outside the edge keeps interaction alive
        core.poll_cursor_state(wx - 30.0, wy + 200.0, t0 + Duration::from_millis(800), true);
        assert!(core.active(), "mouse drag near edge must keep island active");

        // Returning inside cancels leave timer
        core.poll_cursor_at(cx, cy, t0 + Duration::from_millis(840));
        assert!(core.active());

        // Actually leaving for >= LEAVE_GRACE_MS collapses promptly
        core.poll_cursor_at(wx - 30.0, wy + 200.0, t0 + Duration::from_millis(900));
        core.poll_cursor_at(wx - 30.0, wy + 200.0, t0 + Duration::from_millis(950));
        assert!(!core.active(), "sustained exit must collapse promptly");
    }

    #[test]
    fn thin_line_in_chrome_never_triggers_1cm_away_and_opens_closes_promptly() {
        let (mut core, _) = test_core();
        core.features.set(&json!({ "browserLine": true, "addrIsland": false }));
        core.place_window();
        let (wx, wy) = core.win.pos();
        core.on_browser(&json!({ "x": wx as i32, "y": wy as i32, "w": 1200, "h": 800 }));
        assert!(core.is_line(), "browser over top tabs must enter thin-line mode");
        assert_eq!(core.island_rect.width, THIN_LINE_W);
        assert_eq!(core.island_rect.height, THIN_LINE_H);

        // Simulate stale mid-shrink DOM rect arriving right after close — must be ignored
        core.set_island_rect(Rect { x: 46.0, y: 0.0, width: 548.0, height: 232.0 });
        assert_eq!(core.island_rect.width, THIN_LINE_W);
        assert_eq!(core.island_rect.height, THIN_LINE_H);

        let line_cx = wx + core.island_rect.x + THIN_LINE_W / 2.0;
        let t0 = Instant::now();
        // Cursor 15px (~0.4cm) and 38px (~1cm) below the thin line must NEVER open it
        for dy in [15.0, 38.0] {
            core.poll_cursor_at(line_cx, wy + dy, t0);
            core.poll_cursor_at(line_cx, wy + dy, t0 + Duration::from_millis(500));
            assert!(!core.active(), "cursor {dy}px below thin line must never trigger island");
        }
        // Cursor 20px to the left/right of the thin line must NEVER open it
        core.poll_cursor_at(wx + core.island_rect.x - 20.0, wy + 2.0, t0 + Duration::from_millis(600));
        core.poll_cursor_at(wx + core.island_rect.x - 20.0, wy + 2.0, t0 + Duration::from_millis(900));
        assert!(!core.active(), "cursor beside thin line must never trigger island");

        // Directly hovering the 96x6 thin line opens promptly (~50ms)
        let t1 = t0 + Duration::from_millis(1000);
        core.poll_cursor_at(line_cx, wy + 3.0, t1);
        core.poll_cursor_at(line_cx, wy + 3.0, t1 + Duration::from_millis(55));
        assert!(core.active(), "directly hovering the thin line must open promptly");

        // Moving away collapses promptly (~45ms) and immediately snaps hit-box back to 96x6
        core.set_island_rect(Rect { x: 46.0, y: 0.0, width: 548.0, height: 232.0 });
        let t2 = t1 + Duration::from_millis(200);
        core.poll_cursor_at(line_cx, wy + 290.0, t2);
        core.poll_cursor_at(line_cx, wy + 290.0, t2 + Duration::from_millis(45));
        assert!(!core.active(), "moving away must collapse promptly");
        assert_eq!(core.island_rect.width, THIN_LINE_W, "hit-box must snap back to thin line immediately on close");
        assert_eq!(core.island_rect.height, THIN_LINE_H);
    }
}
