// The tray icon and the app menu (nova.js buildTray / appMenu): the same items in the same order,
// shown by a click on the tray icon and by a right-click on the island.
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
};

pub const WM_TRAY: u32 = WM_APP + 2;
pub const ID_SHOW: u32 = 1;
pub const ID_VERTICAL: u32 = 2;
pub const ID_RESET: u32 = 3;
pub const ID_STARTUP: u32 = 4;
pub const ID_TOGGLE: u32 = 5;
pub const ID_SETTINGS: u32 = 6;
pub const ID_QUIT: u32 = 7;
pub const ID_SNIP: u32 = 8;

/// Adds the tray icon (icon file from the embedded app icon), tooltip "Nova Island".
pub fn add(hwnd: HWND) {
    let path = std::env::temp_dir().join("nova-island-tray.ico");
    if let Some((bytes, _)) = nova_web_assets_icon() {
        let _ = std::fs::write(&path, bytes);
    }
    unsafe {
        let icon = LoadImageW(None, &HSTRING::from(path.to_string_lossy().as_ref()), IMAGE_ICON, 16, 16, LR_LOADFROMFILE).map(|h| HICON(h.0)).ok();
        let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP, uCallbackMessage: WM_TRAY, ..Default::default() };
        if let Some(i) = icon {
            nid.hIcon = i;
        }
        let tip: Vec<u16> = "Nova Island".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn nova_web_assets_icon() -> Option<(&'static [u8], &'static str)> {
    nova_web::asset("build/icon.ico")
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

pub struct MenuState {
    pub shown: bool,
    pub vertical: bool,
    pub startup: Option<bool>, // None: the Store version (Windows manages startup)
}

/// Shows the app menu at the cursor; returns the chosen item's id (0: none).
extern "system" fn owner_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// The menu's owner: a hidden window that may come to the front. The island never takes focus, and
/// a menu whose owner isn't in front doesn't close when you click elsewhere.
fn owner() -> HWND {
    thread_local! { static OWNER: std::cell::Cell<isize> = const { std::cell::Cell::new(0) }; }
    let h = OWNER.with(|o| o.get());
    if h != 0 {
        return HWND(h as *mut _);
    }
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cls = WNDCLASSW { lpfnWndProc: Some(owner_proc), hInstance: HINSTANCE(inst.0), lpszClassName: windows::core::w!("NovaIslandMenu"), ..Default::default() };
        RegisterClassW(&cls);
        let h = CreateWindowExW(WS_EX_TOOLWINDOW, windows::core::w!("NovaIslandMenu"), windows::core::w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(HINSTANCE(inst.0)), None).unwrap_or_default();
        OWNER.with(|o| o.set(h.0 as isize));
        h
    }
}

pub fn popup(_hwnd: HWND, s: &MenuState) -> u32 {
    let hwnd = owner();
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from("Nova Island"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING | check(s.shown), ID_SHOW as usize, &HSTRING::from("Show island"));
        let _ = AppendMenuW(menu, MF_STRING | check(s.vertical), ID_VERTICAL as usize, &HSTRING::from("Vertical (dock left / right)"));
        let _ = AppendMenuW(menu, MF_STRING, ID_RESET as usize, &HSTRING::from("Reset position"));
        if let Some(on) = s.startup {
            let _ = AppendMenuW(menu, MF_STRING | check(on), ID_STARTUP as usize, &HSTRING::from("Start with Windows"));
        }
        let _ = AppendMenuW(menu, MF_STRING, ID_SNIP as usize, &HSTRING::from("Snip & pin\tPrtScn"));
        let _ = AppendMenuW(menu, MF_STRING, ID_TOGGLE as usize, &HSTRING::from("Expand / collapse\tCtrl+Alt+I"));
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS as usize, &HSTRING::from("Settings…"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT as usize, &HSTRING::from("Quit"));
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        // the menu closes properly only if its owner is in front while it is open
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON).0, p.x, p.y, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        id
    }
}
