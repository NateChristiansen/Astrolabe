//! `astrolabe hook <event>`, and the report files it leaves. A file because a hook must
//! not wait on anything: the core reads them when it runs, a launch can read them with
//! no core at all, and each fire is its own file, so they can be counted. Files stay:
//! retention isn't settled, though a receipt's cost doesn't grow past a listing.
use crate::host::{Clock, SystemClock};
use crate::layer::{DATA_DIR_VAR, HookEvent, WORKTREE_VAR};
use crate::store::default_db_path;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Reports we write are far smaller. Anything bigger is not one of ours.
const REPORT_LIMIT: u64 = 1 << 16;
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
/// is for the launch to judge (step 4).
pub fn run(event: &str) {
    let input = read_input();
    let session_id = input["session_id"].as_str();
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let cwd = cwd.or_else(|| std::env::current_dir().ok());
    let (Some(dir), Some(cwd)) = (reports_dir(), cwd) else {
        return;
    };
    if let Err(error) = write_report(&dir, event, session_id, &cwd) {
        // Not `eprintln!`, which panics if stderr is closed.
        let _ = writeln!(io::stderr(), "astrolabe hook: {error}");
    }
    if event == HookEvent::SessionStart.slug()
        && let Some(data) = dir.parent()
    {
        let text = crate::voice::context(data);
        crate::voice::record(data, SystemClock.now_ms(), &cwd, &input, &text);
        let _ = writeln!(io::stdout(), "{}", crate::voice::hook_output(&text));
    }
}

/// On a thread, because a client that holds stdin open must not hold the hook. Ends
/// as soon as the input is whole, whether or not the client closes the pipe.
fn read_input() -> Value {
    serde_json::from_slice(&read_bytes()).unwrap_or(Value::Null)
}

/// What arrived, whole or not.
fn read_bytes() -> Vec<u8> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin().take(STDIN_LIMIT);
        let mut chunk = [0; 8192];
        while let Ok(read @ 1..) = stdin.read(&mut chunk) {
            if sender.send(chunk[..read].to_vec()).is_err() {
                return;
            }
        }
    });
    let deadline = Instant::now() + STDIN_WAIT;
    let mut input = Vec::new();
    while let Ok(chunk) = receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        input.extend(chunk);
        if serde_json::from_slice::<Value>(&input).is_ok() {
            break;
        }
    }
    input
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

/// Whether this client session reported its start at or after `since_ms`. Names first, so
/// a long-lived folder of old reports costs a directory listing and no reads.
pub fn session_start_since(dir: &Path, session_id: &str, since_ms: i64) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let suffix = format!("-{}.json", HookEvent::SessionStart.slug());
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str().and_then(|name| name.strip_suffix(&suffix)) else {
            return false;
        };
        let at_ms = name.split('-').next().and_then(|ms| ms.parse::<i64>().ok());
        at_ms.is_some_and(|at| at >= since_ms)
            && read_report(&entry)
                .is_some_and(|report| report.session_id.as_deref() == Some(session_id))
    })
}

/// Oldest first. A missing folder or an unreadable file is no report. Only regular
/// files, read to a limit: a pipe or a link to a device here must not hold up the core.
pub fn read_reports(dir: &Path) -> Vec<Report> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut reports: Vec<_> = entries
        .flatten()
        .filter_map(|entry| read_report(&entry))
        .collect();
    reports.sort_by(|a, b| (a.at_ms, &a.file).cmp(&(b.at_ms, &b.file)));
    reports
}

/// How a denial shows among the reports the core reads.
pub const DENIAL_EVENT: &str = "pre-command-denied";
/// One JSON line per denial, in the data folder.
pub const DENIAL_LOG: &str = "guard-denials.log";
/// A self-test's input: never reported, since nothing was attempted.
pub const SELF_TEST_SESSION: &str = "astrolabe-self-test";
const LOGGED_COMMAND: usize = 2000;

