//! Git helpers for reading repo identity and syncing the owner's notes.
use crate::deadline::{Git, Run};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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

/// A git call of one kind, which decides whether it may be ended (see `deadline`): a
/// fetch or push, a call in the notes checkout, or a read.
pub(crate) fn git_as(kind: Git, cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(kind, Command::new("git").args(args), cwd)
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git(kind: Git, command: &mut Command, cwd: &Path) -> Result<Vec<u8>, String> {
    let verb = verb(command);
    crate::deadline::stall(&verb);
    let run = crate::deadline::git_run(kind);
    let limit = match &run {
        Run::Through => None,
        Run::Within(limit) => Some(limit.time),
        // A hook with no time left for this call doesn't start it.
        Run::No => return Err(crate::deadline::not_started(&verb)),
    };
    command.current_dir(cwd).env("GIT_TERMINAL_PROMPT", "0");
    let ran = match kind {
        Git::Notes => {
            // After the hook's answer, the backstop waits for a call in the notes checkout.
            let _under_way = crate::deadline::under_way();
            run_kept(command)
        }
        Git::Network | Git::Read => self::run(command, limit),
    };
    match ran {
        Err(error) => Err(format!("git could not start: {error}")),
        Ok(Ran::Stopped) => match run {
            Run::Within(limit) => Err(limit.stopped(&verb)),
            _ => Err(format!("git {verb} was stopped")),
        },
        Ok(Ran::CutOff) => Err(format!(
            "git {verb} ended but its output was cut off: something it started kept the output open"
        )),
        Ok(Ran::Finished(done)) if done.status.success() => Ok(done.stdout),
        Ok(Ran::Finished(done)) => Err(String::from_utf8_lossy(&done.stderr).trim().to_owned()),
    }
}

/// Runs a child that may outlive this process to its end, with its output in files
/// instead of pipes. A git that changes the notes checkout is never ended, and a hook
/// exits at its time whatever is still running. Pipes this process holds close when it
/// exits, and git's next line of output then ends it, partway through what it was
/// changing: a rebase prints as it finishes. Files stay, so git finishes; and with no pipe
/// to wait on, something git started can't hold this call open after git is done.
fn run_kept(command: &mut Command) -> std::io::Result<Ran> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let Some(dir) = kept_dir() else {
        return run(command, None);
    };
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = |stream: &str| dir.join(format!("{}-{n}.{stream}", std::process::id()));
    let (out, err) = (name("out"), name("err"));
    let (Ok(stdout), Ok(stderr)) = (std::fs::File::create(&out), std::fs::File::create(&err))
    else {
        return run(command, None);
    };
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    crate::notes::keep_stdio_here();
    let status = command.spawn()?.wait()?;
    // Read back whole, or not at all: what git printed is an answer to its caller.
    let (stdout, stderr) = (std::fs::read(&out)?, std::fs::read(&err)?);
    let _ = std::fs::remove_file(out);
    let _ = std::fs::remove_file(err);
    Ok(Ran::Finished(Finished {
        status,
        stdout,
        stderr,
    }))
}

/// Where a kept child's output goes: beside the plugin's state, or the temp folder. Files
/// a process left there by exiting before its git did are cleared once they're a day old.
fn kept_dir() -> Option<std::path::PathBuf> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = match crate::activate::state_dir() {
            Some(state) => state.join("git-output"),
            None => std::env::temp_dir().join("astrolabe-git-output"),
        };
        std::fs::create_dir_all(&dir).ok()?;
        let day = Duration::from_secs(24 * 60 * 60);
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|at| at.elapsed().ok())
                .is_some_and(|age| age > day);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Some(dir)
    })
    .clone()
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

/// The least a limited child's output gets to reach its end after the child exits, when
/// the call's own time is nearly spent: a reader thread may not have run yet.
const DRAIN: Duration = Duration::from_millis(200);

