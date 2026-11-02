#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::fs::{self, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The core beside this program, in a group of its own so it outlives the window. An app
/// opened from Finder has a bare `PATH`, so it asks the owner's shell for the real one.
fn core_command() -> Result<Command, String> {
    let here = std::env::current_exe().map_err(|why| why.to_string())?;
    let name = format!("astrolabe{}", std::env::consts::EXE_SUFFIX);
    let mut command = Command::new(here.with_file_name(name));
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    #[cfg(unix)]
    {
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
        let asked = Command::new(shell)
            .args(["-lc", "printf %s \"$PATH\""])
            .output();
        if let Some(path) = asked.ok().filter(|asked| !asked.stdout.is_empty()) {
            command.env("PATH", String::from_utf8_lossy(&path.stdout).as_ref());
        }
    }
    Ok(command)
}

/// The lock says whether a core is serving. One is started if not, then its file is read.
#[tauri::command(async)]
fn core_connection() -> Result<(u16, String), String> {
    let text = |why: std::io::Error| why.to_string();
    let found = core_command()?.arg("where").output().map_err(text)?;
    if !found.status.success() {
        return Err(String::from_utf8_lossy(&found.stderr).into_owned());
    }
    let data = PathBuf::from(String::from_utf8_lossy(&found.stdout).trim());
    fs::create_dir_all(&data).map_err(text)?;
    let mut lock = OpenOptions::new();
    let lock = lock.create(true).truncate(false).write(true);
    let lock = lock.open(data.join("astrolabe.lock")).map_err(text)?;
    let mut started = false;
    for _ in 0..150 {
        if let Err(TryLockError::WouldBlock) = lock.try_lock() {
            let file = fs::read_to_string(data.join("bridge.txt")).unwrap_or_default();
            if let Some((port, secret)) = file.trim().split_once('\n')
                && let Ok(port) = port.parse()
            {
                return Ok((port, secret.to_owned()));
            }
        } else if !std::mem::replace(&mut started, true) {
            let _ = lock.unlock();
            let mut serve = core_command()?;
            let said =
                fs::File::create(data.join("core-start.log")).map_or(Stdio::null(), Stdio::from);
            let quiet = serve.arg("serve").stdin(Stdio::null());
            quiet
                .stdout(Stdio::null())
                .stderr(said)
                .spawn()
                .map_err(text)?;
        } else {
            let _ = lock.unlock();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let said = fs::read_to_string(data.join("core-start.log")).unwrap_or_default();
    Err(format!("the core did not start. {}", said.trim()))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![core_connection])
        .run(tauri::generate_context!())
        .expect("failed to run Astrolabe");
}
