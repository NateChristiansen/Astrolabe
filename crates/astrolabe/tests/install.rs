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

#[test]
fn path_diagnostics_name_only_real_final_destination_conflicts() {
    let temp = TempDir::new();
    let installed = temp.0.join("installed");
    let other = temp.0.join("other");
    fs::create_dir(&other).unwrap();
    fs::copy(binary(), other.join(name())).unwrap();
    for (first, warning) in [(&installed, false), (&other, true)] {
        let path = std::env::join_paths([first, &installed]).unwrap();
        let out = Command::new(binary())
            .args(["install", "--dir"])
            .arg(&installed)
            .env("PATH", path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout.contains("PATH finds"), warning, "{stdout}");
        if warning {
            assert!(stdout.contains(&other.join(name()).display().to_string()));
            assert!(stdout.contains(&installed.join(name()).display().to_string()));
        }
    }
}

#[cfg(unix)]
#[test]
fn nonexecutable_files_and_links_to_the_install_do_not_warn() {
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new();
    let installed = temp.0.join("installed");
    let earlier = temp.0.join("earlier");
    fs::create_dir(&earlier).unwrap();
    fs::write(earlier.join(name()), "not a command").unwrap();
    fs::set_permissions(earlier.join(name()), fs::Permissions::from_mode(0o644)).unwrap();
    let path = std::env::join_paths([&earlier, &installed]).unwrap();
    for link in [false, true] {
        if link {
            fs::remove_file(earlier.join(name())).unwrap();
            std::os::unix::fs::symlink(installed.join(name()), earlier.join(name())).unwrap();
        }
        let out = Command::new(binary())
            .args(["install", "--dir"])
            .arg(&installed)
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("PATH finds"));
    }
}

#[cfg(windows)]
#[test]
fn an_earlier_windows_command_shim_is_named() {
    let temp = TempDir::new();
    let installed = temp.0.join("installed");
    let earlier = temp.0.join("earlier");
    fs::create_dir(&earlier).unwrap();
    let shim = earlier.join("astrolabe.cmd");
    fs::write(&shim, "@echo old shim").unwrap();
    let path = std::env::join_paths([&earlier, &installed]).unwrap();
    let out = Command::new(binary())
        .args(["install", "--dir"])
        .arg(&installed)
        .env("PATH", path)
        .env("PATHEXT", ".EXE;.CMD")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("PATH finds") && stdout.contains(&shim.display().to_string()),
        "{stdout}"
    );
}

#[test]
fn a_local_build_does_not_replace_a_published_one_unless_forced() {
    // A build CI stamped is a published one, which the guard lets through.
    if option_env!("ASTROLABE_COMMIT").is_some() {
        return;
    }
    use sha2::{Digest, Sha256};
    let temp = TempDir::new();
    let dir = temp.0.join("bin");
    assert!(run(&dir, &[]).status.success());
    let program = fs::read(dir.join(name())).unwrap();
    let published = serde_json::json!({
        "format": 1, "version": "0.5.0",
        "sha256": format!("{:x}", Sha256::digest(&program)), "commit": "1a2b3c4d5e6f"
    });
    fs::write(dir.join(".astrolabe-install.json"), published.to_string()).unwrap();

    let refused = run(&dir, &[]);
    assert!(!refused.status.success());
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(why.contains("the published build 1a2b3c4"), "{why}");
    assert!(why.contains("astrolabe update"), "{why}");
    assert_eq!(fs::read(dir.join(name())).unwrap(), program);

    assert!(run(&dir, &["--force"]).status.success());
}

#[cfg(unix)]
#[test]
fn a_nightly_from_before_receipts_held_the_commit_is_still_recognised() {
    // A build CI stamped is a published one, which the guard lets through.
    if option_env!("ASTROLABE_COMMIT").is_some() {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new();
    let dir = temp.0.join("bin");
    fs::create_dir_all(&dir).unwrap();
    let nightly = b"#!/bin/sh\necho 'astrolabe 0.5.0 (nightly c3621c1)'\n";
    fs::write(dir.join(name()), nightly).unwrap();
    fs::set_permissions(dir.join(name()), fs::Permissions::from_mode(0o755)).unwrap();
    receipt(&dir, nightly);

    let refused = run(&dir, &[]);
    assert!(!refused.status.success());
    let why = String::from_utf8_lossy(&refused.stderr);
    assert!(why.contains("0.5.0 (nightly c3621c1)"), "{why}");
    assert_eq!(fs::read(dir.join(name())).unwrap(), nightly);

    assert!(run(&dir, &["--force"]).status.success());
    assert_ne!(fs::read(dir.join(name())).unwrap(), nightly);
}
