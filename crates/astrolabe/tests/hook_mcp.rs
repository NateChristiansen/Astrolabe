//! `astrolabe hook` and `astrolabe mcp`, run as the real binary.
mod common;

use astrolabe::hook::{comparable, read_reports, reports_dir_in};
use astrolabe::layer::DATA_DIR_VAR;
use common::{Harness, TempDir};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn astrolabe(data: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
    command
        .args(args)
        .env(DATA_DIR_VAR, data)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn fire(data: &Path, cwd: &Path, input: &str) -> std::process::Output {
    let mut child = astrolabe(data, cwd, &["hook", "session-start"])
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn hook_leaves_one_receipt_per_fire_and_says_what_it_delivered() {
    let dir = TempDir::new();
    let input = json!({ "session_id": "s1", "cwd": dir.0 }).to_string();
    for _ in 0..2 {
        let output = fire(&dir.0, &dir.0, &input);
        assert!(output.status.success());
        // A SessionStart hook's stdout goes into the session's context.
        let said: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let context = &said["hookSpecificOutput"]["additionalContext"];
        assert_eq!(context, astrolabe::voice::NO_AGENT);
    }
    let reports = read_reports(&reports_dir_in(&dir.0));
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].event, "session-start");
    assert_eq!(reports[0].session_id.as_deref(), Some("s1"));
    assert_eq!(reports[0].cwd, comparable(&dir.0));
    assert!(reports[0].at_ms > 0);
    assert_ne!(reports[0].file, reports[1].file);
}

#[test]
fn hook_never_fails_the_agent() {
    let dir = TempDir::new();
    // Bad input: still a receipt, from what the process itself knows.
    assert!(fire(&dir.0, &dir.0, "not json").status.success());
    let reports = read_reports(&reports_dir_in(&dir.0));
    assert_eq!(reports[0].session_id, None);
    assert_eq!(reports[0].cwd, comparable(&dir.0));

    // Nowhere to write: a file sits where the folder would be.
    let blocked = dir.0.join("blocked");
    std::fs::write(&blocked, "").unwrap();
    let output = fire(&blocked, &dir.0, "{}");
    assert!(output.status.success());
    // The model is told, as there's no launch here to stop.
    assert!(String::from_utf8_lossy(&output.stdout).contains("NOT delivered"));
}

#[test]
fn hook_exits_zero_with_stderr_closed_and_nowhere_to_write() {
    let dir = TempDir::new();
    let blocked = dir.0.join("blocked");
    std::fs::write(&blocked, "").unwrap();
    let mut child = astrolabe(&blocked, &dir.0, &["hook", "session-start"])
        .spawn()
        .unwrap();
    drop(child.stderr.take());
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn hook_keeps_input_from_a_client_that_keeps_stdin_open() {
    let dir = TempDir::new();
    let mut child = astrolabe(&dir.0, &dir.0, &["hook", "session-start"])
        .spawn()
        .unwrap();
    let mut held_open = child.stdin.take().unwrap();
    let input = json!({ "session_id": "s1" }).to_string();
    held_open.write_all(input.as_bytes()).unwrap();
    held_open.flush().unwrap();
    assert!(child.wait().unwrap().success());
    let reports = read_reports(&reports_dir_in(&dir.0));
    assert_eq!(reports[0].session_id.as_deref(), Some("s1"));
}

#[test]
fn only_regular_files_of_a_sane_size_are_reports() {
    let dir = TempDir::new();
    fire(&dir.0, &dir.0, "{}");
    let reports = reports_dir_in(&dir.0);
    let big = serde_json::json!({
        "event": "x".repeat(1 << 17), "cwd": "c", "at_ms": 1, "session_id": null,
    });
    std::fs::write(reports.join("big.json"), big.to_string()).unwrap();
    std::fs::create_dir(reports.join("folder.json")).unwrap();
    #[cfg(unix)]
    {
        let fifo = reports.join("blocked.json");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let link = reports.join("endless.json");
        std::os::unix::fs::symlink("/dev/zero", link).unwrap();
    }
    // On a thread: with the bug this never returns.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(read_reports(&reports)));
    let found = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(found.len(), 1);
}

#[test]
fn hook_does_not_wait_for_a_client_that_keeps_stdin_open() {
    let dir = TempDir::new();
    let mut child = astrolabe(&dir.0, &dir.0, &["hook", "session-start"])
        .spawn()
        .unwrap();
    let _held_open = child.stdin.take();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "the hook is waiting on stdin");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success());
    assert_eq!(read_reports(&reports_dir_in(&dir.0)).len(), 1);
}

