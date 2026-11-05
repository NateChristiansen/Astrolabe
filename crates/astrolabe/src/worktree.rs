//! Git helpers for reading repo identity and syncing the owner's notes.
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(Command::new("git").args(args), cwd)
}

type Output = std::result::Result<String, String>;

fn run_git(command: &mut Command, cwd: &Path) -> Output {
    let output = command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git could not start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
