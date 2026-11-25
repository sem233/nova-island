//! Prayer times calculated on this PC, no network (prayer.js): the PrayTimes method (praytimes.org,
//! the same one the AlAdhan service uses), with the calculation method each country follows. The
//! Hijri date is the Umm al-Qura calendar. Every step mirrors prayer.js, so the minutes are the same.
pub mod icu;
pub mod tz;

use serde_json::{json, Value};

/// fajr / isha: sun angle below the horizon, or isha as minutes after maghrib;
/// maghrib: an angle, or minutes after sunset (default: at sunset)
#[derive(Clone, Copy, PartialEq)]
pub enum Spec {
    Angle(f64),
    Minutes(f64),
    None,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Method {
    pub id: &'static str,
    pub name: &'static str,
    pub fajr: Spec,
    pub isha: Spec,
    pub maghrib: Spec,
    /// the authority's own minute offsets: (dhuhr, maghrib)
    pub tune: (f64, f64),
}

use Spec::*;
const fn m(id: &'static str, name: &'static str, fajr: f64, isha: Spec, maghrib: Spec, tune: (f64, f64)) -> Method {
    Method { id, name, fajr: Angle(fajr), isha, maghrib, tune }
}

pub const METHODS: &[Method] = &[
    m("MWL", "Muslim World League", 18.0, Angle(17.0), None, (0.0, 0.0)),
    m("ISNA", "Islamic Society of North America", 15.0, Angle(15.0), None, (0.0, 0.0)),
    m("EGYPT", "Egyptian General Authority of Survey", 19.5, Angle(17.5), None, (0.0, 0.0)),
    m("MAKKAH", "Umm Al-Qura University, Makkah", 18.5, Minutes(90.0), None, (0.0, 0.0)),
    m("KARACHI", "University of Islamic Sciences, Karachi", 18.0, Angle(18.0), None, (0.0, 0.0)),
    m("TEHRAN", "Institute of Geophysics, Tehran", 17.7, Angle(14.0), Angle(4.5), (0.0, 0.0)),
    m("GULF", "Gulf Region", 19.5, Minutes(90.0), None, (0.0, 0.0)),
    m("KUWAIT", "Kuwait", 18.0, Angle(17.5), None, (0.0, 0.0)),
    m("QATAR", "Qatar", 18.0, Minutes(90.0), None, (0.0, 0.0)),
    m("SINGAPORE", "Majlis Ugama Islam Singapura", 20.0, Angle(18.0), None, (0.0, 0.0)),
    m("JAKIM", "JAKIM, Malaysia", 20.0, Angle(18.0), None, (0.0, 0.0)),
    m("KEMENAG", "Kemenag, Indonesia", 20.0, Angle(18.0), None, (0.0, 0.0)),
    m("FRANCE", "Union des Organisations Islamiques de France", 12.0, Angle(12.0), None, (0.0, 0.0)),
    m("TURKEY", "Diyanet, Turkey", 18.0, Angle(17.0), None, (0.0, 0.0)),
    m("RUSSIA", "Spiritual Administration of Muslims of Russia", 16.0, Angle(15.0), None, (0.0, 0.0)),
    m("DUBAI", "Dubai", 18.2, Angle(18.2), None, (3.0, 3.0)),
    m("TUNISIA", "Tunisia", 18.0, Angle(18.0), None, (0.0, 0.0)),
    m("ALGERIA", "Algeria", 18.0, Angle(17.0), None, (0.0, 0.0)),
    m("MOROCCO", "Morocco", 19.0, Angle(17.0), None, (0.0, 0.0)),
    m("JORDAN", "Ministry of Awqaf, Jordan", 18.0, Angle(18.0), Minutes(5.0), (0.0, 0.0)),
];

fn method(id: &str) -> Method {
    *METHODS.iter().find(|m| m.id == id).unwrap_or(&METHODS[0])
}

pub fn method_by_id(id: &str) -> Option<Method> {
    METHODS.iter().copied().find(|m| m.id.eq_ignore_ascii_case(id))
}

fn by_country(code: &str) -> Option<&'static str> {
    Some(match code {
        "EG" | "SD" | "LY" | "SY" | "LB" => "EGYPT",
        "IQ" => "MWL",
        "SA" | "YE" => "MAKKAH",
        "AE" => "DUBAI",
        "KW" => "KUWAIT",
        "QA" => "QATAR",
        "BH" | "OM" => "GULF",
        "JO" | "PS" => "JORDAN",
        "MA" => "MOROCCO",
        "DZ" => "ALGERIA",
        "TN" => "TUNISIA",
        "TR" => "TURKEY",
        "IR" => "TEHRAN",
        "PK" | "IN" | "BD" | "AF" => "KARACHI",
        "SG" | "BN" => "SINGAPORE",
        "MY" => "JAKIM",
        "ID" => "KEMENAG",
        "US" | "CA" => "ISNA",
        "FR" => "FRANCE",
        "RU" => "RUSSIA",
        _ => return Option::None,
    })
}

