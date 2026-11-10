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
        text.contains("Smooth ink")
            && text.contains(
                "Notes not refreshed from origin (another session held the notes): this is the local copy."
            ),
        "{text}"
    );
    // Another session writing is not a write of this one's that failed.
    assert!(!text.contains("Unfinished notes writes"), "{text}");
    assert!(!text.contains("Run the same command again"), "{text}");
    assert!(took < Duration::from_secs(8), "{took:?}");
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
    // The real git, holding its index lock while a commit hook of the checkout's runs until
    // 24 s after the hook starts, by the clock: past the 20 s the app gives the hook.
    let begun = mac.dir.0.join("hook.began");
    mac.notes_hook(
        "pre-commit",
        &format!(
            "began=$(cat '{begun}')\nwhile [ \"$(date +%s)\" -lt $((began + 24)) ]; do sleep 1; done\nexit 0",
            begun = begun.display().to_string().replace('\\', "/")
        ),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    write(&begun, &now.as_secs().to_string());
    // The hook's real time: it answers at 16 s, is gone by 18.5, and the app gives it 20.
    let hook = mac.command(&mac.demo, &["hook", "pre-command", "--if-active"]);
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
        seen.answered >= Duration::from_millis(15900) && seen.answered < seen.exited,
        "answered at {:?}, exited at {:?}",
        seen.answered,
        seen.exited
    );
    assert!(seen.exited < Duration::from_secs(20), "{:?}", seen.exited);
    // The app sees the end of the hook's output when the hook ends, not when git does: a
    // git holding that pipe would have the app wait out its timeout and discard the answer.
    assert!(
        seen.closed < Duration::from_secs(20),
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
    // On the hook's real time: it answers at 16 s and is gone by 18.5, inside the app's 20.
    let env = [("ASTROLABE_HOOK_TEST_STALL", "commit:60000")];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(40));
    let Answer::Denied(why) = answer else {
        panic!("the backstop's denial");
    };
    assert!(why.starts_with(OUT_OF_TIME), "{why}");
    assert!(
        took >= Duration::from_millis(15900) && took < Duration::from_secs(20),
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
    // The real git, held up once in the middle of that rebase until 11 to 12 s after the
    // hook starts: past the last moment a write may begin (10.5 s of the hook's 16), with
    // 4 s left before the backstop for the rest of the pull and the reply. The wait is to a
    // time on the clock, not a length, because how soon the rebase begins varies by machine.
    let (once, begun) = (mac.dir.0.join("slow.once"), mac.dir.0.join("hook.began"));
    let unix = |path: &Path| path.display().to_string().replace('\\', "/");
    mac.notes_hook(
        "reference-transaction",
        &format!(
            "dir=$(git rev-parse --git-dir)\nif [ -d \"$dir/rebase-merge\" ] && [ ! -e '{once}' ]; then\n  : > '{once}'\n  began=$(cat '{begun}')\n  while [ \"$(date +%s)\" -lt $((began + 12)) ]; do sleep 1; done\nfi\nexit 0",
            once = unix(&once),
            begun = unix(&begun)
        ),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    write(&begun, &now.as_secs().to_string());
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
    assert!(took >= Duration::from_millis(10500), "{took:?}");
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

/// The whole save, pull included, against a rebase the owner left partway by hand: nothing
/// is fetched, the rebase and the resolution they staged are untouched, and the save is
/// refused. On main the save's own pull failed to start a rebase, cleaned up with an abort
/// that ended the owner's, and the save then landed.
#[test]
fn a_save_and_its_pull_leave_the_owners_rebase_and_resolution_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
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
    // Resolved and staged, not continued.
    std::fs::write(mac.notes.join("knowledge/owner.md"), "BOTH").unwrap();
    git(&mac.notes, &["add", "knowledge/owner.md"]);
    // Something new on origin since, which a pull would fetch.
    write(&pc.notes.join("knowledge/later.md"), "LATER");
    git(&pc.notes, &["add", "-A"]);
    git(&pc.notes, &["commit", "-q", "-m", "later"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    let seen = |mac: &Machine| {
        (
            git(&mac.notes, &["rev-parse", "HEAD"]),
            git(&mac.notes, &["rev-parse", "origin/main"]),
            git(&mac.notes, &["reflog", "--format=%gs"]),
            git(&mac.notes, &["diff", "--cached", "--name-only"]),
            std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
            std::fs::read_to_string(mac.notes.join(".git/rebase-merge/orig-head")).unwrap(),
        )
    };
    let before = seen(&mac);
    assert!(before.3.contains("knowledge/owner.md"), "{}", before.3);
    // No write is waiting from before, so the save gets as far as its pull.
    assert!(mac.unfinished().is_empty());
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(
        !ok && out.contains(
            "the notes checkout is in the middle of a rebase; finish it there, or end it with `git rebase --abort`, first. Nothing was written"
        ),
        "{out}"
    );
    assert_eq!(seen(&mac), before, "the owner's rebase was touched");
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    // A session start pulls too, and says why it didn't.
    let context = mac.session_start("s1", "resume");
    assert!(
        context.contains(
            "Notes not refreshed from origin (a rebase is already under way in the notes checkout)"
        ),
        "{context}"
    );
    assert_eq!(seen(&mac), before, "the owner's rebase was touched");
    // The owner finishes it, with what they resolved, and the save goes through.
    let continued = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(["-c", "core.editor=true", "rebase", "--continue"])
        .current_dir(&mac.notes)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        continued.status.success(),
        "{}",
        String::from_utf8_lossy(&continued.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "BOTH"
    );
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
    assert!(mac.notes.join("knowledge/later.md").exists(), "pulled now");
}

/// The same for a checkout off its branch: nothing is fetched and nothing written.
#[test]
fn a_save_on_a_detached_checkout_pulls_and_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let pc = world.machine("pc");
    write(&pc.notes.join("knowledge/later.md"), "LATER");
    git(&pc.notes, &["add", "-A"]);
    git(&pc.notes, &["commit", "-q", "-m", "later"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    git(&mac.notes, &["checkout", "-q", "--detach"]);
    let seen = |mac: &Machine| {
        (
            git(&mac.notes, &["rev-parse", "HEAD"]),
            git(&mac.notes, &["rev-parse", "origin/main"]),
            git(&mac.notes, &["reflog", "--format=%gs"]),
        )
    };
    let before = seen(&mac);
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(
        !ok && out.contains("the notes checkout isn't on a branch (its HEAD is detached)"),
        "{out}"
    );
    assert_eq!(seen(&mac), before);
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    git(&mac.notes, &["checkout", "-q", "main"]);
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
}

/// A folder to put first on PATH, holding a `git` that runs `script` and then the real
/// one. The script sees git's arguments as `$@` and the real git as `$real`.
#[cfg(unix)]
fn git_shim(mac: &Machine, script: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
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
            "#!/bin/sh\nreal='{}'\n{script}\nexec \"$real\" \"$@\"\n",
            real.trim()
        ),
    );
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
}

#[cfg(unix)]
#[test]
fn a_flooding_fetch_is_not_a_timeout() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // More than a pipe holds, on the stream git reports progress on, before the real fetch.
    let path = git_shim(
        &mac,
        "for a in \"$@\"; do [ \"$a\" = fetch ] && head -c 1048576 /dev/zero | tr '\\0' x >&2; done",
    );
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

/// A fetch that takes longer than a hook can wait, and less than the 15 s a sync gives
/// it, is cut short in the hook. That says the hook couldn't wait, not that the network
/// is down: it starts no backoff, so the sync the save asks for still fetches and pushes.
#[cfg(unix)]
#[test]
fn a_fetch_a_hook_cut_short_starts_no_backoff() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let path = git_shim(
        &mac,
        "for a in \"$@\"; do [ \"$a\" = fetch ] && sleep 8; done",
    );
    let env = [("PATH", path.as_str())];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(40));
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    // The save doesn't need the pull: it lands on the local copy.
    assert!(ok, "{text}");
    assert!(
        took >= Duration::from_secs(4),
        "the fetch wasn't slow: {took:?}"
    );
    let record = std::fs::read_to_string(mac.state().join("sync.json")).unwrap_or_default();
    assert!(!record.contains("backoff_until_ms"), "{record}");
    // The same slow fetch, in the sync, has the time it needs.
    let mut sync = mac.command(&mac.notes, &["notes", "sync"]);
    sync.env("PATH", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (output, _) = common::within(sync.spawn().unwrap(), Vec::new(), Duration::from_secs(60))
        .expect("the sync ends");
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && out.contains("pushed"),
        "{out}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The owner starts a rebase by hand while a save's pull is fetching, after the pull
/// looked and found none. The pull looks again before its own rebase and leaves theirs
/// alone; the save is refused.
#[cfg(unix)]
#[test]
fn a_rebase_the_owner_starts_during_the_fetch_is_left_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A conflict waiting to happen: the same line changed here and on origin.
    let pc = world.machine("pc");
    std::fs::write(pc.notes.join("knowledge/owner.md"), "THEIRS").unwrap();
    git(&pc.notes, &["commit", "-q", "-am", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    std::fs::write(mac.notes.join("knowledge/owner.md"), "OURS").unwrap();
    git(&mac.notes, &["commit", "-q", "-am", "ours"]);
    git(&mac.notes, &["fetch", "-q"]);
    // During the save's fetch, once: the owner rebases, hits the conflict, resolves it and
    // stages the resolution, and hasn't continued.
    let once = mac.dir.0.join("owner.once");
    let path = git_shim(
        &mac,
        &format!(
            "for a in \"$@\"; do if [ \"$a\" = fetch ] && [ ! -e '{once}' ]; then\n  : > '{once}'\n  \"$real\" rebase origin/main >/dev/null 2>&1\n  printf BOTH > knowledge/owner.md\n  \"$real\" add knowledge/owner.md\nfi; done",
            once = once.display()
        ),
    );
    assert!(!mac.mid_rebase());
    let env = [("PATH", path.as_str())];
    let (answer, _) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(40));
    assert!(once.exists(), "the owner's rebase never began");
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.contains("the notes checkout is in the middle of a rebase"),
        "{text}"
    );
    assert!(mac.mid_rebase(), "the owner's rebase was ended");
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "BOTH"
    );
    assert!(
        git(&mac.notes, &["diff", "--cached", "--name-only"]).contains("knowledge/owner.md"),
        "their resolution is no longer staged"
    );
    let reflog = git(&mac.notes, &["reflog", "--format=%gs"]);
    assert!(!reflog.contains("abort"), "{reflog}");
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
}

