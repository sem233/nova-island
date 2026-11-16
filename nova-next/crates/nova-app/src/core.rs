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
const HOVER_INTENT_MS: u128 = 280;
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
    pub island_rect: Rect,
    /// The pill at rest (not opened, no notification): what "over a tab" is decided on, so the
    /// opened island covering a tab doesn't close itself.
    rest_rect: Rect,
    /// A new size of the pill at rest, taken once it has held still (not mid-animation).
    pending_rest: Option<(Rect, Instant)>,
    inside: bool,
    active: bool,
    entered_at: Instant,
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
            user_hidden: false, fullscreen_hidden: false, tab_rects: Vec::new(), line: false, line_rect: Rect::default(), line_off_since: None, browser_rect: None, ghost: false, ghost_open: false, dimmed: false, clickable: false,
            island_rect: Rect::default(), rest_rect: Rect::default(), pending_rest: None, inside: false, active: false, entered_at: Instant::now(), suppress_until_leave: false,
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
        self.features.bool("addrIsland") && !self.vertical() && !self.active && !self.clickable
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

    fn layout_address_bar(&mut self) {
        let o = self.omni.clone().unwrap();
        // the helper reports physical pixels; place windows in DIPs
        let r = screen::rect_to_dip(o["x"].as_i64().unwrap_or(0) as i32, o["y"].as_i64().unwrap_or(0) as i32, o["w"].as_i64().unwrap_or(0) as i32, o["h"].as_i64().unwrap_or(0) as i32);
        let b = screen::matching(r).bounds;
        let third = r.width / 3.0;
        let sh = round(30f64.min(22f64.max(r.height + 6.0)));
        let sw = round((third - 12.0).min(380.0));
        let cx = r.x + r.width - third / 2.0; // centre of the right third
        let top = round(r.y + r.height / 2.0 - sh / 2.0);
        let wx = round(clamp(cx - WIN_W / 2.0, b.x, b.x + b.width - WIN_W));
        let wy = round(clamp(top - FLOAT_TOP, b.y, b.y + b.height - WIN_H));
        self.win.set_pos(wx, wy);
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
        let (wx, wy);
        if self.vertical() {
            a.x = if self.side() == "left" { b.x } else { b.x + b.width };
            a.y = clamp(a.y, wa.y + V_PILL_HALF_H, wa.y + wa.height - V_PILL_HALF_H);
            wx = if self.side() == "left" { b.x } else { b.x + b.width - WIN_W };
            wy = round(clamp(a.y - WIN_H / 2.0, wa.y, wa.y + wa.height - WIN_H));
        } else {
            a.x = clamp(a.x, b.x + PILL_HALF_W, b.x + b.width - PILL_HALF_W);
            a.y = if self.docked() { b.y } else { clamp(a.y, b.y, wa.y + wa.height - PILL_H - 6.0) };
            wx = round(clamp(a.x - WIN_W / 2.0, b.x, b.x + b.width - WIN_W));
            wy = round(if self.docked() { b.y } else { clamp(a.y - FLOAT_TOP, b.y, wa.y + wa.height - WIN_H) });
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
            self.win.set_pos(wx, b.y);
            self.emit_layout(json!({ "x": round(a.x - wx), "y": 0, "docked": true, "vertical": false, "side": self.side(), "addr": "line" }));
            return;
        }
        self.win.set_pos(wx, wy);
        let msg = json!({ "x": round(a.x - wx), "y": round(a.y - wy), "docked": !self.vertical() && self.docked(), "vertical": self.vertical(), "side": self.side() });
        self.emit_layout(msg);
    }

    /// (1.8.7) Settings › In the browser › Thin line.
    fn browser_line(&self) -> bool {
        self.features.bool("browserLine") && !self.features.bool("addrIsland") && !self.vertical()
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
        (self.send)("hover", json!(false));
        // dropped onto the tabs, or off them
        self.update_ghost();
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
        if next != self.browser_rect {
            self.browser_rect = next;
            if !self.active && !self.clickable {
                self.update_line();
            }
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
        // opened (by the mouse or a notification): it stays as it is until it closes again
        if self.active || self.clickable {
            self.update_dim();
            return;
        }
        self.update_line();
        if !self.features.bool("addrIsland") && !self.browser_line() && self.rest_rect.width > 0.0 {
            let (wx, wy) = self.win.pos();
            let (x, y, r) = (wx + self.rest_rect.x, wy + self.rest_rect.y, self.rest_rect);
            over = self.tab_rects.iter().any(|t| x < t.x + t.width && x + r.width > t.x && y < t.y + t.height && y + r.height > t.y);
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

    /// Half opacity while the island lets clicks through to the browser; a notification that
    /// can be clicked shows at full strength.
    fn update_dim(&mut self) {
        let dim = self.ghost && !self.clickable;
        if dim != self.dimmed {
            self.dimmed = dim;
            (self.send)(DIM, json!(dim));
        }
    }

    pub fn set_clickable(&mut self, on: bool) {
        self.clickable = on;
        // it's gone: over the browser the island lets the mouse through again (and doesn't open)
        if !on && self.ghost {
            self.set_active(false);
        }
        self.update_dim();
    }

    pub fn set_pinned(&mut self, on: bool) {
        if !on && self.ghost_open {
            self.ghost_open = false;
            self.update_ghost();
        }
    }

    /// Ctrl+Alt+I
    pub fn toggle(&mut self) {
        if self.ghost {
            self.ghost_open = true;
            self.update_ghost();
        }
        (self.send)("toggle", Value::Null);
    }

    pub fn set_island_rect(&mut self, r: Rect) {
        self.island_rect = r;
        if !self.active && !self.clickable {
            self.pending_rest = Some((r, Instant::now()));
        }
    }

    fn set_active(&mut self, on: bool) {
        if on == self.active {
            return;
        }
        self.active = on;
        self.win.set_ignore_mouse(!on);
        if !on {
            self.win.set_focusable(false);
        }
        (self.send)("hover", json!(on));
        if !on {
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
        // the line waiting to go: looked at again until it's back over the tabs or LINE_KEEP is up
        if self.line_off_since.is_some() && !self.active {
            self.update_line();
        }
        // the pill's size at rest, once it has stopped changing for 300 ms
        if let Some((r, t)) = self.pending_rest {
            if self.active || self.clickable {
                self.pending_rest = None;
            } else if t.elapsed().as_millis() >= 300 {
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
        let (px, py) = screen::cursor();
        let (bx, by) = self.win.pos();
        let (x, y) = (px - bx, py - by);
        let r = self.island_rect;
        let fx = (r.x - x).max(0.0).max(x - r.x - r.width);
        let fy = (r.y - y).max(0.0).max(y - r.y - r.height);
        self.cursor_far = fx.hypot(fy) > FAR_PX;
        self.send_near(x, y, fx.hypot(fy));
        // the thin line is 4px: a little more room to rest the cursor on it
        let pad = if self.active { 8.0 } else if self.line { 6.0 } else { 2.0 };
        let now = x >= r.x - pad && x <= r.x + r.width + pad && y >= r.y - pad && y <= r.y + r.height + pad;
        if now && !self.inside {
            self.entered_at = Instant::now();
        }
        self.inside = now;
        if !now {
            self.suppress_until_leave = false;
            self.set_active(false);
        } else if !self.active && !self.suppress_until_leave && (!self.ghost || self.clickable) && self.entered_at.elapsed().as_millis() >= HOVER_INTENT_MS {
            self.set_active(true);
        }
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
        } else if self.cursor_far {
            100
        } else {
            40
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