/// A location saved before countries were known: the country from its time zone.
fn zone_country(zone: &str) -> &'static str {
    match zone {
        "Africa/Cairo" => "EG", "Asia/Riyadh" => "SA", "Asia/Dubai" => "AE", "Asia/Kuwait" => "KW", "Asia/Qatar" => "QA",
        "Asia/Bahrain" => "BH", "Asia/Muscat" => "OM", "Asia/Amman" => "JO", "Africa/Casablanca" => "MA", "Africa/Algiers" => "DZ",
        "Africa/Tunis" => "TN", "Europe/Istanbul" => "TR", "Asia/Tehran" => "IR", "Asia/Karachi" => "PK", "Asia/Kolkata" => "IN",
        "Asia/Dhaka" => "BD", "Asia/Singapore" => "SG", "Asia/Kuala_Lumpur" => "MY", "Asia/Jakarta" => "ID", "Europe/Paris" => "FR",
        "Asia/Baghdad" => "IQ",
        _ => "",
    }
}

pub fn method_for(country: &str, timezone: &str) -> Method {
    let code = country.to_uppercase();
    let c = if code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()) { code.as_str() } else { zone_country(timezone) };
    method(by_country(c).unwrap_or("MWL"))
}

// ---- PrayTimes (degrees, hours) ----
fn rad(d: f64) -> f64 {
    d * std::f64::consts::PI / 180.0
}
fn deg(r: f64) -> f64 {
    r * 180.0 / std::f64::consts::PI
}
fn sin(d: f64) -> f64 {
    rad(d).sin()
}
fn cos(d: f64) -> f64 {
    rad(d).cos()
}
fn tan(d: f64) -> f64 {
    rad(d).tan()
}
fn asin(x: f64) -> f64 {
    deg(x.asin())
}
fn acos(x: f64) -> f64 {
    deg(x.acos())
}
fn atan2(y: f64, x: f64) -> f64 {
    deg(y.atan2(x))
}
fn acot(x: f64) -> f64 {
    deg((1.0 / x).atan())
}
fn fix(a: f64, b: f64) -> f64 {
    let r = a - b * (a / b).floor();
    if r < 0.0 { r + b } else { r }
}
fn fix_angle(a: f64) -> f64 {
    fix(a, 360.0)
}
fn fix_hour(h: f64) -> f64 {
    fix(h, 24.0)
}

fn julian(mut y: f64, mut m: f64, d: f64) -> f64 {
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + d + b - 1524.5
}

/// (declination, equation of time)
fn sun_position(jd: f64) -> (f64, f64) {
    let d = jd - 2451545.0;
    let g = fix_angle(357.529 + 0.98560028 * d);
    let q = fix_angle(280.459 + 0.98564736 * d);
    let l = fix_angle(q + 1.915 * sin(g) + 0.020 * sin(2.0 * g));
    let e = 23.439 - 0.00000036 * d;
    let ra = atan2(cos(e) * sin(l), cos(l)) / 15.0;
    (asin(sin(e) * sin(l)), q / 15.0 - fix_hour(ra))
}

/// Times in UTC hours for the civil day y-m-d at lat/lon.
#[derive(Debug, Clone, Copy)]
pub struct Times {
    pub fajr: f64,
    pub sunrise: f64,
    pub dhuhr: f64,
    pub asr: f64,
    pub sunset: f64,
    pub maghrib: f64,
    pub isha: f64,
}

