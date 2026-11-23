//! (1.8.7 / 1.8.15) The adhan's sound: Settings › Prayer & Dhikr › "Adhan sound" plays it with the
//! prayer alert (pages/views/adhan.js), and the alert lasts as long as the adhan. Supports both
//! local user audio files and one-click curated online Adhans cached locally on disk (zero ongoing
//! network or RAM usage once downloaded). The pages play https://nova.island/adhan-audio and
//! /adhan-fajr-audio, which nova-web answers from here.
use std::path::PathBuf;

use serde_json::{json, Value};

const EXTS: &[&str] = &["mp3", "m4a", "ogg", "opus", "wav", "aac"];
/// a few minutes of audio is well under this
const MAX_BYTES: u64 = 30 * 1024 * 1024;
const MIN_MP3_BYTES: usize = 8 * 1024;

pub struct AdhanPreset {
    pub id: &'static str,
    pub en: &'static str,
    pub ar: &'static str,
    pub url: &'static str,
    pub fajr_only: bool,
}

/// Curated public-domain / open-source Adhan MP3 recordings (downloaded once on user demand).
pub const PRESETS: &[AdhanPreset] = &[
    AdhanPreset {
        id: "makkah",
        en: "Makkah (Al-Haram)",
        ar: "أذان الحرم المكي (مكة)",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Makkah.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "alafasy",
        en: "Mishary Rashid Alafasy",
        ar: "مشاري راشد العفاسي",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Mishary%20Alafasi.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "abdulbasit",
        en: "Abdul Basit Abdus Samad",
        ar: "عبد الباسط عبد الصمد",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Abed%20Albase6.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "refaat",
        en: "Sheikh Muhammad Refaat",
        ar: "الشيخ محمد رفعت",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Mohammad%20Ref3at.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "minshawi",
        en: "Muhammad Siddiq Al-Minshawi",
        ar: "محمد صديق المنشاوي",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Mohammad%20Almenshawy.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "qatami",
        en: "Nasser Al-Qatami",
        ar: "ناصر القطامي",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Nasser%20Alqatami.mp3",
        fajr_only: false,
    },
    AdhanPreset {
        id: "fajr_malek",
        en: "Fajr Adhan (Malek Chebae)",
        ar: "أذان الفجر (مالك شيبة)",
        url: "https://raw.githubusercontent.com/abodehq/Athan-MP3/master/Sounds/Athan%20Al-fajer%20-%20Malek%20chebae.mp3",
        fajr_only: true,
    },
];

pub fn find_preset(id: &str) -> Option<&'static AdhanPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn looks_like_mp3(bytes: &[u8]) -> bool {
    if bytes.len() < MIN_MP3_BYTES || (bytes.len() as u64) > MAX_BYTES {
        return false;
    }
    bytes.starts_with(b"ID3") || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0)
}

/// "fajr" or the others ("")
fn stem(fajr: bool) -> &'static str {
    if fajr {
        "adhan-fajr-custom"
    } else {
        "adhan-custom"
    }
}

fn custom(fajr: bool) -> Option<PathBuf> {
    let dir = crate::data_dir();
    EXTS.iter().map(|e| dir.join(format!("{}.{e}", stem(fajr)))).find(|p| p.is_file())
}

/// Points nova-web at the chosen files (or back at Nova's own). At start and after a change.
pub fn apply() {
    nova_web::set_adhan_file(false, custom(false));
    nova_web::set_adhan_file(true, custom(true));
}

fn one(fajr: bool) -> Value {
    let dir = crate::data_dir();
    let has_custom = custom(fajr).is_some();
    let name = has_custom.then(|| std::fs::read_to_string(dir.join(format!("{}.txt", stem(fajr)))).unwrap_or_else(|_| "adhan".into()));
    let preset = has_custom
        .then(|| std::fs::read_to_string(dir.join(format!("{}.preset", stem(fajr)))).ok())
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let own = nova_assets::get(if fajr { "sounds/adhan-fajr.mp3" } else { "sounds/adhan.mp3" }).is_some();
    json!({ "name": name, "custom": has_custom, "hasDefault": own, "preset": preset })
}

