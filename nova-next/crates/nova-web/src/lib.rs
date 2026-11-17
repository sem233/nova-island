//! WebView2 hosting for Nova's pages.
//!
//! One environment for the whole app; each page (island, Settings, mini player) is a transparent
//! windowed controller inside its own window. Page files come from `nova-assets` at
//! https://nova.island/, and each page gets its original Electron preload file running on the
//! `require('electron')` stand-in in `bridge/electron-shim.js`.
use std::{cell::RefCell, rc::Rc, sync::mpsc};

pub mod sniff;
pub use sniff::{sniff_stream, SniffBlock, SniffResult};

use serde_json::{json, Value};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    core::{Interface, HSTRING, PCWSTR, PWSTR},
    Win32::{Foundation::*, UI::Shell::SHCreateMemStream},
};

pub type Result<T> = std::result::Result<T, String>;

fn err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

pub const ORIGIN: &str = "https://nova.island/";

/// (1.8.7) The adhan the pages play at https://nova.island/adhan-audio (and the Fajr one at
/// /adhan-fajr-audio): a file the user chose (nova-app adhan.rs), or Nova's own
/// (sounds/adhan.mp3, sounds/adhan-fajr.mp3) when None. Fajr without either: the other adhan.
static ADHAN_FILE: std::sync::Mutex<[Option<std::path::PathBuf>; 2]> = std::sync::Mutex::new([None, None]);

pub fn set_adhan_file(fajr: bool, path: Option<std::path::PathBuf>) {
    ADHAN_FILE.lock().unwrap()[fajr as usize] = path;
}

fn adhan_audio(fajr: bool) -> Option<(std::borrow::Cow<'static, [u8]>, &'static str)> {
    if fajr {
        let own = ADHAN_FILE.lock().unwrap()[1].clone();
        if let Some(r) = own.and_then(|p| read_audio(&p)) {
            return Some(r);
        }
        if let Some((b, _)) = nova_assets::get("sounds/adhan-fajr.mp3") {
            return Some((std::borrow::Cow::Borrowed(b), "audio/mpeg"));
        }
    }
    let own = ADHAN_FILE.lock().unwrap()[0].clone();
    if let Some(r) = own.and_then(|p| read_audio(&p)) {
        return Some(r);
    }
    nova_assets::get("sounds/adhan.mp3").map(|(b, _)| (std::borrow::Cow::Borrowed(b), "audio/mpeg"))
}

fn read_audio(p: &std::path::Path) -> Option<(std::borrow::Cow<'static, [u8]>, &'static str)> {
    {
        let mime = match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str() {
            "mp3" => "audio/mpeg",
            "m4a" => "audio/mp4",
            "ogg" | "opus" => "audio/ogg",
            "wav" => "audio/wav",
            "aac" => "audio/aac",
            _ => "application/octet-stream",
        };
        std::fs::read(p).ok().map(|b| (std::borrow::Cow::Owned(b), mime))
    }
}
const SHIM: &str = include_str!("../bridge/electron-shim.js");

/// Which preload file a page runs.
#[derive(Clone, Copy)]
pub enum Bridge {
    Nova,
    Prefs,
    Mini,
    Radio,
    Tasks,
}

impl Bridge {
    fn preload(self) -> &'static str {
        match self {
            Bridge::Nova => nova_assets::PRELOAD_NOVA,
            Bridge::Prefs => nova_assets::PRELOAD_PREFS,
            Bridge::Mini => nova_assets::PRELOAD_MINI,
            Bridge::Radio => nova_assets::PRELOAD_RADIO,
            Bridge::Tasks => nova_assets::PRELOAD_TASKS,
        }
    }

    /// The script injected before any page script runs: the shim, then the preload in its own scope.
    /// Child frames (the YouTube embed in the mini player) are left alone.
    pub fn script(self) -> String {
        format!(
            "(() => {{\nif (window.top !== window || !window.chrome || !window.chrome.webview) return;\n{SHIM}\n(function (require) {{\n{}\n}})(require);\n}})();\n",
            self.preload()
        )
    }
}

/// A message from a page.
#[derive(Debug)]
pub enum Msg {
    Send { channel: String, args: Vec<Value> },
    Invoke { id: u64, channel: String, args: Vec<Value> },
}

