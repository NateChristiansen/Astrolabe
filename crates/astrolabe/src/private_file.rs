//! A new file only its owner can read, before anything is written to it, wherever it is. Fails
//! if it exists. On Windows it starts with the folder's entries but unshared, so nobody can open
//! its contents, while permissions, which sharing doesn't cover, become one entry: this user. No
//! SYSTEM or Administrators: only the owner's processes read it, and they can take it over anyway.
use std::{fs::File, io, path::Path};

pub fn create(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::share_mode(&mut options, 0);
    let file = options.open(path)?;
    #[cfg(windows)]
    owner_only(path)?;
    Ok(file)
}

/// Read back, as a file system may not hold permissions.
#[cfg(windows)]
fn owner_only(path: &Path) -> io::Result<()> {
    let tool = |name: &str, file: Option<&Path>, args: &[&str]| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut command = std::process::Command::new(Path::new(&root).join("System32").join(name));
        std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
        let out = command.args(file).args(args).output()?;
        let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let failed = io::Error::other(format!("{name} said: {said}"));
        out.status.success().then(|| said.clone()).ok_or(failed)
    };
    let user = tool("whoami.exe", None, &["/user", "/fo", "csv", "/nh"])?;
    let sid = user.rsplit(',').next().unwrap_or_default();
    let only = format!("*{}:F", sid.trim_matches('"'));
    let on = Some(path);
    tool("icacls.exe", on, &["/inheritance:r", "/grant:r", &only])?;
    let text = tool("icacls.exe", on, &[])?;
    let one = text.matches(":(").count() == 1 && text.contains(":(F)");
    one.then_some(())
        .ok_or(io::Error::other(format!("its permissions read: {text}")))
}
