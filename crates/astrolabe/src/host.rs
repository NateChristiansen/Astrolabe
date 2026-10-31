//! The core's two views of the outside world, injected so tests control them.
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64)
    }
}

pub trait ProcessProbe {
    fn is_alive(&self, pid: u32) -> bool;
}

/// Asks the OS tool for the process table, which needs no native bindings.
/// When the tool cannot run, the answer is "alive": wrongly interrupting a
/// working session is worse than leaving it for the stall watch to flag.
pub struct OsProcessProbe;

impl ProcessProbe for OsProcessProbe {
    fn is_alive(&self, pid: u32) -> bool {
        let mut command = if cfg!(windows) {
            let mut tasklist = Command::new("tasklist");
            tasklist.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
            tasklist
        } else {
            let mut ps = Command::new("ps");
            ps.args(["-o", "pid=", "-p", &pid.to_string()]);
            ps
        };
        let output = command.stdin(Stdio::null()).stderr(Stdio::null()).output();
        let Ok(output) = output else {
            return true;
        };
        // ps exits non-zero for "no such process"; tasklist only when it failed.
        if cfg!(windows) && !output.status.success() {
            return true;
        }
        // tasklist's "no tasks" message is localized and exits zero, so match the row.
        let wanted = if cfg!(windows) {
            format!("\"{pid}\"")
        } else {
            pid.to_string()
        };
        String::from_utf8_lossy(&output.stdout)
            .split(|c: char| c.is_whitespace() || c == ',')
            .any(|field| field == wanted)
    }
}
