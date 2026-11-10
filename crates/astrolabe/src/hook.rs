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
/// Tests set the most a hook spends reading its input, in milliseconds.
const STDIN_DEADLINE_VAR: &str = "ASTROLABE_HOOK_STDIN_DEADLINE_MS";

/// What arrived, and whether it ran past `STDIN_LIMIT` and was cut there. Read until the
/// JSON is whole, the client closes the pipe, nothing more comes for `STDIN_WAIT`, so a
/// big input that's still arriving isn't cut, or `within` passes in all: the event's share
/// of its own time (see `deadline`), so a client trickling bytes can't hold the hook past
/// the app's timeout, and still far more than a pipe takes to carry 64 MiB. Input still
/// incomplete then is read as it stands, so the plugin blocks it as unreadable.
pub(crate) fn read_stdin(within: Duration) -> (Vec<u8>, bool) {
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
    let deadline = Instant::now() + total.map_or(within, Duration::from_millis);
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

/// The folder a hook input's command runs in, by the guard's rule: the tool's own folder
/// when its input names one, taken from the session's `cwd` if it's relative, else that
/// `cwd`. The one reader for everything the hook does with a command.
pub(crate) fn folder(input: &Value) -> Option<PathBuf> {
    let base = input["cwd"].as_str().map(PathBuf::from);
    match crate::guard::tool_folder(&input["tool_input"]) {
        Some(own) if Path::new(own).is_absolute() => Some(own.into()),
        Some(own) => Some(base?.join(own)),
        None => base,
    }
}

/// One JSON line per denial, in the plugin state folder.
pub const DENIAL_LOG: &str = "guard-denials.log";
const LOGGED_COMMAND: usize = 2000;

/// The guard's answer to one plugin hook input. Denials in the state folder prove
/// that this session's hook intercepted the probe.
pub(crate) fn guard(bytes: &[u8], data: Option<&Path>) {
    if let Some(denial) = verdict(bytes, data) {
        say(&denial);
    }
}

/// The guard's verdict on one plugin hook input: its denial, or None to let the command
/// run. Reached, it's what the hook answers even if its time runs out before it prints.
pub(crate) fn verdict(bytes: &[u8], data: Option<&Path>) -> Option<Value> {
    let verdict = decide(bytes, data);
    crate::deadline::judged(verdict.clone());
    crate::deadline::stall("verdict");
    verdict
}

fn decide(bytes: &[u8], data: Option<&Path>) -> Option<Value> {
    let reason = std::panic::catch_unwind(|| judge(bytes)).unwrap_or_else(|_| {
        crate::guard::names_denied_program(&String::from_utf8_lossy(bytes))
            .then(|| "a command the guard could not read".to_owned())
    })?;
    if let Some(data) = data {
        let used = std::panic::catch_unwind(|| {
            let input: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
            crate::activate::allow::consume(data, &input, &reason)
        });
        if matches!(used, Ok(Ok(true))) {
            return None;
        }
        let _ = std::panic::catch_unwind(|| record_denial(bytes, &reason, data));
    }
    let hint = if crate::activate::allow::covered(&reason) {
        format!(" {}", crate::activate::allow::HINT)
    } else {
        String::new()
    };
    Some(deny(&format!(
        "Astrolabe denied {reason}. Ask the owner if this is really needed.{hint}"
    )))
}

pub(crate) fn deny(reason: &str) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    }})
}

/// The hook's one answer on stdout, unless its time already ran out and the backstop
/// answered for it.
pub(crate) fn say(answer: &Value) {
    crate::deadline::say(|| print(answer));
}

/// The same, and whether this answer went out rather than the backstop's before it. If it
/// does, `first` runs just before it's printed, under the one-answer lock, so what it
/// records can't be recorded for an answer the backstop gave instead.
pub(crate) fn said(answer: &Value, first: impl FnOnce()) -> bool {
    crate::deadline::said(|| {
        first();
        print(answer);
    })
}

/// One line on stdout. Never panics, as stdout may be closed.
pub(crate) fn print(answer: &Value) {
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
    let powershell = crate::activate::powershell_input(&input);
    crate::guard::decide_in(powershell, tool, &input["tool_input"], &scope)
}

/// The nearest folder holding a `.git`.
fn worktree_root(cwd: Option<&Path>) -> Option<String> {
    let found = cwd?.ancestors().find(|dir| dir.join(".git").exists())?;
    Some(comparable(found))
}

fn record_denial(bytes: &[u8], reason: &str, data: &Path) {
    let input: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let session_id = input["session_id"].as_str();
    let Some(cwd) = folder(&input).or_else(|| std::env::current_dir().ok()) else {
        return;
    };
    // The field the guard judged, so a probe is proven wherever its command arrived.
    let command = crate::guard::command_field(&input["tool_input"])
        .map(crate::guard::command_text)
        .unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::folder;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn a_commands_folder_is_the_tools_own_when_it_names_one() {
        let base = std::env::temp_dir();
        let own = base.join("own");
        for (tool_input, expected) in [
            (json!({"command": "ls"}), Some(base.clone())),
            (
                json!({"command": "ls", "workdir": "sub"}),
                Some(base.join("sub")),
            ),
            (json!({"cmd": "ls", "cwd": own}), Some(own.clone())),
            (
                json!({"cmd": "ls", "working_directory": own}),
                Some(own.clone()),
            ),
        ] {
            let input = json!({"cwd": base, "tool_input": tool_input});
            assert_eq!(folder(&input), expected, "{input}");
        }
        assert_eq!(folder(&json!({"tool_input": {"workdir": own}})), Some(own));
        let relative = json!({"tool_input": {"workdir": "sub"}});
        assert_eq!(folder(&relative), None::<PathBuf>);
        assert_eq!(folder(&json!({"tool_input": {"command": "ls"}})), None);
    }
}
