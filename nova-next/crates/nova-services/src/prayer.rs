//! The prayer-times service (prayer.js refresh / retry / snapshot): today and tomorrow for the
//! place, saved to prayer-cache.json, recalculated just after local midnight and when the place or
//! the settings change; after a failure (no place known yet) every 5 minutes, logged once.
use std::sync::{Arc, Mutex};

use nova_prayer::{calc_day_ext, day_in, next_day, Settings};
use serde_json::{json, Value};

use crate::{
    ctx::{now_ms, Ctx, Wake},
    system::System,
};

pub struct Prayer {
    ctx: Ctx,
    system: Arc<System>,
    data: Mutex<Value>,
    failed: Mutex<bool>,
    approx: Mutex<bool>,
    wake: Wake,
}

impl Prayer {
    pub fn new(ctx: Ctx, system: Arc<System>) -> Arc<Prayer> {
        let data = ctx.store().load("prayer-cache.json").unwrap_or(Value::Null);
        Arc::new(Prayer { ctx, system, data: Mutex::new(data), failed: Mutex::new(false), approx: Mutex::new(false), wake: Wake::new() })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            let wait = me.run();
            me.wake.sleep(wait);
        });
    }

    pub fn snapshot(&self) -> Value {
        self.data.lock().unwrap().clone()
    }

    /// Recalculate now (place or settings changed).
    pub fn refresh(&self) {
        self.wake.wake();
    }

    /// After sleep or a lost connection: recalculate if the last run failed or was approximate.
    pub fn retry(&self) {
        if *self.failed.lock().unwrap() || *self.approx.lock().unwrap() {
            self.wake.wake();
        }
    }

    /// One calculation; returns ms until the next one.
    fn run(&self) -> u64 {
        match self.calc() {
            Ok(data) => {
                let _ = self.ctx.store().save("prayer-cache.json", &data);
                *self.data.lock().unwrap() = data.clone();
                let zone = data["today"]["tz"].as_str().filter(|z| !z.is_empty()).map(str::to_string).unwrap_or_else(nova_prayer::icu::default_zone);
                self.ctx.send("prayer", data);
                let mut failed = self.failed.lock().unwrap();
                if *failed {
                    nova_core::log::warn("[prayer] back");
                }
                *failed = false;
                // next run just after midnight where the place is (1.8.7: it was the PC's, so a
                // city hours ahead of the PC kept yesterday's times until the PC's midnight)
                let f = nova_prayer::wall(&zone, now_ms()).map(|f| (f.hour * 3600 + f.minute * 60 + f.second) as u64).unwrap_or(0);
                (24 * 3600 + 60 - f.min(24 * 3600)) * 1000
            }
            Err(e) => {
                let mut failed = self.failed.lock().unwrap();
                if !*failed {
                    nova_core::log::warn(&format!("[prayer] {e} (retrying quietly)"));
                }
                *failed = true;
                5 * 60 * 1000
            }
        }
    }

    fn calc(&self) -> Result<Value, String> {
        let loc = self.system.get_location()?;
        let zone = loc["timezone"].as_str().filter(|z| !z.is_empty()).map(str::to_string).unwrap_or_else(nova_prayer::icu::default_zone);
        let today = day_in(&zone, now_ms()).ok_or("bad time zone")?;
        let tomorrow = next_day(today);
        let approx = loc["approx"] == true;
        *self.approx.lock().unwrap() = approx;
        let s = Settings { asr_hanafi: self.ctx.on("asrHanafi"), hijri_adjust: self.ctx.feature("hijriAdjust").as_i64().unwrap_or(0) };
        let method_override = self.ctx.feature("prayerMethod").as_str().unwrap_or("auto").to_string();
        let offset_min = self.ctx.feature("prayerOffsetMin").as_i64().unwrap_or(0).clamp(-15, 15);
        let (country, lat, lon) = (loc["country"].as_str().unwrap_or(""), loc["lat"].as_f64().unwrap_or(0.0), loc["lon"].as_f64().unwrap_or(0.0));
        let tz = loc["timezone"].as_str().unwrap_or("");
        let day = |d: nova_prayer::Day| -> Result<Value, String> {
            let mut v = calc_day_ext(d, country, lat, lon, tz, &zone, &s, &method_override, offset_min)?;
            // (1.8.7) the place's minutes ahead of UTC that day, for the desktop widgets: they
            // have no time-zone data, and showed the PC's day and countdown for another city
            if let Some(o) = utc_offset_min(&zone, d) {
                v["utcOffset"] = json!(o);
            }
            Ok(v)
        };
        Ok(json!({
            "city": format!("{}{}", loc["city"].as_str().unwrap_or(""), if approx { " (approx.)" } else { "" }),
            "today": day(today)?,
            "tomorrow": day(tomorrow)?,
        }))
    }

    /// Milliseconds until the very next enabled prayer event (pre-prayer, adhan, or iqama), so the
    /// native Win32 alarm timer wakes WebView2 on the exact second even while throttled or hidden.
    pub fn next_alert_ms(&self) -> Option<u64> {
        if !self.ctx.on("prayerAlerts") {
            return None;
        }
        let data = self.snapshot();
        if !data.is_object() {
            return None;
        }
        let soon_min = self.ctx.feature("prayerSoonMin").as_i64().unwrap_or(10).clamp(0, 60);
        let iqama_on = self.ctx.on("iqamaAlert");
        let iqama_delay = self.ctx.feature("iqamaDelayMin").as_i64().unwrap_or(15).clamp(5, 45);
        let iqama_before = self.ctx.feature("iqamaBeforeMin").as_i64().unwrap_or(2).clamp(0, 15);
        next_alert_delay_ms(&data, now_ms(), soon_min, iqama_on, iqama_delay, iqama_before)
    }
}

