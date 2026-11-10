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
    run_git(Git::Read, Command::new("git").args(args), cwd).map_err(|failed| failed.why)
}

type Output = std::result::Result<String, String>;

/// A git call of one kind, which decides whether it may be ended (see `deadline`): a
/// fetch or push, a call in the notes checkout, or a read.
pub(crate) fn git_as(kind: Git, cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    git_or(kind, cwd, args).map_err(|failed| failed.why)
}

/// How a git call failed: what it said, and the status it exited with when it ran to an
/// end. The status is the same in every language and whatever a commit is called, where
/// the message is neither.
pub(crate) struct Failed {
    pub(crate) why: String,
    pub(crate) code: Option<i32>,
}

/// The same as `git_as`, keeping the exit status of a call that failed.
pub(crate) fn git_or(kind: Git, cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<String, Failed> {
    run_git(kind, Command::new("git").args(args), cwd)
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run_git(kind: Git, command: &mut Command, cwd: &Path) -> Result<Vec<u8>, Failed> {
    let unrun = |why: String| Failed { why, code: None };
    let verb = verb(command);
    crate::deadline::stall(&verb);
    let run = crate::deadline::git_run(kind);
    let limit = match &run {
        Run::Through => None,
        Run::Within(limit) => Some(limit.time),
        // A hook with no time left for this call doesn't start it.
        Run::No => return Err(unrun(crate::deadline::not_started(&verb))),
    };
    // In git's own language, whatever the user's: the few places that read a message (no
    // upstream, a push refused for being behind) must read the same on every machine.
    command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    let ran = match kind {
        Git::Notes => {
            // After the hook's answer, the backstop waits for a call in the notes checkout.
            let _under_way = crate::deadline::under_way();
            run_kept(command)
        }
        Git::Network | Git::Read => self::run(command, limit),
    };
    match ran {
        Err(error) => Err(unrun(format!("git could not start: {error}"))),
        Ok(Ran::Stopped) => Err(unrun(match run {
            Run::Within(limit) => limit.stopped(&verb),
            _ => format!("git {verb} was stopped"),
        })),
        Ok(Ran::CutOff) => Err(unrun(format!(
            "git {verb} ended but its output was cut off: something it started kept the output open"
        ))),
        Ok(Ran::Finished(done)) if done.status.success() => Ok(done.stdout),
        Ok(Ran::Finished(done)) => Err(Failed {
            why: String::from_utf8_lossy(&done.stderr).trim().to_owned(),
            code: done.status.code(),
        }),
    }
}

/// Runs a child that may outlive this process to its end, with its output in files
/// instead of pipes. A git that changes the notes checkout is never ended, and a hook
/// exits at its time whatever is still running. Pipes this process holds close when it
/// exits, and git's next line of output then ends it, partway through what it was
/// changing: a rebase prints as it finishes. Files stay, so git finishes; and with no pipe
/// to wait on, something git started can't hold this call open after git is done.
fn run_kept(command: &mut Command) -> std::io::Result<Ran> {
    match kept_dir() {
        Some(dir) => run_kept_in(command, &dir, crate::deadline::in_hook()),
        None => run(command, None),
    }
}

/// `run_kept` with its folder named. With `marked`, the child also holds a marker there
/// until it ends (see `marker`), which is how a hook's git is known to be still running
/// after the hook is gone.
fn run_kept_in(command: &mut Command, dir: &Path, marked: bool) -> std::io::Result<Ran> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = |stream: &str| dir.join(format!("{}-{n}.{stream}", std::process::id()));
    let (out, err) = (name("out"), name("err"));
    let (Ok(stdout), Ok(stderr)) = (std::fs::File::create(&out), std::fs::File::create(&err))
    else {
        return run(command, None);
    };
    // The marker is the child's input: empty, so it reads as the end of input, as before.
    let running = name(MARKER);
    let stdin = match marked.then(|| marker(&running)).flatten() {
        Some(marker) => Stdio::from(marker),
        None => Stdio::null(),
    };
    command.stdin(stdin).stdout(stdout).stderr(stderr);
    crate::notes::keep_stdio_here();
    let status = command.spawn().and_then(|mut child| child.wait());
    // This process lets go of the marker, and it's removed: the child has ended, or never
    // started.
    command.stdin(Stdio::null());
    let _ = std::fs::remove_file(running);
    let status = status?;
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

/// What a marker file's name ends in.
const MARKER: &str = "run";

/// A file that reads as held for as long as the child given it as its input is running,
/// whether or not the process that started the child still is. A hook exits at its time
/// and leaves a git that changes the notes checkout to finish, and the notes lock goes
/// with the hook: the marker is what the next holder of that lock waits on, so it never
/// works in the checkout beside that git. No process is named, so there is no process id
/// to mistake for another's later. None if it couldn't be made, and the child runs
/// unmarked, as every child did before there were markers.
///
/// On Unix the file is locked here and the child has the lock with the file, since a lock
/// belongs to the open file and not to a process. It's let go when the last process with
/// that input ends: git, and anything git started that kept git's input. Git gives its
/// hooks no input of its own, and what it detaches (a background gc) closes its input
/// first, so those don't keep it.
#[cfg(unix)]
fn marker(path: &Path) -> Option<std::fs::File> {
    use fs4::fs_std::FileExt;
    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .ok()?;
    FileExt::try_lock_exclusive(&file).ok()?.then_some(file)
}

/// On Windows a lock belongs to the process that took it and would go with the hook, so
/// the file is opened unshared instead: nothing else can open it while any process has it
/// open, and the child has it open as its input until it ends.
#[cfg(windows)]
fn marker(path: &Path) -> Option<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(path)
        .ok()
}

/// Whether the child a marker was made for is still running. A marker left by a process
/// that exited before removing it, with its child ended too, reads as free.
#[cfg(unix)]
fn held(marker: &Path) -> bool {
    use fs4::fs_std::FileExt;
    std::fs::File::open(marker)
        .is_ok_and(|file| matches!(FileExt::try_lock_exclusive(&file), Ok(false)))
}

#[cfg(windows)]
fn held(marker: &Path) -> bool {
    // ERROR_SHARING_VIOLATION: another process has it open, unshared.
    std::fs::File::open(marker).is_err_and(|error| error.raw_os_error() == Some(32))
}

/// Whether git that a hook started in the notes checkout is still running, the hook
/// itself gone or not. `state` is the state folder of those notes, where hooks keep
/// their git's output and markers.
pub(crate) fn left_running(state: &Path) -> bool {
    any_held(&state.join(KEPT))
}

fn any_held(dir: &Path) -> bool {
    (std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .any(|path| path.extension().is_some_and(|x| x == MARKER) && held(&path))
}

/// Where a kept child's output goes: a folder of its own in the plugin's state folder,
/// which is the user's. With no state folder there's nowhere of the user's own to put it,
/// and the caller uses pipes: never a shared temp folder, where another user could have
/// put a link under the name this would write to. Files a process left by exiting before
/// its git did, its marker among them, are cleared once they're a day old.
fn kept_dir() -> Option<std::path::PathBuf> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| kept_dir_in(crate::activate::state_dir()))
        .clone()
}

