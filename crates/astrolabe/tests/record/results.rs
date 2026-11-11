//! A finished command's result: kept for the session that asked for it however late it
//! reads, and a retry of the same command is that request again, not a second write.
use super::*;
use std::time::{Duration, SystemTime};

impl Machine {
    /// A Bash command through the pre-command hook, and the file its rewritten command
    /// reads: what the app would print, whenever it gets to it.
    fn result_file(&self, session: &str, call: &str, command: &str) -> PathBuf {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let answer = self.hook_json("pre-command", input).unwrap();
        let shown = answer["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{answer}"));
        let path = (shown.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        PathBuf::from(path)
    }
}

/// A result's files, as if the hook had left them `ago` earlier.
fn age(text: &Path, ago: Duration) {
    for path in [text.to_path_buf(), text.with_extension("json")] {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - ago).unwrap();
    }
}

#[test]
fn a_result_read_after_a_minute_and_another_sessions_write_is_still_there() {
    let world = World::new();
    let mac = world.machine("mac");
    // The hook wrote the effort; the app hasn't run the read yet, say behind a prompt.
    let slow = mac.result_file("s1", "call-1", NEW);
    age(&slow, Duration::from_secs(5 * 60));
    // Another session's command leaves its own result meanwhile.
    let other = "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nanother goal\nASTROLABE\n";
    assert!(mac.bash("s2", other).1);
    let late = std::fs::read_to_string(&slow)
        .unwrap_or_else(|why| panic!("the first session's result was removed: {why}"));
    assert!(late.starts_with("Effort 1 created"), "{late}");
    // Only age removes one: past its week it goes when the next is written.
    age(&slow, Duration::from_secs(8 * 24 * 3600));
    assert!(
        mac.bash(
            "s2",
            "astrolabe effort save --decision \"chose a over b: c\""
        )
        .1
    );
    assert!(!slow.exists());
}

const OUT_OF_TIME: &str = "Astrolabe's hook ran out of time before it finished with this command";
/// Held up between writing the files and committing them until the hook's time is up: it
/// answers at 8 s and is gone soon after, with the write left planned.
const HELD_UP: [(&str, &str); 2] = [
    ("ASTROLABE_HOOK_TEST_STALL", "commit:60000"),
    ("ASTROLABE_HOOK_BUDGET_MS", "8000"),
];

