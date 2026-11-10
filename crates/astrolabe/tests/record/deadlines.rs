//! Notes writes through a hook: refused with a reply before anything is written when the
//! hook has no time for them, and never ended once begun.
use super::*;
use std::time::{Duration, Instant};

const SAVE: &str = "astrolabe effort save --journal \"a line for the day\"";
const OUT_OF_TIME: &str = "Astrolabe's hook ran out of time before it finished with this command";

/// What one hook run came to.
enum Answer {
    /// Nothing printed.
    Silent,
    Denied(String),
    /// The text of the ticket the rewritten command reads, and whether it worked.
    Shown(String, bool),
}

fn answer_in(out: &str) -> Answer {
    if out.trim().is_empty() {
        return Answer::Silent;
    }
    let answer: Value = serde_json::from_str(out.trim()).unwrap();
    let inner = &answer["hookSpecificOutput"];
    if inner["permissionDecision"] == "deny" {
        let why = inner["permissionDecisionReason"].as_str().unwrap();
        return Answer::Denied(why.to_owned());
    }
    let shown = inner["updatedInput"]["command"].as_str().unwrap();
    let path = (shown.strip_prefix("cat '"))
        .or_else(|| shown.strip_prefix("Get-Content -Raw -Encoding UTF8 -LiteralPath '"))
        .and_then(|p| p.strip_suffix('\''))
        .unwrap_or_else(|| panic!("{shown}"));
    let ticket: Value =
        serde_json::from_str(&std::fs::read_to_string(path.replace(".txt", ".json")).unwrap())
            .unwrap();
    Answer::Shown(std::fs::read_to_string(path).unwrap(), ticket["ok"] == true)
}

fn save_input(mac: &Machine, session: &str, call: &str, command: &str) -> Value {
    json!({"session_id": session, "cwd": mac.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}})
}

impl Machine {
    /// A Bash command through the pre-command hook with extra environment: its answer and
    /// how long the hook took. A hook still running at `limit` fails the test.
    fn timed(
        &self,
        session: &str,
        call: &str,
        command: &str,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (Answer, Duration) {
        let input = save_input(self, session, call, command);
        let (out, took) = self.timed_event("pre-command", &input, env, limit);
        (answer_in(&out), took)
    }

    fn timed_event(
        &self,
        event: &str,
        input: &Value,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (String, Duration) {
        let mut command = self.command(&self.demo, &["hook", event, "--if-active"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        let (output, took) = common::within(child, input.to_string().into_bytes(), limit)
            .unwrap_or_else(|| panic!("the {event} hook was still running after {limit:?}"));
        assert!(output.status.success(), "a hook always exits 0");
        (String::from_utf8(output.stdout).unwrap(), took)
    }

    /// The session's next save, after a hook's recovery finished a write an earlier hook
    /// left. If that write changed the effort file (its stamp is to the minute, so only
    /// when the minute turned in between), the save is refused once, as any save over
    /// text the session hasn't seen is, and goes through when made again.
    fn saves_after_a_recovered_write(&self) {
        let save = "astrolabe effort save --journal \"the next line\"";
        let (out, ok) = self.bash("s1", save);
        if !ok && out.contains("changed since it was delivered to this session") {
            let (out, ok) = self.bash("s1", save);
            assert!(ok, "{out}");
        } else {
            assert!(ok, "{out}");
        }
    }

    fn unfinished(&self) -> Vec<Value> {
        std::fs::read_dir(self.state().join("ops"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|x| x == "json"))
            .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
            .collect()
    }

    /// A hook of the notes checkout's own git, which git runs with its bundled shell on
    /// every platform: the way these tests make the real git slow partway through a step.
    fn notes_hook(&self, name: &str, script: &str) {
        let hook = self.notes.join(".git/hooks").join(name);
        write(&hook, &format!("#!/bin/sh\n{script}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn mid_rebase(&self) -> bool {
        let dot = self.notes.join(".git");
        dot.join("rebase-merge").exists() || dot.join("rebase-apply").exists()
    }

    /// The lock files git leaves in the checkout while it works, or when it's ended partway.
    fn git_locks(&self) -> Vec<String> {
        std::fs::read_dir(self.notes.join(".git"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".lock"))
            .collect()
    }
}

/// Waits for `done`, failing the test if it doesn't come true.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(60), "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_held_notes_lock_refuses_a_save_with_a_reply() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (before, log) = (mac.effort_text(1), mac.log());
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let held = notes.lock().unwrap();
    // The lock wait ends a little past half the hook's time; the rest is room for a slow
    // machine to start the program and its git calls.
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "8000")];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(25));
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.starts_with("Effort NOT saved: the notes are busy: another session's write or a sync held them for the "),
        "{text}"
    );
    assert!(
        text.contains("so nothing was written. Run the same command again"),
        "{text}"
    );
    assert!(took < Duration::from_secs(8), "{took:?}");
    assert_eq!(mac.effort_text(1), before);
    assert_eq!(mac.log(), log);
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    drop(held);
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
    assert!(mac.journal_today().contains("a line for the day"));
}

#[test]
fn a_held_notes_lock_at_session_start_still_delivers_context() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let _held = notes.lock().unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "8000")];
    let input = json!({"session_id": "s1", "cwd": mac.demo, "source": "resume"});
    let (out, took) = mac.timed_event("session-start", &input, &env, Duration::from_secs(25));
    let answer: Value = serde_json::from_str(out.trim()).expect("context, not silence");
    let text = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("Smooth ink") && text.contains("Notes not refreshed from origin (no lock)"),
        "{text}"
    );
    assert!(took < Duration::from_secs(8), "{took:?}");
}

