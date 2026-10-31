//! Drives the real binary and the real process probe.
mod common;

use astrolabe::model::SessionDisplay;
use astrolabe::{Engine, OsProcessProbe, ProcessProbe, SystemClock};
use common::TempDir;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_astrolabe");

fn spawn_serve(dir: &TempDir) -> Child {
    Command::new(BIN)
        .args(["serve", "--db"])
        .arg(dir.db())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn first_line(child: &mut Child) -> String {
    let stdout = child.stdout.take().unwrap();
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    line
}

fn dead_pid() -> u32 {
    let mut child = Command::new(BIN)
        .arg("--version")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// Catches: `serve` exiting after start, missing reconcile, or a second core
/// running on the same database and alerting twice.
#[test]
fn serve_reconciles_stays_up_excludes_a_second_core_and_survives_being_killed() {
    let dir = TempDir::new();
    let gone = {
        let engine = Engine::open(&dir.db(), SystemClock, OsProcessProbe).unwrap();
        engine.launch(dead_pid()).unwrap()
    };

    let mut first = spawn_serve(&dir);
    let line = first_line(&mut first);
    assert!(line.contains("1 interrupted"), "unexpected: {line}");
    assert_eq!(first.try_wait().unwrap(), None, "serve must keep running");

    let second = spawn_serve(&dir).wait_with_output().unwrap();
    assert_eq!(second.status.code(), Some(1));
    let error = String::from_utf8_lossy(&second.stderr);
    assert!(error.contains("another core"), "unexpected: {error}");

    first.kill().unwrap();
    first.wait().unwrap();

    let mut third = spawn_serve(&dir);
    let line = first_line(&mut third);
    assert!(line.contains("0 interrupted"), "unexpected: {line}");
    third.kill().unwrap();
    third.wait().unwrap();

    let engine = Engine::open(&dir.db(), SystemClock, OsProcessProbe).unwrap();
    assert_eq!(
        engine.inspect(gone).unwrap().display,
        SessionDisplay::Interrupted
    );
}

/// Catches: a probe that misreads the process table on this OS, which would
/// interrupt live sessions or leave dead ones looking alive.
#[test]
fn the_os_probe_tells_live_processes_from_gone_ones() {
    assert!(OsProcessProbe.is_alive(std::process::id()));
    assert!(!OsProcessProbe.is_alive(dead_pid()));
}

/// Catches: a database created without its schema, or reopened destructively.
#[test]
fn opening_creates_the_store_and_reopening_keeps_it() {
    let dir = TempDir::new();
    let nested = dir.0.join("not/yet/there/astrolabe.db");
    let session = Engine::open(&nested, SystemClock, OsProcessProbe)
        .unwrap()
        .launch(std::process::id())
        .unwrap();
    let engine = Engine::open(&nested, SystemClock, OsProcessProbe).unwrap();
    assert_eq!(engine.inspect(session).unwrap().session.id, session);
}

/// Catches: an argument mistake starting a core on the wrong database.
#[test]
fn serve_rejects_bad_arguments() {
    let output = Command::new(BIN).args(["serve", "--db"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}
