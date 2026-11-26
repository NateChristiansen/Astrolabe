//! Saves that find another writer holding the notes lock wait their turn: each is planned and
//! queued, then committed by whoever holds the lock, in its own commit, or left out when its
//! effort changed while it waited. Every reply says which.
use super::*;
use std::time::{Duration, Instant};

const SAVE: &str = "astrolabe effort save --journal \"a queued line\"";

/// Waits for `done`, failing the test if it doesn't come true.
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(30), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Machine {
    /// The queued saves waiting in the state folder, by key.
    fn queued(&self) -> Vec<Value> {
        (std::fs::read_dir(self.state().join("ops")).into_iter())
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok())
            .filter(|op: &Value| op["queued"] == true)
            .collect()
    }

    /// How many commits carry this key.
    fn commits_with(&self, key: &str) -> usize {
        let log = git(&self.notes, &["log", "--format=%B"]);
        log.matches(&format!("Op-Key: {key}\n")).count()
    }

    fn notes_lib(&self) -> astrolabe::notes::Notes {
        astrolabe::notes::Notes::new(self.notes.clone(), self.state())
    }

    /// A Bash command through the pre-command hook as its own tool call, with extra
    /// environment: the ticket's text and whether it worked. A hook still running at `limit`
    /// fails the test.
    fn timed_bash(
        &self,
        session: &str,
        call: &str,
        command: &str,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (String, bool) {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let mut hook = self.command(&self.demo, &["hook", "pre-command", "--if-active"]);
        hook.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            hook.env(key, value);
        }
        let (output, _) =
            common::within(hook.spawn().unwrap(), input.to_string().into_bytes(), limit)
                .unwrap_or_else(|| panic!("the hook was still running after {limit:?}"));
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
        let shown = answer["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("a ticket, not {answer}"));
        let read = (shown.strip_suffix("; false")).unwrap_or(shown);
        let path = (read.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        let ticket: Value =
            serde_json::from_str(&std::fs::read_to_string(path.replace(".txt", ".json")).unwrap())
                .unwrap();
        (std::fs::read_to_string(path).unwrap(), ticket["ok"] == true)
    }
}

/// The defect: a save that couldn't get the lock waited for it, and behind a holder busy
/// for longer than its hook could wait it was refused, though nothing of it needed more than
/// a commit. Here the lock's holder commits three waiting saves without letting go of it.
#[test]
fn saves_waiting_for_the_lock_are_committed_by_its_holder_each_in_its_own_commit() {
    let world = World::new();
    let mac = world.machine("mac");
    for s in ["s1", "s2", "s3"] {
        assert!(mac.bash(s, NEW).1);
    }
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    let replies = std::thread::scope(|scope| {
        let mac = &mac;
        let saves: Vec<_> = ["s1", "s2", "s3"]
            .map(|s| scope.spawn(move || mac.bash(s, SAVE)))
            .into_iter()
            .collect();
        // The holder, still holding the lock, commits each as it's queued: waiting for all
        // three first let the first save's wait for the lock run out on a busy machine.
        let mut keys = std::collections::BTreeSet::new();
        until("three saves committed by the holder", || {
            keys.extend((mac.queued().iter()).map(|op| op["key"].as_str().unwrap().to_owned()));
            assert!(notes.reconcile().is_empty());
            keys.len() == 3 && keys.iter().all(|key| mac.commits_with(key) == 1)
        });
        let replies: Vec<_> = saves.into_iter().map(|s| s.join().unwrap()).collect();
        for key in &keys {
            assert_eq!(mac.commits_with(key), 1, "{key}");
        }
        replies
    });
    drop(held);
    for (n, (out, ok)) in replies.iter().enumerate() {
        assert!(
            *ok && out.starts_with(&format!("Saved effort {}: status ", n + 1)),
            "{out}"
        );
        assert!(out.contains("journal line written"), "{out}");
    }
    assert_eq!(mac.journal_today().matches("a queued line").count(), 3);
    assert!(
        git(&mac.notes, &["status", "--porcelain"])
            .trim()
            .is_empty()
    );
    // Each session's next save goes straight through: its delivered note is the one queued.
    for s in ["s1", "s2", "s3"] {
        let (out, ok) = mac.bash(s, "astrolabe effort save --journal \"after\"");
        assert!(ok, "{out}");
    }
}

/// The defect a queue invites: a save planned without the lock and committed later would
/// write over a save that landed in between. The queued save is checked again when its
/// turn comes, left out, and its session is shown the effort as it is now.
#[test]
fn a_queued_save_whose_effort_changed_is_left_out_and_its_session_told() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    let (out, ok) = std::thread::scope(|scope| {
        let save = scope.spawn(|| {
            mac.bash(
                "s1",
                "astrolabe effort save <<'ASTROLABE'\nTHE QUEUED TEXT\nASTROLABE",
            )
        });
        until("the save queued", || mac.queued().len() == 1);
        // Another write lands on the effort while the save waits.
        let rel = astrolabe::notes::effort_files(&mac.notes)[0].1.clone();
        let path = mac.notes.join(&rel);
        let text = std::fs::read_to_string(&path).unwrap();
        let changed = text.replacen("smooth the ink", "LANDED IN BETWEEN, smooth the ink", 1);
        assert_ne!(text, changed);
        write(&path, &changed);
        git(&mac.notes, &["commit", "-qam", "a write in between"]);
        let log = mac.log();
        assert!(notes.reconcile().is_empty());
        assert_eq!(mac.log(), log, "nothing committed");
        save.join().unwrap()
    });
    drop(held);
    assert!(
        !ok && out.starts_with("Effort NOT saved: effort 1 changed while this save waited its turn behind other saves on this machine, so this save was left out and nothing was overwritten. Its Current state now:"),
        "{out}"
    );
    let text = mac.effort_text(1);
    assert!(text.contains("LANDED IN BETWEEN") && !text.contains("THE QUEUED TEXT"));
    // Shown the effort anew, the session's next save goes through.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nTHE QUEUED TEXT, FOLDED IN\nASTROLABE",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("THE QUEUED TEXT, FOLDED IN"));
}