/// `astrolabe hook pre-command`. Opposite of the other hooks, it fails closed as far as
/// it can: a panic is caught, and input it can't read is denied if it names a denied
/// program. A hook that dies outright, hangs or is never run fails open on both clients
/// and they give no sign. Nothing here can change that; the plain rules and the receipt
/// are there for it. Always exit 0: exit 2 fails open on Codex on Windows, and shows the
/// hook's command line on Claude. No network, core or database.
pub fn pre_command() {
    std::panic::set_hook(Box::new(|_| {}));
    let bytes = read_bytes();
    let reason = std::panic::catch_unwind(|| judge(&bytes)).unwrap_or_else(|_| {
        crate::guard::names_denied_program(&String::from_utf8_lossy(&bytes))
            .then(|| "a command the guard could not read".to_owned())
    });
    let Some(reason) = reason else { return };
    let _ = std::panic::catch_unwind(|| record_denial(&bytes, &reason));
    let answer = json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason":
            format!("Astrolabe denied {reason}. Ask the owner if this is really needed."),
    }});
    let mut stdout = io::stdout();
    let _ = writeln!(stdout, "{answer}");
    let _ = stdout.flush();
}

fn judge(bytes: &[u8]) -> Option<String> {
    if std::env::var_os("ASTROLABE_GUARD_TEST_PANIC").is_some() {
        panic!("test");
    }
    let Ok(input) = serde_json::from_slice::<Value>(bytes) else {
        return crate::guard::names_denied_program(&String::from_utf8_lossy(bytes))
            .then(|| "a command the guard could not read".to_owned());
    };
    let tool = input["tool_name"].as_str().unwrap_or_default();
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let cwd = cwd.or_else(|| std::env::current_dir().ok());
    let scope = crate::guard::Scope {
        root: worktree_root(cwd.as_deref()),
        cwd: cwd.as_deref().map(comparable),
    };
    crate::guard::decide(tool, &input["tool_input"], &scope)
}

/// What the launch named, else the nearest folder holding a `.git`.
fn worktree_root(cwd: Option<&Path>) -> Option<String> {
    if let Some(root) = std::env::var_os(WORKTREE_VAR) {
        return Some(comparable(Path::new(&root)));
    }
    let found = cwd?.ancestors().find(|dir| dir.join(".git").exists())?;
    Some(comparable(found))
}

fn record_denial(bytes: &[u8], reason: &str) {
    let input: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let session_id = input["session_id"].as_str();
    if session_id == Some(SELF_TEST_SESSION) {
        return;
    }
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let Some(cwd) = cwd.or_else(|| std::env::current_dir().ok()) else {
        return;
    };
    let Some(dir) = reports_dir() else { return };
    let _ = write_report(&dir, DENIAL_EVENT, session_id, &cwd);
    let tool_input = &input["tool_input"];
    let command = tool_input["command"].as_str().unwrap_or_default();
    let line = json!({
        "at_ms": SystemClock.now_ms(),
        "session_id": session_id,
        "agent_id": input["agent_id"],
        "tool": input["tool_name"],
        "reason": reason,
        "command": command.chars().take(LOGGED_COMMAND).collect::<String>(),
    });
    let Some(data) = dir.parent() else { return };
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join(DENIAL_LOG));
    if let Ok(mut log) = log {
        let _ = log.write_all(format!("{line}\n").as_bytes());
    }
}

fn read_report(entry: &std::fs::DirEntry) -> Option<Report> {
    let file = entry.file_name().into_string().ok()?;
    if !file.ends_with(".json") {
        return None;
    }
    if !entry.file_type().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    let mut opened = std::fs::File::open(entry.path())
        .ok()?
        .take(REPORT_LIMIT + 1);
    opened.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > REPORT_LIMIT {
        return None;
    }
    let body: Value = serde_json::from_slice(&bytes).ok()?;
    Some(Report {
        file,
        event: body["event"].as_str()?.to_owned(),
        session_id: body["session_id"].as_str().map(str::to_owned),
        cwd: body["cwd"].as_str()?.to_owned(),
        at_ms: body["at_ms"].as_i64()?,
    })
}