/// {main, fajr, presets} for Settings.
pub fn info() -> Value {
    let presets: Vec<Value> = PRESETS
        .iter()
        .map(|p| json!({ "id": p.id, "en": p.en, "ar": p.ar, "fajrOnly": p.fajr_only }))
        .collect();
    json!({ "main": one(false), "fajr": one(true), "presets": presets })
}

fn clear(fajr: bool) {
    let dir = crate::data_dir();
    for e in EXTS {
        let _ = std::fs::remove_file(dir.join(format!("{}.{e}", stem(fajr))));
    }
    let _ = std::fs::remove_file(dir.join(format!("{}.txt", stem(fajr))));
    let _ = std::fs::remove_file(dir.join(format!("{}.preset", stem(fajr))));
}

/// Downloads an allowlisted online Adhan preset once and caches it locally in Nova's data folder.
pub fn download_preset(fajr: bool, preset_id: &str) -> Value {
    let Some(p) = find_preset(preset_id) else {
        return json!({ "error": "unknown" });
    };
    let resp = match nova_core::http::get(p.url, 30_000) {
        Ok(r) if r.ok() => r,
        Ok(r) => {
            nova_core::log::warn(&format!("adhan preset {} HTTP {}", p.id, r.status));
            return json!({ "error": "network" });
        }
        Err(e) => {
            nova_core::log::warn(&format!("adhan preset {} failed: {e}", p.id));
            return json!({ "error": "network" });
        }
    };
    if !looks_like_mp3(&resp.body) {
        return json!({ "error": "type" });
    }
    let dir = crate::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    clear(fajr);
    let dest = dir.join(format!("{}.mp3", stem(fajr)));
    if std::fs::write(&dest, &resp.body).is_err() {
        clear(fajr);
        return json!({ "error": "copy" });
    }
    let _ = std::fs::write(dir.join(format!("{}.txt", stem(fajr))), p.en);
    let _ = std::fs::write(dir.join(format!("{}.preset", stem(fajr))), p.id);
    nova_core::log::info(&format!("adhan: downloaded preset {} ({})", p.id, if fajr { "fajr" } else { "main" }));
    apply();
    json!({ "ok": true, "id": p.id, "name": p.en })
}

/// Takes a file the user picked: {ok} or {error}.
pub fn set_file(fajr: bool, path: &str) -> Value {
    let p = PathBuf::from(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !EXTS.contains(&ext.as_str()) {
        return json!({ "error": "type" });
    }
    if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(u64::MAX) > MAX_BYTES {
        return json!({ "error": "size" });
    }
    clear(fajr);
    let dir = crate::data_dir();
    if std::fs::copy(&p, dir.join(format!("{}.{ext}", stem(fajr)))).is_err() {
        clear(fajr);
        return json!({ "error": "copy" });
    }
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("adhan").to_string();
    let _ = std::fs::write(dir.join(format!("{}.txt", stem(fajr))), name);
    nova_core::log::info(if fajr { "adhan: own Fajr file chosen" } else { "adhan: own file chosen" });
    apply();
    json!({ "ok": true })
}

/// Back to Nova's own adhan.
pub fn reset(fajr: bool) {
    clear(fajr);
    apply();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_lookup_only_matches_allowlisted_ids() {
        assert!(find_preset("makkah").is_some());
        assert!(find_preset("alafasy").is_some());
        assert!(find_preset("fajr_malek").is_some());
        assert!(find_preset("https://evil.example/x.mp3").is_none());
        assert!(find_preset("").is_none());
    }

    #[test]
    fn mp3_validation_checks_size_and_header() {
        let mut valid = vec![0u8; MIN_MP3_BYTES + 128];
        valid[0..3].copy_from_slice(b"ID3");
        assert!(looks_like_mp3(&valid));
        let mut frame = vec![0u8; MIN_MP3_BYTES + 128];
        frame[0] = 0xFF;
        frame[1] = 0xFB;
        assert!(looks_like_mp3(&frame));
        assert!(!looks_like_mp3(b"ID3tiny"));
        let html = vec![b'<'; MIN_MP3_BYTES + 128];
        assert!(!looks_like_mp3(&html));
    }
}