/// The defects at a hook's end: a save still queued when its hook runs out of time told
/// "saved" or "not saved" (it's neither yet), and a holder whose hook runs out partway
/// through the queue losing or doubling a save. Each timed-out save says it's queued and
/// kept; a holder short of time commits what it can and leaves the rest; whatever commits
/// the rest commits each once; and the same command again says it landed.
#[test]
fn saves_left_queued_by_hooks_out_of_time_land_once_whoever_commits_them() {
    let world = World::new();
    let mac = world.machine("mac");
    for s in ["s1", "s2", "s3", "s4"] {
        assert!(mac.bash(s, NEW).1);
    }
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "4000")];
    let replies = std::thread::scope(|scope| {
        let saves: Vec<_> = ["s1", "s2", "s3"]
            .map(|s| {
                let mac = &mac;
                scope.spawn(move || {
                    mac.timed_bash(s, &format!("{s}-call"), SAVE, &env, Duration::from_secs(20))
                })
            })
            .into_iter()
            .collect();
        saves
            .into_iter()
            .map(|s| s.join().unwrap())
            .collect::<Vec<_>>()
    });
    drop(held);
    for (n, (out, ok)) in replies.iter().enumerate() {
        assert!(
            !ok && out.starts_with(&format!("Effort {} not saved yet: other saves on this machine held the notes for the ", n + 1))
                && out.contains("so this save is queued behind them. It's committed only if the effort still reads as this save found it. The write is kept, and the next notes write tries it again: run the same command again to see whether it landed, changing nothing in it."),
            "{out}"
        );
    }
    let keys: Vec<String> = (mac.queued().iter())
        .map(|op| op["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys.len(), 3);
    // A holder whose hook has time for one more commit: its first commit starts 9 s into
    // its 16 however long it took to get there, and each commit takes a second, which
    // leaves too little for a second queued save and enough for its own. So s4's save
    // commits the oldest queued one, then its own, and leaves the other two queued.
    let env = [("ASTROLABE_HOOK_TEST_STALL", "commit:=9000,commit:+1000")];
    let (out, ok) = mac.timed_bash("s4", "s4-call", SAVE, &env, Duration::from_secs(20));
    assert!(ok && out.starts_with("Saved effort 4"), "{out}");
    let committed: Vec<_> = keys.iter().filter(|k| mac.commits_with(k) == 1).collect();
    assert_eq!(committed.len(), 1, "{committed:?}");
    assert!(keys.iter().all(|k| mac.commits_with(k) <= 1));
    // A sync, which isn't a hook, commits the rest.
    let (out, ok) = mac.run(&mac.notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    for key in &keys {
        assert_eq!(mac.commits_with(key), 1, "{key}");
    }
    // The same command again is told it landed, and writes nothing more.
    for s in ["s1", "s2", "s3"] {
        let log = mac.log();
        let (out, ok) = mac.bash(s, SAVE);
        assert!(ok && out.starts_with("Already saved"), "{out}");
        assert_eq!(mac.log(), log);
    }
    assert_eq!(mac.journal_today().matches("a queued line").count(), 4);
}

/// The same command run again while its first attempt is still queued waits for that one,
/// so it lands once: planned again, it would either land twice or be left out against its
/// own first attempt.
#[test]
fn the_same_save_again_while_its_first_is_queued_lands_once() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "4000")];
    let (out, ok) = mac.timed_bash("s1", "first", SAVE, &env, Duration::from_secs(20));
    assert!(!ok && out.contains("not saved yet"), "{out}");
    let key = mac.queued()[0]["key"].as_str().unwrap().to_owned();
    let (out, ok) = std::thread::scope(|scope| {
        let retry = scope.spawn(|| mac.bash("s1", SAVE));
        // The retry waits on the first attempt and queues nothing of its own. It has 9 s
        // for the lock; 3 s leaves a busy machine time to start it and reach that wait.
        std::thread::sleep(Duration::from_millis(3000));
        assert_eq!(mac.queued().len(), 1);
        drop(held);
        retry.join().unwrap()
    });
    assert!(ok && out.starts_with("Saved effort 1"), "{out}");
    assert_eq!(mac.commits_with(&key), 1);
    assert_eq!(mac.journal_today().matches("a queued line").count(), 1);
}

