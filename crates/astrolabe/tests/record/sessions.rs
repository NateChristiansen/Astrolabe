use super::*;

#[test]
fn a_finding_with_no_effort_is_one_journal_line_and_a_worker_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    // A question answered now: no effort, one sentence in the day file, tagged with the repo.
    let (out, ok) = mac.bash("s1", "astrolabe journal \"Found the Deck preset in the video settings: everything Low, render scale 0.75.\"");
    assert!(ok && out.contains("Journal line written"), "{out}");
    let journal = mac.journal_today();
    assert!(
        journal.starts_with("- Found the Deck preset in the video settings: everything Low, render scale 0.75. <!-- demo mac "),
        "{journal}"
    );
    assert_eq!(mac.log().lines().next(), Some("journal: demo"));
    // Findings don't replace each other: two in a day are two lines.
    assert!(mac.bash("s1", "astrolabe journal <<'ASTROLABE'\nWalk is 3.0 and sprint 4.8, in the player motor.\nASTROLABE\n").1);
    assert_eq!(mac.journal_today().lines().count(), 2);
    let (out, ok) = mac.bash("s1", "astrolabe journal --title x");
    assert!(!ok && out.contains("only --repo"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe journal");
    assert!(!ok && out.contains("needs the sentence"), "{out}");
    // No hook: the plain command explains itself like an effort command does.
    let (plain, ok) = mac.run(&mac.demo, &["journal", "Something new"], "");
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
    // With an effort, the line belongs on the effort.
    assert!(mac.bash("s2", NEW).1);
    let (out, ok) = mac.bash("s2", "astrolabe journal \"x\"");
    assert!(!ok && out.contains("--journal"), "{out}");
    // A worker binds to the dispatcher's effort for context and the guard, and writes nothing.
    let (out, ok) = mac.bash("w1", "astrolabe activate --effort 1 --worker");
    assert!(ok && out.contains("worker under that effort"), "{out}");
    for cmd in [
        NEW,
        "astrolabe effort save --journal \"x\"",
        "astrolabe journal \"x\"",
    ] {
        let (out, ok) = mac.bash("w1", cmd);
        assert!(!ok && out.contains("worker under effort 1"), "{out}");
    }
    // Still a worker after a resume and after loading the effort again without the flag.
    assert!(
        mac.session_start("w1", "resume")
            .contains("worker under that effort")
    );
    let (out, ok) = mac.bash("w1", "astrolabe activate --effort 1");
    assert!(ok && out.contains("worker under that effort"), "{out}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("w1.json")).unwrap()).unwrap();
    assert_eq!(state["worker"], true);
    // Its edits never nudge.
    mac.edit("w1", "Edit");
    assert!(mac.stop("w1", false, false).is_none());
    assert!(mac.stop("w1", false, true).is_none());
}

#[test]
fn the_stop_nudge_fires_once_per_change_on_both_clients_and_never_loops() {
    let world = World::new();
    let mac = world.machine("mac");
    // Nothing bound, nothing edited: silence, however long the answer. A question answered
    // now is not an effort, and an ask here once turned a code review into a journal line.
    assert!(mac.stop("s1", false, false).is_none());
    let long = json!({"session_id": "s1", "cwd": mac.demo, "stop_hook_active": false, "last_assistant_message": "x".repeat(700)});
    assert!(mac.hook_json("stop", long).is_none());
    // An edit with no effort: one nudge to start one, then silence.
    mac.edit("s1", "Edit");
    let nudge = mac.stop("s1", false, false).unwrap();
    let text = nudge["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("no effort recorded") && text.contains("not the owner"),
        "{text}"
    );
    assert!(mac.stop("s1", false, false).is_none(), "said once");
    // Bound and saved: silence; a commit in the repo moves the fingerprint: one nudge.
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.stop("s1", false, false).is_none(),
        "baseline taken at create"
    );
    // Created, never saved, long answer: one ask, then silence until a save.
    let long = json!({"session_id": "s1", "cwd": mac.demo, "stop_hook_active": false, "last_assistant_message": "y".repeat(700)});
    let asked = mac.hook_json("stop", long.clone()).unwrap();
    assert!(
        asked["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("never saved"),
        "{asked}"
    );
    assert!(mac.hook_json("stop", long.clone()).is_none());
    assert!(mac.bash("s1", "astrolabe effort save --decision \"d0\"").1);
    assert!(mac.hook_json("stop", long).is_none(), "saved: no ask");
    write(&mac.demo.join("a.txt"), "a");
    git(&mac.demo, &["add", "a.txt"]);
    git(&mac.demo, &["commit", "-q", "-m", "a"]);
    let nudge = mac.stop("s1", false, true).unwrap();
    assert_eq!(
        nudge["decision"], "block",
        "Codex continues only by a block"
    );
    let text = nudge["reason"].as_str().unwrap();
    assert!(
        text.contains("the repo changed since the last save of effort 1")
            && text.contains("not the owner"),
        "{text}"
    );
    assert!(mac.stop("s1", true, true).is_none(), "already continued");
    assert!(
        mac.stop("s1", false, true).is_none(),
        "acknowledged: no second nudge for the same state"
    );
    // A second edit to a dirty file changes the print; a save clears everything.
    write(&mac.demo.join("a.txt"), "aa");
    assert!(mac.stop("s1", false, false).is_some());
    write(&mac.demo.join("a.txt"), "aaa");
    assert!(
        mac.stop("s1", false, false).is_some(),
        "content, not file names"
    );
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\nASTROLABE\n"
        )
        .1
    );
    assert!(mac.stop("s1", false, false).is_none(), "saved");
    // The guard's view of a commit, and an apply_patch, both mark.
    let (_, ok) = mac.bash("s1", "git commit --allow-empty -q -m nothing");
    assert!(ok);
    assert!(mac.stop("s1", false, false).is_some());
    assert!(mac.stop("s1", false, false).is_none());
    mac.edit("s1", "apply_patch");
    assert!(mac.stop("s1", false, false).is_some());
    // Session end closes the state; a fresh session on the same id still reloads it.
    assert!(
        mac.hook_json(
            "session-end",
            json!({"session_id": "s1", "cwd": mac.demo, "reason": "other"})
        )
        .is_none()
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert!(state["closed_ms"].is_number());
}

#[test]
fn a_pickup_records_the_delivered_hash_for_stale_save_detection() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A pickup in another session records what was delivered, so its save is checked too.
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s2.json")).unwrap()).unwrap();
    assert!(
        state["delivered"].is_string() && state["baseline"].is_string(),
        "{state}"
    );
    assert!(
        mac.bash("s1", "astrolabe effort save --decision \"from s1\"")
            .1
    );
    let (out, ok) = mac.bash("s2", "astrolabe effort save --decision \"from s2\"");
    assert!(
        !ok && out.contains("changed since it was delivered"),
        "{out}"
    );
}

