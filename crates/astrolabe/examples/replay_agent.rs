//! Plays back a recording of a real adapter as a child process, for `cargo test`.
//! Usage: `replay_agent <recording.jsonl>`. Each line is `{"dir","t","msg"}`:
//! `out` lines are sent, each `in` line waits for the client's next message and
//! checks it, `exit` ends with the recorded code. A mismatch, including a client that
//! stops short of the recording or goes on past it, writes `replay.mismatch` in the
//! working directory and exits 99. Provider-neutral.
use serde_json::Value;
use std::io::{BufRead, Write};

const MISMATCH_FILE: &str = "replay.mismatch";

fn main() {
    let path = std::env::args().nth(1).expect("usage: replay_agent <file>");
    let text = std::fs::read_to_string(&path).expect("recording");
    let mut input = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    for line in text.lines() {
        let entry: Value = serde_json::from_str(line).expect("recording line");
        match entry["dir"].as_str() {
            Some("out") => {
                let sent = writeln!(out, "{}", entry["msg"]).and_then(|()| out.flush());
                if sent.is_err() {
                    fail("the client stopped reading before the recording ended");
                }
            }
            Some("in") => {
                let mut got = String::new();
                if input.read_line(&mut got).unwrap() == 0 {
                    // The client closed the pipe where the real one wrote on: a mismatch.
                    fail(&format!("pipe closed, expected {}", entry["msg"]));
                }
                let got: Value = serde_json::from_str(&got).expect("client JSON");
                check(&entry["msg"], &got);
            }
            Some("exit") => {
                // A clean exit was the adapter leaving when its pipe closed.
                let code = entry["code"].as_i64().unwrap_or(0) as i32;
                if code == 0 {
                    wait_for_close(&mut input);
                }
                std::process::exit(code);
            }
            other => fail(&format!("unknown direction {other:?}")),
        }
    }
    wait_for_close(&mut input);
}

/// Like the real adapter, stays until its pipe closes. Anything more the client says
/// was never said to the real one.
fn wait_for_close(input: &mut impl BufRead) {
    let mut rest = String::new();
    if input.read_line(&mut rest).is_ok_and(|n| n > 0) {
        fail(&format!("unrecorded message after the end: {rest}"));
    }
}

/// The whole message must match, but for what differs by design: the test runs in
/// another directory, and the crate's version moves.
fn check(want: &Value, got: &Value) {
    if comparable(want) != comparable(got) {
        fail(&format!("expected {want}, got {got}"));
    }
}

fn comparable(message: &Value) -> Value {
    let mut message = message.clone();
    if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
        params.remove("cwd");
        if let Some(info) = params.get_mut("clientInfo").and_then(Value::as_object_mut) {
            info.remove("version");
        }
    }
    message
}

/// Leaves a mark in the working directory too: a client already closing never reads
/// the exit code, and the test looks for this file afterwards.
fn fail(message: &str) -> ! {
    eprintln!("replay mismatch: {message}");
    let _ = std::fs::write(MISMATCH_FILE, message);
    std::process::exit(99);
}