/// A queued save is a local commit like any other: another machine's push in between is
/// merged in by the sync, and both reach origin.
#[test]
fn another_machines_push_between_a_queued_save_and_its_sync_still_merges() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.run(&mac.notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    let (out, ok) = pc.run(&pc.notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    assert!(pc.bash("p1", NEW).1);
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    std::thread::scope(|scope| {
        let save = scope.spawn(|| mac.bash("s1", SAVE));
        until("the save queued", || mac.queued().len() == 1);
        assert!(notes.reconcile().is_empty());
        let (out, ok) = save.join().unwrap();
        assert!(ok && out.starts_with("Saved effort 1"), "{out}");
    });
    drop(held);
    // The other machine pushes before this one's sync runs.
    let (out, ok) = pc.run(&pc.notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    let mut out = String::new();
    until("this machine's sync pushed", || {
        let (said, ok) = mac.run(&mac.notes, &["notes", "sync"], "");
        out = said;
        ok && out.contains("pushed")
    });
    let origin = world.origin.0.join("notes.git");
    let pushed = git(&origin, &["log", "-p", "main"]);
    assert!(pushed.contains("a queued line"), "{pushed}");
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 2);
}

/// The defect: a save queued while a sync rebased planned from the folder as the rebase left
/// it, origin's older copy, so it was refused as changed since delivered and had that older
/// copy delivered, which refused the session's next save too. Planned without the lock, a
/// note that reads as changed sends the save to wait for the lock and plan again under it.
#[test]
fn a_save_queued_during_a_rebase_waits_for_the_lock_instead_of_reading_it_as_changed() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.run(&mac.notes, &["notes", "sync"], "").1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nSECOND TEXT\nASTROLABE",
    );
    assert!(ok, "{out}");
    let rel = astrolabe::notes::effort_files(&mac.notes)[0].1.clone();
    let path = mac.notes.join(&rel);
    let mine = std::fs::read_to_string(&path).unwrap();
    let origin = git(&mac.notes, &["show", &format!("origin/main:{rel}")]);
    assert!(
        !origin.contains("SECOND TEXT"),
        "origin already has the second save"
    );
    let notes = mac.notes_lib();
    let held = notes.lock().unwrap();
    // A rebase's first checkout: the effort reads as origin has it.
    write(&path, &origin);
    let (out, ok) = std::thread::scope(|scope| {
        let mac = &mac;
        let save = scope.spawn(move || {
            mac.bash(
                "s1",
                "astrolabe effort save <<'ASTROLABE'\nTHIRD TEXT\nASTROLABE",
            )
        });
        std::thread::sleep(Duration::from_millis(1500));
        // The rebase replays the local commit and lets go.
        write(&path, &mine);
        drop(held);
        save.join().unwrap()
    });
    assert!(ok && out.starts_with("Saved"), "{out}");
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("THIRD TEXT")
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nFOURTH TEXT\nASTROLABE",
    );
    assert!(ok && out.starts_with("Saved"), "{out}");
}