impl Machine {
    /// A Bash command through the pre-command hook as its own tool call: the denial's
    /// reason, or the result the rewritten command reads.
    fn call(
        &self,
        session: &str,
        call: &str,
        command: &str,
        env: &[(&str, &str)],
    ) -> Result<String, String> {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let mut hook = self.command(&self.demo, &["hook", "pre-command", "--if-active"]);
        hook.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            hook.env(key, value);
        }
        let child = hook.spawn().unwrap();
        let limit = Duration::from_secs(60);
        let (output, _) = common::within(child, input.to_string().into_bytes(), limit)
            .expect("the hook answers inside its time");
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
        let inner = &answer["hookSpecificOutput"];
        if inner["permissionDecision"] == "deny" {
            return Err(inner["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .to_owned());
        }
        let shown = inner["updatedInput"]["command"].as_str().unwrap();
        let path = (shown.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        Ok(std::fs::read_to_string(path).unwrap())
    }

    /// The writes still on record as never answered or never finished.
    fn unanswered(&self) -> Vec<PathBuf> {
        let left = std::fs::read_dir(self.state().join("ops"));
        (left.into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .collect()
    }
}

const FOUND: &str = "astrolabe journal \"Found the Deck preset in the video settings.\"";

/// The hook ran out of time while committing, so the session was denied and ran the same
/// command again: a new tool call. Recovery finishes the first write, and the retry is that
/// request, not a second line.
#[test]
fn a_journal_line_retried_after_the_out_of_time_denial_is_written_once() {
    let world = World::new();
    let mac = world.machine("mac");
    let denied = mac.call("s1", "call-1", FOUND, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    let retried = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(
        retried.contains("nothing new went in the journal"),
        "{retried}"
    );
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        1,
        "{journal}"
    );
    assert_eq!(
        mac.log().matches("journal: demo").count(),
        1,
        "{}",
        mac.log()
    );
}

#[test]
fn a_new_effort_retried_after_the_out_of_time_denial_is_created_once() {
    let world = World::new();
    let mac = world.machine("mac");
    let twice = "astrolabe effort new --title \"Twice\" <<'ASTROLABE'\none goal\nASTROLABE\n";
    let denied = mac.call("s1", "call-1", twice, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    let retried = mac.call("s1", "call-2", twice, &[]).unwrap();
    assert!(
        retried.starts_with("Effort 1 was already created")
            && retried.contains("this session is bound to it"),
        "{retried}"
    );
    let efforts = astrolabe::notes::effort_files(&mac.notes);
    assert_eq!(efforts.len(), 1, "{efforts:?}");
    assert!(mac.notes.join(shelf("0001-twice.md")).exists());
    // The session works on from the one effort.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(ok && out.starts_with("Saved effort 1"), "{out}");
}

/// A session that got its answer and runs the same command again means it again: a second
/// journal line after rerunning the tests, a decision recorded twice. Only an attempt whose
/// own hook never answered is waited on by a retry.
#[test]
fn a_command_run_again_after_its_answer_is_a_new_request() {
    let world = World::new();
    let mac = world.machine("mac");
    let first = mac.call("s1", "call-1", FOUND, &[]).unwrap();
    assert!(first.starts_with("Journal line written for"), "{first}");
    let again = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(again.starts_with("Journal line written for"), "{again}");
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        2,
        "{journal}"
    );
    assert_eq!(mac.log().matches("journal: demo").count(), 2);
    assert!(mac.bash("s3", NEW).1);
    let chose = "astrolabe effort save --decision \"chose a over b: c\"";
    assert!(mac.call("s3", "call-2", chose, &[]).is_ok());
    assert!(mac.call("s3", "call-3", chose, &[]).is_ok());
    assert_eq!(mac.effort_text(1).matches("chose a over b: c").count(), 2);
    // An answered write leaves nothing behind for a later command to be matched against.
    assert_eq!(mac.unanswered().len(), 0);
}

/// The write committed and its reply was made, but the hook's time ran out before the
/// reply was left for the session, so the session was denied. Its retry gets that reply.
#[test]
fn a_retry_of_a_write_that_committed_but_was_never_answered_gets_its_reply() {
    let world = World::new();
    let mac = world.machine("mac");
    let unanswered = [
        ("ASTROLABE_HOOK_TEST_STALL", "ticket:60000"),
        ("ASTROLABE_HOOK_BUDGET_MS", "8000"),
    ];
    let denied = mac.call("s1", "call-1", FOUND, &unanswered).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    assert_eq!(mac.log().matches("journal: demo").count(), 1);
    let retried = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(retried.starts_with("Journal line written for"), "{retried}");
    let lines = || mac.journal_today().matches("Found the Deck preset").count();
    assert_eq!(lines(), 1);
    // The retry was answered, so the same words after it are a new request.
    assert!(mac.call("s1", "call-3", FOUND, &[]).is_ok());
    assert_eq!(lines(), 2);
}

/// A write is matched by when it was first attempted, not by when a later run finished it:
/// a line whose hook died long ago, finished by today's identical command, is not that
/// command's first attempt.
#[test]
fn a_write_recovered_long_after_its_attempt_isnt_taken_for_a_new_commands_first_try() {
    let world = World::new();
    let mac = world.machine("mac");
    let denied = mac.call("s1", "call-1", FOUND, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    // As if that had been yesterday.
    let left = std::fs::read_dir(mac.state().join("ops")).unwrap();
    let left: Vec<PathBuf> = left.flatten().map(|entry| entry.path()).collect();
    assert_eq!(left.len(), 1, "{left:?}");
    let mut op: Value = serde_json::from_slice(&std::fs::read(&left[0]).unwrap()).unwrap();
    op["at_ms"] = (Machine::now_ms() - 24 * 3_600_000).into();
    std::fs::write(&left[0], op.to_string()).unwrap();
    let today = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(today.starts_with("Journal line written for"), "{today}");
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        2,
        "yesterday's line recovered, and today's written: {journal}"
    );
}

/// The retry is for the record the session was on. A session that moved to another effort
/// before saying the same words is saving that other effort.
#[test]
fn the_same_save_after_the_session_moved_to_another_effort_is_written_there() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let other = "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nanother goal\nASTROLABE\n";
    assert!(mac.bash("s2", other).1);
    let chose = "astrolabe effort save --decision \"chose a over b: c\"";
    let denied = mac.call("s1", "call-2", chose, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    // Loading the other effort finishes the write left for the first.
    let loaded = mac.call("s1", "call-3", "astrolabe activate --effort 2", &[]);
    assert!(loaded.is_ok(), "{loaded:?}");
    assert_eq!(mac.effort_text(1).matches("chose a over b: c").count(), 1);
    let saved = mac.call("s1", "call-4", chose, &[]).unwrap();
    assert!(saved.starts_with("Saved effort 2"), "{saved}");
    assert_eq!(mac.effort_text(2).matches("chose a over b: c").count(), 1);
    assert_eq!(mac.effort_text(1).matches("chose a over b: c").count(), 1);
}

/// The command the app ran itself, with no hook, is given its arguments with the shell's
/// quoting already gone. One with an apostrophe and no space is still the command the hook
/// answered.
#[test]
fn the_command_run_with_no_hook_finds_the_result_of_one_with_an_apostrophe() {
    let world = World::new();
    let mac = world.machine("mac");
    let inks = "astrolabe effort new --title \"Ink's\" <<'ASTROLABE'\nsmooth it\nASTROLABE\n";
    assert!(mac.bash("s1", inks).1);
    let (out, ok) = mac.plain(inks);
    assert!(
        ok && out.starts_with("Effort 1 created")
            && out.contains("This is the result of a command with this exact text"),
        "{out}"
    );
}

/// Waits for `done`, failing the test if it doesn't come true.
fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(60), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Held after its write, with the notes lock let go, for long enough that another process
/// takes the lock before the hook answers: what the sync a write starts does every time.
const ANSWERS_LATE: [(&str, &str); 1] = [("ASTROLABE_HOOK_TEST_STALL", "ticket:4000")];

/// That a session was answered is recorded without the notes lock, which a sync started by
/// the write itself is usually holding by then. Were it to need the lock, the write would
/// stay on record as never answered and swallow the session's next identical command.
#[test]
fn an_answer_given_while_another_process_holds_the_notes_still_counts() {
    let world = World::new();
    let mac = world.machine("mac");
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let held = std::thread::scope(|scope| {
        let hook = scope.spawn(|| mac.call("s1", "call-1", FOUND, &ANSWERS_LATE));
        wait_for("the line is committed", || {
            mac.log().contains("journal: demo")
        });
        let held = notes.lock().unwrap();
        let first = hook.join().unwrap().unwrap();
        assert!(first.starts_with("Journal line written for"), "{first}");
        // Cleanup cannot take the lock we still hold, but the answer is durable.
        let ops = notes.intents();
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(ops[0].committed && ops[0].result.is_some(), "{ops:?}");
        assert!(notes.is_answered(&ops[0].key), "{ops:?}");
        let before = mac.log();
        assert_eq!(notes.reconcile(), Vec::<String>::new());
        assert_eq!(
            std::fs::read_dir(mac.state().join("ops")).unwrap().count(),
            0
        );
        assert_eq!(mac.log(), before, "cleanup must not commit again");
        held
    });
    drop(held);
    let again = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(again.starts_with("Journal line written for"), "{again}");
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        2,
        "{journal}"
    );
}

/// The same for a retry: once it's answered, the attempt it was answered from is answered
/// too, whoever holds the notes at that moment.
#[test]
fn a_retry_answered_while_another_process_holds_the_notes_settles_its_first_attempt() {
    let world = World::new();
    let mac = world.machine("mac");
    let denied = mac.call("s1", "call-1", FOUND, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let held = std::thread::scope(|scope| {
        let hook = scope.spawn(|| mac.call("s1", "call-2", FOUND, &ANSWERS_LATE));
        wait_for("recovery commits the line", || {
            mac.log().contains("journal: demo")
        });
        let held = notes.lock().unwrap();
        let retried = hook.join().unwrap().unwrap();
        assert!(
            retried.contains("nothing new went in the journal"),
            "{retried}"
        );
        held
    });
    drop(held);
    let lines = || mac.journal_today().matches("Found the Deck preset").count();
    assert_eq!(lines(), 1);
    // The retry was answered, so the same words now are a new request.
    let again = mac.call("s1", "call-3", FOUND, &[]).unwrap();
    assert!(again.starts_with("Journal line written for"), "{again}");
    assert_eq!(lines(), 2);
}

/// A write that has finished but whose hook hasn't answered yet is not an unanswered
/// attempt: nothing says its session won't be told. The same command arriving in that
/// moment, as a second tool call sent alongside the first, is its own request and writes.
#[test]
fn the_same_command_arriving_before_the_first_is_answered_writes_too() {
    let world = World::new();
    let mac = world.machine("mac");
    std::thread::scope(|scope| {
        let first = scope.spawn(|| mac.call("s1", "call-1", FOUND, &ANSWERS_LATE));
        wait_for("the first line is committed", || {
            mac.log().contains("journal: demo")
        });
        let second = mac.call("s1", "call-2", FOUND, &[]).unwrap();
        assert!(second.starts_with("Journal line written for"), "{second}");
        let first = first.join().unwrap().unwrap();
        assert!(first.starts_with("Journal line written for"), "{first}");
    });
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        2,
        "{journal}"
    );
}

/// The backstop answered, and the write the hook was in the middle of finished a moment
/// later, before the hook was gone. The session was denied all the same, so its retry is
/// answered from that write and nothing is written twice. Whether the write finishes in
/// that moment or is left for recovery depends on the machine; either way it's one line.
#[test]
fn a_write_that_finishes_just_after_the_backstop_answered_is_still_unanswered() {
    let world = World::new();
    let mac = world.machine("mac");
    let just_late = [
        ("ASTROLABE_HOOK_TEST_STALL", "commit:8200"),
        ("ASTROLABE_HOOK_BUDGET_MS", "8000"),
    ];
    let denied = mac.call("s1", "call-1", FOUND, &just_late).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    // The hook is gone once nothing of it holds the notes.
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    drop(notes.lock().unwrap());
    let retried = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(
        retried.starts_with("Journal line written for")
            || retried.contains("nothing new went in the journal"),
        "{retried}"
    );
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        1,
        "{journal}"
    );
}

/// The hook was gone with its commit still running: git is never ended, and here the
/// checkout's own commit hook keeps it going past the hook's exit. The retry sent straight
/// away waits for that git, as it would for a writer holding the notes, and is answered
/// from the write it finished. It isn't refused because git still holds the index.
#[test]
fn a_retry_sent_while_the_first_hooks_commit_is_still_running_waits_for_it() {
    let world = World::new();
    let mac = world.machine("mac");
    mac.notes_hook("pre-commit", "sleep 12\nexit 0");
    // Answered at 8 s and gone by 9.25, with the commit running until about 12.
    let short = [("ASTROLABE_HOOK_BUDGET_MS", "8000")];
    let denied = mac.call("s1", "call-1", FOUND, &short).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    assert_eq!(mac.log().matches("journal: demo").count(), 0, "git is done");
    let retried = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(
        retried.contains("nothing new went in the journal"),
        "{retried}"
    );
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        1,
        "{journal}"
    );
    assert_eq!(
        mac.log().matches("journal: demo").count(),
        1,
        "{}",
        mac.log()
    );
}

/// The session can be told of a write by the command itself, run bare, when the hook that
/// made the write never answered: the bare command finds the record and prints its result.
/// From then the session has been told, so the same command through the hook is a new
/// request and writes.
#[test]
fn a_result_shown_by_the_bare_command_counts_as_the_sessions_answer() {
    let world = World::new();
    let mac = world.machine("mac");
    let unanswered = [
        ("ASTROLABE_HOOK_TEST_STALL", "ticket:60000"),
        ("ASTROLABE_HOOK_BUDGET_MS", "8000"),
    ];
    let denied = mac.call("s1", "call-1", FOUND, &unanswered).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    // What that leaves: the write's record with its result, marked as never answered.
    let left = mac.unanswered();
    let marked = |ext: &str| left.iter().any(|p| p.extension().is_some_and(|x| x == ext));
    assert!(marked("json") && marked("unanswered"), "{left:?}");
    let (out, ok) = mac.plain(FOUND);
    assert!(
        ok && out.starts_with("Journal line written for")
            && out.contains("Recovered from the hook's record of this command"),
        "{out}"
    );
    let again = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(again.starts_with("Journal line written for"), "{again}");
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        2,
        "{journal}"
    );
}
