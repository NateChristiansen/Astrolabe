//! A new file only its owner can read, whole from its first moment, wherever it is. Fails if it
//! exists. On Windows an entry inherited even briefly could open a handle that keeps its reach.
use std::{io, io::Write, path::Path};

#[cfg(unix)]
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    let mut file = options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())
}

/// One entry, this user, nothing inherited. No SYSTEM or Administrators: only the owner's
/// processes read it, and those accounts can take a file over anyway. Then read back, as a file
/// system may not hold permissions. The text goes by stdin, never in a command line.
#[cfg(windows)]
const CREATE: &str = include_str!("private_file.ps1");

#[cfg(windows)]
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let shell = Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = Command::new(shell);
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = (command.args(["-NoProfile", "-NonInteractive", "-Command", CREATE]))
        .env("ASTROLABE_PRIVATE_FILE", path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .map_or(Ok(()), |mut to| to.write_all(text.as_bytes()))?;
    let out = child.wait_with_output()?;
    let said = String::from_utf8_lossy(&out.stderr);
    out.status
        .success()
        .then_some(())
        .ok_or_else(|| io::Error::other(said.trim()))
}
