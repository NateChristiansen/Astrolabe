//! Git helpers for reading repo identity and syncing the owner's notes.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::deadline::Git;

/// A read of the repo at `cwd`, for what a session is told.
pub(crate) fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    git_as(Git::Read, cwd, args)
}

/// The same, with stdout as git wrote it: file names in `-z` output are bytes, and a lossy
/// decode could make two names one.
pub(crate) fn git_bytes(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    run_git(Git::Read, Command::new("git").args(args), cwd)
}

type Output = std::result::Result<String, String>;

/// A git call of one kind, which decides how long it may run (see `deadline`): a fetch or
/// push, work in the notes checkout, a read, or putting a checkout back.
pub(crate) fn git_as(kind: Git, cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(kind, Command::new("git").args(args), cwd)
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git(kind: Git, command: &mut Command, cwd: &Path) -> Result<Vec<u8>, String> {
    let verb = verb(command);
    // A hook with no time left for this call doesn't start it: started, it would be
    // stopped at once, partway through whatever it changes.
    let Some(limit) = crate::deadline::git_limit(kind) else {
        return Err(crate::deadline::not_started(&verb));
    };
    crate::deadline::stall(&verb);
    command.current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0");
    match run(command, limit.time) {
        Err(error) => Err(format!("git could not start: {error}")),
        Ok(Ran::Stopped) => Err(limit.stopped(&verb)),
        Ok(Ran::CutOff) => Err(format!(
            "git {verb} ended but its output was cut off: something it started kept the output open for the {} s the call had",
            crate::deadline::seconds(limit.time)
        )),
        Ok(Ran::Finished(done)) if done.status.success() => Ok(done.stdout),
        Ok(Ran::Finished(done)) => Err(String::from_utf8_lossy(&done.stderr).trim().to_owned()),
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

/// The least a child's output gets to reach its end after the child exits, when the call's
/// own time is nearly spent: a reader thread may not have run yet.
const DRAIN: Duration = Duration::from_millis(200);
/// How long a child asked to stop gets to clean up before it's killed. Git removes its lock
/// files in that time; killed outright it leaves `index.lock`, and every later write to
/// the checkout fails until someone removes it by hand.
const STOP_GRACE: Duration = Duration::from_millis(500);

struct Finished {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum Ran {
    Finished(Finished),
    /// Still running at its limit, and stopped.
    Stopped,
    /// It exited, but its output never reached its end: something it started held a
    /// stream open. What was read may be short, and short output can read as an answer
    /// (no unmerged files, no remote), so it's never returned.
    CutOff,
}

/// Runs a child to its end or to `limit`, whichever is first. Both of its streams are read
/// while it runs, so one that prints more than a pipe holds isn't stalled by it and then
/// taken for a hang. Its output counts only once both streams reached their end.
fn run(command: &mut Command, limit: Duration) -> std::io::Result<Ran> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    own_group(command);
    let mut child = command.spawn()?;
    let _running = Running::new(&child);
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
            stop(&mut child);
            return Ok(Ran::Stopped);
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
    // The rest of the call's own time for the output to end. A grandchild holding a stream
    // costs that time, never a short answer.
    let end = end.max(Instant::now() + DRAIN);
    while open > 0 {
        let left = end.saturating_duration_since(Instant::now());
        match closing.recv_timeout(left) {
            Ok(()) => open -= 1,
            Err(_) => {
                stop(&mut child);
                return Ok(Ran::CutOff);
            }
        }
    }
    let take = |stream: &Mutex<Vec<u8>>| {
        std::mem::take(
            &mut *stream
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    };
    Ok(Ran::Finished(Finished {
        status,
        stdout: take(&stdout),
        stderr: take(&stderr),
    }))
}

/// The child this process is waiting on, so a hook whose time is up can take it along
/// instead of leaving it to work on a checkout nobody holds the lock for.
static RUNNING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

struct Running;

impl Running {
    fn new(child: &std::process::Child) -> Self {
        RUNNING.store(child.id(), std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Stops the git call under way, if any, as the process exits: asked, then killed.
#[cfg(unix)]
pub(crate) fn stop_running() {
    let id = RUNNING.load(std::sync::atomic::Ordering::SeqCst);
    if id == 0 {
        return;
    }
    let group = -(id as libc::pid_t);
    // Safety: `kill` takes no pointers; the group is one this process made for its child.
    unsafe { libc::kill(group, libc::SIGTERM) };
    std::thread::sleep(Duration::from_millis(100));
    // Safety: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
}

/// Windows can't reach a child by its id from here; it's left running.
#[cfg(not(unix))]
pub(crate) fn stop_running() {
    let _ = RUNNING.load(std::sync::atomic::Ordering::SeqCst);
}

/// A process group of the child's own, so stopping it reaches what it started (a commit
/// hook, a credential helper), which could otherwise touch the checkout after the notes
/// lock is released.
#[cfg(unix)]
fn own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn own_group(_: &mut Command) {}

/// Stops the child and what it started: asked first, so git removes its lock files, then
/// killed. Also clears a group whose leader already exited.
#[cfg(unix)]
fn stop(child: &mut std::process::Child) {
    let group = -(child.id() as libc::pid_t);
    // Safety: `kill` takes no pointers. The group is the child's own, made at spawn; if
    // nothing is left in it the call fails with no effect.
    unsafe { libc::kill(group, libc::SIGTERM) };
    let asked = Instant::now();
    while asked.elapsed() < STOP_GRACE && matches!(child.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // Safety: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
    let _ = child.kill();
    let _ = child.wait();
}

/// Windows has no signal a console program cleans up on, and no process group to stop:
/// only the child itself is ended, at once. What it started lives on, and a stopped
/// `git commit` can leave `index.lock` behind.
#[cfg(not(unix))]
fn stop(child: &mut std::process::Child) {
    let _ = STOP_GRACE;
    let _ = child.kill();
    let _ = child.wait();
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
        let Ran::Finished(done) = run(&mut child("flood"), Duration::from_secs(60)).unwrap() else {
            panic!("it ends by itself");
        };
        assert_eq!(done.status.code(), Some(3));
        let count = |bytes: &[u8]| bytes.iter().filter(|b| **b == b'x').count();
        assert!(count(&done.stdout) >= FLOOD, "{}", count(&done.stdout));
        assert!(count(&done.stderr) >= FLOOD, "{}", count(&done.stderr));
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[test]
    fn a_hanging_child_is_stopped_at_its_time() {
        let started = Instant::now();
        let ran = run(&mut child("hang"), Duration::from_millis(500)).unwrap();
        assert!(matches!(ran, Ran::Stopped));
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(500) && took < Duration::from_secs(20),
            "{took:?}"
        );
    }

    #[cfg(unix)]
    fn alive(pid: &str) -> bool {
        let status = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status();
        status.unwrap().success()
    }

    #[cfg(unix)]
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("astrolabe-runner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Output that never reached its end is an error, however much of it was read: short
    /// output can read as an answer. The call waits its own time for the end, no longer,
    /// and takes the grandchild holding the stream with it.
    #[cfg(unix)]
    #[test]
    fn output_a_grandchild_holds_open_is_cut_off_not_returned() {
        let dir = scratch("cut-off");
        let started = Instant::now();
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 60 & echo $! > pid; echo said"])
            .current_dir(&dir);
        let ran = run(&mut command, Duration::from_millis(1500)).unwrap();
        assert!(matches!(ran, Ran::CutOff));
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(1500) && took < Duration::from_secs(20),
            "{took:?}"
        );
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        assert!(!alive(&pid), "the grandchild outlived the call");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A grandchild that lets go of the streams doesn't hold the call at all.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_that_lets_go_of_the_output_is_not_waited_for() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 >/dev/null 2>&1 & echo said"]);
        let Ran::Finished(done) = run(&mut command, Duration::from_secs(60)).unwrap() else {
            panic!("it ends by itself");
        };
        assert_eq!(done.stdout, b"said\n");
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[cfg(unix)]
    #[test]
    fn a_stopped_child_is_asked_first_and_what_it_started_stops_too() {
        let dir = scratch("stopped");
        let mut command = Command::new("sh");
        // Cleans up when asked to stop, as git does with its lock files.
        let script = "trap 'rm -f lock; exit 0' TERM; touch lock; sleep 60 & echo $! > pid; wait";
        command.args(["-c", script]).current_dir(&dir);
        let ran = run(&mut command, Duration::from_millis(500)).unwrap();
        assert!(matches!(ran, Ran::Stopped));
        assert!(
            !dir.join("lock").exists(),
            "it was killed before it cleaned up"
        );
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        assert!(!alive(&pid), "what it started outlived it");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One that ignores the ask is killed once its grace is up.
    #[cfg(unix)]
    #[test]
    fn a_child_that_ignores_the_ask_is_killed() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "trap '' TERM; sleep 60; sleep 60"]);
        let ran = run(&mut command, Duration::from_millis(300)).unwrap();
        assert!(matches!(ran, Ran::Stopped));
        assert!(started.elapsed() < Duration::from_secs(20));
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