pub fn compute_day(y: i32, mo: u32, d: u32, lat: f64, lon: f64, method: &Method, asr_factor: f64) -> Times {
    let j_date = julian(y as f64, mo as f64, d as f64) - lon / (15.0 * 24.0);
    let mid_day = |t: f64| fix_hour(12.0 - sun_position(j_date + t).1);
    let sun_angle_time = |angle: f64, t: f64, ccw: bool| {
        let decl = sun_position(j_date + t).0;
        let noon = mid_day(t);
        let h = acos((-sin(angle) - sin(decl) * sin(lat)) / (cos(decl) * cos(lat))) / 15.0;
        noon + if ccw { -h } else { h }
    };
    let asr_time = |factor: f64, t: f64| {
        let decl = sun_position(j_date + t).0;
        sun_angle_time(-acot(factor + tan((lat - decl).abs())), t, false)
    };
    // one refinement pass from rough guesses (day fractions), as PrayTimes does
    let g = |h: f64| h / 24.0;
    let angle = |s: Spec| if let Angle(a) = s { Some(a) } else { Option::None };
    let mut fajr = sun_angle_time(angle(method.fajr).unwrap(), g(5.0), true);
    let mut sunrise = sun_angle_time(0.833, g(6.0), true);
    let mut dhuhr = mid_day(g(12.0));
    let mut asr = asr_time(asr_factor, g(13.0));
    let mut sunset = sun_angle_time(0.833, g(18.0), false);
    let mut maghrib = angle(method.maghrib).map(|a| sun_angle_time(a, g(18.0), false));
    let mut isha = angle(method.isha).map(|a| sun_angle_time(a, g(18.0), false));
    let shift = lon / 15.0;
    for v in [&mut fajr, &mut sunrise, &mut dhuhr, &mut asr, &mut sunset] {
        *v -= shift;
    }
    if let Some(v) = maghrib.as_mut() {
        *v -= shift;
    }
    if let Some(v) = isha.as_mut() {
        *v -= shift;
    }
    // high latitudes: Fajr / Isha no later than a share of the night ("angle based")
    let night = fix_hour(sunrise - sunset);
    let portion = |a: f64| (a / 60.0) * night;
    if let Angle(a) = method.fajr {
        let p = portion(a);
        if fajr.is_nan() || fix_hour(sunrise - fajr) > p {
            fajr = sunrise - p;
        }
    }
    let minutes = |s: Spec| if let Minutes(n) = s { n } else { 0.0 };
    let maghrib = maghrib.unwrap_or(sunset + minutes(method.maghrib) / 60.0);
    let isha = match method.isha {
        Minutes(n) => maghrib + n / 60.0,
        _ => {
            let p = portion(angle(method.isha).unwrap_or(0.0));
            let i = isha.unwrap_or(f64::NAN);
            if i.is_nan() || fix_hour(i - sunset) > p { sunset + p } else { i }
        }
    };
    Times { fajr, sunrise, dhuhr, asr, sunset, maghrib, isha }
}

// ---- days, zones and the Hijri date ----
/// Days since 1970-01-01 for a civil date; the month/day may overflow like Date.UTC's.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let (y, m) = (y + (m - 1).div_euclid(12), (m - 1).rem_euclid(12) + 1);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468 + (d - 1)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Day {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

/// The calendar date in `zone` for a moment (ms since 1970).
pub fn day_in(zone: &str, ms: f64) -> Option<Day> {
    wall(zone, ms).map(|f| Day { y: f.year, m: f.month, d: f.day })
}

/// Local time in `zone`. ICU's tz data (frozen at 2022g) has the full history; Windows' own zone
/// data is kept current by Windows Update but only knows recent years. So: ICU before 2023, Windows
/// from 2023 on (ICU when Windows doesn't know the zone).
pub fn wall(zone: &str, ms: f64) -> Option<icu::Fields> {
    const Y2023: f64 = 1_672_531_200_000.0;
    if ms < Y2023 {
        return icu::fields(zone, "gregorian", ms).or_else(|| tz::local(zone, ms));
    }
    tz::local(zone, ms).or_else(|| icu::fields(zone, "gregorian", ms))
}

pub fn next_day(day: Day) -> Day {
    let ms = (days_from_civil(day.y as i64, day.m as i64, day.d as i64 + 1) * 86_400_000) as f64;
    let f = icu::fields("UTC", "gregorian", ms).expect("UTC");
    Day { y: f.year, m: f.month, d: f.day }
}

/// UTC hours on a day -> "HH:MM" in `zone`, rounded to the nearest minute. Errors like Date does
/// when the time can't exist (no sunrise near the poles): "Invalid time value".
fn clock_in(zone: &str, day: Day, hours: f64) -> Result<String, String> {
    let ms = (days_from_civil(day.y as i64, day.m as i64, day.d as i64) * 86_400_000) as f64 + hours * 3600e3 + 30e3;
    if !ms.is_finite() {
        return Err("Invalid time value".into());
    }
    let f = wall(zone, ms.trunc()).ok_or("bad time zone")?;
    Ok(format!("{:02}:{:02}", f.hour, f.minute))
}

const HIJRI_MONTHS: [&str; 12] = ["محرم", "صفر", "ربيع الأول", "ربيع الآخر", "جمادى الأولى", "جمادى الآخرة", "رجب", "شعبان", "رمضان", "شوال", "ذو القعدة", "ذو الحجة"];