/// The owner-reply check on the way to a verdict never waits for another writer: a held
/// lock means it's skipped, as before there was a clock.
#[test]
fn a_held_notes_lock_doesnt_hold_up_an_ordinary_command() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let _held = notes.lock().unwrap();
    // A file an earlier build left in the state folder, which made every look at the notes
    // wait for the lock.
    write(&mac.state().join("rebase-started"), "");
    let (answer, took) = mac.timed("s1", "call-2", "echo hi", &[], Duration::from_secs(40));
    assert!(matches!(answer, Answer::Silent), "the command runs");
    // A hook that waited for the lock would have waited 9 s of its 16.
    assert!(took < Duration::from_secs(5), "{took:?}");
}

/// One hook run, watched as the app sees it: when its answer arrived, when the process
/// ended, and when its output closed.
struct Watched {
    answer: Answer,
    answered: Duration,
    exited: Duration,
    closed: Duration,
}

fn watch(mut command: Command, input: &Value) -> Watched {
    use std::io::{BufRead, BufReader, Read};
    let started = Instant::now();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    (child.stdin.take().unwrap())
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let answered = started.elapsed();
        let mut rest = String::new();
        stdout.read_to_string(&mut rest).unwrap();
        assert_eq!(rest.trim(), "", "one answer");
        (line, answered, started.elapsed())
    });
    let status = child.wait().unwrap();
    let exited = started.elapsed();
    assert!(status.success(), "a hook always exits 0");
    let (line, answered, closed) = reader.join().unwrap();
    Watched {
        answer: answer_in(&line),
        answered,
        exited,
        closed,
    }
}

/// A write whose git is still at work when the hook's time is up: the answer comes on
/// time, the hook is gone before the app would give up on it, git is left to finish, and
/// nothing tells the session the write didn't happen.
#[test]
fn a_write_still_under_way_at_the_answer_is_never_ended_or_called_not_written() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The real git, holding its index lock while a commit hook of the checkout's takes
    // longer than the hook has.
    mac.notes_hook("pre-commit", "sleep 9");
    let mut hook = mac.command(&mac.demo, &["hook", "pre-command", "--if-active"]);
    // Answers at 6 s and is gone by 7; an app giving it 20 s for its 16 would give this 7.5.
    hook.env("ASTROLABE_HOOK_BUDGET_MS", "6000");
    let before = mac.log();
    let seen = watch(hook, &save_input(&mac, "s1", "call-2", SAVE));
    let Answer::Denied(why) = &seen.answer else {
        panic!("the backstop's denial, not a reply that the write failed");
    };
    assert!(
        why.starts_with(OUT_OF_TIME)
            && why.contains("run `astrolabe notes status` to see whether its write landed"),
        "{why}"
    );
    assert!(
        seen.answered >= Duration::from_millis(5900) && seen.answered < seen.exited,
        "answered at {:?}, exited at {:?}",
        seen.answered,
        seen.exited
    );
    assert!(
        seen.exited < Duration::from_millis(7500),
        "{:?}",
        seen.exited
    );
    // The app sees the end of the hook's output when the hook ends, not when git does: a
    // git holding that pipe would have the app wait out its timeout and discard the answer.
    assert!(
        seen.closed < Duration::from_millis(7500),
        "output closed at {:?}",
        seen.closed
    );
    // Git wasn't ended: the commit is still on its way, and lands by itself.
    assert_eq!(
        mac.log(),
        before,
        "git was still committing when the hook left"
    );
    until("the commit lands", || mac.log() != before);
    until("git lets go of the checkout", || mac.git_locks().is_empty());
    assert!(mac.log().starts_with("0001: checkpoint\n"), "{}", mac.log());
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
    // The next hook finds the write done and commits nothing more for it.
    std::fs::remove_file(mac.notes.join(".git/hooks/pre-commit")).unwrap();
    mac.saves_after_a_recovered_write();
    let history = git(&mac.notes, &["log", "-p", "--format=%s"]);
    assert_eq!(
        history.matches("+- a line for the day").count(),
        1,
        "{history}"
    );
    assert!(mac.unfinished().iter().all(|op| op["result"].is_string()));
}

