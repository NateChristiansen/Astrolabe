//! Git helpers for reading repo identity and syncing the owner's notes.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub(crate) fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(Command::new("git").args(args), cwd)
}

/// The same, with stdout as git wrote it: file names in `-z` output are bytes, and a lossy
/// decode could make two names one.
pub(crate) fn git_bytes(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    run_git_bytes(Command::new("git").args(args), cwd)
}

type Output = std::result::Result<String, String>;

/// A git call that may touch the network: stopped sooner than a local one, and when it
/// uses its whole allowance the failure says "took over", which callers take as offline.
pub(crate) fn git_network(cwd: &Path, args: &[&str]) -> Output {
    let limit = crate::deadline::git_limit(true);
    run_git_within(Command::new("git").args(args), cwd, &limit)
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git(command: &mut Command, cwd: &Path) -> Output {
    run_git_bytes(command, cwd).map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git_bytes(command: &mut Command, cwd: &Path) -> Result<Vec<u8>, String> {
    run_git_within(command, cwd, &crate::deadline::git_limit(false))
}

fn run_git_within(
    command: &mut Command,
    cwd: &Path,
    limit: &crate::deadline::Limit,
) -> Result<Vec<u8>, String> {
    let verb = verb(command);
    crate::deadline::stall(&verb);
    command.current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0");
    match run(command, limit.time) {
        Err(error) => Err(format!("git could not start: {error}")),
        Ok(None) => Err(limit.stopped(&verb)),
        Ok(Some(done)) if done.status.success() => Ok(done.stdout),
        Ok(Some(done)) => Err(String::from_utf8_lossy(&done.stderr).trim().to_owned()),
    }
}

/// The git command a call runs, past any `-c name=value` before it.
fn verb(command: &Command) -> String {
    let mut args = command.get_args().map(|arg| arg.to_string_lossy());
    while let Some(arg) = args.next() {
        if arg == "-c" {
            args.next();
        } else if !arg.starts_with('-') {
            return arg.into_owned();
        }
    }
    "command".to_owned()
}

/// After a child exits, how long its output may keep arriving before what's there is taken.
const DRAIN: Duration = Duration::from_millis(200);

struct Finished {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Runs a child to its end or to `limit`, whichever is first; None when it was stopped.
/// Both of its streams are read while it runs, so one that prints more than a pipe holds
/// isn't stalled by it and then taken for a hang. They're never waited on past the child:
/// a grandchild (a credential helper, a file watcher) can hold a pipe open long after.
fn run(command: &mut Command, limit: Duration) -> std::io::Result<Option<Finished>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let (closed, closing) = mpsc::channel();
    let stdout = drain(child.stdout.take(), closed.clone());
    let stderr = drain(child.stderr.take(), closed);
    let end = Instant::now() + limit;
    let mut open = 2;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        // A child closes its streams as it exits, so this wakes when it's done or nearly.
        match closing.recv_timeout(left.min(Duration::from_millis(50))) {
            Ok(()) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(left.min(Duration::from_millis(1)));
            }
        }
    };
    let drained = Instant::now() + DRAIN;
    while open > 0 {
        let left = drained.saturating_duration_since(Instant::now());
        match closing.recv_timeout(left) {
            Ok(()) => open -= 1,
            Err(_) => break,
        }
    }
    let take = |stream: &Mutex<Vec<u8>>| {
        std::mem::take(
            &mut *stream
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    };
    Ok(Some(Finished {
        status,
        stdout: take(&stdout),
        stderr: take(&stderr),
    }))
}

/// Reads a stream to its end on a thread of its own, into a buffer the caller can take
/// from at any time, and says when the stream closed.
fn drain(
    stream: Option<impl Read + Send + 'static>,
    closed: mpsc::Sender<()>,
) -> Arc<Mutex<Vec<u8>>> {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let into = buffer.clone();
    std::thread::spawn(move || {
        let mut chunk = vec![0; 1 << 16];
        if let Some(mut stream) = stream {
            while let Ok(read @ 1..) = stream.read(&mut chunk) {
                into.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .extend_from_slice(&chunk[..read]);
            }
        }
        let _ = closed.send(());
    });
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test program again, as a child that does what `ASTROLABE_TEST_CHILD` says: the
    /// one program every platform's test run is sure to have.
    fn child(role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "worktree::tests::a_child_plays_its_part"])
            .args(["--nocapture", "--test-threads", "1"])
            .env("ASTROLABE_TEST_CHILD", role);
        command
    }

    const FLOOD: usize = 1 << 20;

    #[test]
    fn a_child_plays_its_part() {
        use std::io::Write;
        match std::env::var("ASTROLABE_TEST_CHILD").as_deref() {
            Ok("flood") => {
                // Far more than a pipe holds, on both streams, before it exits.
                let line = [b'x'; 1 << 12];
                for _ in 0..FLOOD / line.len() {
                    std::io::stdout().write_all(&line).unwrap();
                    std::io::stderr().write_all(&line).unwrap();
                }
                std::process::exit(3);
            }
            Ok("hang") => std::thread::sleep(Duration::from_secs(120)),
            _ => {}
        }
    }

    #[test]
    fn output_past_a_pipe_buffer_is_read_whole() {
        let started = Instant::now();
        let done = run(&mut child("flood"), Duration::from_secs(60))
            .unwrap()
            .expect("it ends by itself");
        assert_eq!(done.status.code(), Some(3));
        let count = |bytes: &[u8]| bytes.iter().filter(|b| **b == b'x').count();
        assert!(count(&done.stdout) >= FLOOD, "{}", count(&done.stdout));
        assert!(count(&done.stderr) >= FLOOD, "{}", count(&done.stderr));
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn a_hanging_child_is_stopped_at_its_time() {
        let started = Instant::now();
        let done = run(&mut child("hang"), Duration::from_millis(500)).unwrap();
        assert!(done.is_none());
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(500) && took < Duration::from_secs(20),
            "{took:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_grandchild_holding_the_pipe_is_not_waited_for() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & echo said"]);
        let done = run(&mut command, Duration::from_secs(20)).unwrap().unwrap();
        assert!(done.status.success());
        assert_eq!(done.stdout, b"said\n");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_program_that_isnt_there_is_an_error_not_a_hang() {
        let mut command = Command::new("astrolabe-no-such-program");
        assert!(run(&mut command, Duration::from_secs(5)).is_err());
    }

    #[test]
    fn the_verb_is_found_past_configuration() {
        let mut command = Command::new("git");
        command.args(["-c", "user.useConfigOnly=false", "commit", "--quiet"]);
        assert_eq!(verb(&command), "commit");
        assert_eq!(
            verb(Command::new("git").args(["fetch", "--quiet"])),
            "fetch"
        );
    }
}
