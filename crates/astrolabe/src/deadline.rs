//! One clock per hook event. Both apps let a command run when its pre-command hook times out
//! or fails (docs/hosts.md), so a hook has to answer inside the time its manifest gives it,
//! whatever holds it up. A backstop thread answers at the event's answer-by time, for
//! anything that blocks. Before that, lock waits are bounded, and a hook does a step (a
//! pull, a rebase, a notes write) only when it has the time for it.
//!
//! A hook never ends a git call that changes the notes checkout: stopped partway, a
//! rebase or a commit leaves the checkout in a state the next write has to be protected
//! from. It decides before starting one whether there's time, and once started lets it
//! finish. Fetches and reads change nothing and are ended at their limit.
//!
//! A process that is not a hook (a terminal command, the detached sync, the board server)
//! starts no clock: it waits for a lock as long as it takes, and only its network git
//! calls are limited.
use serde_json::{Value, json};
use std::fs::File;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
/// Tests hold a hook up at a named point: `<point>:<milliseconds>` waits that long there,
/// and `<point>:+<milliseconds>` moves the hook's clock on by that much without waiting,
/// so a test can put a hook late in its time with the backstop still far off. The points
/// are `read` (input read, nothing judged), `verdict` (the guard's verdict reached, not
/// yet printed) and a git verb (before that git call starts).
const STALL_VAR: &str = "ASTROLABE_HOOK_TEST_STALL";

/// The most a fetch or push may take inside a hook: a pull is the one step a hook can do
/// without, so it gets the early part of the hook's time and no more. Cut off here it
/// proves nothing about the network, only that the hook couldn't wait: it doesn't count as
/// offline, and starts no backoff.
const HOOK_NETWORK: Duration = Duration::from_secs(5);
/// The same outside a hook, where nothing waits on it but a background sync. One that
/// uses all of this counts as offline, and later calls back off for a while.
const NETWORK: Duration = Duration::from_secs(15);

struct Clock {
    start: Instant,
    /// When the backstop answers.
    answer: Duration,
    /// Time a test has moved the clock on by, in milliseconds.
    moved_on: AtomicU64,
}

impl Clock {
    fn new(answer: Duration) -> Self {
        Self {
            start: Instant::now(),
            answer,
            moved_on: AtomicU64::new(0),
        }
    }

    /// A notes write starts before here, and lock waits end sooner still, so what follows
    /// (the write, the reply, its ticket) fits before the backstop.
    fn work(&self) -> Duration {
        self.answer * 3 / 4
    }

    /// Kept after a lock wait for the write that follows it.
    fn reserve(&self) -> Duration {
        (self.work() / 4).min(Duration::from_secs(3))
    }

    /// A pull is the one step a hook can do without, so it has the early part of the
    /// hook's time: its fetch ends by here, and its rebase starts before here or not at all.
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

    /// Git calls that only read a repo, for a session's context, end here: later than
    /// the notes work, so a slow pull or write can't make a repo read as not a repo.
    fn read(&self) -> Duration {
        self.answer * 7 / 8
    }

    /// The hook exits by here whatever is still running: after the answer, and just
    /// inside the app's timeout, because an app that gives up on a hook discards what the
    /// hook printed.
    fn exit(&self) -> Duration {
        self.answer + self.answer * 5 / 32
    }

    fn spent(&self) -> Duration {
        self.start.elapsed() + Duration::from_millis(self.moved_on.load(Ordering::Relaxed))
    }

    fn left(&self, until: Duration) -> Duration {
        until.saturating_sub(self.spent())
    }
}

static CLOCK: OnceLock<Clock> = OnceLock::new();

/// How many things are under way that the backstop waits for after it answers: a git call
/// in the notes checkout, a pull.
static UNDER_WAY: AtomicUsize = AtomicUsize::new(0);
/// A notes write passed its gate in this hook: it runs to its end.
static WRITING: AtomicBool = AtomicBool::new(false);
/// The hook's own thread is finished.
static DONE: AtomicBool = AtomicBool::new(false);
/// How long this hook's pull of the notes took, in milliseconds; 0 when it made none.
static PULL_MS: AtomicU64 = AtomicU64::new(0);

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
    let clock = Clock::new(answer_at);
    let exit_after = clock.exit() - answer_at;
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
        {
            // The answer is on time whatever git is doing. From here the hook's own thread
            // prints nothing: `said` is set, so there is one answer.
            let mut state = answer();
            if !state.said
                && let Some(fallback) = state.fallback.take()
            {
                crate::hook::print(&fallback);
            }
            state.said = true;
        }
        // Then a notes write or a pull still under way gets what's left before the app's
        // own timeout to finish, since ending it partway is what leaves a checkout
        // needing repair. Past that the hook exits anyway and git is left running, with
        // the write's intent for the next writer: the same as when the app ends a hook.
        let exit_at = Instant::now() + exit_after;
        while !DONE.load(Ordering::SeqCst)
            && Instant::now() < exit_at
            && (WRITING.load(Ordering::SeqCst) || UNDER_WAY.load(Ordering::SeqCst) > 0)
        {
            std::thread::sleep(Duration::from_millis(10));
        }
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
    DONE.store(true, Ordering::SeqCst);
}