#[test]
fn a_hook_that_dies_mid_write_leaves_a_write_the_next_hook_finishes() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // Held up between writing the files and committing them, past the hook's whole time.
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "10000"),
        ("ASTROLABE_HOOK_TEST_STALL", "commit:60000"),
    ];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(30));
    let Answer::Denied(why) = answer else {
        panic!("the backstop's denial");
    };
    assert!(why.starts_with(OUT_OF_TIME), "{why}");
    assert!(
        took >= Duration::from_millis(9900) && took < Duration::from_millis(12500),
        "{took:?}"
    );
    let left = mac.unfinished();
    assert!(left.len() == 1 && left[0]["committed"] != true, "{left:?}");
    assert_ne!(git(&mac.notes, &["status", "--porcelain"]), "");
    // The lock died with the process, and the next hook's recovery finishes that write.
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(
        status.contains("1 notes write(s) left unfinished"),
        "{status}"
    );
    mac.saves_after_a_recovered_write();
    // Both landed, each once: the day keeps an effort's latest line, history has both.
    let history = git(&mac.notes, &["log", "-p", "--format=%s"]);
    assert_eq!(
        history.matches("+- a line for the day").count(),
        1,
        "{history}"
    );
    assert!(mac.journal_today().contains("the next line"));
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
    assert!(mac.unfinished().iter().all(|op| op["result"].is_string()));
    assert_eq!(
        mac.log().matches("0001: checkpoint").count(),
        2,
        "{}",
        mac.log()
    );
}

/// A pull whose rebase takes the hook's time isn't ended: it finishes, and the save that
/// asked for it is refused before anything is planned or written, saying where the time
/// went.
#[test]
fn a_pull_that_takes_the_hooks_time_finishes_and_its_save_is_refused_naming_it() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // Another machine's commit on origin, so the pull has a local commit to replay, and a
    // hand edit in the tree, which the rebase stashes and puts back.
    let pc = world.machine("pc");
    write(
        &pc.notes.join("knowledge/other.md"),
        "FROM THE OTHER MACHINE",
    );
    git(&pc.notes, &["add", "-A"]);
    git(&pc.notes, &["commit", "-q", "-m", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    std::fs::write(
        mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT\nA HAND EDIT\n",
    )
    .unwrap();
    // The real git, held up once in the middle of that rebase until past the time a write
    // may start (12 s of the hook's 16).
    let once = mac.dir.0.join("slow.once");
    mac.notes_hook(
        "reference-transaction",
        &format!(
            "dir=$(git rev-parse --git-dir)\nif [ -d \"$dir/rebase-merge\" ] && [ ! -e '{once}' ]; then\n  : > '{once}'\n  sleep 13\nfi\nexit 0",
            once = once.display().to_string().replace('\\', "/")
        ),
    );
    let before = mac.effort_text(1);
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &[], Duration::from_secs(45));
    assert!(once.exists(), "the rebase was never held up");
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket: the hook answers before its backstop");
    };
    assert!(
        !ok && text.starts_with("Effort NOT saved: this hook's pull of the notes took ")
            && text.contains(
                "too little to write them safely, so nothing was written. Run the same command again"
            ),
        "{text}"
    );
    assert!(took >= Duration::from_secs(12), "{took:?}");
    // The rebase ran to its end: the other machine's commit is in, this one's replayed on
    // it, the hand edit is back in the tree and nothing is left partway.
    assert_eq!(mac.log(), "0001: started\ntheirs\nseed\n");
    assert!(!mac.mid_rebase());
    assert!(mac.git_locks().is_empty());
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT\nA HAND EDIT\n"
    );
    assert_eq!(git(&mac.notes, &["stash", "list"]), "");
    // And the save wrote nothing: no file, no plan to finish later.
    assert_eq!(mac.effort_text(1), before);
    assert!(!mac.journal_today().contains("a line for the day"));
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
    assert!(mac.journal_today().contains("a line for the day"));
}

