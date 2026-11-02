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

fn run(dir: &TempDir, answer: &str, custom: &[(&str, &Path)]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["doctor", "adapters"])
        .env("ASTROLABE_DATA_DIR", dir.0.join("data"))
        .env("HOME", &dir.0)
        .env("PATH", dir.0.join("bin"))
        .envs(custom.iter().copied())
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
        let log = r#"[ "$1" = install ] || exit 0; echo "$@" >> "$HOME/pnpm.log"; /bin/pwd >> "$HOME/pnpm.log""#;
        stand_in(&bin, "pnpm", log);
    }
    dir
}

#[test]
fn a_missing_tool_is_said_plainly_and_nothing_is_written() {
    let dir = setup(false);
    let output = run(&dir, "yes", &[]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("pnpm was not found"));
    assert!(!dir.0.join("data/adapters/package.json").exists());
}

#[test]
fn only_a_typed_yes_writes_and_the_install_is_frozen_to_the_lockfile() {
    let dir = setup(true);
    assert!(String::from_utf8_lossy(&run(&dir, "no", &[]).stdout).contains("nothing written"));
    assert!(!dir.0.join("data/adapters/package.json").exists());
    assert!(run(&dir, "yes", &[]).status.success());
    assert!(dir.0.join("data/adapters/pnpm-lock.yaml").exists());
    let log = std::fs::read_to_string(dir.0.join("pnpm.log")).unwrap();
    assert!(log.contains("install --frozen-lockfile --ignore-scripts"));
    assert!(log.trim_end().ends_with("data/adapters"), "{log}");
}

#[test]
fn it_installs_where_a_launch_looks() {
    let dir = setup(true);
    let custom = dir.0.join("custom");
    assert!(
        run(&dir, "yes", &[("ASTROLABE_ADAPTERS", &custom)])
            .status
            .success()
    );
    assert!(custom.join("pnpm-lock.yaml").exists());
    assert!(!dir.0.join("data/adapters").exists());
}
