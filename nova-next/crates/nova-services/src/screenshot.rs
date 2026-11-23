//! Copied images (nova.js onClipboardSeq / shotToShelf): a copied image shows as a toast with a
//! small thumbnail; with "Save screenshots to Shelf" it goes on the shelf by itself for a while.
//! The Snipping Tool already saves every capture to Pictures\Screenshots: that file goes on the
//! shelf (no second copy). Any other copied image is saved to Pictures\Nova Island first.
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;

use crate::{
    ctx::{now_ms, Ctx},
    shelf::Shelf,
};

const SNIP_WAIT_MS: u64 = 2500;

fn pictures() -> PathBuf {
    unsafe {
        windows::Win32::UI::Shell::SHGetKnownFolderPath(&windows::Win32::UI::Shell::FOLDERID_Pictures, windows::Win32::UI::Shell::KNOWN_FOLDER_FLAG(0), None)
            .ok()
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
                PathBuf::from(s)
            })
            .unwrap_or_default()
    }
}

type PinCb = Arc<dyn Fn(u32, u32, Vec<u8>) + Send + Sync>;

/// Fast sampled fingerprint of a clipboard image so Windows Snipping Tool updating the clipboard
/// twice for the same snip (CF_DIB + save metadata) never spawns duplicate pins or duplicate toasts.
fn image_fingerprint(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mix = |h: &mut u64, v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(0x100000001b3);
    };
    mix(&mut hash, w as u64);
    mix(&mut hash, h as u64);
    mix(&mut hash, px.len() as u64);
    if !px.is_empty() {
        let step = (px.len() / 64).max(1);
        let mut i = 0;
        while i < px.len() {
            mix(&mut hash, px[i] as u64);
            i += step;
        }
    }
    hash
}

pub struct Screenshots {
    ctx: Ctx,
    shelf: Arc<Shelf>,
    last: Mutex<Option<(u32, u32, Vec<u8>)>>,
    last_hash: Mutex<Option<(u64, f64)>>,
    on_pin: Mutex<Option<PinCb>>,
}

impl Screenshots {
    pub fn new(ctx: Ctx, shelf: Arc<Shelf>) -> Arc<Screenshots> {
        Arc::new(Screenshots { ctx, shelf, last: Mutex::new(None), last_hash: Mutex::new(None), on_pin: Mutex::new(None) })
    }

    /// Registers the callback that spawns a native 1:1 floating pin overlay for a captured screenshot.
    pub fn on_pin(&self, f: impl Fn(u32, u32, Vec<u8>) + Send + Sync + 'static) {
        *self.on_pin.lock().unwrap() = Some(Arc::new(f));
    }

    /// Most recently observed clipboard image as top-down 32-bit BGRA.
    pub fn last_image(&self) -> Option<(u32, u32, Vec<u8>)> {
        self.last.lock().unwrap().clone()
    }

