//! Upcoming meetings from a private iCal link (calendar.js + what it uses of node-ical 0.27 and
//! rrule-temporal). Reads VEVENTs, expands recurring series in their own time zone (so a weekly
//! 9:00 stays 9:00 across DST), honours EXDATE, moved and cancelled occurrences, and returns the
//! occurrences of the next 24 h the way the island shows them.
use serde_json::{json, Value};

pub const MAX_EVENTS: usize = 12;

/// Meeting links worth a Join button (the JOIN pattern in calendar.js).
pub fn join_in(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let stop = |c: char| c.is_whitespace() || "\"'<>".contains(c);
    let mut from = 0;
    while let Some(i) = lower[from..].find("https://").map(|i| i + from) {
        let rest = &text[i + 8..];
        let lrest = &lower[i + 8..];
        let host_end = lrest.find('/').unwrap_or(lrest.len());
        let host = &lrest[..host_end];
        let path_ok = |prefixes: &[&str]| prefixes.iter().any(|p| lrest[host_end..].starts_with(p));
        let label = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        let matched_len = if host == "meet.google.com" {
            let tail = &lrest[host_end..];
            let n = tail.strip_prefix('/').map(|t| t.bytes().take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-').count()).unwrap_or(0);
            (n > 0).then_some(host_end + 1 + n)
        } else {
            let zoom = host == "zoom.us" || host.strip_suffix(".zoom.us").is_some_and(label);
            let teams = host == "teams.microsoft.com" || host == "teams.live.com";
            let webex = host.strip_suffix(".webex.com").is_some_and(label);
            let ok = (teams && path_ok(&["/"])) || (zoom && path_ok(&["/j/", "/my/", "/w/"])) || (webex && path_ok(&["/"]));
            if ok {
                let start = host_end + if zoom { lrest[host_end + 1..].find('/').map(|k| k + 2).unwrap_or(1) } else { 1 };
                let n = rest[start..].chars().take_while(|c| !stop(*c)).map(char::len_utf8).sum::<usize>();
                (n > 0).then_some(start + n)
            } else {
                None
            }
        };
        if let Some(len) = matched_len {
            let m = &text[i..i + 8 + len];
            return Some(m.trim_end_matches([')', '.', ',', ';']).to_string());
        }
        from = i + 8;
    }
    None
}

/// What node-ical keeps for a text property: plain text, text with parameters ({params, val}),
/// or several values (an array). calendar.js only searches plain text for join links.
#[derive(Clone, Debug, Default)]
enum Field {
    #[default]
    Missing,
    Text(String),
    WithParams(String),
    Many,
}

impl Field {
    fn add(&mut self, v: Field) {
        *self = match self {
            Field::Missing => v,
            _ => Field::Many,
        };
    }
    /// calendar.js text(): the string or its .val, trimmed
    fn text(&self) -> String {
        match self {
            Field::Text(s) | Field::WithParams(s) => s.trim().to_string(),
            _ => String::new(),
        }
    }
    fn plain(&self) -> Option<&str> {
        if let Field::Text(s) = self { Some(s) } else { None }
    }
}

#[derive(Clone, Copy, Debug)]
struct When {
    ms: f64,
    date_only: bool,
}

#[derive(Clone, Debug, Default)]
struct Event {
    uid: String,
    start: Option<When>,
    start_wall: Option<(f64, Option<String>)>, // wall ms + zone (None = UTC)
    end: Option<When>,
    duration_ms: Option<f64>,
    status: String,
    summary: Field,
    description: Field,
    location: Field,
    url: Field,
    google: Field,
    x_google: Field,
    rrule: Option<String>,
    exdate: Vec<f64>,
    recurrence_id: Option<(f64, String)>, // ms + date key
}

fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some('n') | Some('N') => {
                    it.next();
                    out.push('\n');
                }
                Some(',') | Some(';') | Some('\\') => out.push(it.next().unwrap()),
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// "TZID=..." etc.: (name, value) pairs, quotes removed
fn params(raw: &[&str]) -> Vec<(String, String)> {
    raw.iter().filter_map(|p| p.split_once('=')).map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string())).collect()
}

