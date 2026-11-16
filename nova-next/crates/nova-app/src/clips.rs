//! QuickClips — saved text snippets manager for Nova Island.
//! Stores clips in userData/clips.json. Zero RAM when idle.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Clip {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(rename = "createdAt", default = "now_ms")]
    pub created_at: u64,
    #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const MAX_CLIPS: usize = 200;

#[derive(Clone)]
pub struct ClipsManager {
    path: PathBuf,
    clips: Arc<Mutex<Option<Vec<Clip>>>>,
}

impl ClipsManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("clips.json"),
            clips: Arc::new(Mutex::new(None)),
        }
    }

    fn load_internal(&self) -> Vec<Clip> {
        let mut guard = self.clips.lock().unwrap();
        if let Some(ref c) = *guard {
            return c.clone();
        }
        let list = match fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str::<Vec<Clip>>(&s).unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let valid: Vec<Clip> = list
            .into_iter()
            .filter(|c| !c.text.trim().is_empty() && c.text != "[object Promise]")
            .collect();
        *guard = Some(valid.clone());
        valid
    }

    fn save_internal(&self, list: &[Clip]) {
        let mut guard = self.clips.lock().unwrap();
        *guard = Some(list.to_vec());
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json_str) = serde_json::to_string_pretty(list) {
            let _ = fs::write(&self.path, json_str);
        }
    }

    pub fn list(&self) -> Value {
        let mut list = self.load_internal();
        // pinned first, then by createdAt desc
        list.sort_by(|a, b| {
            if a.pinned != b.pinned {
                b.pinned.cmp(&a.pinned)
            } else {
                b.created_at.cmp(&a.created_at)
            }
        });
        serde_json::to_value(list).unwrap_or_else(|_| json!([]))
    }

    pub fn add(&self, text: &str) -> Value {
        let text = text.trim();
        if text.is_empty() || text == "[object Promise]" {
            return Value::Null;
        }
        let text = if text.len() > 10000 { &text[..10000] } else { text }.to_string();
        let mut list = self.load_internal();

        // deduplicate: if already exists, move to front and update timestamp
        if let Some(pos) = list.iter().position(|c| c.text == text) {
            let mut existing = list.remove(pos);
            existing.created_at = now_ms();
            list.insert(0, existing.clone());
            self.save_internal(&list);
            return serde_json::to_value(existing).unwrap_or(Value::Null);
        }

        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("c:{}:{:x}", now_ms(), seq);
        let clip = Clip {
            id,
            text,
            pinned: false,
            created_at: now_ms(),
            updated_at: None,
        };
        list.insert(0, clip.clone());
        while list.len() > MAX_CLIPS {
            list.pop();
        }
        self.save_internal(&list);
        serde_json::to_value(clip).unwrap_or(Value::Null)
    }

    pub fn remove(&self, id: &str) -> Value {
        let mut list = self.load_internal();
        list.retain(|c| c.id != id);
        self.save_internal(&list);
        self.list()
    }

    pub fn pin(&self, id: &str, pinned: bool) -> Value {
        let mut list = self.load_internal();
        if let Some(c) = list.iter_mut().find(|c| c.id == id) {
            c.pinned = pinned;
            self.save_internal(&list);
        }
        self.list()
    }

    pub fn edit(&self, id: &str, text: &str) -> Value {
        let text = text.trim();
        if text.is_empty() {
            return self.list();
        }
        let mut list = self.load_internal();
        if let Some(c) = list.iter_mut().find(|c| c.id == id) {
            c.text = if text.len() > 10000 { &text[..10000] } else { text }.to_string();
            c.updated_at = Some(now_ms());
            self.save_internal(&list);
        }
        self.list()
    }

    pub fn clear(&self, keep_pinned: bool) -> Value {
        let mut list = self.load_internal();
        if keep_pinned {
            list.retain(|c| c.pinned);
        } else {
            list.clear();
        }
        self.save_internal(&list);
        self.list()
    }

    pub fn copy(&self, id: &str) -> bool {
        let list = self.load_internal();
        if let Some(c) = list.iter().find(|c| c.id == id) {
            nova_sys::clip::write_text(&c.text)
        } else {
            false
        }
    }

    pub fn add_from_clipboard(&self) -> Value {
        let text = nova_sys::clip::read_text();
        if text.trim().is_empty() {
            return Value::Null;
        }
        self.add(&text)
    }
}
