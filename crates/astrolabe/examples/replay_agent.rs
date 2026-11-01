//! Plays back a recording of a real adapter as a child process, for `cargo test`.
//! Usage: `replay_agent <recording.jsonl> [process]`. Two formats, one per provider:
//! `{"dir","t","msg"}` lines with `in`, `out` and `exit`, and `{"process","direction",
//! "message"}` lines with `to_agent`, `from_agent`, `stderr` and `exit_code`, of which
//! only the given process (default 0) plays.
//!
//! Agent lines are sent; each client line waits for the client's next message and
//! compares it whole. A mismatch, including a client that stops short of the recording
//! or goes on past it, writes `replay.mismatch` in the working directory and exits 99:
//! a client already closing never reads the exit code, so tests look for the file.
//! No provider or network is used.
use serde_json::Value;
use std::io::Write;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const MISMATCH_FILE: &str = "replay.mismatch";
/// How long a `process` recording's end listens for more from the client. That client
/// waits for an exit with its pipe open, so this is bounded, not proof of silence.
const TRAILING_QUIET: Duration = Duration::from_millis(250);

type Input = Receiver<std::io::Result<String>>;

enum Step {
    Send(Value),
    Expect(Value),
    Stderr(String),
    Exit(i32),
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: replay_agent <file> [process]");
    let process: u64 = args
        .next()
        .map_or(0, |n| n.parse().expect("process number"));
    let text = std::fs::read_to_string(&path).expect("recording");
    let input = read_input();
    let mut out = std::io::stdout().lock();
    let mut exit_code = None;
    let mut exits_by_itself = false;
    for line in text.lines() {
        let row: Value = serde_json::from_str(line).expect("recording line");
        if let Some(owner) = row.get("process") {
            exits_by_itself = true;
            if owner != process {
                continue;
            }
        }
        match step(&row) {
            Step::Send(message) => {
                let sent = writeln!(out, "{message}").and_then(|()| out.flush());
                if sent.is_err() {
                    fail("the client stopped reading before the recording ended");
                }
            }
            Step::Expect(want) => match input.recv() {
                Ok(Ok(got)) => check(&want, &got),
                // The client closed the pipe where the real one wrote on.
                _ => fail(&format!("pipe closed, expected {want}")),
            },
            Step::Stderr(text) => eprint!("{text}"),
            Step::Exit(code) => exit_code = Some(code),
        }
    }
    if exits_by_itself {
        // These recordings end with the adapter leaving, pipe open or not.
        if let Ok(Ok(rest)) = input.recv_timeout(TRAILING_QUIET) {
            unrecorded(&rest);
        }
    } else if exit_code.unwrap_or(0) == 0 {
        // A clean exit was the adapter leaving when its pipe closed.
        if let Ok(Ok(rest)) = input.recv() {
            unrecorded(&rest);
        }
    }
    std::process::exit(exit_code.unwrap_or(0));
}

fn step(row: &Value) -> Step {
    if let Some(text) = row["stderr"].as_str() {
        return Step::Stderr(text.to_owned());
    }
    if let Some(code) = row["exit_code"].as_i64() {
        return Step::Exit(code as i32);
    }
    let message = || row.get("msg").or(row.get("message")).cloned();
    let direction = row.get("dir").or(row.get("direction"));
    match (direction.and_then(Value::as_str), message()) {
        (Some("out" | "from_agent"), Some(message)) => Step::Send(message),
        (Some("in" | "to_agent"), Some(message)) => Step::Expect(message),
        (Some("exit"), _) => Step::Exit(row["code"].as_i64().unwrap_or(0) as i32),
        _ => fail(&format!("unknown recording line {row}")),
    }
}

/// A thread, so the end of a recording can listen for a bounded time.
fn read_input() -> Input {
    let (lines, input) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines() {
            if lines.send(line).is_err() {
                return;
            }
        }
    });
    input
}

fn unrecorded(message: &str) -> ! {
    fail(&format!("unrecorded message after the end: {message}"))
}

/// The whole message must match, but for what differs by design: the test runs in
/// another directory, and the crate's version moves.
fn check(want: &Value, got: &str) {
    let Ok(got) = serde_json::from_str::<Value>(got) else {
        fail(&format!("client wrote invalid JSON: {got}"));
    };
    if want.pointer("/params/cwd").is_some() {
        let Some(cwd) = got.pointer("/params/cwd").and_then(Value::as_str) else {
            fail(&format!("expected a cwd string, got {got}"));
        };
        let here = std::env::current_dir().and_then(std::fs::canonicalize);
        let theirs = std::fs::canonicalize(cwd);
        if !matches!((&here, &theirs), (Ok(here), Ok(theirs)) if here == theirs) {
            fail(&format!("cwd {cwd} is not the working directory {here:?}"));
        }
    }
    if comparable(want) != comparable(&got) {
        fail(&format!("expected {want}, got {got}"));
    }
}

fn comparable(message: &Value) -> Value {
    let mut message = message.clone();
    if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
        // Kept as a key: a client that leaves the cwd out still differs.
        if let Some(cwd) = params.get_mut("cwd") {
            *cwd = Value::Null;
        }
        // The recordings predate this capability, and only children behave differently
        // with it. Real child wire is checked by real runs, not here.
        if let Some(capabilities) = params
            .get_mut("clientCapabilities")
            .and_then(Value::as_object_mut)
        {
            capabilities.remove("subagents");
        }
        if let Some(info) = params.get_mut("clientInfo").and_then(Value::as_object_mut) {
            info.remove("version");
        }
    }
    message
}

fn fail(message: &str) -> ! {
    eprintln!("replay mismatch: {message}");
    let _ = std::fs::write(MISMATCH_FILE, message);
    std::process::exit(99);
}
