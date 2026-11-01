//! `astrolabe hook <event>`, and the report files it leaves. A file because a hook must
//! not wait on anything: the core reads them when it runs, a launch can read them with
//! no core at all, and each fire is its own file, so they can be counted. Files stay
//! until step 4 settles retention.
use crate::host::{Clock, SystemClock};
use crate::layer::DATA_DIR_VAR;
use crate::store::default_db_path;
use serde_json::{Value, json};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A client writes the hook's input at once. One that doesn't still gets its receipt.
const STDIN_WAIT: Duration = Duration::from_secs(1);
const STDIN_LIMIT: u64 = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Unique per fire.
    pub file: String,
    pub event: String,
    /// The client's id for the session. None from a tool server, which isn't told it.
    pub session_id: Option<String>,
    pub cwd: String,
    pub at_ms: i64,
}

pub fn reports_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join("reports")
}

/// None only if no data folder can be named. A hook then has nowhere to report.
pub fn reports_dir() -> Option<PathBuf> {
    let data = std::env::var_os(DATA_DIR_VAR)
        .map(PathBuf::from)
        .or_else(|| default_db_path()?.parent().map(Path::to_path_buf))?;
    Some(reports_dir_in(&data))
}

/// Always returns: whatever goes wrong here must not reach the agent. A missing receipt
/// is for the launch to judge (step 4). A pre-command guard goes here as its own event
/// (step 4), and is the one event that may answer the client.
pub fn run(event: &str) {
    let input = read_input();
    let session_id = input["session_id"].as_str();
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let cwd = cwd.or_else(|| std::env::current_dir().ok());
    let (Some(dir), Some(cwd)) = (reports_dir(), cwd) else {
        return;
    };
    if let Err(error) = write_report(&dir, event, session_id, &cwd) {
        eprintln!("astrolabe hook: {error}");
    }
}

/// On a thread, because a client that holds stdin open must not hold the hook.
fn read_input() -> Value {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut input = Vec::new();
        let _ = io::stdin().take(STDIN_LIMIT).read_to_end(&mut input);
        let _ = sender.send(input);
    });
    let input = receiver.recv_timeout(STDIN_WAIT).unwrap_or_default();
    serde_json::from_slice(&input).unwrap_or(Value::Null)
}

/// Appears whole or not at all: written aside, then renamed.
pub fn write_report(
    dir: &Path,
    event: &str,
    session_id: Option<&str>,
    cwd: &Path,
) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let at_ms = SystemClock.now_ms();
    let pid = std::process::id();
    let slug: String = event
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let body = json!({
        "event": event,
        "session_id": session_id,
        "cwd": comparable(cwd),
        "at_ms": at_ms,
    });
    let aside = dir.join(format!(".{at_ms}-{pid}-{slug}.tmp"));
    std::fs::write(&aside, body.to_string())?;
    std::fs::rename(&aside, dir.join(format!("{at_ms}-{pid}-{slug}.json")))
}

/// The same folder reads the same from a client and from here (`/tmp` and `/private/tmp`).
pub fn comparable(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.to_string_lossy().into_owned()
}

/// Oldest first. A missing folder or an unreadable file is no report.
pub fn read_reports(dir: &Path) -> Vec<Report> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut reports: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().into_string().ok()?;
            if !file.ends_with(".json") {
                return None;
            }
            let body: Value = serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()?;
            Some(Report {
                file,
                event: body["event"].as_str()?.to_owned(),
                session_id: body["session_id"].as_str().map(str::to_owned),
                cwd: body["cwd"].as_str()?.to_owned(),
                at_ms: body["at_ms"].as_i64()?,
            })
        })
        .collect();
    reports.sort_by(|a, b| (a.at_ms, &a.file).cmp(&(b.at_ms, &b.file)));
    reports
}