/// A check of the checkout that can't be run refuses the write: nothing is written on a
/// guess that the checkout is fine.
#[test]
fn a_check_that_cant_be_run_refuses_the_write_before_anything_is_written() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.effort_text(1);
    // Git can't say what branch this is, or anything else about it.
    let head = mac.notes.join(".git/HEAD");
    let was = std::fs::read_to_string(&head).unwrap();
    std::fs::write(&head, "not a head\n").unwrap();
    let (text, ok) = mac.bash("s1", SAVE);
    assert!(
        !ok && text.starts_with(
            "Effort NOT saved: the notes checkout couldn't be checked before writing ("
        ) && text.contains("). Nothing was written"),
        "{text}"
    );
    assert_eq!(mac.effort_text(1), before);
    assert!(!mac.journal_today().contains("a line for the day"));
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    std::fs::write(&head, was).unwrap();
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
}

#[test]
fn a_write_against_a_checkout_mid_rebase_is_refused_and_loses_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The owner, rebasing by hand in the notes checkout, stopped partway: the conflict is
    // resolved in the index and the rebase not continued, so HEAD is detached.
    let pc = world.machine("pc");
    std::fs::write(pc.notes.join("knowledge/owner.md"), "THEIRS").unwrap();
    git(&pc.notes, &["commit", "-q", "-am", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    std::fs::write(mac.notes.join("knowledge/owner.md"), "OURS").unwrap();
    git(&mac.notes, &["commit", "-q", "-am", "ours"]);
    git(&mac.notes, &["fetch", "-q"]);
    let rebase = Command::new("git")
        .args(["rebase", "origin/main"])
        .current_dir(&mac.notes)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(!rebase.status.success(), "the fixture should conflict");
    std::fs::write(mac.notes.join("knowledge/owner.md"), "BOTH").unwrap();
    git(&mac.notes, &["add", "knowledge/owner.md"]);
    assert!(mac.mid_rebase());
    let head = git(&mac.notes, &["rev-parse", "HEAD"]);
    // A write an earlier hook planned and never finished.
    let effort = shelf("0001-smooth-ink.md");
    let text = std::fs::read_to_string(mac.notes.join(&effort)).unwrap();
    let intent = json!({
        "key": "crashed-key", "files": [[effort, text.replace("smooth the ink", "CRASHED smooth the ink")]],
        "renames": [], "journal": [], "message": "0001: checkpoint", "base": null, "states": []
    });
    write(
        &mac.state().join("ops/crashed-key.json"),
        &intent.to_string(),
    );
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(
        !ok && out.contains("the notes checkout is in the middle of a rebase"),
        "{out}"
    );
    // Nothing went onto the detached HEAD, where the rebase's end would drop it.
    assert_eq!(git(&mac.notes, &["rev-parse", "HEAD"]), head);
    let left = mac.unfinished();
    assert!(left.len() == 1 && left[0]["committed"] != true, "{left:?}");
    // The rebase is the owner's to end: neither the refusal nor the save's pull touched it.
    assert!(mac.mid_rebase(), "the owner's rebase was ended");
    git(&mac.notes, &["rebase", "--abort"]);
    let (out, _) = mac.bash("s1", "astrolabe effort save --journal \"the next line\"");
    let log = git(&mac.notes, &["log", "--format=%s", "main"]);
    assert!(log.contains("0001: checkpoint"), "{out}\n{log}");
    assert!(mac.effort_text(1).contains("CRASHED smooth the ink"));
    assert!(mac.unfinished().iter().all(|op| op["result"].is_string()));
}

#[cfg(unix)]
#[test]
fn a_flooding_fetch_is_not_a_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A `git` first on PATH that prints more than a pipe holds, on the stream git reports
    // progress on, before it runs the real fetch.
    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap();
    let bin = mac.dir.0.join("shim");
    let shim = bin.join("git");
    write(
        &shim,
        &format!(
            "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = fetch ] && head -c 1048576 /dev/zero | tr '\\0' x >&2; done\nexec '{}' \"$@\"\n",
            real.trim()
        ),
    );
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let mut command = mac.command(&mac.notes, &["notes", "sync"]);
    command
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (output, took) = common::within(
        command.spawn().unwrap(),
        Vec::new(),
        Duration::from_secs(40),
    )
    .expect("the sync ends");
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && out.contains("pushed"), "{out}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    let record = std::fs::read_to_string(mac.state().join("sync.json")).unwrap();
    assert!(!record.contains("backoff_until_ms"), "{record}");
}