/// A repo a hook couldn't read whole is unknown, not clean: the turn end says nothing
/// about it. Read as clean, it would look changed against a fingerprint taken with files
/// in it, and the session would be told so at every turn end.
#[test]
fn a_repo_read_that_ran_out_of_time_brings_no_reminder() {
    let world = World::new();
    let mac = world.machine("mac");
    // Something in the session's repo that a whole read sees and an empty one doesn't.
    write(&mac.demo.join("untracked.txt"), "work in progress");
    assert!(mac.bash("s1", NEW).1);
    let input = json!({"session_id": "s1", "cwd": mac.demo, "stop_hook_active": false, "last_assistant_message": "done"});
    // Nothing moved since the effort was made, so a turn end says nothing.
    let (out, _) = mac.timed_event("stop", &input, &[], Duration::from_secs(20));
    assert_eq!(out.trim(), "", "the fixture should start quiet");
    // The same turn end, with the hook's clock moved past its time for reads as it comes
    // to read the repo's status: that read and the one after it aren't started.
    let env = [("ASTROLABE_HOOK_TEST_STALL", "status:+4000")];
    let (out, _) = mac.timed_event("stop", &input, &env, Duration::from_secs(20));
    assert_eq!(
        out.trim(),
        "",
        "a read that didn't finish was taken for a change"
    );
    // And nothing was recorded from it: the next turn end is as quiet as the first.
    let (out, _) = mac.timed_event("stop", &input, &[], Duration::from_secs(20));
    assert_eq!(out.trim(), "");
}

