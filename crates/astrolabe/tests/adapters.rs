//! `doctor adapters`, the real binary on a temporary home with stand-ins for node and pnpm.
#![cfg(unix)]
mod common;

use common::TempDir;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn stand_in(bin: &Path, name: &str, body: &str) {
    let file = bin.join(name);
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn run(dir: &TempDir, answer: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["doctor", "adapters"])
        .env("ASTROLABE_DATA_DIR", dir.0.join("data"))
        .env("HOME", &dir.0)
        .env("PATH", dir.0.join("bin"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{answer}").unwrap();
    child.wait_with_output().unwrap()
}

fn setup(with_pnpm: bool) -> TempDir {
    let dir = TempDir::new();
    let bin = dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    stand_in(&bin, "node", "exit 0");
    if with_pnpm {
        stand_in(
            &bin,
            "pnpm",
            r#"[ "$1" = install ] || exit 0; echo "$@" >> "$HOME/pnpm.log"; /bin/pwd >> "$HOME/pnpm.log""#,
        );
    }
    dir
}

#[test]
fn a_missing_tool_is_said_plainly_and_nothing_is_written() {
    let dir = setup(false);
    let output = run(&dir, "yes");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("pnpm was not found"));
    assert!(!dir.0.join("data/adapters/package.json").exists());
}

#[test]
fn only_a_typed_yes_writes_and_the_install_is_frozen_to_the_lockfile() {
    let dir = setup(true);
    let output = run(&dir, "no");
    assert!(String::from_utf8_lossy(&output.stdout).contains("nothing written"));
    assert!(!dir.0.join("data/adapters/package.json").exists());
    assert!(!dir.0.join("pnpm.log").exists());

    let output = run(&dir, "yes");
    assert!(output.status.success(), "{output:?}");
    let adapters = dir.0.join("data/adapters");
    let lock = std::fs::read_to_string(adapters.join("pnpm-lock.yaml")).unwrap();
    assert!(lock.contains("codex-acp") && lock.contains("claude-agent-acp"));
    assert!(adapters.join("package.json").exists());
    let log = std::fs::read_to_string(dir.0.join("pnpm.log")).unwrap();
    assert!(
        log.contains("install --frozen-lockfile --ignore-scripts"),
        "{log}"
    );
    assert!(log.trim_end().ends_with("data/adapters"), "{log}");
}
