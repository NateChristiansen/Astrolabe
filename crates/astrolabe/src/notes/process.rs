//! Detached background work and platform-specific process handles.
use super::{ASKED, Notes, write_text};
use std::fs::File;
use std::process::{Command, Stdio};

impl Notes {
    /// Runs `astrolabe notes sync` as its own process, so a hook or a save returns at once and
    /// the push survives the app closing. Returns whether the process was launched, not
    /// whether synchronization finished.
    pub fn spawn_sync(&self) -> bool {
        // Noted in a file of its own (its mtime), so a turn end doesn't ask again while this
        // one is still on its way, and nothing here rewrites the sync record unlocked.
        let _ = write_text(&self.state.join(ASKED), "");
        // Tests run syncs in order themselves; a marker says one was asked for.
        if std::env::var_os("ASTROLABE_NO_DETACHED_SYNC").is_some() {
            return write_text(&self.state.join("sync-requested"), "").is_ok();
        }
        self.spawn_detached(&["notes", "sync"], "sync.log")
    }

    /// Runs this program again with `args`, in the notes checkout, as its own process that
    /// outlives the caller, its output appended to `log` in the state folder. Returns whether
    /// the process was launched.
    pub fn spawn_detached(&self, args: &[&str], log: &str) -> bool {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let log = self.state.join(log);
        let out = File::options().create(true).append(true).open(&log).ok();
        let err = File::options().create(true).append(true).open(&log).ok();
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(out.map_or(Stdio::null(), Stdio::from))
            .stderr(err.map_or(Stdio::null(), Stdio::from));
        detach(&mut cmd);
        // The hook's own stdio must not reach the child: on Windows every inheritable handle
        // passes to a new process, and a child holding the hook's stdout pipe open means the
        // app never sees end-of-stream and refuses the hook's reply (seen in Claude Code on the
        // PC, 2026-11-03). The child gets its own files above.
        keep_stdio_here();
        cmd.spawn().is_ok()
    }
}

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    // A hidden console rather than none: a process with no console makes Windows open a
    // visible one for every console program it starts, so each git call the board server
    // or a sync made flashed a window. Children share this hidden one instead.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(windows)]
fn keep_stdio_here() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // Safety: GetStdHandle and SetHandleInformation take no pointers we own; an invalid
        // handle makes SetHandleInformation fail, which is ignored.
        unsafe {
            let handle = GetStdHandle(which);
            if !handle.is_null() {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
fn keep_stdio_here() {}
