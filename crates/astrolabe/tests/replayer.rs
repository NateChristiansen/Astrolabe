//! The one replayer, held to its checks in both recording formats. A mismatch must
//! leave its mark: `shutdown` hides the exit code from the tests that use it.
mod common;

use common::TempDir;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn replay_agent() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    let name = format!("replay_agent{}", std::env::consts::EXE_SUFFIX);
    profile.join("examples").join(name)
}

#[derive(Clone, Copy)]
enum Format {
    Claude,
    Codex,
}

const BOTH: [Format; 2] = [Format::Claude, Format::Codex];

fn expect(format: Format, message: &Value) -> Value {
    match format {
        Format::Claude => json!({ "dir": "in", "t": 0, "msg": message }),
        Format::Codex => {
            json!({ "process": 0, "elapsed_ms": 0, "direction": "to_agent", "message": message })
        }
    }
}

fn exit(format: Format, code: i32) -> Value {
    match format {
        Format::Claude => json!({ "dir": "exit", "t": 0, "code": code }),
        Format::Codex => json!({ "process": 0, "exit_code": code }),
    }
}

/// Plays `rows`, the client writes `said` and closes. Returns the exit code and mark.
fn run(dir: &Path, rows: &[Value], said: &[Value]) -> (Option<i32>, Option<String>) {
    let trace = dir.join("trace.jsonl");
    let text: Vec<String> = rows.iter().map(Value::to_string).collect();
    std::fs::write(&trace, text.join("\n")).unwrap();
    let mut child = Command::new(replay_agent())
        .arg(&trace)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in said {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let status = child.wait().unwrap();
    let mark = std::fs::read_to_string(dir.join("replay.mismatch")).ok();
    (status.code(), mark)
}

fn caught(result: (Option<i32>, Option<String>), what: &str) {
    assert_eq!(result.0, Some(99), "{what}");
    assert!(result.1.is_some(), "{what}: no mark");
}

#[test]
fn a_faithful_client_passes_in_both_formats() {
    let message = json!({ "id": 1, "method": "m", "params": { "a": 1 } });
    for format in BOTH {
        let dir = TempDir::new();
        let rows = [expect(format, &message), exit(format, 0)];
        assert_eq!(
            run(&dir.0, &rows, std::slice::from_ref(&message)),
            (Some(0), None)
        );
    }
}

#[test]
fn any_difference_in_a_message_is_caught() {
    let want = json!({ "id": 1, "method": "m", "params": { "a": 1 } });
    let got = json!({ "id": 1, "method": "m", "params": { "a": 1, "extra": 2 } });
    for format in BOTH {
        let dir = TempDir::new();
        caught(
            run(&dir.0, &[expect(format, &want)], std::slice::from_ref(&got)),
            "field",
        );
    }
}

#[test]
fn a_message_past_the_end_is_caught_with_or_without_an_exit_row() {
    let extra = json!({ "id": 9 });
    for format in BOTH {
        let dir = TempDir::new();
        let rows = [exit(format, 0)];
        caught(
            run(&dir.0, &rows, std::slice::from_ref(&extra)),
            "after exit",
        );
        let dir = TempDir::new();
        caught(run(&dir.0, &[], std::slice::from_ref(&extra)), "empty");
    }
}

#[test]
fn a_client_stopping_short_is_caught() {
    let want = json!({ "id": 1 });
    for format in BOTH {
        let dir = TempDir::new();
        caught(run(&dir.0, &[expect(format, &want)], &[]), "closed");
    }
}

/// The client's path is canonicalized before comparing, as a Windows temp path and a
/// macOS `/var` path both differ from their canonical form. Another directory fails,
/// and so does leaving the cwd out.
#[test]
fn the_cwd_is_compared_canonically_on_both_sides() {
    let cwd_message = |cwd: Value| json!({ "id": 1, "params": { "cwd": cwd } });
    let want = cwd_message(json!("/fixture/project"));
    for format in BOTH {
        let dir = TempDir::new();
        std::fs::create_dir(dir.0.join("sub")).unwrap();
        let rows = [expect(format, &want), exit(format, 0)];
        let roundabout = dir.0.join("sub").join("..");
        let said = cwd_message(json!(roundabout.to_str().unwrap()));
        assert_eq!(run(&dir.0, &rows, &[said]), (Some(0), None));

        let elsewhere = cwd_message(json!(std::env::temp_dir().to_str().unwrap()));
        caught(run(&dir.0, &rows, &[elsewhere]), "another directory");

        let dir = TempDir::new();
        let rows = [expect(format, &want), exit(format, 0)];
        let missing = json!({ "id": 1, "params": {} });
        caught(run(&dir.0, &rows, &[missing]), "no cwd");
    }
}

/// Only the given process plays, in the format that has them.
#[test]
fn another_process_rows_are_skipped() {
    let dir = TempDir::new();
    let rows = [
        json!({ "process": 1, "direction": "to_agent", "message": { "id": 7 } }),
        json!({ "process": 0, "direction": "to_agent", "message": { "id": 1 } }),
        json!({ "process": 1, "exit_code": 3 }),
    ];
    assert_eq!(run(&dir.0, &rows, &[json!({ "id": 1 })]), (Some(0), None));
}