/// Held while something is under way that the backstop waits for after it answers.
pub(crate) struct UnderWay;

pub(crate) fn under_way() -> UnderWay {
    UNDER_WAY.fetch_add(1, Ordering::SeqCst);
    UnderWay
}

impl Drop for UnderWay {
    fn drop(&mut self) {
        UNDER_WAY.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A notes write passed its gate: from here it isn't refused for time, and the backstop
/// waits for it after answering.
pub(crate) fn writing() {
    WRITING.store(true, Ordering::SeqCst);
}

/// This hook's pull of the notes took `time`, for a write refused after it to say so.
pub(crate) fn pulled(time: Duration) {
    PULL_MS.store(time.as_millis() as u64, Ordering::SeqCst);
}

/// Holds a hook up at `point`, or moves its clock on, for tests.
pub(crate) fn stall(point: &str) {
    let Some(clock) = CLOCK.get() else {
        return;
    };
    let Ok(stall) = std::env::var(STALL_VAR) else {
        return;
    };
    let Some((at, time)) = stall.split_once(':') else {
        return;
    };
    if at != point {
        return;
    }
    match time.strip_prefix('+') {
        Some(ms) => {
            clock
                .moved_on
                .fetch_add(ms.parse().unwrap_or(0), Ordering::Relaxed);
        }
        None => std::thread::sleep(Duration::from_millis(time.parse().unwrap_or(0))),
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

/// A step a hook does only when it has the time for it. Each is decided before the step
/// starts and never again: a step that has started runs to its end.
pub(crate) enum Step {
    /// Fetching the notes: skipped, the hook works from the local copy.
    Pull,
    /// Rebasing onto what was fetched: skipped, and done by the next pull.
    Rebase,
    /// Writing the notes: refused before anything is planned or touched.
    Write,
}

/// How much time a hook has for `step` when that's too little to start it; None when it
/// has enough, or this isn't a hook.
pub(crate) fn short_of(step: Step) -> Option<Duration> {
    short_of_on(CLOCK.get()?, step)
}

fn short_of_on(clock: &Clock, step: Step) -> Option<Duration> {
    let (until, need) = match step {
        Step::Pull => (clock.pull(), clock.rebase() * 2),
        Step::Rebase => (clock.pull(), clock.rebase()),
        Step::Write => (clock.work(), clock.write()),
    };
    let left = clock.left(until);
    (left < need).then_some(left)
}

/// Why a notes write isn't started, when this hook has too little time left for one. It
/// names the pull when the hook made one, since that is where the time went.
pub(crate) fn no_time_to_write() -> Option<String> {
    let left = seconds(short_of(Step::Write)?);
    let pull = Duration::from_millis(PULL_MS.load(Ordering::SeqCst));
    Some(match pull.is_zero() {
        true => format!(
            "this hook had {left} s left, too little to write the notes safely, so nothing was written. Run the same command again"
        ),
        false => format!(
            "this hook's pull of the notes took {} s and left {left} s, too little to write them safely, so nothing was written. Run the same command again",
            seconds(pull)
        ),
    })
}

/// What a git call is for, which decides whether it may be ended.
#[derive(Clone, Copy)]
pub(crate) enum Git {
    /// A fetch or push: ended at its limit.
    Network,
    /// A call in the notes checkout, as part of a pull or a write: never ended. Whether
    /// there's time for it was decided before its step began.
    Notes,
    /// A read of some repo, for what a session is told: ended at its limit in a hook.
    Read,
}

/// How long one git call may run, and what to say if it's stopped.
pub(crate) struct Limit {
    pub(crate) time: Duration,
    cut: Cut,
}

enum Cut {
    /// A network call that used its whole allowance outside a hook: offline, and worth
    /// backing off from.
    Network,
    /// Stopped because the hook's own time ran out, which says nothing about the network.
    Hook,
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
        }
    }
}

/// Whether a git call starting now may run, and for how long.
pub(crate) enum Run {
    /// To its own end.
    Through,
    /// Ended at this limit.
    Within(Limit),
    /// Not at all: a hook has no time left for a call of its kind.
    No,
}

pub(crate) fn git_run(kind: Git) -> Run {
    match CLOCK.get() {
        Some(clock) => git_run_on(clock, kind),
        None => match kind {
            Git::Network => Run::Within(Limit {
                time: NETWORK,
                cut: Cut::Network,
            }),
            Git::Notes | Git::Read => Run::Through,
        },
    }
}

fn git_run_on(clock: &Clock, kind: Git) -> Run {
    let within = |time: Duration, cut: Cut| match time.is_zero() {
        true => Run::No,
        false => Run::Within(Limit { time, cut }),
    };
    match kind {
        Git::Network => {
            // A rebase may follow a fetch, inside the pull's own time.
            let left = clock.left(clock.pull()).saturating_sub(clock.rebase());
            within(left.min(HOOK_NETWORK), Cut::Hook)
        }
        Git::Notes => Run::Through,
        Git::Read => within(clock.left(clock.read()), Cut::Hook),
    }
}

/// Whether a hook's time for reading repos has run out, so its git reads no longer start.
pub(crate) fn reading_time_is_up() -> bool {
    CLOCK
        .get()
        .is_some_and(|clock| clock.left(clock.read()).is_zero())
}

/// A failure that says a hook's time ran out, not that git found something wrong.
pub(crate) fn out_of_time(why: &str) -> bool {
    why.ends_with("when this hook's time ran out")
        || why.ends_with("because this hook's time had run out")
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
            let clock = Clock::new(answer);
            assert!(clock.work() < answer && clock.reserve() < clock.work());
            assert!(read_time(event) < clock.work());
            assert!(clock.work() < clock.read() && clock.read() < answer);
            // The hook is gone, whatever git is doing, with room before the app gives up.
            assert!(answer < clock.exit(), "{event}");
            assert!(
                clock.exit() <= timeout * 19 / 20,
                "{event}: {:?}",
                clock.exit()
            );
            // A pull that ends on time leaves a write its time.
            assert!(clock.pull() + clock.write() < clock.work());
        }
    }