#[test]
fn the_notes_checkout_names_the_repo_a_new_effort_is_for() {
    let world = World::new();
    let mac = world.machine("mac");
    let notes = mac.notes.clone();
    let (out, ok) = mac.bash_from("n1", &notes, NEW, None);
    assert!(!ok && out.contains("--repo <name>"), "{out}");
    assert!(!notes.join(shelf("0001-smooth-ink.md")).exists());
    let (out, ok) = mac.bash_from(
        "n1",
        &notes,
        "astrolabe effort new --title \"Seat\" --repo nope <<'ASTROLABE'\ngoal\nASTROLABE\n",
        None,
    );
    assert!(!ok && out.contains("repo add nope"), "{out}");
    let (out, ok) = mac.bash_from(
        "n1",
        &notes,
        "astrolabe effort new --title \"Seat\" --repo demo <<'ASTROLABE'\ngoal\nASTROLABE\n",
        None,
    );
    assert!(ok && out.contains("Effort 1 created"), "{out}");
    assert!(
        mac.effort_text(1).contains("repo: demo"),
        "{}",
        mac.effort_text(1)
    );
    // The seat's own saves commit under the session; that isn't work moving, so no nudge.
    let stop = || json!({"session_id": "n1", "cwd": notes, "stop_hook_active": false, "last_assistant_message": "done"});
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "a create is no work"
    );
    assert!(
        mac.bash_from(
            "n1",
            &notes,
            "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree.\nASTROLABE\n",
            None
        )
        .1
    );
    assert!(mac.hook_json("stop", stop()).is_none(), "a save is no work");
    // A journal edited by hand nudges once; the save that then commits it doesn't.
    let day = notes
        .join("journal")
        .join(format!("{}.md", chrono::Local::now().format("%Y-%m-%d")));
    write(&day, "- Hand line.\n");
    assert!(
        mac.hook_json("stop", stop()).is_some(),
        "a hand edit is work"
    );
    assert!(
        mac.bash_from(
            "n1",
            &notes,
            "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree more.\nASTROLABE\n",
            None
        )
        .1
    );
    assert!(
        git(&notes, &["status", "--porcelain"]).trim().is_empty(),
        "the save committed the hand edit"
    );
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "its own commit isn't work"
    );
    // A save cut off between its commit and its state: the replay takes the baseline
    // again too, so the commit it carried never nudges.
    let state_path = mac.state().join("n1.json");
    let mut stale: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    stale["baseline"] = "stale".into();
    let intent = json!({
        "key": "cut-key", "files": [], "renames": [], "journal": [], "message": "0001: checkpoint",
        "base": null, "states": [[state_path, stale]], "committed": true, "rebaseline": notes,
    });
    write(&mac.state().join("ops/cut-key.json"), &intent.to_string());
    let (out, ok) = mac.run(&notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    let replayed: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_ne!(replayed["baseline"], "stale", "{replayed}");
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "the replayed save isn't work"
    );
    write(
        &notes.join("knowledge/owner.md"),
        "OWNER-TEXT edited by hand",
    );
    assert!(
        mac.hook_json("stop", stop()).is_some(),
        "a hand edit in the notes is"
    );
    assert!(mac.hook_json("stop", stop()).is_none(), "said once");
    // A finding with no effort names its repo the same way, from another session.
    let (out, ok) = mac.bash_from("n2", &notes, "astrolabe journal \"Found a thing.\"", None);
    assert!(!ok && out.contains("--repo <name>"), "{out}");
    let (out, ok) = mac.bash_from(
        "n2",
        &notes,
        "astrolabe journal --repo demo \"Found a thing.\"",
        None,
    );
    assert!(ok, "{out}");
    assert!(
        mac.journal_today().contains("Found a thing. <!-- demo "),
        "{}",
        mac.journal_today()
    );
    let (out, ok) = mac.bash_from("n2", &notes, "astrolabe journal --tick x \"No.\"", None);
    assert!(!ok && out.contains("only --repo"), "{out}");
    // Stamped by the work's repo, not the notes': the seat's branch and head say nothing.
    let text = mac.effort_text(1);
    assert!(
        text.contains("branch: \"\"") || text.contains("branch:\n") || text.contains("branch: ''"),
        "{text}"
    );
    assert!(
        !text.contains("head: ") || text.contains("head: \"\"") || text.contains("head: ''"),
        "{text}"
    );
    // In the repo's own folder the folder says which, so --repo is refused there.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Two\" --repo demo <<'ASTROLABE'\ngoal\nASTROLABE\n",
    );
    assert!(!ok && out.contains("drop --repo"), "{out}");
}