/// The same from the other side: a repo whose status and diff git can't give has no facts
/// at all, never the facts of a clean tree.
#[test]
fn a_repo_git_cant_read_has_no_facts() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(astrolabe::notes::RepoFacts::read(&mac.demo).is_some());
    std::fs::write(mac.demo.join(".git/index"), "not an index").unwrap();
    assert!(astrolabe::notes::RepoFacts::read(&mac.demo).is_none());
}

/// A pull whose rebase is still at work when the hook has to exit: git is left running,
/// and finishes. A rebase prints as it ends ("Applied autostash."), even when asked to be
/// quiet; with its output in pipes the hook held, that print would end it after it moved
/// the branch and before it cleared its own state, leaving the checkout mid-rebase.
#[test]
fn a_rebase_that_outlives_the_hook_finishes_though_it_prints() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let pc = world.machine("pc");
    write(
        &pc.notes.join("knowledge/other.md"),
        "FROM THE OTHER MACHINE",
    );
    git(&pc.notes, &["add", "-A"]);
    git(&pc.notes, &["commit", "-q", "-m", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    // A hand edit, which the rebase stashes, puts back at its end, and says so.
    std::fs::write(
        mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT\nA HAND EDIT\n",
    )
    .unwrap();
    // Held up once in the middle of that rebase until 24 s after the hook starts, by the
    // clock: past the 20 s the app gives the hook.
    let (once, begun) = (mac.dir.0.join("slow.once"), mac.dir.0.join("hook.began"));
    let unix = |path: &Path| path.display().to_string().replace('\\', "/");
    mac.notes_hook(
        "reference-transaction",
        &format!(
            "dir=$(git rev-parse --git-dir)\nif [ -d \"$dir/rebase-merge\" ] && [ ! -e '{once}' ]; then\n  : > '{once}'\n  began=$(cat '{begun}')\n  while [ \"$(date +%s)\" -lt $((began + 24)) ]; do sleep 1; done\nfi\nexit 0",
            once = unix(&once),
            begun = unix(&begun)
        ),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    write(&begun, &now.as_secs().to_string());
    let hook = mac.command(&mac.demo, &["hook", "pre-command", "--if-active"]);
    let seen = watch(hook, &save_input(&mac, "s1", "call-2", SAVE));
    assert!(once.exists(), "the rebase was never held up");
    let Answer::Denied(why) = &seen.answer else {
        panic!("the backstop's denial");
    };
    assert!(why.starts_with(OUT_OF_TIME), "{why}");
    assert!(seen.exited < Duration::from_secs(20), "{:?}", seen.exited);
    assert!(
        mac.mid_rebase(),
        "git was still rebasing when the hook left"
    );
    // Left alone, the rebase runs to its end and clears up after itself.
    until("the rebase ends", || !mac.mid_rebase());
    until("git lets go of the checkout", || mac.git_locks().is_empty());
    assert_eq!(mac.log(), "0001: started\ntheirs\nseed\n");
    assert_eq!(
        git(&mac.notes, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT\nA HAND EDIT\n"
    );
    assert_eq!(git(&mac.notes, &["stash", "list"]), "");
    // Nothing of the save was written, and the next one goes through.
    assert!(mac.unfinished().is_empty());
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
}

impl Machine {
    fn session_state(&self, session: &str) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.state().join(format!("{session}.json"))).unwrap(),
        )
        .unwrap()
    }

    /// What a turn end says to the session, if anything.
    fn turn_end(&self, session: &str) -> String {
        let input = json!({"session_id": session, "cwd": self.demo, "stop_hook_active": false, "last_assistant_message": "done"});
        let (out, _) = self.timed_event("stop", &input, &[], Duration::from_secs(20));
        out.trim().to_owned()
    }
}

