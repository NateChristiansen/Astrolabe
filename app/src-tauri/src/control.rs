//! The test control, compiled only with the `test-control` feature. A private local channel into
//! this window: read the page, click, type, take a picture. A control channel could click Allow
//! on an approval, so no normal build has any of this, and a test build refuses to run on the
//! default data folder. CI looks for `MARK` in the packaged app and fails if it is there.
use serde_json::{Value, json};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;
use tauri::{Manager, WebviewWindow, Wry};

#[path = "../../../crates/astrolabe/src/private_file.rs"]
mod private_file;

const MARK: &str = "astrolabe-test-control";
const PAGE: &str = include_str!("control.js");
const DATA_VAR: &str = "ASTROLABE_DATA_DIR";
type IoResult = std::io::Result<()>;

/// Where the core's data goes with no folder set, by the core's rule (`store::default_db_path`).
/// The core can't simply be asked: `astrolabe where` opens the database in the folder it names.
fn default_folder() -> Option<PathBuf> {
    let (var, under) = match cfg!(windows) {
        true => ("LOCALAPPDATA", "Astrolabe"),
        false => ("HOME", "Library/Application Support/Astrolabe"),
    };
    Some(PathBuf::from(std::env::var_os(var)?).join(under))
}

/// The folder this test build may use: set by name, a plain full path, apart from the default.
fn apart_folder() -> Result<PathBuf, String> {
    let set = std::env::var_os(DATA_VAR).filter(|set| !set.is_empty());
    let set = set.ok_or(format!("{DATA_VAR} is not set to a folder of its own."))?;
    let set = PathBuf::from(set);
    let shown = set.display();
    if !set.is_absolute() || set.components().any(|part| part == Component::ParentDir) {
        return Err(format!("{DATA_VAR} must be a plain full path: {shown}."));
    }
    let default = default_folder().ok_or("it can't tell where the default data folder is.")?;
    let lower = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    let overlap = |one: &Path, two: &Path| {
        let (one, two) = (lower(one), lower(two));
        one.starts_with(&two) || two.starts_with(&one)
    };
    let usual = default.display();
    let refuse = format!("{shown} is or overlaps the default data folder {usual}.");
    if overlap(&set, &default) {
        return Err(refuse);
    }
    fs::create_dir_all(&set).map_err(|why| why.to_string())?;
    let real = set.canonicalize().map_err(|why| why.to_string())?;
    // Links are gone by now. The default may not exist, but its parent may be a link.
    let parent = default.parent().and_then(|up| up.canonicalize().ok());
    let real_default = parent.map_or(default.clone(), |up| up.join("Astrolabe"));
    if overlap(&real, &real_default) {
        return Err(refuse);
    }
    Ok(set)
}

/// Adds the control to the app, or ends the program, saying why, before any window or core.
pub fn wire(app: tauri::Builder<Wry>) -> tauri::Builder<Wry> {
    let data = apart_folder().unwrap_or_else(|why| {
        eprintln!("This test build will not start: {why}");
        std::process::exit(3)
    });
    if std::env::args().any(|word| word == "--check-folder") {
        std::process::exit(0);
    }
    app.setup(move |app| {
        let window = app.get_webview_window("main").ok_or("no window")?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|why| why.to_string())?;
        let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let file = data.join(format!("{MARK}.txt"));
        let _ = fs::remove_file(&file);
        let port = listener.local_addr()?.port();
        private_file::write(&file, &format!("{port}\n{secret}\n"))?;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (window, secret, data) = (window.clone(), secret.clone(), data.clone());
                std::thread::spawn(move || one_ask(stream, &secret, &window, &data));
            }
        });
        Ok(())
    })
}

/// The secret on the first line, one ask on the second, one answer back. Without the secret, the
/// window hears nothing and the caller gets only the refusal.
fn one_ask(stream: TcpStream, secret: &str, window: &WebviewWindow, data: &Path) -> IoResult {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut lines = BufReader::new((&stream).take(1 << 20)).lines();
    let mut out = &stream;
    if lines.next().transpose()?.as_deref() != Some(secret) {
        return writeln!(out, "{}", json!({"ok": false, "why": "refused"}));
    }
    let ask = lines.next().transpose()?.unwrap_or_default();
    let said = answer(&ask, window, data);
    let failed = |why: String| json!({"ok": false, "why": why}).to_string();
    writeln!(out, "{}", said.unwrap_or_else(failed))?;
    if ask == "exit" {
        window.app_handle().exit(0);
    }
    Ok(())
}