/// One shared environment. `data_dir` holds WebView2's own profile (cache, localStorage).
pub fn environment(data_dir: &str, args: &str) -> Result<ICoreWebView2Environment> {
    let opts = CoreWebView2EnvironmentOptions::default();
    unsafe { opts.set_additional_browser_arguments(args.to_string()) };
    let opts: ICoreWebView2EnvironmentOptions = opts.into();
    let data = CoTaskMemPWSTR::from(data_dir);
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), *data.as_ref().as_pcwstr(), &opts, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |code, env| {
            code?;
            tx.send(env.ok_or_else(|| windows::core::Error::from(E_POINTER))).ok();
            Ok(())
        }),
    )
    .map_err(err)?;
    rx.recv().map_err(err)?.map_err(err)
}

pub struct Page {
    pub controller: ICoreWebView2Controller,
    pub webview: ICoreWebView2,
    /// (1.8.7) the origins `navigate` was sent to: the only ones the page may go to and hear from
    homes: Rc<RefCell<Vec<String>>>,
}

/// "https://nova.island/" for "https://Nova.Island/index.html?x" (scheme://host[:port]/, lower
/// case); None for an address with no host (about:blank, data:, file:///...).
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    // user@host is never one of ours
    if scheme.is_empty() || host.is_empty() || host.contains('@') || host.contains('\\') {
        return None;
    }
    Some(format!("{}://{}/", scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

fn is_home(homes: &RefCell<Vec<String>>, url: &str) -> bool {
    origin_of(url).is_some_and(|o| homes.borrow().iter().any(|h| *h == o))
}

impl Page {
    /// The page drawn at exactly `scale` (the screen's: 1.25 at 125%), whatever else Windows
    /// asks. WebView2 also scales by Windows' Text size (Accessibility): at 125% text the
    /// island's 640-wide window held a 512-wide page, narrower than the open island (548), which
    /// then slid out of the window's left side (1.8.6). The island's window has a fixed size in
    /// screen units, so its page must too.
    pub fn pin_scale(&self, scale: f64) {
        unsafe {
            if let Ok(c3) = self.controller.cast::<ICoreWebView2Controller3>() {
                let _ = c3.SetShouldDetectMonitorScaleChanges(false);
                let mut now = 0.0;
                if c3.RasterizationScale(&mut now).is_err() || (now - scale).abs() > 0.001 {
                    let _ = c3.SetRasterizationScale(scale);
                }
            }
        }
    }

    /// Tests: draws the page at `scale` as WebView2 does with a larger Windows Text size.
    pub fn force_scale(&self, scale: f64) {
        unsafe {
            if let Ok(c3) = self.controller.cast::<ICoreWebView2Controller3>() {
                let _ = c3.SetShouldDetectMonitorScaleChanges(false);
                let _ = c3.SetRasterizationScale(scale);
            }
        }
    }

    /// A transparent page filling `hwnd` (w x h physical pixels). `on_msg` gets every message the
    /// page sends; answer invokes with `resolve`.
    pub fn create(
        env: &ICoreWebView2Environment,
        hwnd: HWND,
        w: i32,
        h: i32,
        bridge: Option<Bridge>,
        on_msg: impl FnMut(Msg) + 'static,
    ) -> Result<Page> {
        let (tx, rx) = mpsc::channel();
        let env2 = env.clone();
        CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe { env2.CreateCoreWebView2Controller(hwnd, &handler).map_err(webview2_com::Error::WindowsError) }),
            Box::new(move |code, c| {
                code?;
                tx.send(c.ok_or_else(|| windows::core::Error::from(E_POINTER))).ok();
                Ok(())
            }),
        )
        .map_err(err)?;
        let controller = rx.recv().map_err(err)?.map_err(err)?;
        let webview;
        unsafe {
            controller.SetBounds(RECT { left: 0, top: 0, right: w, bottom: h }).map_err(err)?;
            controller.SetIsVisible(true).map_err(err)?;
            let c2: ICoreWebView2Controller2 = controller.cast().map_err(err)?;
            c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 0, R: 0, G: 0, B: 0 }).map_err(err)?;
            webview = controller.CoreWebView2().map_err(err)?;
            let s = webview.Settings().map_err(err)?;
            s.SetAreDefaultContextMenusEnabled(false).map_err(err)?;
            s.SetIsStatusBarEnabled(false).map_err(err)?;
            s.SetIsZoomControlEnabled(false).map_err(err)?;
            s.SetAreDevToolsEnabled(cfg!(debug_assertions)).map_err(err)?;
            if let Ok(s3) = s.cast::<ICoreWebView2Settings3>() {
                s3.SetAreBrowserAcceleratorKeysEnabled(false).map_err(err)?;
            }
        }
        let page = Page { controller, webview, homes: Rc::new(RefCell::new(Vec::new())) };
        page.guard_navigation()?;
        page.serve_assets(env)?;
        if let Some(b) = bridge {
            page.add_script(&b.script())?;
        }
        page.on_messages(on_msg)?;
        Ok(page)
    }

    /// (1.8.7) The page stays where Nova sent it: a link, a dropped file or a script can't take the
    /// window (and the bridge the preload gives every top document) to another address, and no
    /// pop-up window opens. Pages that open links do it through the bridge (shell.openExternal).
    fn guard_navigation(&self) -> Result<()> {
        let mut token = 0;
        let homes = self.homes.clone();
        unsafe {
            self.webview
                .add_NavigationStarting(
                    &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else { return Ok(()) };
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        if !is_home(&homes, &CoTaskMemPWSTR::from(uri).to_string()) {
                            args.SetCancel(true)?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(err)?;
            self.webview
                .add_NewWindowRequested(
                    &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            args.SetHandled(true)?;
                        }
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Answers requests for https://nova.island/* from the embedded page files.
    fn serve_assets(&self, env: &ICoreWebView2Environment) -> Result<()> {
        let env = env.clone();
        let mut token = 0;
        unsafe {
            self.webview
                .AddWebResourceRequestedFilter(&HSTRING::from(format!("{ORIGIN}*")), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)
                .map_err(err)?;
            self.webview
                .add_WebResourceRequested(
                    &WebResourceRequestedEventHandler::create(Box::new(move |_wv, args| {
                        let Some(args) = args else { return Ok(()) };
                        let request = args.Request()?;
                        let mut uri = PWSTR::null();
                        request.Uri(&mut uri)?;
                        let uri = CoTaskMemPWSTR::from(uri).to_string();
                        let path = uri.strip_prefix(ORIGIN).unwrap_or("").split(['?', '#']).next().unwrap_or("");
                        let path = if path.is_empty() { "index.html" } else { path };
                        let found = if path == "adhan-audio" || path == "adhan-fajr-audio" { adhan_audio(path == "adhan-fajr-audio") } else { nova_assets::get(path).map(|(b, m)| (std::borrow::Cow::Borrowed(b), m)) };
                        let response = match found {
                            Some((bytes, mime)) => {
                                let stream = SHCreateMemStream(Some(&bytes));
                                env.CreateWebResourceResponse(
                                    stream.as_ref(),
                                    200,
                                    &HSTRING::from("OK"),
                                    &HSTRING::from(format!("Content-Type: {mime}\r\nCache-Control: no-store")),
                                )?
                            }
                            None => env.CreateWebResourceResponse(None, 404, &HSTRING::from("Not Found"), &HSTRING::new())?,
                        };
                        args.SetResponse(&response)?;
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Runs `script` before every page script, on every navigation.
    fn add_script(&self, script: &str) -> Result<()> {
        let wv = self.webview.clone();
        let script = CoTaskMemPWSTR::from(script);
        AddScriptToExecuteOnDocumentCreatedCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                wv.AddScriptToExecuteOnDocumentCreated(*script.as_ref().as_pcwstr(), &handler).map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(|code, _id| code),
        )
        .map_err(err)
    }

    fn on_messages(&self, on_msg: impl FnMut(Msg) + 'static) -> Result<()> {
        let on_msg = Rc::new(RefCell::new(on_msg));
        let homes = self.homes.clone();
        let mut token = 0;
        unsafe {
            self.webview
                .add_WebMessageReceived(
                    &WebMessageReceivedEventHandler::create(Box::new(move |_wv, args| {
                        let Some(args) = args else { return Ok(()) };
                        // (1.8.7) only Nova's own page is heard, whatever reached the window
                        let mut src = PWSTR::null();
                        args.Source(&mut src)?;
                        if !is_home(&homes, &CoTaskMemPWSTR::from(src).to_string()) {
                            return Ok(());
                        }
                        let mut p = PWSTR::null();
                        args.WebMessageAsJson(&mut p)?;
                        let Ok(m) = serde_json::from_str::<Value>(&CoTaskMemPWSTR::from(p).to_string()) else { return Ok(()) };
                        let paths = file_paths(&args);
                        let channel = m["c"].as_str().unwrap_or("").to_string();
                        let args_v = match m["a"].clone() {
                            Value::Array(a) => a.into_iter().map(|v| with_paths(v, &paths)).collect(),
                            _ => Vec::new(),
                        };
                        let msg = match m["t"].as_str() {
                            Some("snd") => Msg::Send { channel, args: args_v },
                            Some("inv") => Msg::Invoke { id: m["i"].as_u64().unwrap_or(0), channel, args: args_v },
                            _ => return Ok(()),
                        };
                        (on_msg.borrow_mut())(msg);
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// (1.8.7) WebView2's processes failing: `on_fail(kind, gone)` with the kind's name, and
    /// whether the whole browser process is gone (then the page can't come back on its own: the
    /// app has to make a new one). Before this, a crashed page left the island an empty,
    /// invisible window, and opening Nova again did nothing.
    pub fn on_process_failed(&self, on_fail: impl FnMut(&str, bool) + 'static) -> Result<()> {
        let on_fail = RefCell::new(on_fail);
        let mut token = 0;
        unsafe {
            self.webview
                .add_ProcessFailed(
                    &ProcessFailedEventHandler::create(Box::new(move |_wv, args| {
                        let Some(args) = args else { return Ok(()) };
                        let mut k = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                        args.ProcessFailedKind(&mut k)?;
                        let name = match k {
                            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser process exited",
                            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "page process exited",
                            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => "page not responding",
                            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => "frame process exited",
                            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => "GPU process exited",
                            _ => "a helper process exited",
                        };
                        (on_fail.borrow_mut())(name, k == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED);
                        Ok(())
                    })),
                    &mut token,
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Loads the page again (after its process failed).
    pub fn reload(&self) {
        unsafe {
            let _ = self.webview.Reload();
        }
    }

    /// Page errors and console.error/warn, through the DevTools protocol (what 1.7 forwards to the
    /// log as "[renderer] ..."). `on_line` gets one readable line per message.
    pub fn on_console(&self, on_line: impl FnMut(String) + 'static) -> Result<()> {
        let on_line = Rc::new(RefCell::new(on_line));
        unsafe {
            for (event, kind) in [("Runtime.exceptionThrown", "exception"), ("Runtime.consoleAPICalled", "console")] {
                let sink = on_line.clone();
                let receiver = self.webview.GetDevToolsProtocolEventReceiver(&HSTRING::from(event)).map_err(err)?;
                let mut token = 0;
                receiver
                    .add_DevToolsProtocolEventReceived(
                        &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_wv, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut p = PWSTR::null();
                            args.ParameterObjectAsJson(&mut p)?;
                            let v: Value = serde_json::from_str(&CoTaskMemPWSTR::from(p).to_string()).unwrap_or_default();
                            let line = if kind == "exception" {
                                let d = &v["exceptionDetails"];
                                let text = d["exception"]["description"].as_str().or(d["text"].as_str()).unwrap_or("").lines().next().unwrap_or("");
                                Some(format!("{text} ({}:{})", d["url"].as_str().unwrap_or("").rsplit('/').next().unwrap_or(""), d["lineNumber"].as_i64().unwrap_or(0) + 1))
                            } else if matches!(v["type"].as_str(), Some("error" | "warning")) {
                                let parts: Vec<String> = v["args"].as_array().map(|a| a.iter().map(|x| x["value"].as_str().map(str::to_string).or(x["description"].as_str().map(str::to_string)).unwrap_or_default()).collect()).unwrap_or_default();
                                Some(parts.join(" "))
                            } else {
                                None
                            };
                            if let Some(line) = line {
                                (sink.borrow_mut())(line);
                            }
                            Ok(())
                        })),
                        &mut token,
                    )
                    .map_err(err)?;
            }
            self.webview
                .CallDevToolsProtocolMethod(
                    &HSTRING::from("Runtime.enable"),
                    &HSTRING::from("{}"),
                    &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(()))),
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Background colour behind the page (transparent by default).
    pub fn set_background(&self, r: u8, g: u8, b: u8, a: u8) -> Result<()> {
        unsafe {
            let c2: ICoreWebView2Controller2 = self.controller.cast().map_err(err)?;
            c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: a, R: r, G: g, B: b }).map_err(err)
        }
    }

    /// Saves what the page shows as a PNG (WebView2's own rendering, without the desktop behind).
    pub fn capture_png(&self, path: &str) -> Result<()> {
        use windows::Win32::{Storage::FileSystem::FILE_ATTRIBUTE_NORMAL, System::Com::{STGM_CREATE, STGM_WRITE}, UI::Shell::SHCreateStreamOnFileEx};
        let stream = unsafe { SHCreateStreamOnFileEx(&HSTRING::from(path), (STGM_CREATE | STGM_WRITE).0, FILE_ATTRIBUTE_NORMAL.0, true, None).map_err(err)? };
        let wv = self.webview.clone();
        let s2 = stream.clone();
        CapturePreviewCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                wv.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, &s2, &handler).map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(|code| code),
        )
        .map_err(err)?;
        unsafe { stream.Commit(windows::Win32::System::Com::STGC_DEFAULT).map_err(err) }
    }

    /// Goes to `url`, which becomes one of the page's own addresses (see `guard_navigation`).
    pub fn navigate(&self, url: &str) -> Result<()> {
        if let Some(o) = origin_of(url) {
            let mut homes = self.homes.borrow_mut();
            if !homes.contains(&o) {
                homes.push(o);
            }
        }
        let url = CoTaskMemPWSTR::from(url);
        unsafe { self.webview.Navigate(*url.as_ref().as_pcwstr()).map_err(err) }
    }

    fn post(&self, v: &Value) {
        let s = CoTaskMemPWSTR::from(v.to_string().as_str());
        unsafe {
            let _ = self.webview.PostWebMessageAsJson(*s.as_ref().as_pcwstr());
        }
    }

    /// An event for the page's ipcRenderer.on(channel) listeners.
    pub fn event(&self, channel: &str, data: Value) {
        self.post(&json!({ "t": "ev", "c": channel, "d": data }));
    }

    /// The answer to an invoke.
    pub fn resolve(&self, id: u64, value: Value) {
        self.post(&json!({ "t": "res", "i": id, "ok": true, "v": value }));
    }

    pub fn reject(&self, id: u64, message: &str) {
        self.post(&json!({ "t": "res", "i": id, "ok": false, "v": message }));
    }
}

/// Real paths of the File objects sent along with a message ("" when a file has no path).
fn file_paths(args: &ICoreWebView2WebMessageReceivedEventArgs) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(a2) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>() else { return out };
        let Ok(list) = a2.AdditionalObjects() else { return out };
        let mut n = 0u32;
        let _ = list.Count(&mut n);
        for i in 0..n {
            let path = list
                .GetValueAtIndex(i)
                .ok()
                .and_then(|o| o.cast::<ICoreWebView2File>().ok())
                .and_then(|f| {
                    let mut p = PWSTR::null();
                    f.Path(&mut p).ok().map(|_| CoTaskMemPWSTR::from(p).to_string())
                })
                .unwrap_or_default();
            out.push(path);
        }
    }
    out
}

/// Puts the real paths back where the page had {$file: n} placeholders.
fn with_paths(v: Value, paths: &[String]) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.into_iter().map(|x| with_paths(x, paths)).filter(|x| !x.is_null()).collect()),
        Value::Object(o) if o.len() == 1 && o.contains_key("$file") => {
            let i = o["$file"].as_u64().unwrap_or(u64::MAX) as usize;
            match paths.get(i) {
                Some(p) if !p.is_empty() => Value::String(p.clone()),
                _ => Value::Null,
            }
        }
        other => other,
    }
}

/// One of the embedded page files (the app icon for the tray, for example).
pub fn asset(path: &str) -> Option<(&'static [u8], &'static str)> {
    nova_assets::get(path)
}

impl Page {
    /// Runs `js` in the page without waiting for it.
    pub fn run(&self, js: &str) {
        unsafe {
            let _ = self.webview.ExecuteScript(&windows::core::HSTRING::from(js), None);
        }
    }

    /// Runs `js` in the page and returns its result as JSON text (tests and checks only).
    pub fn eval(&self, js: &str) -> Result<String> {
        let wv = self.webview.clone();
        let js = CoTaskMemPWSTR::from(js);
        let (tx, rx) = mpsc::channel();
        ExecuteScriptCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe { wv.ExecuteScript(*js.as_ref().as_pcwstr(), &handler).map_err(webview2_com::Error::WindowsError) }),
            Box::new(move |code, result| {
                code?;
                tx.send(result).ok();
                Ok(())
            }),
        )
        .map_err(err)?;
        rx.recv().map_err(err)
    }
}