const MOVED: &str = "the repo changed since the last save";

/// A session with no fingerprint of its repo stored, because the repo couldn't be read
/// when it was bound, is told nothing about the repo having moved: there's nothing to
/// compare with. The turn end takes the repo as it is, and the next one compares with that.
#[test]
fn a_session_with_no_stored_fingerprint_isnt_told_its_repo_moved() {
    let world = World::new();
    let mac = world.machine("mac");
    write(&mac.demo.join("untracked.txt"), "work in progress");
    assert!(mac.bash("s1", NEW).1);
    // A second session picks the effort up with its clock moved past its time for reads as
    // it comes to read the repo's status, so that read isn't started.
    let env = [("ASTROLABE_HOOK_TEST_STALL", "status:+14500")];
    let (answer, _) = mac.timed(
        "s2",
        "call-1",
        "astrolabe activate --effort 1",
        &env,
        Duration::from_secs(14),
    );
    assert!(matches!(answer, Answer::Shown(_, true)), "the effort loads");
    assert_eq!(
        mac.session_state("s2")["baseline"],
        Value::Null,
        "a read that didn't finish was stored as the repo's fingerprint"
    );
    // Nothing changed in the repo, and the turn end says nothing.
    assert_eq!(mac.turn_end("s2"), "");
    assert!(mac.session_state("s2")["baseline"].is_string());
    assert_eq!(mac.turn_end("s2"), "");
    // From there a real change is seen.
    write(&mac.demo.join("more.txt"), "more work");
    assert!(mac.turn_end("s2").contains(MOVED));
}