    /// Fast, non-blocking check for a freshly saved Windows Snipping Tool capture in Pictures\Screenshots.
    fn has_recent_windows_snip(now: f64) -> bool {
        let dir = pictures().join("Screenshots");
        std::fs::read_dir(&dir).ok().is_some_and(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| matches!(x.to_string_lossy().to_lowercase().as_str(), "png" | "jpg" | "jpeg")))
                .filter_map(|p| std::fs::metadata(&p).ok())
                .any(|m| {
                    m.len() > 0
                        && m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .is_some_and(|d| {
                                let t = d.as_millis() as f64;
                                t >= now - 3500.0 && t <= now + 5000.0
                            })
                })
        })
    }

    /// After a clipboard change (not a private one). `snip_hint` is true when PrtScn, Win+Shift+S,
    /// Snipping Tool, or Nova's snip trigger was active.
    pub fn clipboard_changed(self: &Arc<Self>, snip_hint: bool) {
        let want_toast = self.ctx.on("screenshotToast");
        let want_pin_btn = self.ctx.on("shotPinOverlay");
        let want_auto_pin = self.ctx.on("shotAutoPin");
        if !want_toast && !want_pin_btn && !want_auto_pin {
            return;
        }
        let Some((w, h, px)) = nova_sys::clip::read_image() else { return };
        let now = now_ms();
        let fp = image_fingerprint(w, h, &px);
        {
            let mut prev = self.last_hash.lock().unwrap();
            if prev.is_some_and(|(old_fp, old_at)| old_fp == fp && (now - old_at).abs() < 4500.0) {
                return;
            }
            *prev = Some((fp, now));
        }

        let is_snip = snip_hint || Self::has_recent_windows_snip(now);
        let mut auto_pinned = false;
        if want_auto_pin && is_snip {
            if let Some(cb) = self.on_pin.lock().unwrap().clone() {
                cb(w, h, px.clone());
                auto_pinned = true;
            }
        }
        *self.last.lock().unwrap() = Some((w, h, px.clone()));

        if !want_toast && !want_pin_btn {
            return;
        }
        let thumb = nova_sys::icons::bgra_png(w, h, &px, Some(72)).map(|b| format!("data:image/png;base64,{}", nova_sys::media::base64(&b))).unwrap_or_default();
        let auto = self.ctx.on("shotAutoShelf") && self.ctx.on("tabShelf");
        self.ctx.send(
            "screenshot",
            json!({
                "thumb": thumb,
                "width": w,
                "height": h,
                "auto": auto,
                "canPin": want_pin_btn,
                "autoPinned": auto_pinned,
            }),
        );
        if auto {
            let minutes = self.ctx.feature("shotShelfMinutes").as_f64().unwrap_or(20.0);
            self.to_shelf(now + minutes * 60.0 * 1000.0);
        }
    }

    /// The newest picture the Snipping Tool saved since `since` (it may take a moment).
    fn windows_screenshot(since: f64) -> Option<PathBuf> {
        let dir = pictures().join("Screenshots");
        let mut waited = 0;
        while waited <= SNIP_WAIT_MS {
            let fresh = std::fs::read_dir(&dir).ok().and_then(|r| {
                r.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| matches!(x.to_string_lossy().to_lowercase().as_str(), "png" | "jpg" | "jpeg")))
                    .filter_map(|p| std::fs::metadata(&p).ok().map(|m| (p, m)))
                    .filter(|(_, m)| m.len() > 0 && m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).is_some_and(|d| d.as_millis() as f64 >= since - 1500.0))
                    .max_by_key(|(_, m)| m.modified().ok())
            });
            if let Some((p, _)) = fresh {
                return Some(p);
            }
            std::thread::sleep(Duration::from_millis(250));
            waited += 250;
        }
        None
    }

    /// "Add to Shelf", or the automatic one (until > 0: leaves the shelf then).
    pub fn to_shelf(self: &Arc<Self>, until: f64) {
        let me = self.clone();
        // (1.8.7) the picture and its time as they are now: a newer copy before the thread runs
        // isn't saved in this one's place
        let Some((w, h, px)) = me.last.lock().unwrap().clone() else { return };
        let copied_at = now_ms();
        std::thread::spawn(move || {
            let file = if until > 0.0 { Self::windows_screenshot(copied_at) } else { None };
            let file = match file {
                Some(f) => f,
                None => {
                    let dir = pictures().join("Nova Island");
                    let zone = nova_prayer::icu::default_zone();
                    let Some(t) = nova_prayer::wall(&zone, copied_at) else { return };
                    let base = format!("Screenshot {}-{:02}-{:02} {:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
                    let Some(png) = nova_sys::icons::bgra_png(w, h, &px, None) else { return };
                    // (1.8.7) two in the same second: "… (2).png", never over the first one
                    let saved = std::fs::create_dir_all(&dir).ok().and_then(|_| {
                        (1..100).find_map(|i| {
                            let path = dir.join(if i == 1 { format!("{base}.png") } else { format!("{base} ({i}).png") });
                            use std::io::Write;
                            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                                Ok(mut f) => Some(f.write_all(&png).map(|_| path)),
                                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                                Err(e) => Some(Err(e)),
                            }
                        })
                    });
                    let Some(Ok(path)) = saved else {
                        nova_core::log::error("[screenshot] can't save the picture");
                        return;
                    };
                    path
                }
            };
            me.shelf.add(&[file.to_string_lossy().into_owned()], until);
            if until == 0.0 {
                let name = Path::new(&file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                me.ctx.send("shot-saved", json!({ "name": name }));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::image_fingerprint;

    #[test]
    fn fingerprint_matches_identical_and_separates_different_captures() {
        let a = vec![0x42u8; 400];
        let b = a.clone();
        let mut c = a.clone();
        c[12] = 0x99;
        assert_eq!(image_fingerprint(10, 10, &a), image_fingerprint(10, 10, &b));
        assert_ne!(image_fingerprint(10, 10, &a), image_fingerprint(10, 10, &c));
        assert_ne!(image_fingerprint(10, 10, &a), image_fingerprint(20, 5, &a));
        assert_ne!(image_fingerprint(0, 0, &[]), image_fingerprint(1, 1, &[1, 2, 3, 4]));
    }
}