#[test]
fn a_shell_edit_to_an_untracked_file_moves_the_print_with_spaces_and_from_a_subfolder() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.stop("s1", false, false).is_none(), "baseline at create");
    // A file with a space in its name, edited twice by shell, no edit tool marking it.
    let (_, ok) = mac.bash("s1", "printf a > 'b c.txt'");
    assert!(ok);
    assert!(mac.stop("s1", false, false).is_some(), "new file");
    let (_, ok) = mac.bash("s1", "printf aa > 'b c.txt'");
    assert!(ok);
    assert!(
        mac.stop("s1", false, false).is_some(),
        "second edit, same name"
    );
    // From a subfolder: paths come from the repo root, not the session's folder.
    let sub = mac.demo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let stop_in = |dir: &Path| {
        mac.hook_json(
            "stop",
            json!({"session_id": "s1", "cwd": dir, "stop_hook_active": false, "last_assistant_message": "done"}),
        )
    };
    write(&sub.join("x.txt"), "x");
    assert!(stop_in(&sub).is_some(), "new file in a subfolder");
    write(&sub.join("x.txt"), "xx");
    assert!(stop_in(&sub).is_some(), "second edit from the subfolder");
    assert!(stop_in(&sub).is_none(), "said once per change");
}

#[test]
fn activation_reads_the_same_current_state_a_save_does() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let rel = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 1)
        .unwrap()
        .1;
    let text = mac.effort_text(1);
    let (head, rest) = text.split_once("\n## Current state\n").unwrap();
    let edited = format!(
        "{head}\n\nHow a note looks:\n\n```md\n## Current state\nFENCED EXAMPLE\n```\n\n## Current state\n{rest}"
    );
    write(&mac.notes.join(&rel), &edited);
    git(&mac.notes, &["commit", "-qam", "hand edit"]);
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let shown = out.split(", Current state\n").nth(1).unwrap_or_default();
    assert!(shown.contains("smooth the ink"), "{out}");
    assert!(!shown.contains("FENCED EXAMPLE"), "{out}");
    // A save from that delivery agrees, and keeps the example as text.
    let (out, ok) = mac.bash("s2", "astrolabe effort save --decision \"agreed\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("smooth the ink"));
}