/// The folder, in the state folder, for kept output and markers.
const KEPT: &str = "git-output";

fn kept_dir_in(state: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    let dir = state?.join(KEPT);
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

    /// A folder of this test's own for markers and kept output.
    fn kept(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("astrolabe-marker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Starts `command` with a marker as its input and lets go of it here, as a hook that
    /// exited would have: only the child, and what it starts, has the marker from then.
    fn left_by_a_hook(mut command: Command, dir: &Path) -> std::process::Child {
        let marker = marker(&dir.join(format!("1.{MARKER}"))).expect("a marker");
        command
            .stdin(Stdio::from(marker))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    }

    fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(started.elapsed() < Duration::from_secs(30), "never: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The marker is held for as long as the child runs and no longer, and is gone once
    /// the call returns.
    #[test]
    fn a_marked_childs_marker_is_held_while_it_runs_and_removed_after() {
        let dir = kept("runs");
        assert!(!any_held(&dir));
        std::thread::scope(|scope| {
            let call = scope.spawn(|| run_kept_in(&mut child("slow"), &dir, true));
            wait_until("the marker is held", || any_held(&dir));
            let Ran::Finished(done) = call.join().unwrap().unwrap() else {
                panic!("it's never ended");
            };
            assert!(String::from_utf8_lossy(&done.stdout).contains("done at last"));
        });
        assert!(!any_held(&dir));
        let left = std::fs::read_dir(&dir).unwrap().flatten();
        let markers = left.filter(|entry| entry.path().extension().is_some_and(|x| x == MARKER));
        assert_eq!(markers.count(), 0);
        // An unmarked child, as outside a hook, makes no marker at all.
        std::thread::scope(|scope| {
            let call = scope.spawn(|| run_kept_in(&mut child("slow"), &dir, false));
            std::thread::sleep(Duration::from_millis(400));
            assert!(!any_held(&dir));
            call.join().unwrap().unwrap();
        });
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The case the marker is for: whoever started the child is gone, and the marker is
    /// still held until the child ends. What's left then is a file nobody removed, which
    /// reads as free.
    #[test]
    fn a_marker_outlives_whoever_started_the_child_and_a_leftover_one_is_free() {
        let dir = kept("left");
        let mut running = left_by_a_hook(child("slow"), &dir);
        assert!(any_held(&dir), "held with only the child holding it");
        running.wait().unwrap();
        assert!(!any_held(&dir));
        assert!(dir.join(format!("1.{MARKER}")).exists(), "left behind");
        // One whose child never started, or that a crash left: free too.
        drop(marker(&dir.join(format!("2.{MARKER}"))).unwrap());
        std::fs::write(dir.join(format!("3.{MARKER}")), "").unwrap();
        assert!(!any_held(&dir));
        // A file that isn't a marker is never read as one.
        let other = std::fs::File::create(dir.join("4.out")).unwrap();
        assert!(!any_held(&dir));
        drop(other);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// What the child starts in the background doesn't keep the marker: a shell gives a
    /// background command no input of the shell's, as git gives its hooks none. Only
    /// something that takes the child's input on purpose and outlives it holds the marker
    /// on, which is why the wait for one is bounded (`deadline::wait_for_git`).
    #[cfg(unix)]
    #[test]
    fn only_what_keeps_the_childs_input_keeps_its_marker() {
        let dir = kept("background");
        let mut shell = Command::new("sh");
        shell
            .args(["-c", "sleep 3 & echo $! > pid"])
            .current_dir(&dir);
        left_by_a_hook(shell, &dir).wait().unwrap();
        assert!(!any_held(&dir), "a background command has no input of ours");
        let kill = |dir: &Path| {
            let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
            let _ = Command::new("kill").arg(pid.trim()).status();
        };
        kill(&dir);
        let mut shell = Command::new("sh");
        shell
            .args(["-c", "exec 3<&0; sleep 3 <&3 3<&- & echo $! > pid"])
            .current_dir(&dir);
        left_by_a_hook(shell, &dir).wait().unwrap();
        assert!(any_held(&dir), "kept on purpose, it's held");
        kill(&dir);
        wait_until("it ends, and the marker is free", || !any_held(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn with_no_state_folder_output_is_never_kept_in_a_shared_one() {
        assert_eq!(kept_dir_in(None), None);
        let state = std::env::temp_dir().join(format!("astrolabe-kept-{}", std::process::id()));
        assert_eq!(
            kept_dir_in(Some(state.clone())),
            Some(state.join("git-output"))
        );
        let _ = std::fs::remove_dir_all(state);
    }

    /// Git is run in its own language, so a message this program reads says the same
    /// whatever the user's.
    #[test]
    fn git_is_run_in_its_own_language() {
        let cwd = std::env::current_dir().unwrap();
        for kind in [Git::Read, Git::Notes, Git::Network] {
            let args = ["-c", "alias.env=!env", "env"].map(AsRef::as_ref);
            let env = git_as(kind, &cwd, &args).unwrap();
            assert!(env.lines().any(|line| line == "LC_ALL=C"), "{env}");
        }
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