fn parse_wall_utc(zone: &str, date: &str, hhmm: &str) -> Option<f64> {
    let mut dp = date.split('-');
    let (y, mo, d) = (dp.next()?.parse::<i64>().ok()?, dp.next()?.parse::<i64>().ok()?, dp.next()?.parse::<i64>().ok()?);
    let (h, mi) = hhmm.split_once(':')?;
    let (h, mi) = (h.parse::<i64>().ok()?, mi.parse::<i64>().ok()?);
    let wall = (nova_prayer::days_from_civil(y, mo, d) * 86_400_000 + (h * 3600 + mi * 60) * 1000) as f64;
    nova_prayer::local_to_utc(zone, wall)
}

fn next_alert_delay_ms(data: &Value, now: f64, soon_min: i64, iqama_on: bool, iqama_delay: i64, iqama_before: i64) -> Option<u64> {
    const PRAYERS: &[&str] = &["fajr", "dhuhr", "asr", "maghrib", "isha"];
    let mut best: Option<f64> = None;
    let mut consider = |ts: f64| {
        if ts > now + 500.0 {
            best = Some(best.map_or(ts, |b| b.min(ts)));
        }
    };
    for key in ["today", "tomorrow"] {
        let day = &data[key];
        let Some(date) = day["date"].as_str() else { continue };
        let zone = day["tz"].as_str().filter(|z| !z.is_empty()).map(str::to_string).unwrap_or_else(nova_prayer::icu::default_zone);
        for p in PRAYERS {
            let Some(hhmm) = day["times"][*p].as_str() else { continue };
            let Some(adhan_utc) = parse_wall_utc(&zone, date, hhmm) else { continue };
            consider(adhan_utc);
            if soon_min > 0 {
                consider(adhan_utc - (soon_min as f64) * 60_000.0);
            }
            if iqama_on {
                let iqama_alert = adhan_utc + ((iqama_delay - iqama_before).max(1) as f64) * 60_000.0;
                consider(iqama_alert);
            }
        }
    }
    best.map(|ts| ((ts - now).max(1000.0).min(86_400_000.0)) as u64)
}

/// Minutes `zone` is ahead of UTC at noon on `day`.
fn utc_offset_min(zone: &str, day: nova_prayer::Day) -> Option<i64> {
    let noon = nova_prayer::days_from_civil(day.y as i64, day.m as i64, day.d as i64) * 86_400_000 + 12 * 3_600_000;
    let f = nova_prayer::wall(zone, noon as f64)?;
    let wall = nova_prayer::days_from_civil(f.year as i64, f.month as i64, f.day as i64) * 86_400_000 + ((f.hour * 3600 + f.minute * 60 + f.second) as i64) * 1000;
    Some((wall - noon) / 60_000)
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets_of_places() {
        let day = nova_prayer::Day { y: 2026, m: 11, d: 13 };
        assert_eq!(super::utc_offset_min("Asia/Tokyo", day), Some(540));
        assert_eq!(super::utc_offset_min("Africa/Cairo", day), Some(120));
        assert_eq!(super::utc_offset_min("America/Los_Angeles", day), Some(-480));
        assert_eq!(super::utc_offset_min("America/Los_Angeles", nova_prayer::Day { y: 2026, m: 7, d: 1 }), Some(-420));
    }

    #[test]
    fn next_alert_finds_soon_adhan_and_iqama_in_order() {
        let data = serde_json::json!({
            "today": {
                "date": "2026-11-26",
                "tz": "Africa/Cairo",
                "times": { "fajr": "04:58", "dhuhr": "11:42", "asr": "14:37", "maghrib": "16:55", "isha": "18:17" }
            }
        });
        // 11:30 Cairo (09:30 UTC): next event with soon=5m is 11:37 (7 min = 420,000 ms away)
        let t_before = super::parse_wall_utc("Africa/Cairo", "2026-11-26", "11:30").unwrap();
        assert_eq!(super::next_alert_delay_ms(&data, t_before, 5, true, 15, 2), Some(7 * 60_000));
        // 11:38 Cairo: next event is Dhuhr adhan 11:42 (4 min = 240,000 ms away)
        let t_soon_done = super::parse_wall_utc("Africa/Cairo", "2026-11-26", "11:38").unwrap();
        assert_eq!(super::next_alert_delay_ms(&data, t_soon_done, 5, true, 15, 2), Some(4 * 60_000));
        // 11:43 Cairo: next event is Iqama alert at 11:42 + (15-2) = 11:55 (12 min = 720,000 ms away)
        let t_after_adhan = super::parse_wall_utc("Africa/Cairo", "2026-11-26", "11:43").unwrap();
        assert_eq!(super::next_alert_delay_ms(&data, t_after_adhan, 5, true, 15, 2), Some(12 * 60_000));
    }
}
