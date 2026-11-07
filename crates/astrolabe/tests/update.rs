//! The updater must not touch a build output or reach downloads before choosing an install.
mod common;
use common::TempDir;
#[cfg(unix)]
use std::fs;
use std::path::Path;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_astrolabe")
}

fn isolated(home: &Path) -> Command {
    let mut command = Command::new(binary());
    command.env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home);
    command.env("PATH", home.join("no-commands"));
    command
}

#[test]
fn a_build_without_an_install_fails_before_downloading() {
    let home = TempDir::new();
    let out = isolated(&home.0)
        .args(["update", "--channel", "nightly", "--dry-run"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no verified user-local installation"));
    assert!(!home.0.join(".local").exists());
}

#[cfg(unix)]
#[test]
fn a_build_delegates_all_flags_and_propagates_failure() {
    use std::os::unix::fs::PermissionsExt;
    for exit_code in [0, 7] {
        let home = TempDir::new();
        let source = home.0.join("old-installed-program");
        // Stand in for an older installed updater, whose own version logic must run.
        fs::write(&source, format!("#!/bin/sh\nprintf '%s\\n' \"$@\"\nprintf 'installed updater ran\\n'\nexit {exit_code}\n")).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        let directory = home.0.join(".local/bin");
        astrolabe::install::install(&source, &directory).unwrap();
        let out = isolated(&home.0)
            .args(["update", "--channel", "nightly", "--dry-run", "--force"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.success(), exit_code == 0, "{stdout}");
        assert!(
            stdout
                .contains("update\n--channel\nnightly\n--dry-run\n--force\ninstalled updater ran"),
            "{stdout}"
        );
        assert!(stdout.contains(&directory.join("astrolabe").display().to_string()));
        assert_eq!(
            fs::read(directory.join("astrolabe")).unwrap(),
            fs::read(source).unwrap()
        );
    }
}

#[cfg(unix)]
#[test]
fn the_installed_copy_previews_its_own_destination_without_delegation() {
    use std::os::unix::fs::PermissionsExt;
    let home = TempDir::new();
    let custom = home.0.join("custom installation");
    astrolabe::install::install(Path::new(binary()), &custom).unwrap();
    let commands = home.0.join("commands");
    fs::create_dir(&commands).unwrap();
    let curl = commands.join("curl");
    fs::write(&curl, "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in --output) shift; output=$1 ;; esac\n  shift\ndone\nprintf '%s' '{\"version\":\"0.0.1\",\"commit\":\"1111111111111111111111111111111111111111\"}' > \"$output\"\n").unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new(custom.join("astrolabe"))
        .env("HOME", &home.0)
        .env("PATH", commands)
        .args(["update", "--channel", "nightly", "--dry-run", "--force"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(&format!(
            "installer into {}.",
            std::fs::canonicalize(&custom).unwrap().display()
        )),
        "{stdout}"
    );
    assert!(
        !stdout.contains("Using installed Astrolabe"),
        "delegation must terminate"
    );
}