#[test]
fn core_takes_in_each_report_once_including_those_left_while_it_was_down() {
    let h = Harness::new();
    let reports = reports_dir_in(&h.dir.0);
    assert_eq!(h.engine.ingest_reports(&reports).unwrap(), vec![]);
    let input = json!({ "session_id": "s1" }).to_string();
    fire(&h.dir.0, &h.dir.0, &input);
    let new = h.engine.ingest_reports(&reports).unwrap();
    assert_eq!(new.len(), 1);
    assert_eq!(h.engine.ingest_reports(&reports).unwrap(), vec![]);
    assert_eq!(h.engine.session_reports("s1").unwrap(), new);
    assert_eq!(h.engine.session_reports("other").unwrap(), vec![]);
}

struct Server {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Server {
    fn start(data: &Path, cwd: &Path) -> Self {
        let mut child = astrolabe(data, cwd, &["mcp"]).spawn().unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn call(&mut self, message: &Value) -> Value {
        writeln!(self.stdin, "{message}").unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn close(mut self) {
        drop(self.stdin);
        assert!(self.child.wait().unwrap().success());
    }
}

#[test]
fn server_offers_one_tool_that_reports_what_reached_this_folder() {
    let dir = TempDir::new();
    let elsewhere = TempDir::new();
    fire(&dir.0, &dir.0, &json!({ "session_id": "s1" }).to_string());
    fire(
        &dir.0,
        &elsewhere.0,
        &json!({ "session_id": "s2" }).to_string(),
    );

    let mut server = Server::start(&dir.0, &dir.0);
    let hello = server.call(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-03-26" },
    }));
    assert_eq!(hello["result"]["protocolVersion"], "2025-03-26");
    let future = server.call(&json!({
        "jsonrpc": "2.0", "id": 10, "method": "initialize",
        "params": { "protocolVersion": "2099-01-01" },
    }));
    assert_eq!(future["result"]["protocolVersion"], "2025-06-18");
    // A notification gets no answer: the next line read answers the next call.
    writeln!(
        server.stdin,
        "{}",
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    )
    .unwrap();
    let tools = server.call(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "layer_check");

    let answer = server.call(&json!({
        "jsonrpc": "2.0", "id": "three", "method": "tools/call",
        "params": { "name": "layer_check", "arguments": {} },
    }));
    assert_eq!(answer["id"], "three");
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    let check: Value = serde_json::from_str(text).unwrap();
    assert_eq!(check["cwd"], comparable(&dir.0));
    assert_eq!(check["mcp_starts"], 1);
    let reports = check["reports"].as_array().unwrap();
    let events: Vec<_> = reports.iter().map(|report| &report["event"]).collect();
    assert_eq!(events, ["session-start", "mcp-start"]);
    assert_eq!(reports[0]["session_id"], "s1");

    let unknown = server.call(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/nope" }));
    assert_eq!(unknown["error"]["code"], -32601);
    let other = server.call(&json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "other" },
    }));
    assert_eq!(other["error"]["code"], -32601);
    writeln!(server.stdin, "not json").unwrap();
    let mut line = String::new();
    server.stdout.read_line(&mut line).unwrap();
    assert!(line.contains("-32700"));
    server.close();
}
