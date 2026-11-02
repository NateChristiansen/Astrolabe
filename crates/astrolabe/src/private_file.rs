//! A new file only its owner can read, before anything is written to it, wherever it is.
use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
pub fn create(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600).open(path)
}

/// Fails if the file exists. It starts with the folder's entries but unshared, so nobody can
/// open its contents, while permissions, which sharing doesn't cover, become one entry: this
/// user. No SYSTEM or Administrators, as only the owner's processes read it and those accounts
/// can take a file over anyway. Read back, since a file system may not hold permissions.
#[cfg(windows)]
pub fn create(path: &Path) -> io::Result<File> {
    use std::ffi::OsStr;
    use std::os::windows::{fs::OpenOptionsExt, process::CommandExt};
    let tool = |name: &str, args: &[&OsStr]| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut command = std::process::Command::new(Path::new(&root).join("System32").join(name));
        let out = command.args(args).creation_flags(0x0800_0000).output()?;
        let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        match out.status.success() {
            true => Ok(said),
            false => Err(io::Error::other(format!("{name} said: {said}"))),
        }
    };
    let arg = |text| OsStr::new(text);
    let user = tool(
        "whoami.exe",
        &[arg("/user"), arg("/fo"), arg("csv"), arg("/nh")],
    )?;
    let user = user
        .rsplit(',')
        .next()
        .unwrap_or_default()
        .trim_matches('"');
    let mut options = std::fs::OpenOptions::new();
    let file = options
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path)?;
    let only = format!("*{user}:F");
    let set = [
        path.as_os_str(),
        arg("/inheritance:r"),
        arg("/grant:r"),
        arg(&only),
    ];
    let entries = tool("icacls.exe", &set).and_then(|_| tool("icacls.exe", &[path.as_os_str()]));
    match entries {
        Ok(text) if text.matches(":(").count() == 1 && text.contains(":(F)") => Ok(file),
        Ok(text) => Err(io::Error::other(format!("its permissions read: {text}"))),
        Err(why) => Err(why),
    }
}