/// A save whose own read of the repo failed makes no claim about the repo: not the
/// fingerprint of the save before it, which the work since has left behind.
#[test]
fn a_save_that_couldnt_read_its_repo_makes_no_claim_about_it() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let saved_with = mac.session_state("s1")["baseline"].clone();
    assert!(saved_with.is_string());
    // Work since that save, then a save while git can't read the repo's status.
    write(&mac.demo.join("work.txt"), "work since the first save");
    let index = mac.demo.join(".git/index");
    let was = std::fs::read(&index).unwrap();
    std::fs::write(&index, "not an index").unwrap();
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
    std::fs::write(&index, was).unwrap();
    assert_eq!(
        mac.session_state("s1")["baseline"],
        Value::Null,
        "the save kept a fingerprint it didn't take"
    );
    // The turn end right after that save has nothing to say about the repo.
    assert_eq!(mac.turn_end("s1"), "");
    write(&mac.demo.join("later.txt"), "work after the second save");
    assert!(mac.turn_end("s1").contains(MOVED));
}

/// This pull's own rebase, stopped on a conflict, is put back whatever the conflicting
/// commit is called: whose rebase it is comes from what git recorded, not from its message,
/// which quotes the commit's subject.
#[test]
fn a_conflict_whose_subject_reads_like_gits_own_words_is_still_put_back() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let pc = world.machine("pc");
    std::fs::write(pc.notes.join("knowledge/owner.md"), "THEIRS").unwrap();
    git(&pc.notes, &["commit", "-q", "-am", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    std::fs::write(mac.notes.join("knowledge/owner.md"), "OURS").unwrap();
    let subject = "owner: there is already a rebase in progress on the plan";
    git(&mac.notes, &["commit", "-q", "-am", subject]);
    let (out, ok) = mac.bash("s1", SAVE);
    // The pull couldn't replay that commit, so it put the checkout back and said so; the
    // save landed on the local copy.
    assert!(ok && out.contains("Notes diverged from origin"), "{out}");
    assert!(!mac.mid_rebase(), "this pull's own rebase was left partway");
    assert_eq!(
        git(&mac.notes, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
    assert!(mac.log().starts_with("0001: checkpoint\n"), "{}", mac.log());
}

/// The owner starts a rebase by hand in the instant after the pull looked again and before
/// its own rebase starts, from the same commit and onto the same one. The pull's rebase
/// fails on finding it, and the pull leaves it: git recorded theirs as not a quiet one,
/// and this pull asked for a quiet one.
#[cfg(unix)]
#[test]
fn a_rebase_the_owner_starts_in_the_same_instant_is_left_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let pc = world.machine("pc");
    std::fs::write(pc.notes.join("knowledge/owner.md"), "THEIRS").unwrap();
    git(&pc.notes, &["commit", "-q", "-am", "theirs"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    std::fs::write(mac.notes.join("knowledge/owner.md"), "OURS").unwrap();
    git(&mac.notes, &["commit", "-q", "-am", "ours"]);
    // As the pull's own rebase is about to start, once: the owner's, to the conflict,
    // resolved and staged and not continued.
    let once = mac.dir.0.join("owner.once");
    let path = git_shim(
        &mac,
        &format!(
            "case \" $* \" in *\" rebase \"*\" --autostash \"*) if [ ! -e '{once}' ]; then\n  : > '{once}'\n  \"$real\" rebase origin/main >/dev/null 2>&1\n  printf BOTH > knowledge/owner.md\n  \"$real\" add knowledge/owner.md\nfi;; esac",
            once = once.display()
        ),
    );
    let env = [("PATH", path.as_str())];
    let (answer, _) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(40));
    assert!(once.exists(), "the owner's rebase never began");
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.contains("the notes checkout is in the middle of a rebase"),
        "{text}"
    );
    assert!(mac.mid_rebase(), "the owner's rebase was ended");
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "BOTH"
    );
    assert!(
        git(&mac.notes, &["diff", "--cached", "--name-only"]).contains("knowledge/owner.md"),
        "their resolution is no longer staged"
    );
    let reflog = git(&mac.notes, &["reflog", "--format=%gs"]);
    assert!(!reflog.contains("abort"), "{reflog}");
}
