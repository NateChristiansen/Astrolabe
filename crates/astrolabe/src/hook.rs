//! Input and denial output for the plugin's gated hooks.
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a hook waits for more input once none comes. A client writes the hook's input
/// at once; one that holds the pipe open must not hold up the hook.
const STDIN_WAIT: Duration = Duration::from_secs(1);
/// Input over this isn't read whole: far above any one tool call (a big Write or patch),
/// and a bound on memory for a client that streams without end.
pub(crate) const STDIN_LIMIT: usize = 64 << 20;
/// The most a hook spends reading its input in all, so a client trickling bytes can't hold it:
/// well under the apps' own hook timeouts (Claude's is 60 seconds), and far more than a pipe
/// takes to carry 64 MiB. Input still incomplete then is read as it stands, so the plugin
/// blocks it as unreadable.
const STDIN_DEADLINE: Duration = Duration::from_secs(30);
/// Tests set a shorter deadline, in milliseconds.
const STDIN_DEADLINE_VAR: &str = "ASTROLABE_HOOK_STDIN_DEADLINE_MS";

/// What arrived, and whether it ran past `STDIN_LIMIT` and was cut there. Read until the
/// JSON is whole, the client closes the pipe, nothing more comes for `STDIN_WAIT`, so a
/// big input that's still arriving isn't cut, or `STDIN_DEADLINE` passes in all.
pub(crate) fn read_stdin() -> (Vec<u8>, bool) {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        // The limit is on what follows a mark, so room for one, and a byte to see it's over.
        let mut stdin = io::stdin().take((STDIN_LIMIT + BOM.len()) as u64 + 1);
        let mut chunk = vec![0; 1 << 16];
        while let Ok(read @ 1..) = stdin.read(&mut chunk) {
            if sender.send(chunk[..read].to_vec()).is_err() {
                return;
            }
        }
    });
    let total = std::env::var(STDIN_DEADLINE_VAR)
        .ok()
        .and_then(|ms| ms.parse().ok());
    let deadline = Instant::now() + total.map_or(STDIN_DEADLINE, Duration::from_millis);
    let mut input = Vec::new();
    let wait = || STDIN_WAIT.min(deadline.saturating_duration_since(Instant::now()));
    while let Ok(chunk) = receiver.recv_timeout(wait()) {
        input.extend(chunk);
        // Checked as it grows, as the mark may arrive split across reads.
        if input.starts_with(BOM) {
            input.drain(..BOM.len());
        }
        if input.len() > STDIN_LIMIT {
            input.truncate(STDIN_LIMIT);
            return (input, true);
        }
        // Parsed only when it could be whole, so a big input isn't parsed once per read.
        let last = input.iter().rev().find(|b| !b.is_ascii_whitespace());
        if last == Some(&b'}') && serde_json::from_slice::<Value>(&input).is_ok() {
            break;
        }
    }
    (input, false)
}

/// The same folder reads the same from a client and from here (`/tmp` and `/private/tmp`).
pub fn comparable(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    path.to_string_lossy().into_owned()
}

/// One JSON line per denial, in the plugin state folder.
pub const DENIAL_LOG: &str = "guard-denials.log";
const LOGGED_COMMAND: usize = 2000;

/// The guard's answer to one plugin hook input. Denials in the state folder prove
/// that this session's hook intercepted the probe.
pub(crate) fn guard(bytes: &[u8], data: Option<&Path>) {
    let reason = std::panic::catch_unwind(|| judge(bytes)).unwrap_or_else(|_| {
        crate::guard::names_denied_program(&String::from_utf8_lossy(bytes))
            .then(|| "a command the guard could not read".to_owned())
    });
    let Some(reason) = reason else { return };
    if let Some(data) = data {
        let used = std::panic::catch_unwind(|| {
            let input: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
            crate::activate::allow::consume(data, &input, &reason)
        });
        if matches!(used, Ok(Ok(true))) {
            return;
        }
        let _ = std::panic::catch_unwind(|| record_denial(bytes, &reason, data));
    }
    let hint = if crate::activate::allow::covered(&reason) {
        format!(" {}", crate::activate::allow::HINT)
    } else {
        String::new()
    };
    say(&deny(&format!(
        "Astrolabe denied {reason}. Ask the owner if this is really needed.{hint}"
    )));
}

pub(crate) fn deny(reason: &str) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
}

/// One answer on stdout. Never panics, as stdout may be closed.
pub(crate) fn say(answer: &Value) {
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

/// The nearest folder holding a `.git`.
fn worktree_root(cwd: Option<&Path>) -> Option<String> {
    let found = cwd?.ancestors().find(|dir| dir.join(".git").exists())?;
    Some(comparable(found))
}

fn record_denial(bytes: &[u8], reason: &str, data: &Path) {
    let input: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let session_id = input["session_id"].as_str();
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let Some(cwd) = cwd.or_else(|| std::env::current_dir().ok()) else {
        return;
    };
    let tool_input = &input["tool_input"];
    let command = tool_input["command"].as_str().unwrap_or_default();
    let line = json!({
        "at_ms": crate::notes::now_ms(),
        "session_id": session_id,
        "cwd": comparable(&cwd),
        "agent_id": input["agent_id"],
        "tool": input["tool_name"],
        "shell": crate::activate::shell_of(&input),
        "reason": reason,
        "command": command.chars().take(LOGGED_COMMAND).collect::<String>(),
        "hook": crate::activate::hook_id(),
    });
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join(DENIAL_LOG));
    if let Ok(mut log) = log {
        let _ = log.write_all(format!("{line}\n").as_bytes());
    }
}