    /// A pre-command hook's clock with `spent` seconds gone.
    fn pre_command(spent: u64) -> Clock {
        let clock = Clock::new(answer_by("pre-command"));
        clock.moved_on.store(spent * 1000, Ordering::Relaxed);
        clock
    }

    #[test]
    fn a_hook_never_ends_git_in_the_notes_checkout() {
        for spent in [0, 5, 11, 13, 15, 30] {
            assert!(matches!(
                git_run_on(&pre_command(spent), Git::Notes),
                Run::Through
            ));
        }
    }

    #[test]
    fn a_fetch_and_a_read_are_ended_and_not_started_late() {
        let time = |spent, kind| match git_run_on(&pre_command(spent), kind) {
            Run::Within(limit) => Some(limit.time),
            Run::No => None,
            Run::Through => panic!("never ended"),
        };
        // A fetch has the early part of the hook and no more.
        let whole = time(0, Git::Network).unwrap();
        assert!(whole <= HOOK_NETWORK && whole > HOOK_NETWORK - Duration::from_millis(100));
        assert!(time(3, Git::Network).unwrap() <= Duration::from_secs(2));
        assert_eq!(time(5, Git::Network), None);
        // A read runs until late, then isn't started.
        assert!(time(1, Git::Read).unwrap() <= Duration::from_secs(13));
        assert!(time(13, Git::Read).is_some());
        assert_eq!(time(14, Git::Read), None);
        // A fetch a hook cut short never says "took over", which is what starts the
        // backoff every process on the machine then obeys: only the whole 15 s outside a
        // hook says the network is down.
        let said = |spent: u64| match git_run_on(&pre_command(spent), Git::Network) {
            Run::Within(limit) => limit.stopped("fetch"),
            _ => panic!("started"),
        };
        assert_eq!(
            said(0),
            "git fetch was stopped after 5 s, when this hook's time ran out"
        );
        assert_eq!(
            said(3),
            "git fetch was stopped after 2 s, when this hook's time ran out"
        );
    }

    #[test]
    fn steps_are_gated_before_they_start() {
        let short = |spent, step| short_of_on(&pre_command(spent), step).is_some();
        // A pull starts only early, and its rebase only inside the pull's time.
        assert!(!short(0, Step::Pull) && !short(3, Step::Pull));
        assert!(short(5, Step::Pull));
        assert!(!short(4, Step::Rebase));
        assert!(short(6, Step::Rebase));
        // A write starts with time to finish before the hook's work mark.
        assert!(!short(10, Step::Write));
        assert!(short(11, Step::Write));
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
        // Free now, give or take a child another test is starting at this moment, which
        // holds a copy of every open file until it becomes its own program.
        assert!(matches!(
            lock_within(&waiter, Duration::from_secs(5)).unwrap(),
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
    fn outside_a_hook_only_network_git_is_limited() {
        // No test here starts a clock, so this is a process that isn't a hook.
        let Run::Within(network) = git_run(Git::Network) else {
            panic!("a fetch is always limited");
        };
        assert_eq!(network.stopped("fetch"), "git fetch took over 15 s");
        assert!(matches!(git_run(Git::Notes), Run::Through));
        assert!(matches!(git_run(Git::Read), Run::Through));
        assert!(short_of(Step::Write).is_none());
        assert!(no_time_to_write().is_none());
    }
}
