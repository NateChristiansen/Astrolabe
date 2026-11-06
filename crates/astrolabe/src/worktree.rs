//! Git helpers for reading repo identity and syncing the owner's notes.
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(Command::new("git").args(args), cwd)
}

/// The same, with stdout as git wrote it: file names in `-z` output are bytes, and a lossy
/// decode could make two names one.
pub(crate) fn git_bytes(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    run_git_bytes(Command::new("git").args(args), cwd)
}

type Output = std::result::Result<String, String>;

fn run_git(command: &mut Command, cwd: &Path) -> Output {
    run_git_bytes(command, cwd).map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git_bytes(command: &mut Command, cwd: &Path) -> Result<Vec<u8>, String> {
    let output = command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git could not start: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
