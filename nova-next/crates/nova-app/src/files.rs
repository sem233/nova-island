// Files from the shelf: open with their app (shell.openPath), show in Explorer
// (shell.showItemInFolder), and drag out to Explorer or another app (webContents.startDrag).
use windows::{
    core::HSTRING,
    Win32::{
        Foundation::HWND,
        System::{Com::IDataObject, Ole::DROPEFFECT_COPY},
        UI::{
            Shell::*,
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
};

pub fn open(p: &str) {
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(p), None, None, SW_SHOWNORMAL);
    }
}

pub fn reveal(p: &str) {
    unsafe {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            let _ = SHOpenFolderAndSelectItems(pidl, None, 0);
            windows::Win32::System::Com::CoTaskMemFree(Some(pidl as _));
        }
    }
}

/// A shell drag of the file (copy), from the window thread; returns when it's dropped or cancelled.
pub fn drag_out(hwnd: HWND, p: &str) {
    unsafe {
        let data = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(p), None).and_then(|item| item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject));
        if let Ok(data) = data {
            let _ = SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY);
        }
    }
}

/// The Open dialog: a folder (`folder`) or a file of the given (name, pattern) kind.
pub fn pick(owner: Option<HWND>, title: &str, folder: bool, filter: Option<(&str, &str)>) -> Option<String> {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        UI::Shell::{Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH},
    };
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = d.SetTitle(&HSTRING::from(title));
        let mut opts = d.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        if folder {
            opts |= FOS_PICKFOLDERS;
        } else {
            opts |= FOS_FILEMUSTEXIST;
        }
        let _ = d.SetOptions(opts);
        let (name, spec) = filter.map(|(n, s)| (HSTRING::from(n), HSTRING::from(s))).unzip();
        if let (Some(n), Some(s)) = (&name, &spec) {
            let f = [COMDLG_FILTERSPEC { pszName: windows::core::PCWSTR(n.as_ptr()), pszSpec: windows::core::PCWSTR(s.as_ptr()) }];
            let _ = d.SetFileTypes(&f);
        }
        d.Show(owner).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// The Save dialog for a single file (e.g. PNG screenshot).
pub fn save_file(owner: Option<HWND>, title: &str, default_name: &str, filter: (&str, &str), default_ext: &str) -> Option<String> {
    use windows::Win32::{
        System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER},
        UI::Shell::{Common::COMDLG_FILTERSPEC, FileSaveDialog, IFileSaveDialog, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, SIGDN_FILESYSPATH},
    };
    unsafe {
        let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = d.SetTitle(&HSTRING::from(title));
        let _ = d.SetFileName(&HSTRING::from(default_name));
        let _ = d.SetDefaultExtension(&HSTRING::from(default_ext));
        let opts = d.GetOptions().ok()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT;
        let _ = d.SetOptions(opts);
        let name = HSTRING::from(filter.0);
        let spec = HSTRING::from(filter.1);
        let f = [COMDLG_FILTERSPEC { pszName: windows::core::PCWSTR(name.as_ptr()), pszSpec: windows::core::PCWSTR(spec.as_ptr()) }];
        let _ = d.SetFileTypes(&f);
        d.Show(owner).ok()?;
        let item = d.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        s
    }
}
