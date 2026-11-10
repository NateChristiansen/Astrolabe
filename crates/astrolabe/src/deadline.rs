//! One clock per hook event. Both apps let a command run when its pre-command hook times out
//! or fails (docs/hosts.md), so a hook has to answer inside the time its manifest gives it,
//! whatever holds it up. Two layers: a backstop thread that answers and exits at the event's
//! answer-by time, for anything that blocks; and limits on the lock waits and git calls that
//! end sooner, so the hook normally has time left to say why a write was refused.
//!
//! A process that is not a hook (a terminal command, the detached sync, the board server)
//! starts no clock: it waits for a lock as long as it takes, and only its git calls are capped.
use serde_json::{Value, json};
use std::fs::File;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// When each event answers, from the hook's start. The manifest gives a pre-command or
/// session-start hook 20 s, a prompt, post-tool or stop hook 5 s, and a session-end hook 1 s
/// (plugin/astrolabe/hooks/hooks.json). The gap covers starting this program, which the
/// app's clock counts and this one can't, and exiting.
fn answer_by(event: &str) -> Duration {
    match event {
        "pre-command" | "session-start" => Duration::from_secs(16),
        "session-end" => Duration::from_millis(700),
        _ => Duration::from_millis(3500),
    }
}

/// Tests shorten an event's time, in milliseconds. It can only shorten.
const BUDGET_VAR: &str = "ASTROLABE_HOOK_BUDGET_MS";
/// Tests hold a hook up at a named point: `<point>:<milliseconds>`. The points are `read`
/// (input read, nothing judged), `verdict` (the guard's verdict reached, not yet printed) and
/// a git verb (before that git call starts).
const STALL_VAR: &str = "ASTROLABE_HOOK_TEST_STALL";

/// The most a fetch or push may take inside a hook: a pull is the one step a hook can do
/// without, so it gets the early part of the hook's time and no more.
const HOOK_NETWORK: Duration = Duration::from_secs(5);
/// A fetch or push given at least this long that still didn't finish counts as offline and
/// starts the backoff, though the hook had less than the whole allowance left for it.
const NETWORK_FLOOR: Duration = Duration::from_secs(3);
/// The same outside a hook, where nothing waits on it but a background sync.
const NETWORK: Duration = Duration::from_secs(15);
/// The most a local git call may take outside a hook, so one that hangs can't hold the
/// notes lock for good. Far above any real call: the slowest seen was 43 s.
const LOCAL: Duration = Duration::from_secs(120);

struct Clock {
    start: Instant,
    /// When the backstop answers and exits.
    answer: Duration,
}

impl Clock {
    /// Lock waits and git calls end here, so what follows them (the reply, its ticket)
    /// still fits before the backstop.
    fn work(&self) -> Duration {
        self.answer * 3 / 4
    }

    /// Kept after a lock wait for the write that follows it.
    fn reserve(&self) -> Duration {
        (self.work() / 4).min(Duration::from_secs(3))
    }

    /// A pull ends here, fetch and rebase both, so that with the worst case after it (a
    /// rebase stopped, then put back) the rest of the hook still has its time.
    fn pull(&self) -> Duration {
        self.work() / 2
    }

    /// The least a rebase is started with.
    fn rebase(&self) -> Duration {
        self.work() / 12
    }

    /// The least a notes write is started with.
    fn write(&self) -> Duration {
        self.work() / 8
    }

    /// What a call that puts the checkout back gets, whatever the hook has left: it runs
    /// after a step was stopped, which is when nothing is left.
    fn cleanup(&self) -> Duration {
        self.work() / 6
    }

    /// Git calls that only read, for a session's context, end here: later than the notes
    /// work, so a slow pull or write can't make a repo read as not a repo.
    fn read(&self) -> Duration {
        self.answer * 7 / 8
    }

    fn left(&self, until: Duration) -> Duration {
        until.saturating_sub(self.start.elapsed())
    }
}

static CLOCK: OnceLock<Clock> = OnceLock::new();

/// The one answer a hook gives. Whoever holds this prints or exits; nobody prints twice.
struct Answer {
    said: bool,
    /// Whether the guard reached a verdict, so `fallback` is that verdict.
    judged: bool,
    /// What the backstop prints if the hook hasn't answered: None is silence.
    fallback: Option<Value>,
    /// What an escaped panic prints before a verdict.
    failed: Option<Value>,
}