fn param<'a>(ps: &'a [(String, String)], name: &str) -> Option<&'a str> {
    ps.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn local_zone() -> String {
    nova_prayer::icu::default_zone()
}

/// A DTSTART/DTEND/EXDATE/RECURRENCE-ID value: (ms, date-only, wall ms, zone for wall times)
fn parse_date(value: &str, ps: &[(String, String)], local_tz: &str) -> Option<(When, f64, Option<String>)> {
    let v = value.trim();
    let digits = |s: &str| s.parse::<i64>().ok();
    let date_only = matches!(param(ps, "VALUE").map(|s| s.to_ascii_uppercase()).as_deref(), Some("DATE")) || (v.len() == 8 && v.bytes().all(|b| b.is_ascii_digit()));
    let (y, m, d) = (digits(v.get(0..4)?)?, digits(v.get(4..6)?)?, digits(v.get(6..8)?)?);
    let day_ms = (nova_prayer::days_from_civil(y, m, d) * 86_400_000) as f64;
    if date_only {
        // all-day: local midnight of that date (only its length matters to calendar.js)
        let zone = local_tz.to_string();
        let ms = nova_prayer::local_to_utc(&zone, day_ms)?;
        return Some((When { ms, date_only: true }, day_ms, Some(zone)));
    }
    // (1.8.7) by byte ranges that can fall inside a letter: get() says no instead of a panic
    let t = v.get(9..15)?;
    let wall = day_ms + ((digits(t.get(0..2)?)? * 3600 + digits(t.get(2..4)?)? * 60 + digits(t.get(4..6)?)?) * 1000) as f64;
    if v.ends_with('Z') {
        return Some((When { ms: wall, date_only: false }, wall, None));
    }
    match param(ps, "TZID") {
        Some(zone) => {
            let ms = nova_prayer::local_to_utc(zone, wall).unwrap_or(wall);
            Some((When { ms, date_only: false }, wall, Some(zone.to_string())))
        }
        // a floating time is read in the PC's zone once; a series then repeats it in UTC (node-ical
        // keeps no zone for it), so it moves an hour on the clock after a DST change, as in 1.7
        None => {
            let ms = nova_prayer::local_to_utc(local_tz, wall).unwrap_or(wall);
            Some((When { ms, date_only: false }, ms, None))
        }
    }
}

/// RFC 5545 DURATION: [+-]P[nW][nD][T[nH][nM][nS]]
fn parse_duration(v: &str) -> Option<f64> {
    let v = v.trim();
    let (sign, v) = match v.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, v.strip_prefix('+').unwrap_or(v)),
    };
    let mut rest = v.strip_prefix('P')?;
    let mut total = 0.0;
    let mut num = String::new();
    let mut time = false;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '0'..='9' => num.push(c),
            'T' => time = true,
            _ => {
                let n: f64 = num.parse().ok()?;
                num.clear();
                total += n * match (c, time) {
                    ('W', _) => 7.0 * 86400.0,
                    ('D', _) => 86400.0,
                    ('H', true) => 3600.0,
                    ('M', true) => 60.0,
                    ('S', true) => 1.0,
                    _ => return None,
                };
            }
        }
    }
    Some(sign * total * 1000.0)
}

/// Lines with RFC 5545 folding undone.
fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    lines
}