struct Finished {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum Ran {
    Finished(Finished),
    /// Still running at its limit, and ended.
    Stopped,
    /// It exited, but its output never reached its end inside the call's time: something
    /// it started held a stream open. What was read may be short, and short output can
    /// read as an answer (no unmerged files, no remote), so it's never returned.
    CutOff,
}

/// Runs a child that changes nothing (a fetch, a read) to its end, or to `limit` if it has
/// one. Both of its streams are read while it runs, so one that prints more than a pipe
/// holds isn't stalled by it and then taken for a hang. Its output counts only once both
/// streams reached their end. A child with no limit is waited for as long as it and its
/// output take, as `Command::output` would. One at its limit is ended as that would end
/// it: the child alone, at once.
fn run(command: &mut Command, limit: Option<Duration>) -> std::io::Result<Ran> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The hook's own stdout and stderr must not reach the child: on Windows every
    // inheritable handle passes to a new process, and a git that outlives the hook would
    // hold the app's pipe open, so the app never sees the end of the hook's answer.
    crate::notes::keep_stdio_here();
    let mut child = command.spawn()?;
    let (closed, closing) = mpsc::channel();
    let stdout = drain(child.stdout.take(), closed.clone());
    let stderr = drain(child.stderr.take(), closed);
    let end = limit.map(|limit| Instant::now() + limit);
    let left = |end: Option<Instant>| end.map(|end| end.saturating_duration_since(Instant::now()));
    let mut open = 2;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let left = left(end);
        if left.is_some_and(|left| left.is_zero()) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(Ran::Stopped);
        }
        let pause = |most: Duration| left.map_or(most, |left| left.min(most));
        // A child closes its streams as it exits, so this wakes when it's done or nearly.
        match closing.recv_timeout(pause(Duration::from_millis(50))) {
            Ok(()) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                std::thread::sleep(pause(Duration::from_millis(1)));
            }
        }
    };
    // The rest of the call's own time for the output to end: a grandchild holding a stream
    // costs that time, never a short answer.
    let end = end.map(|end| end.max(Instant::now() + DRAIN));
    while open > 0 {
        let ended = match left(end) {
            Some(left) => closing.recv_timeout(left).is_ok(),
            None => closing.recv().is_ok(),
        };
        if !ended {
            return Ok(Ran::CutOff);
        }
        open -= 1;
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
            Ok("slow") => {
                std::thread::sleep(Duration::from_millis(1200));
                println!("done at last");
            }
            _ => {}
        }
    }

    #[test]
    fn output_past_a_pipe_buffer_is_read_whole() {
        for limit in [Some(Duration::from_secs(60)), None] {
            let started = Instant::now();
            let Ran::Finished(done) = run(&mut child("flood"), limit).unwrap() else {
                panic!("it ends by itself");
            };
            assert_eq!(done.status.code(), Some(3));
            let count = |bytes: &[u8]| bytes.iter().filter(|b| **b == b'x').count();
            assert!(count(&done.stdout) >= FLOOD, "{}", count(&done.stdout));
            assert!(count(&done.stderr) >= FLOOD, "{}", count(&done.stderr));
            assert!(started.elapsed() < Duration::from_secs(30));
        }
    }

    #[test]
    fn a_child_at_its_limit_is_ended() {
        let started = Instant::now();
        let ran = run(&mut child("hang"), Some(Duration::from_millis(500))).unwrap();
        assert!(matches!(ran, Ran::Stopped));
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(500) && took < Duration::from_secs(20),
            "{took:?}"
        );
    }

    /// With no limit a child is never ended, however long it takes: this is how a hook runs
    /// every git call in the notes checkout.
    #[test]
    fn a_child_with_no_limit_runs_to_its_own_end() {
        let started = Instant::now();
        let Ran::Finished(done) = run(&mut child("slow"), None).unwrap() else {
            panic!("it's never ended");
        };
        assert!(done.status.success());
        assert!(String::from_utf8_lossy(&done.stdout).contains("done at last"));
        assert!(started.elapsed() >= Duration::from_millis(1200));
    }

    /// Output that never reached its end is an error, however much of it was read: short
    /// output can read as an answer. The call waits its own time for the end, no longer.
    #[cfg(unix)]
    #[test]
    fn output_a_grandchild_holds_open_is_cut_off_not_returned() {
        let dir =
            std::env::temp_dir().join(format!("astrolabe-runner-cut-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let started = Instant::now();
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 60 & echo $! > pid; echo said"])
            .current_dir(&dir);
        let ran = run(&mut command, Some(Duration::from_millis(1500))).unwrap();
        assert!(matches!(ran, Ran::CutOff));
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(1500) && took < Duration::from_secs(20),
            "{took:?}"
        );
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        let _ = Command::new("kill").arg(pid.trim()).status();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A grandchild that lets go of the streams doesn't hold the call at all.
    #[cfg(unix)]
    #[test]
    fn a_grandchild_that_lets_go_of_the_output_is_not_waited_for() {
        for limit in [Some(Duration::from_secs(60)), None] {
            let started = Instant::now();
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5 >/dev/null 2>&1 & echo said"]);
            let Ran::Finished(done) = run(&mut command, limit).unwrap() else {
                panic!("it ends by itself");
            };
            assert_eq!(done.stdout, b"said\n");
            assert!(started.elapsed() < Duration::from_secs(4));
        }
    }

    /// A child whose output is kept in files gives it back whole, however much, and isn't
    /// held by something it started that keeps the output open.
    #[test]
    fn a_kept_childs_output_is_read_back_whole() {
        let Ran::Finished(done) = run_kept(&mut child("flood")).unwrap() else {
            panic!("it's never ended");
        };
        assert_eq!(done.status.code(), Some(3));
        let count = |bytes: &[u8]| bytes.iter().filter(|b| **b == b'x').count();
        assert!(count(&done.stdout) >= FLOOD, "{}", count(&done.stdout));
        assert!(count(&done.stderr) >= FLOOD, "{}", count(&done.stderr));
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_child_isnt_held_by_what_it_started() {
        let started = Instant::now();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & echo said"]);
        let Ran::Finished(done) = run_kept(&mut command).unwrap() else {
            panic!("it's never ended");
        };
        assert_eq!(done.stdout, b"said\n");
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn a_program_that_isnt_there_is_an_error_not_a_hang() {
        let mut command = Command::new("astrolabe-no-such-program");
        assert!(run(&mut command, Some(Duration::from_secs(5))).is_err());
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