static ANSWER: Mutex<Answer> = Mutex::new(Answer {
    said: false,
    judged: false,
    fallback: None,
    failed: None,
});

fn answer() -> MutexGuard<'static, Answer> {
    ANSWER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const OUT_OF_TIME: &str = "Astrolabe's hook ran out of time before it finished with this command, so it blocks the command rather than let it run unguarded. If it was an `astrolabe` command, run `astrolabe notes status` to see whether its write landed before running it again; any other command can be run again as it is. If this keeps happening, something on this machine is holding the hook up: tell the owner, who can run `astrolabe off` in a terminal to stop Astrolabe on this machine until it's fixed.";

const FAILED: &str = "Astrolabe's hook failed before it finished with this command, so it blocks the command rather than let it run unguarded. If it was an `astrolabe` command, run `astrolabe notes status` to see whether its write landed before running it again; any other command can be run again as it is. If this keeps happening, the plugin is broken: tell the owner, who can run `astrolabe off` in a terminal to stop Astrolabe on this machine until it's fixed.";

pub(crate) const START_OUT_OF_TIME: &str = "Astrolabe's session-start hook ran out of time, so this session got none of its notes or effort context. The guard still runs on each command. Run `astrolabe activate --status` to see where this session stands, and tell the owner the hook was slow.";

/// How long this event's hook may spend reading its input.
pub(crate) fn read_time(event: &str) -> Duration {
    budget(event) / 4
}

fn budget(event: &str) -> Duration {
    let shorter = std::env::var(BUDGET_VAR)
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map(Duration::from_millis);
    let whole = answer_by(event);
    shorter.map_or(whole, |s| s.min(whole))
}

/// Starts the event's clock and its backstop. Called once, before the hook reads anything.
pub(crate) fn start(event: &str) {
    let answer_at = budget(event);
    let clock = Clock {
        start: Instant::now(),
        answer: answer_at,
    };
    if CLOCK.set(clock).is_err() {
        return;
    }
    {
        let mut state = answer();
        match event {
            // Nothing is judged yet, and an unjudged command must not run.
            "pre-command" => {
                state.fallback = Some(crate::hook::deny(OUT_OF_TIME));
                state.failed = Some(crate::hook::deny(FAILED));
            }
            // Silence here is a session with no notes and no idea why.
            "session-start" => {
                state.fallback = Some(json!({ "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": START_OUT_OF_TIME,
                }}));
            }
            // The other events have nothing to deny: late, they say nothing.
            _ => {}
        }
    }
    std::thread::spawn(move || {
        std::thread::sleep(answer_at);
        let mut state = answer();
        if !state.said
            && let Some(fallback) = state.fallback.take()
        {
            crate::hook::print(&fallback);
        }
        state.said = true;
        crate::worktree::stop_running();
        // Exits holding the answer, so the hook's own thread can't add a second one. A write
        // it was in the middle of is left as a crash leaves it: planned, for the next
        // writer to finish.
        std::process::exit(0);
    });
}

/// The hook's own answer, printed unless the backstop got there first.
pub(crate) fn say(print: impl FnOnce()) {
    let mut state = answer();
    if !state.said {
        state.said = true;
        print();
    }
}

/// The guard reached its verdict: from here a hook that runs out of time, or fails, gives
/// that verdict and not a blanket denial.
pub(crate) fn judged(verdict: Option<Value>) {
    let mut state = answer();
    state.judged = true;
    state.fallback = verdict;
}

/// A panic escaped the hook: a command not yet judged is denied, one already judged gets
/// its verdict, and any other event stays silent.
pub(crate) fn failed() {
    let mut state = answer();
    if state.said {
        return;
    }
    state.said = true;
    let last = if state.judged {
        state.fallback.take()
    } else {
        state.failed.take()
    };
    if let Some(last) = last {
        crate::hook::print(&last);
    }
}

/// The hook is returning: whatever it decided stands, and the backstop adds nothing.
pub(crate) fn done() {
    answer().said = true;
}