/// Parses the feed into its series (by UID) with moved/cancelled occurrences attached.
fn parse(ics: &str, local_tz: &str) -> Vec<(Event, Vec<Event>)> {
    let mut series: Vec<(Event, Vec<Event>)> = Vec::new();
    let mut overrides: Vec<Event> = Vec::new();
    let mut cur: Option<Event> = None;
    let mut depth_other = 0;
    for line in unfold(ics) {
        let Some(colon) = find_value_colon(&line) else { continue };
        let (head, value) = (&line[..colon], &line[colon + 1..]);
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or("").to_ascii_uppercase();
        let raw: Vec<&str> = parts.collect();
        let ps = params(&raw);
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") && depth_other == 0 => cur = Some(Event::default()),
            "BEGIN" if cur.is_some() => depth_other += 1,
            "END" if depth_other > 0 => depth_other -= 1,
            "END" if value.eq_ignore_ascii_case("VEVENT") => {
                if let Some(ev) = cur.take() {
                    if ev.recurrence_id.is_some() && !ev.uid.is_empty() {
                        overrides.push(ev);
                    } else {
                        series.push((ev, Vec::new()));
                    }
                }
            }
            _ => {
                let Some(ev) = cur.as_mut() else { continue };
                let text_field = || if raw.is_empty() { Field::Text(unescape(value)) } else { Field::WithParams(unescape(value)) };
                match name.as_str() {
                    "UID" => ev.uid = value.to_string(),
                    "SUMMARY" => ev.summary.add(text_field()),
                    "DESCRIPTION" => ev.description.add(text_field()),
                    "LOCATION" => ev.location.add(text_field()),
                    "URL" => ev.url.add(text_field()),
                    "GOOGLE-CONFERENCE" => ev.google.add(text_field()),
                    "X-GOOGLE-CONFERENCE" => ev.x_google.add(text_field()),
                    "STATUS" => ev.status = value.trim().to_ascii_uppercase(),
                    "DTSTART" if ev.start.is_none() => {
                        if let Some((w, wall, zone)) = parse_date(value, &ps, local_tz) {
                            ev.start = Some(w);
                            ev.start_wall = Some((wall, zone));
                        }
                    }
                    "DTEND" if ev.end.is_none() => ev.end = parse_date(value, &ps, local_tz).map(|x| x.0),
                    "DURATION" => ev.duration_ms = parse_duration(value),
                    "RRULE" => ev.rrule = Some(value.to_string()),
                    "EXDATE" => {
                        for v in value.split(',') {
                            if let Some((w, _, _)) = parse_date(v, &ps, local_tz) {
                                ev.exdate.push(w.ms);
                            }
                        }
                    }
                    "RECURRENCE-ID" => {
                        if let Some((w, wall, _)) = parse_date(value, &ps, local_tz) {
                            // (1.8.7) the occurrence it replaces is found by its exact time. Only a
                            // floating id (no zone, no Z) keeps 1.7's date match, as its series is
                            // repeated in UTC and can be an hour away after a DST change. (1.7 matched
                            // all of them by the written date against the UTC date, so a moved
                            // 8:00 Tokyo meeting was missed, and the next day's one replaced.)
                            let _ = wall;
                            let floating = !w.date_only && !ps.iter().any(|(k, _)| k == "TZID") && !value.trim().ends_with('Z');
                            let key = if floating { date_key(w.ms) } else { String::new() };
                            ev.recurrence_id = Some((w.ms, key));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    for o in overrides {
        if let Some(s) = series.iter_mut().find(|(e, _)| e.uid == o.uid) {
            s.1.push(o);
        }
    }
    series
}

/// The ':' between name;params and the value, skipping ':' inside quoted parameters.
fn find_value_colon(line: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

fn date_key(ms_as_utc: f64) -> String {
    let f = days_to_ymd((ms_as_utc / 86_400_000.0).floor() as i64);
    format!("{:04}-{:02}-{:02}", f.0, f.1, f.2)
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    nova_core::log::civil(days)
}

fn weekday(days: i64) -> u32 {
    // 1970-01-01 was a Thursday; Monday = 0
    (days + 3).rem_euclid(7) as u32
}

fn days_in_month(y: i64, m: u32) -> u32 {
    let next = if m == 12 { nova_prayer::days_from_civil(y + 1, 1, 1) } else { nova_prayer::days_from_civil(y, m as i64 + 1, 1) };
    (next - nova_prayer::days_from_civil(y, m as i64, 1)) as u32
}

/// RRULE occurrences (UTC ms) from the series start, in its own time zone, up to `until_ms`.
fn expand(rule: &str, start_ms: f64, wall: f64, zone: Option<&str>, local_tz: &str, limit_ms: f64) -> Vec<f64> {
    let mut freq = "";
    let mut interval = 1i64;
    let mut count: Option<usize> = None;
    let mut until: Option<f64> = None;
    let mut byday: Vec<(i32, u32)> = Vec::new();
    let mut bymonthday: Vec<i32> = Vec::new();
    let mut bymonth: Vec<u32> = Vec::new();
    let mut bysetpos: Vec<i32> = Vec::new();
    let rule = rule.trim();
    let rule = &rule[rule.rfind("FREQ=").unwrap_or(0)..];
    for part in rule.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => freq = match v.to_ascii_uppercase().as_str() { "DAILY" => "D", "WEEKLY" => "W", "MONTHLY" => "M", "YEARLY" => "Y", _ => "" },
            "INTERVAL" => interval = v.parse().unwrap_or(1).max(1),
            "COUNT" => count = v.parse().ok(),
            "UNTIL" => {
                until = parse_date(v, &[], local_tz).map(|(w, wl, _)| if v.trim().ends_with('Z') || v.len() == 8 { w.ms } else { zone.and_then(|z| nova_prayer::local_to_utc(z, wl)).unwrap_or(w.ms) })
            }
            "BYDAY" => {
                for d in v.split(',') {
                    let d = d.trim().to_ascii_uppercase();
                    let Some((n, code)) = d.len().checked_sub(2).and_then(|i| d.is_char_boundary(i).then(|| d.split_at(i))) else { continue };
                    let wd = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].iter().position(|c| *c == code);
                    if let Some(wd) = wd {
                        byday.push((n.parse().unwrap_or(0), wd as u32));
                    }
                }
            }
            "BYMONTHDAY" => bymonthday = v.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
            "BYMONTH" => bymonth = v.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
            "BYSETPOS" => bysetpos = v.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
            _ => {}
        }
    }
    if freq.is_empty() {
        return Vec::new();
    }
    let day0 = (wall / 86_400_000.0).floor() as i64;
    let tod = wall - (day0 * 86_400_000) as f64;
    let (y0, m0, d0) = days_to_ymd(day0);
    let to_utc = |day: i64| -> f64 {
        let w = (day * 86_400_000) as f64 + tod;
        match zone {
            Some(z) => nova_prayer::local_to_utc(z, w).unwrap_or(w),
            None => w,
        }
    };
    let mut out = Vec::new();
    let mut emitted = 0usize;
    let push_set = |mut days: Vec<i64>, out: &mut Vec<f64>, emitted: &mut usize| -> bool {
        days.sort();
        days.dedup();
        if !bysetpos.is_empty() {
            let n = days.len() as i32;
            days = bysetpos.iter().filter_map(|p| if *p > 0 { days.get((*p - 1) as usize).copied() } else if *p < 0 && -*p <= n { days.get((n + *p) as usize).copied() } else { None }).collect();
            days.sort();
        }
        for d in days {
            if d < day0 {
                continue;
            }
            let t = to_utc(d);
            if t < start_ms {
                continue;
            }
            if until.is_some_and(|u| t > u) || count.is_some_and(|c| *emitted >= c) || t > limit_ms {
                return false;
            }
            out.push(t);
            *emitted += 1;
        }
        true
    };
    let month_days = |y: i64, m: u32| -> Vec<i64> {
        let first = nova_prayer::days_from_civil(y, m as i64, 1);
        let n = days_in_month(y, m) as i64;
        let mut days = Vec::new();
        if !byday.is_empty() {
            for &(ord, wd) in &byday {
                let matches: Vec<i64> = (0..n).map(|i| first + i).filter(|d| weekday(*d) == wd).collect();
                if ord == 0 {
                    days.extend(matches);
                } else if ord > 0 {
                    days.extend(matches.get((ord - 1) as usize));
                } else {
                    days.extend(matches.len().checked_sub((-ord) as usize).and_then(|i| matches.get(i)));
                }
            }
            if !bymonthday.is_empty() {
                days.retain(|d| bymonthday.iter().any(|md| (*md > 0 && *d == first + *md as i64 - 1) || (*md < 0 && *d == first + n + *md as i64)));
            }
        } else {
            let mds = if bymonthday.is_empty() { vec![d0 as i32] } else { bymonthday.clone() };
            for md in mds {
                let d = if md > 0 { md as i64 } else { n + md as i64 + 1 };
                if d >= 1 && d <= n {
                    days.push(first + d - 1);
                }
            }
        }
        days
    };
    // (1.8.7) BYMONTHDAY limits daily and weekly series too (RFC 5545); a negative day counts
    // from the month's end
    let md_ok = |day: i64| {
        let (y, m, md) = days_to_ymd(day);
        let n = days_in_month(y, m) as i64;
        bymonthday.is_empty() || bymonthday.iter().any(|&x| (if x > 0 { x as i64 } else { n + x as i64 + 1 }) == md as i64)
    };
    match freq {
        "D" => {
            let mut day = day0;
            for _ in 0..200_000 {
                let (_, m, _) = days_to_ymd(day);
                let ok = (bymonth.is_empty() || bymonth.contains(&m)) && (byday.is_empty() || byday.iter().any(|(_, wd)| *wd == weekday(day))) && md_ok(day);
                if !push_set(if ok { vec![day] } else { vec![] }, &mut out, &mut emitted) {
                    break;
                }
                day += interval;
            }
        }
        "W" => {
            let week0 = day0 - weekday(day0) as i64;
            let days: Vec<u32> = if byday.is_empty() { vec![weekday(day0)] } else { byday.iter().map(|(_, wd)| *wd).collect() };
            let mut week = week0;
            for _ in 0..50_000 {
                let set = days.iter().map(|wd| week + *wd as i64).filter(|d| (bymonth.is_empty() || bymonth.contains(&days_to_ymd(*d).1)) && md_ok(*d)).collect();
                if !push_set(set, &mut out, &mut emitted) {
                    break;
                }
                week += 7 * interval;
            }
        }
        "M" => {
            let (mut y, mut m) = (y0, m0);
            for _ in 0..20_000 {
                let set = if bymonth.is_empty() || bymonth.contains(&m) { month_days(y, m) } else { vec![] };
                if !push_set(set, &mut out, &mut emitted) {
                    break;
                }
                let next = m as i64 - 1 + interval;
                y += next.div_euclid(12);
                m = (next.rem_euclid(12) + 1) as u32;
            }
        }
        "Y" => {
            let mut y = y0;
            for _ in 0..2_000 {
                let months = if bymonth.is_empty() { vec![m0] } else { bymonth.clone() };
                let mut set = Vec::new();
                for m in months {
                    if byday.is_empty() && bymonthday.is_empty() {
                        if d0 <= days_in_month(y, m) {
                            set.push(nova_prayer::days_from_civil(y, m as i64, d0 as i64));
                        }
                    } else {
                        set.extend(month_days(y, m));
                    }
                }
                if !push_set(set, &mut out, &mut emitted) {
                    break;
                }
                y += interval;
            }
        }
        _ => {}
    }
    out
}

/// calendar.js occurrences(): every occurrence touching [from, to), at most 12, by start time.
pub fn occurrences(ics: &str, from: f64, to: f64) -> Vec<Value> {
    occurrences_in_zone(ics, from, to, &local_zone())
}

/// Same as `occurrences`, using an explicit local time zone for floating / all-day timestamps.
pub fn occurrences_in_zone(ics: &str, from: f64, to: f64, local_tz: &str) -> Vec<Value> {
    let mut out: Vec<(f64, Value)> = Vec::new();
    for (ev, moved) in parse(ics, local_tz) {
        let Some(start) = ev.start else { continue };
        if ev.status == "CANCELLED" {
            continue;
        }
        let end_ms = ev.end.map(|e| e.ms).or(ev.duration_ms.map(|d| start.ms + d));
        let dur = end_ms.unwrap_or(start.ms) - start.ms;
        let all_day = start.date_only;
        let mut push = |s: f64, src: &Event| {
            let e = s + dur;
            if e <= from || s >= to || all_day {
                return;
            }
            let title = src.summary.text();
            let fields = [src.location.plain(), match &src.url { Field::Text(s) | Field::WithParams(s) => Some(s.as_str()), _ => None }, src.description.plain(), src.google.plain(), src.x_google.plain()];
            let link = fields.iter().flatten().find_map(|f| join_in(f)).unwrap_or_default();
            out.push((s, json!({ "id": format!("{}|{}", ev.uid, nova_core::json::number(s)), "title": if title.is_empty() { "(No title)".to_string() } else { title }, "start": s as i64, "end": e as i64, "link": link })));
        };
        match (&ev.rrule, ev.start_wall.as_ref()) {
            (Some(rule), Some((wall, zone))) => {
                for s in expand(rule, start.ms, *wall, zone.as_deref(), local_tz, to) {
                    if s < from - dur {
                        continue;
                    }
                    let key = date_key(s);
                    let replaces = |m: &&Event| m.recurrence_id.as_ref().is_some_and(|(ms, k)| *ms == s || (!k.is_empty() && *k == key));
                    if let Some(m) = moved.iter().find(replaces) {
                        if m.status != "CANCELLED" {
                            if let Some(ms) = m.start {
                                push(ms.ms, m);
                            }
                        }
                    } else if !ev.exdate.contains(&s) {
                        push(s, &ev);
                    }
                }
            }
            _ => push(start.ms, &ev),
        }
    }
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out.into_iter().take(MAX_EVENTS).map(|(_, v)| v).collect()
}

// ---- the service (calendar.js): the private iCal link, read every 10 minutes ----
// No sign-in: the link itself is the key, so it is stored only in calendar.json and never logged.
pub mod service {
    use std::sync::{Arc, Mutex};

    use serde_json::{json, Value};

    use crate::ctx::{now_ms, Ctx, Wake};

    const REFRESH_MS: u64 = 10 * 60 * 1000;
    const RETRY_MS: u64 = 2 * 60 * 1000;
    const AHEAD_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;

    fn normalize(url: &str) -> String {
        let t = url.trim();
        // (1.8.7) get(): a link in Arabic letters is refused, not a crash
        match t.get(..9) {
            Some(p) if p.eq_ignore_ascii_case("webcal://") => format!("https://{}", &t[9..]),
            _ => t.to_string(),
        }
    }

    fn valid_url(url: &str) -> bool {
        url.len() > 8 && url.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) && !url.chars().any(char::is_whitespace)
    }

    /// A link the page asks to open must be a whole meeting link.
    pub fn is_join_link(url: &str) -> bool {
        super::join_in(url).is_some_and(|m| m == url)
    }

    pub struct Calendar {
        ctx: Ctx,
        url: Mutex<String>,
        state: Mutex<Value>,
        wake: Wake,
        on_change: Mutex<Vec<Box<dyn Fn(Value) + Send>>>,
    }

    impl Calendar {
        pub fn new(ctx: Ctx) -> Arc<Calendar> {
            let url = ctx.store().load("calendar.json").and_then(|c| c["url"].as_str().map(str::to_string)).unwrap_or_default();
            Arc::new(Calendar { ctx, url: Mutex::new(url), state: Mutex::new(json!({ "events": [], "error": "", "updated": 0 })), wake: Wake::new(), on_change: Mutex::new(Vec::new()) })
        }

        /// Every new state also goes to Settings.
        pub fn on_change(&self, f: impl Fn(Value) + Send + 'static) {
            self.on_change.lock().unwrap().push(Box::new(f));
        }

        pub fn start(self: &Arc<Self>) {
            let me = self.clone();
            std::thread::spawn(move || loop {
                let wait = me.refresh();
                me.wake.sleep(wait);
            });
        }

        fn publish(&self) {
            let s = self.state.lock().unwrap().clone();
            self.ctx.send("calendar", s.clone());
            for f in self.on_change.lock().unwrap().iter() {
                f(s.clone());
            }
        }

        fn refresh(&self) -> u64 {
            let url = normalize(&self.url.lock().unwrap());
            if url.is_empty() {
                *self.state.lock().unwrap() = json!({ "events": [], "error": "", "updated": 0 });
                self.publish();
                return 24 * 60 * 60 * 1000;
            }
            // (1.8.7) a feed is a few hundred KB; one far bigger is stopped as it comes, not kept
            const MAX_FEED: usize = 10 << 20;
            let r = nova_core::http::request_progress("GET", &url, "accept: text/calendar\r\n", None, 15000, &mut |n, total| n <= MAX_FEED && total <= MAX_FEED).and_then(|r| {
                if !r.ok() {
                    return Err(format!("HTTP {}", r.status));
                }
                let body = r.text();
                if !body.contains("BEGIN:VCALENDAR") {
                    return Err("not an iCal file".into());
                }
                Ok(body)
            });
            let wait = match r {
                Ok(body) => {
                    let now = now_ms();
                    *self.state.lock().unwrap() = json!({ "events": super::occurrences(&body, now, now + AHEAD_MS), "error": "", "updated": now as i64 });
                    REFRESH_MS
                }
                Err(e) => {
                    let msg: String = if e == "timeout" { e } else { e.chars().take(120).collect() };
                    self.state.lock().unwrap()["error"] = json!(msg);
                    RETRY_MS
                }
            };
            self.publish();
            wait
        }

        pub fn set_url(&self, url: &str) -> Value {
            let clean = normalize(url);
            if !clean.is_empty() && !valid_url(&clean) {
                return json!({ "ok": false });
            }
            *self.url.lock().unwrap() = clean.clone();
            let _ = self.ctx.store().save("calendar.json", &json!({ "url": clean }));
            self.wake.wake();
            json!({ "ok": true })
        }

        pub fn has_url(&self) -> bool {
            !self.url.lock().unwrap().is_empty()
        }

        pub fn snapshot(&self) -> Value {
            let mut s = self.state.lock().unwrap().clone();
            s["hasUrl"] = json!(self.has_url());
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::occurrences_in_zone;

    #[test]
    fn floating_event_respects_explicit_local_zone() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u1@nova\r\nDTSTART:20261117T120000\r\nDTEND:20261117T130000\r\nSUMMARY:Lunch\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let day_ms = (nova_prayer::days_from_civil(2026, 11, 17) * 86_400_000) as f64;
        let utc_ev = occurrences_in_zone(ics, day_ms, day_ms + 86_400_000.0, "UTC");
        let cairo_ev = occurrences_in_zone(ics, day_ms, day_ms + 86_400_000.0, "Africa/Cairo");
        assert_eq!(utc_ev[0]["start"].as_i64(), Some(day_ms as i64 + 12 * 3_600_000));
        // Egypt standard time in November is UTC+2 -> 12:00 Cairo == 10:00 UTC.
        assert_eq!(cairo_ev[0]["start"].as_i64(), Some(day_ms as i64 + 10 * 3_600_000));
    }
}
