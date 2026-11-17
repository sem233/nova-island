// The calendar against the current calendar.js (parity/oracle/gen-calendar.cjs): 60 feeds, each
// read at 10 moments; the same events, ids, titles, times and Join links.
use std::{fs, path::PathBuf};

use serde_json::Value;

#[test]
fn calendar_like_calendar_js() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../parity/fixtures/calendar.json");
    let cases: Value = serde_json::from_str(&fs::read_to_string(p).expect("run parity/oracle/gen-calendar.cjs first")).unwrap();
    let mut wrong = Vec::new();
    let mut total = 0;
    for (n, c) in cases.as_array().unwrap().iter().enumerate() {
        let ics = c["ics"].as_str().unwrap();
        for (i, now) in c["now"].as_array().unwrap().iter().enumerate() {
            let now = now.as_f64().unwrap();
            // Floating times in `parity/fixtures/calendar.json` were generated under `Africa/Cairo`.
            let got = Value::Array(nova_services::calendar::occurrences_in_zone(ics, now, now + 86_400_000.0, "Africa/Cairo"));
            total += 1;
            // (1.8.7) calendar.js cancels the wrong day here: RECURRENCE-ID 20 Nov 21:00 Pacific is
            // matched by its written date to the UTC date of the 19th's meeting (20 Nov 05:00 UTC).
            // 1.8 matches the exact time, so the 19th's meeting stays (the 20th's is past `to`).
            if (n, i) == (32, 8) {
                let ids: Vec<&str> = got.as_array().unwrap().iter().filter_map(|e| e["id"].as_str()).collect();
                assert_eq!(ids, ["ev-32-5@nova|1795150800000", "ev-32-0@nova|1795185900000"], "feed 32 moment 8");
                continue;
            }
            if got != c["results"][i] {
                wrong.push(format!("feed {n} moment {i}:\n  got  {got}\n  want {}", c["results"][i]));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {total} differ:\n{}", wrong.len(), wrong.iter().take(6).cloned().collect::<Vec<_>>().join("\n"));
}
