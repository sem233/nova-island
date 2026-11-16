use std::sync::atomic::{AtomicPtr, Ordering};
use windows::core::PCSTR;
use windows::Win32::Foundation::{FARPROC, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

static MODULE: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

pub fn get_module() -> HMODULE {
    let ptr = MODULE.load(Ordering::Acquire);
    if !ptr.is_null() {
        return HMODULE(ptr);
    }

    let hmod = load_webview2_loader();
    MODULE.store(hmod.0, Ordering::Release);
    hmod
}

fn load_webview2_loader() -> HMODULE {
    use windows::core::w;

    // 1. Try loading directly from the current executable directory or PATH
    unsafe {
        if let Ok(h) = LoadLibraryW(w!("WebView2Loader.dll")) {
            if !h.is_invalid() {
                return h;
            }
        }
    }

    // 2. Try AppData/Roaming/dynamic-nova/tools/WebView2Loader.dll
    let embedded_bytes: &[u8] = include_bytes!("../x64/WebView2Loader.dll");

    if let Ok(app_data) = std::env::var("APPDATA") {
        let tools_dir = std::path::PathBuf::from(app_data).join("dynamic-nova").join("tools");
        let target = tools_dir.join("WebView2Loader.dll");
        let write_needed = match std::fs::metadata(&target) {
            Ok(m) => m.len() != embedded_bytes.len() as u64,
            Err(_) => true,
        };
        if write_needed {
            let _ = std::fs::create_dir_all(&tools_dir);
            let _ = std::fs::write(&target, embedded_bytes);
        }
        if let Some(path_str) = target.to_str() {
            let wide: Vec<u16> = path_str.encode_utf16().chain(core::iter::once(0)).collect();
            unsafe {
                if let Ok(h) = LoadLibraryW(windows::core::PCWSTR(wide.as_ptr())) {
                    if !h.is_invalid() {
                        return h;
                    }
                }
            }
        }
    }

    // 3. Fallback: LocalAppData or Temp
    if let Ok(temp) = std::env::var("TEMP") {
        let target = std::path::PathBuf::from(temp).join("WebView2Loader.dll");
        let write_needed = match std::fs::metadata(&target) {
            Ok(m) => m.len() != embedded_bytes.len() as u64,
            Err(_) => true,
        };
        if write_needed {
            let _ = std::fs::write(&target, embedded_bytes);
        }
        if let Some(path_str) = target.to_str() {
            let wide: Vec<u16> = path_str.encode_utf16().chain(core::iter::once(0)).collect();
            unsafe {
                if let Ok(h) = LoadLibraryW(windows::core::PCWSTR(wide.as_ptr())) {
                    if !h.is_invalid() {
                        return h;
                    }
                }
            }
        }
    }

    HMODULE(core::ptr::null_mut())
}

pub fn get_proc(name: &[u8]) -> FARPROC {
    let hmod = get_module();
    if hmod.is_invalid() || hmod.0.is_null() {
        return None;
    }
    unsafe {
        GetProcAddress(hmod, PCSTR(name.as_ptr()))
    }
}