#[test]
fn the_print_reads_untracked_files_under_a_root_ending_in_a_space_and_odd_byte_names() {
    let dir = TempDir::new();
    // Windows drops a trailing space from a folder name, so only Unix can have such a root.
    let repo = dir.0.join(if cfg!(unix) { "proj " } else { "proj" });
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let print = || {
        astrolabe::notes::RepoFacts::read(&repo)
            .unwrap()
            .fingerprint
    };
    write(&repo.join("f.txt"), "a");
    let first = print();
    write(&repo.join("f.txt"), "b");
    assert_ne!(first, print(), "content read from the real root");
    // Names that aren't UTF-8, where the filesystem allows them (not APFS).
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = |b: &[u8]| repo.join(std::ffi::OsStr::from_bytes(b));
        if std::fs::write(name(b"n-\xff"), "x").is_ok() {
            let before = print();
            std::fs::rename(name(b"n-\xff"), name(b"n-\xfe")).unwrap();
            assert_ne!(before, print(), "two names, two prints");
            let before = print();
            std::fs::write(name(b"n-\xfe"), "xy").unwrap();
            assert_ne!(before, print(), "its content counts");
        }
    }
}

#[test]
fn a_command_that_only_names_an_effort_command_runs_as_itself() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.effort_text(1);
    let journal = mac.journal_today();
    // Quoted text, a heredoc body and a comment aren't commands the shell runs, so the hook
    // leaves them to the shell and writes nothing.
    for (cmd, printed) in [
        (
            "echo \"next: astrolabe effort save --status done\"",
            "next: astrolabe effort save --status done",
        ),
        (
            "cat <<'EOF'\nastrolabe journal \"x\"\nEOF",
            "astrolabe journal \"x\"",
        ),
        ("echo ok # astrolabe effort save", "ok"),
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(ok && out.trim() == printed, "{cmd:?}: {out}");
    }
    assert_eq!(mac.effort_text(1), before);
    assert_eq!(mac.journal_today(), journal);
    // Chained after another command, it's still claimed and refused with the forms rather
    // than run by whatever `astrolabe` is on PATH.
    let (out, ok) = mac.bash("s1", "true && astrolabe effort save --status done");
    assert!(!ok && out.contains("Accepted forms"), "{out}");
}