/// Holds a hook up at `point`, for tests.
pub(crate) fn stall(point: &str) {
    if CLOCK.get().is_none() {
        return;
    }
    let Ok(stall) = std::env::var(STALL_VAR) else {
        return;
    };
    if let Some((at, ms)) = stall.split_once(':')
        && at == point
        && let Ok(ms) = ms.parse()
    {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// How a lock wait ended.
pub(crate) enum Wait {
    Held,
    /// Another process held it for all the time this hook could wait.
    Busy(Duration),
}

/// Takes the exclusive lock on `file`. A hook waits a bounded time; any other process waits
/// as long as it takes, as the detached sync should behind a save.
pub(crate) fn lock(file: &File) -> std::io::Result<Wait> {
    use fs4::fs_std::FileExt;
    let Some(clock) = CLOCK.get() else {
        FileExt::lock_exclusive(file)?;
        return Ok(Wait::Held);
    };
    let until = clock.work().saturating_sub(clock.reserve());
    lock_within(file, clock.left(until))
}

/// The lock within `wait`. Neither `flock` nor `LockFileEx` as `fs4` calls it takes a
/// timeout, so this asks without waiting and asks again, the same on every platform. A
/// held lock is `Ok(false)` on all of them; any other failure is returned at once.
pub(crate) fn lock_within(file: &File, wait: Duration) -> std::io::Result<Wait> {
    use fs4::fs_std::FileExt;
    let started = Instant::now();
    let mut pause = Duration::from_millis(5);
    loop {
        if FileExt::try_lock_exclusive(file)? {
            return Ok(Wait::Held);
        }
        let waited = started.elapsed();
        if waited >= wait {
            return Ok(Wait::Busy(waited));
        }
        std::thread::sleep(pause.min(wait - waited));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// A step a hook does only when it has the time for it.
pub(crate) enum Step {
    /// Fetching the notes: skipped, the hook works from the local copy.
    Pull,
    /// Rebasing onto what was fetched: skipped, and done by the next pull.
    Rebase,
    /// Writing the notes: refused before anything is touched.
    Write,
}

/// How much time a hook has for `step` when that's too little to start it; None when it
/// has enough, or this isn't a hook.
pub(crate) fn short_of(step: Step) -> Option<Duration> {
    let clock = CLOCK.get()?;
    let (until, need) = match step {
        Step::Pull => (clock.pull(), clock.rebase() * 2),
        Step::Rebase => (clock.pull(), clock.rebase()),
        Step::Write => (clock.work(), clock.write()),
    };
    let left = clock.left(until);
    (left < need).then_some(left)
}

/// What a git call is for, which decides how long it may run.
#[derive(Clone, Copy)]
pub(crate) enum Git {
    /// A fetch or push.
    Network,
    /// Work in the notes checkout: a write's steps, a rebase.
    Notes,
    /// A read of some repo, for what a session is told.
    Read,
    /// Putting the checkout back after a step was stopped: `rebase --abort`, a reset.
    Cleanup,
}

/// How long one git call may run, and what to say if it's stopped.
pub(crate) struct Limit {
    pub(crate) time: Duration,
    cut: Cut,
}

enum Cut {
    /// A network call that used its whole allowance: offline, and worth backing off from.
    Network,
    /// Stopped because the hook's own time ran out.
    Hook,
    /// A local call outside a hook that ran absurdly long.
    Local,
}

impl Limit {
    /// Why git was stopped, for whoever reads the failure. Only the first says "took over",
    /// which is what starts the backoff.
    pub(crate) fn stopped(&self, verb: &str) -> String {
        let time = seconds(self.time);
        match self.cut {
            Cut::Network => format!("git {verb} took over {time} s"),
            Cut::Hook => {
                format!("git {verb} was stopped after {time} s, when this hook's time ran out")
            }
            Cut::Local => format!("git {verb} was stopped after {time} s"),
        }
    }
}

/// A failure that says the hook's time ran out, not that git found something wrong.
pub(crate) fn out_of_time(why: &str) -> bool {
    why.contains("when this hook's time ran out") || why.contains("this hook's time had run out")
}

/// The limit for a git call starting now, or None when a hook has no time left for one of
/// its kind: a call started with no time is a call killed at once, partway through
/// whatever it was changing.
pub(crate) fn git_limit(kind: Git) -> Option<Limit> {
    let Some(clock) = CLOCK.get() else {
        return Some(match kind {
            Git::Network => Limit {
                time: NETWORK,
                cut: Cut::Network,
            },
            _ => Limit {
                time: LOCAL,
                cut: Cut::Local,
            },
        });
    };
    in_a_hook(clock, kind, !cfg!(windows))
}

/// The limit inside a hook. `stops_cleanly` is whether a git call that has started can be
/// stopped without harm: on Unix it's asked first and removes its lock files; on Windows
/// it can only be ended outright, which leaves `index.lock`, and every later save is then
/// refused until someone removes the file by hand. So there a call that changes the
/// checkout is still not started without time for it, but once started it's left to
/// finish: the backstop answers on time, the process exits, git ends on its own, and the
/// write's intent covers what's left. A fetch or a read changes nothing and is ended.
fn in_a_hook(clock: &Clock, kind: Git, stops_cleanly: bool) -> Option<Limit> {
    let hook = |time: Duration| {
        (!time.is_zero()).then_some(Limit {
            time,
            cut: Cut::Hook,
        })
    };
    let changes = |time: Duration| match stops_cleanly {
        true => hook(time),
        false => (!time.is_zero()).then_some(Limit {
            time: LOCAL,
            cut: Cut::Local,
        }),
    };
    match kind {
        Git::Network => {
            // A rebase may follow a fetch, inside the pull's own time.
            let left = clock.left(clock.pull()).saturating_sub(clock.rebase());
            let time = left.min(HOOK_NETWORK);
            match time >= NETWORK_FLOOR {
                true => Some(Limit {
                    time,
                    cut: Cut::Network,
                }),
                false => hook(time),
            }
        }
        Git::Notes => changes(clock.left(clock.work())),
        Git::Read => hook(clock.left(clock.read())),
        Git::Cleanup => changes(clock.cleanup()),
    }
}

/// Whether a hook's time for reading repos has run out, so its git reads no longer start.
pub(crate) fn reading_time_is_up() -> bool {
    CLOCK
        .get()
        .is_some_and(|clock| clock.left(clock.read()).is_zero())
}

/// What a call not started says.
pub(crate) fn not_started(verb: &str) -> String {
    format!("git {verb} wasn't started, because this hook's time had run out")
}

/// Whole seconds from one up, tenths below, as a reply names a wait.
pub(crate) fn seconds(time: Duration) -> String {
    if time >= Duration::from_secs(1) {
        format!("{:.0}", time.as_secs_f32())
    } else {
        format!("{:.1}", time.as_secs_f32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_file(name: &str) -> (std::path::PathBuf, File) {
        let path = std::env::temp_dir().join(format!(
            "astrolabe-deadline-{name}-{}.lock",
            std::process::id()
        ));
        let open = || {
            File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .unwrap()
        };
        let file = open();
        (path, file)
    }

    #[test]
    fn every_event_answers_before_its_manifest_timeout() {
        let manifest: Value =
            serde_json::from_str(include_str!("../../../plugin/astrolabe/hooks/hooks.json"))
                .unwrap();
        let events = [
            ("SessionStart", "session-start"),
            ("UserPromptSubmit", "prompt"),
            ("PreToolUse", "pre-command"),
            ("PostToolUse", "post-tool"),
            ("Stop", "stop"),
            ("SessionEnd", "session-end"),
        ];
        for (name, event) in events {
            let hook = &manifest["hooks"][name][0]["hooks"][0];
            let command = hook["command"].as_str().unwrap();
            assert!(command.contains(&format!("hook {event} ")), "{command}");
            let timeout = Duration::from_secs(hook["timeout"].as_u64().unwrap());
            let answer = answer_by(event);
            assert!(answer <= timeout * 4 / 5, "{event}: {answer:?}");
            let clock = Clock {
                start: Instant::now(),
                answer,
            };
            assert!(clock.work() < answer && clock.reserve() < clock.work());
            assert!(read_time(event) < clock.work());
            assert!(clock.work() < clock.read() && clock.read() < answer);
            // The two events that pull. The worst pull: a rebase stopped at the pull's end,
            // asked and then killed, then put back with its own time, asked and then
            // killed. The write after it still has what a write needs.
            if timeout == Duration::from_secs(20) {
                let grace = Duration::from_millis(500);
                let worst = clock.pull() + grace + clock.cleanup() + grace;
                assert!(worst + clock.write() <= clock.work(), "{event}: {worst:?}");
            }
        }
    }

    #[test]
    fn where_git_cant_be_stopped_cleanly_a_started_change_is_left_to_finish() {
        let pre_command = |spent: u64| Clock {
            start: Instant::now() - Duration::from_secs(spent),
            answer: answer_by("pre-command"),
        };
        let time = |clock: &Clock, kind, cleanly| in_a_hook(clock, kind, cleanly).map(|l| l.time);
        let early = pre_command(1);
        // Unix: every call ends with the hook's time for it.
        assert!(time(&early, Git::Notes, true).unwrap() <= Duration::from_secs(11));
        assert_eq!(time(&early, Git::Cleanup, true), Some(early.cleanup()));
        // Windows: a call that changes the checkout runs to its own end once started.
        assert_eq!(time(&early, Git::Notes, false), Some(LOCAL));
        assert_eq!(time(&early, Git::Cleanup, false), Some(LOCAL));
        // A fetch and a read change nothing, so they're still ended.
        assert!(time(&early, Git::Network, false).unwrap() <= HOOK_NETWORK);
        assert!(time(&early, Git::Read, false).unwrap() <= Duration::from_secs(13));
        // And on both, past the mark for notes work, one isn't started at all.
        let late = pre_command(13);
        assert!(in_a_hook(&late, Git::Notes, true).is_none());
        assert!(in_a_hook(&late, Git::Notes, false).is_none());
        assert!(in_a_hook(&late, Git::Network, false).is_none());
        // Putting the checkout back always has its time.
        assert!(in_a_hook(&late, Git::Cleanup, true).is_some());
        assert!(in_a_hook(&late, Git::Cleanup, false).is_some());
    }

    #[test]
    fn a_lock_wait_ends_when_the_holder_lets_go() {
        use fs4::fs_std::FileExt;
        let (path, held) = lock_file("lets-go");
        FileExt::lock_exclusive(&held).unwrap();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        let waiter = File::options().write(true).open(&path).unwrap();
        let started = Instant::now();
        let wait = lock_within(&waiter, Duration::from_secs(20)).unwrap();
        assert!(matches!(wait, Wait::Held));
        assert!(started.elapsed() < Duration::from_secs(10));
        holder.join().unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_lock_wait_gives_up_at_its_time() {
        use fs4::fs_std::FileExt;
        let (path, held) = lock_file("gives-up");
        FileExt::lock_exclusive(&held).unwrap();
        let waiter = File::options().write(true).open(&path).unwrap();
        let started = Instant::now();
        let wait = lock_within(&waiter, Duration::from_millis(300)).unwrap();
        let Wait::Busy(waited) = wait else {
            panic!("the lock is held");
        };
        assert!(waited >= Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        // No time to wait is still one ask.
        assert!(matches!(
            lock_within(&waiter, Duration::ZERO).unwrap(),
            Wait::Busy(_)
        ));
        drop(held);
        assert!(matches!(
            lock_within(&waiter, Duration::ZERO).unwrap(),
            Wait::Held
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_wait_is_named_in_whole_seconds_or_tenths() {
        assert_eq!(seconds(Duration::from_millis(8000)), "8");
        assert_eq!(seconds(Duration::from_millis(2250)), "2");
        assert_eq!(seconds(Duration::from_millis(440)), "0.4");
    }

    #[test]
    fn outside_a_hook_only_git_is_capped() {
        // No test here starts a clock, so this is a process that isn't a hook.
        let network = git_limit(Git::Network).unwrap();
        assert_eq!(network.stopped("fetch"), "git fetch took over 15 s");
        for kind in [Git::Notes, Git::Read, Git::Cleanup] {
            let local = git_limit(kind).unwrap();
            assert_eq!(
                local.stopped("rebase"),
                "git rebase was stopped after 120 s"
            );
        }
        assert!(short_of(Step::Write).is_none());
    }
}