fn answer(ask: &str, window: &WebviewWindow, data: &Path) -> Result<String, String> {
    let text = |why: tauri::Error| why.to_string();
    let (tell, said) = channel();
    let (verb, rest) = ask.split_once(' ').unwrap_or((ask, ""));
    match verb {
        "page" => {
            // Parsed and written out again: what reaches the page is only ever data.
            let ask: Value = serde_json::from_str(rest).map_err(|why| why.to_string())?;
            let script = format!("({PAGE})({ask})");
            let told = move |answer| drop(tell.send(answer));
            window.eval_with_callback(script, told).map_err(text)?;
        }
        "screenshot" => window
            .with_webview(move |view| snapshot(view, tell))
            .map_err(text)?,
        "core" => return core_state(data),
        "exit" => return Ok(json!({"ok": true}).to_string()),
        _ => return Err(format!("no such ask: {verb}")),
    }
    let answer = said.recv_timeout(Duration::from_secs(20));
    // A script that threw comes back empty on one system and `null` on the other.
    answer
        .ok()
        .filter(|answer| answer.starts_with('{'))
        .ok_or("no answer from the page".to_owned())
}

/// The lock is what says a core is serving, as it is for the app.
fn core_state(data: &Path) -> Result<String, String> {
    let mut lock = OpenOptions::new();
    let lock = lock.create(true).truncate(false).write(true);
    let lock = lock
        .open(data.join("astrolabe.lock"))
        .map_err(|why| why.to_string())?;
    let serving = matches!(lock.try_lock(), Err(TryLockError::WouldBlock));
    Ok(json!({"ok": true, "core": if serving { "serving" } else { "stopped" }}).to_string())
}

/// WebKit's own picture of the web view. It needs no system permission.
#[cfg(target_os = "macos")]
fn snapshot(view: tauri::webview::PlatformWebview, tell: Sender<String>) {
    use objc2::{class, msg_send, runtime::AnyObject};
    let none: *mut AnyObject = std::ptr::null_mut();
    let done = block2::RcBlock::new(move |image: *mut AnyObject, _why: *mut AnyObject| unsafe {
        if image.is_null() {
            return; // The caller hears that no answer came.
        }
        let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
        let bitmap: *mut AnyObject = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        let options: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        // 4 is NSBitmapImageFileTypePNG.
        let png: *mut AnyObject =
            msg_send![bitmap, representationUsingType: 4usize, properties: options];
        let text: *mut AnyObject = msg_send![png, base64EncodedStringWithOptions: 0usize];
        let utf8: *const std::ffi::c_char = msg_send![text, UTF8String];
        let png = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
        let _ = tell.send(json!({"ok": true, "png": png}).to_string());
    });
    let web_view = view.inner().cast::<AnyObject>();
    unsafe {
        let _: () =
            msg_send![web_view, takeSnapshotWithConfiguration: none, completionHandler: &*done];
    }
}

/// WebView2's own picture of the page, by its DevTools call. It needs no system permission.
#[cfg(windows)]
fn snapshot(view: tauri::webview::PlatformWebview, tell: Sender<String>) {
    use webview2_com::{
        CallDevToolsProtocolMethodCompletedHandler as Done, CoTaskMemPWSTR as Wide,
    };
    let done = Done::create(Box::new(move |_, said: String| {
        let got = serde_json::from_str::<Value>(&said).unwrap_or_default();
        let no = json!({"ok": false, "why": said});
        let answer = got["data"]
            .as_str()
            .map_or(no, |png| json!({"ok": true, "png": png}));
        let _ = tell.send(answer.to_string());
        Ok(())
    }));
    let (method, nothing) = (Wide::from("Page.captureScreenshot"), Wide::from("{}"));
    let (method, nothing) = (*method.as_ref().as_pcwstr(), *nothing.as_ref().as_pcwstr());
    let asked = unsafe {
        let web_view = view.controller().CoreWebView2();
        web_view.and_then(|web_view| web_view.CallDevToolsProtocolMethod(method, nothing, &done))
    };
    // Dropping `done` tells the caller no answer is coming. This goes in the run's stderr.
    let _ = asked.inspect_err(|why| eprintln!("no picture: {why}"));
}