/// Umm al-Qura Hijri date of a civil day, moved by the moon-sighting adjustment (days):
/// ("16 رجب 1447", month 1-12).
pub fn hijri_of(day: Day, adjust: i64) -> (String, u32) {
    let ms = (days_from_civil(day.y as i64, day.m as i64, day.d as i64 + adjust) * 86_400_000 + 12 * 3_600_000) as f64;
    let f = icu::fields("UTC", "islamic-umalqura", ms).expect("Umm al-Qura calendar");
    (format!("{} {} {}", f.day, HIJRI_MONTHS[(f.month - 1) as usize], f.year), f.month)
}

const POLAR_LAT: f64 = 65.0;

pub struct Settings {
    pub asr_hanafi: bool,
    pub hijri_adjust: i64,
}

/// One day's times as prayer.js's calcDay returns them (the same JSON shape and order).
pub fn calc_day(day: Day, country: &str, lat: f64, lon: f64, timezone: &str, zone: &str, s: &Settings) -> Result<Value, String> {
    calc_day_ext(day, country, lat, lon, timezone, zone, s, "", 0)
}

/// Extended prayer calculation supporting an explicit calculation method override (`"EGYPT"`,
/// `"MAKKAH"`, etc.; empty or `"auto"` auto-selects by country) and a manual minute offset (`-15..=15`).
pub fn calc_day_ext(
    day: Day,
    country: &str,
    lat: f64,
    lon: f64,
    timezone: &str,
    zone: &str,
    s: &Settings,
    method_override: &str,
    offset_min: i64,
) -> Result<Value, String> {
    let mut method = if method_override.is_empty() || method_override.eq_ignore_ascii_case("auto") {
        method_for(country, timezone)
    } else {
        method_by_id(method_override).unwrap_or_else(|| method_for(country, timezone))
    };
    let (hijri, hijri_month) = hijri_of(day, s.hijri_adjust);
    // Umm al-Qura: Isha two hours after Maghrib in Ramadan
    if method.id == "MAKKAH" && hijri_month == 9 {
        method.isha = Minutes(120.0);
    }
    let factor = if s.asr_hanafi { 2.0 } else { 1.0 };
    let mut t = compute_day(day.y, day.m, day.d, lat, lon, &method, factor);
    // near the poles the sun may not rise or set at all that day: use the nearest latitude where it
    // does (65°, "aqrab al-bilad"), instead of failing the whole day
    if [t.fajr, t.sunrise, t.dhuhr, t.asr, t.sunset, t.maghrib, t.isha].iter().any(|v| v.is_nan()) {
        t = compute_day(day.y, day.m, day.d, lat.signum() * lat.abs().min(POLAR_LAT), lon, &method, factor);
    }
    // the authority's own minute offsets, plus one minute of precaution on Asr
    t.asr += 1.0 / 60.0;
    t.dhuhr += method.tune.0 / 60.0;
    t.maghrib += method.tune.1 / 60.0;
    let shift_h = (offset_min.clamp(-15, 15) as f64) / 60.0;
    let at = |h: f64| clock_in(zone, day, h + shift_h);
    Ok(json!({
        "date": format!("{}-{:02}-{:02}", day.y, day.m, day.d),
        "times": {
            "imsak": at(t.fajr - 10.0 / 60.0)?,
            "fajr": at(t.fajr)?,
            "sunrise": at(t.sunrise)?,
            "dhuhr": at(t.dhuhr)?,
            "asr": at(t.asr)?,
            "maghrib": at(t.maghrib)?,
            "isha": at(t.isha)?,
        },
        "hijri": hijri,
        "hijriMonth": hijri_month,
        "method": method.name,
        "tz": zone,
    }))
}

/// Wall-clock fields as if they were UTC, in ms (for offsets and local -> UTC).
pub fn wall_ms(f: &icu::Fields) -> f64 {
    (days_from_civil(f.year as i64, f.month as i64, f.day as i64) * 86_400_000) as f64 + (f.hour * 3600 + f.minute * 60 + f.second) as f64 * 1000.0
}

/// The UTC moment of a wall-clock time in `zone` (ms as if UTC). Like Temporal's "compatible"
/// choice: in a repeated hour the earlier moment, in a skipped hour the time moved forward.
pub fn local_to_utc(zone: &str, wall: f64) -> Option<f64> {
    let offset = |utc: f64| self::wall(zone, utc).map(|f| wall_ms(&f) - utc);
    let before = offset(wall - 86_400_000.0)?;
    let after = offset(wall + 86_400_000.0)?;
    let fits = |o: f64| self::wall(zone, wall - o).map(|f| wall_ms(&f)) == Some(wall);
    match (fits(before), fits(after)) {
        (true, true) => Some((wall - before).min(wall - after)),
        (true, false) => Some(wall - before),
        (false, true) => Some(wall - after),
        // a skipped hour: keep the offset from before the change, which lands after it
        (false, false) => Some(wall - before),
    }
}
