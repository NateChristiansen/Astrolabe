mod common;

use common::TempDir;
use std::fs;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_astrolabe")
}
fn name() -> &'static str {
    if cfg!(windows) {
        "astrolabe.exe"
    } else {
        "astrolabe"
    }
}
fn trial() -> &'static str {
    if cfg!(windows) {
        "astrolabe-trial.exe"
    } else {
        "astrolabe-trial"
    }
}
fn run(dir: &std::path::Path, extra: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(["install", "--dir"])
        .arg(dir)
        .args(extra)
        .output()
        .unwrap()
}

fn receipt(dir: &std::path::Path, bytes: &[u8]) {
    use sha2::{Digest, Sha256};
    fs::write(
        dir.join(".astrolabe-install.json"),
        serde_json::json!({
            "format": 1, "version": "0.4.0", "sha256": format!("{:x}", Sha256::digest(bytes))
        })
        .to_string(),
    )
    .unwrap();
}
fn backup(dir: &std::path::Path) -> std::path::PathBuf {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".astrolabe-backup-")
        })
        .unwrap()
        .path()
}

#[test]
fn fresh_install_and_repeat_preserve_unrelated_files() {
    let temp = TempDir::new();
    let dir = temp.0.join("a folder with spaces");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("notes.md"), "my notes").unwrap();
    for _ in 0..2 {
        let output = run(&dir, &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(dir.join(name())).unwrap(),
            fs::read(binary()).unwrap()
        );
        assert!(!dir.join(trial()).exists());
        assert_eq!(
            fs::read_to_string(dir.join("notes.md")).unwrap(),
            "my notes"
        );
        assert!(!dir.join(".astrolabe-install-pending").exists());
    }
    let output = Command::new(dir.join(name()))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        // A nightly build names its commit too, so compare with the program's own line.
        astrolabe::update::version_line()
    );
}

#[test]
fn unreceipted_programs_are_never_replaced() {
    let temp = TempDir::new();
    fs::write(temp.0.join(name()), "some other app").unwrap();
    assert!(!run(&temp.0, &[]).status.success());
    assert_eq!(
        fs::read_to_string(temp.0.join(name())).unwrap(),
        "some other app"
    );
}

#[test]
fn the_migration_flag_is_gone() {
    let temp = TempDir::new();
    assert!(!run(&temp.0, &["--migrate-trial"]).status.success());
    assert!(!temp.0.join(name()).exists());
}

#[test]
fn an_update_replaces_the_program_and_retires_the_old_copy() {
    let temp = TempDir::new();
    let old = b"a previous managed build";
    fs::write(temp.0.join(name()), old).unwrap();
    fs::write(temp.0.join(trial()), old).unwrap();
    receipt(&temp.0, old);
    let output = run(&temp.0, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Retired"));
    assert_eq!(
        fs::read(temp.0.join(name())).unwrap(),
        fs::read(binary()).unwrap()
    );
    assert!(!temp.0.join(trial()).exists());
    let backup = backup(&temp.0);
    assert_eq!(fs::read(backup.join(name())).unwrap(), old);
    assert_eq!(fs::read(backup.join(trial())).unwrap(), old);
}

#[test]
fn a_current_install_still_retires_its_old_copy() {
    let temp = TempDir::new();
    assert!(run(&temp.0, &[]).status.success());
    fs::copy(binary(), temp.0.join(trial())).unwrap();
    let output = run(&temp.0, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!temp.0.join(trial()).exists());
    assert_eq!(
        fs::read(temp.0.join(name())).unwrap(),
        fs::read(binary()).unwrap()
    );
}

#[test]
fn an_old_copy_that_is_not_the_receipted_build_is_kept() {
    let temp = TempDir::new();
    fs::write(temp.0.join(trial()), "an older unreceipted trial build").unwrap();
    let output = run(&temp.0, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("left in place"));
    fs::write(temp.0.join(trial()), "edited").unwrap();
    let output = run(&temp.0, &[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("left in place"));
    assert_eq!(fs::read_to_string(temp.0.join(trial())).unwrap(), "edited");
}

#[test]
fn modified_program_and_corrupt_receipt_are_not_adopted() {
    let temp = TempDir::new();
    assert!(run(&temp.0, &[]).status.success());
    fs::write(temp.0.join(name()), "edited").unwrap();
    assert!(!run(&temp.0, &[]).status.success());
    assert_eq!(fs::read_to_string(temp.0.join(name())).unwrap(), "edited");
    fs::write(temp.0.join(".astrolabe-install.json"), "broken").unwrap();
    assert!(!run(&temp.0, &[]).status.success());
}

#[test]
fn interrupted_install_refuses_to_destroy_recovery_files() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join(".astrolabe-install-pending"),
        "recovery instructions",
    )
    .unwrap();
    assert!(!run(&temp.0, &[]).status.success());
    assert_eq!(
        fs::read_to_string(temp.0.join(".astrolabe-install-pending")).unwrap(),
        "recovery instructions"
    );
    assert!(!temp.0.join(name()).exists());
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed() {
    let temp = TempDir::new();
    std::os::unix::fs::symlink(binary(), temp.0.join(name())).unwrap();
    assert!(!run(&temp.0, &[]).status.success());
    assert!(
        fs::symlink_metadata(temp.0.join(name()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_old_copy_is_left_alone() {
    let temp = TempDir::new();
    assert!(run(&temp.0, &[]).status.success());
    std::os::unix::fs::symlink(temp.0.join(name()), temp.0.join(trial())).unwrap();
    let output = run(&temp.0, &[]);
    assert!(output.status.success());
    assert!(
        fs::symlink_metadata(temp.0.join(trial()))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(windows)]
#[test]
fn a_locked_old_copy_leaves_the_install_as_it_was() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = TempDir::new();
    let old = b"a previous managed build";
    fs::write(temp.0.join(name()), old).unwrap();
    fs::write(temp.0.join(trial()), old).unwrap();
    receipt(&temp.0, old);
    let _held = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(temp.0.join(trial()))
        .unwrap();
    assert!(!run(&temp.0, &[]).status.success());
    assert_eq!(fs::read(temp.0.join(name())).unwrap(), old);
    assert!(temp.0.join(trial()).exists());
}
