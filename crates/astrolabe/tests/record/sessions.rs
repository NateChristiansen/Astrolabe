use super::*;

#[test]
fn a_finding_with_no_effort_is_one_journal_line_and_a_worker_saves_only_its_effort() {
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
    // A worker binds to its own effort for context and the guard, saves that effort's
    // record, and writes nothing else: the dispatcher files, journals and closes.
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe activate --effort 1 --worker --model opus --reasoning medium",
    );
    assert!(
        ok && out.contains("worker under that effort")
            && out.contains("No journal line")
            && !out.contains("--journal")
            && !out.contains("initiative new"),
        "{out}"
    );
    for cmd in [
        NEW,
        "astrolabe effort save --journal \"x\"",
        "astrolabe effort save --effort 2 --status done",
        "astrolabe journal \"x\"",
    ] {
        let (out, ok) = mac.bash("w1", cmd);
        assert!(
            !ok && out.contains("worker under effort 1") && out.contains("journal line"),
            "{cmd}: {out}"
        );
    }
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe effort save --status active --decision \"ink over paint: cheaper\" <<'ASTROLABE'\nHalf done.\n\nNext: the rest.\nASTROLABE\n",
    );
    assert!(ok && out.contains("Saved effort 1"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("Half done.") && text.contains(": ink over paint: cheaper"),
        "{text}"
    );
    assert_eq!(
        mac.journal_today().lines().count(),
        2,
        "a worker journals nothing"
    );
    // A patch that quotes one is an edit: Codex's patch tool carries the patch as `command`,
    // and this context line, run by a shell, would be a real command substitution.
    let patch = "*** Begin Patch\n*** Update File: docs/commands.md\n@@\n `astrolabe effort save` updates the bound effort.\n+It says when the notes have no remote.\n*** End Patch\n";
    let input = json!({"session_id": "w1", "cwd": mac.demo, "tool_name": "apply_patch", "tool_use_id": "call-2", "tool_input": {"command": patch}});
    assert_eq!(mac.hook_json("pre-command", input), None);
    // Through a shell, the same words are still claimed and refused.
    let (out, ok) = mac.bash("w1", &format!("echo {}", patch.lines().nth(3).unwrap()));
    assert!(!ok && out.contains("refused"), "{out}");
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
    // Its edits nudge a save like any session's, since its effort's record is its own.
    mac.edit("w1", "Edit");
    assert!(mac.stop("w1", false, false).is_some());
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

/// Catches: a session in plan mode holding back from `effort new`, because the start text no
/// longer said plan mode allows it (the slimmed start text of 2026-11-06 dropped it), and a
/// turn-end nudge that calls the plan file an edit.
#[test]
fn plan_mode_is_no_reason_to_skip_the_effort_and_a_plan_alone_is_nudged_as_a_plan() {
    let world = World::new();
    let mac = world.machine("mac");
    let start = mac.session_start("s1", "startup");
    assert!(
        start.contains("Plan mode doesn't block this command")
            && start.contains("create the effort before writing the plan"),
        "{start}"
    );
    let write = |session: &str, file: &str, mode: &str| {
        let input = json!({"session_id": session, "cwd": mac.demo, "permission_mode": mode, "tool_name": "Write", "tool_input": {"file_path": file}, "tool_response": {}});
        assert!(mac.hook_json("post-tool", input).is_none());
    };
    let said = |session: &str| {
        let nudge = mac.stop(session, false, false).unwrap();
        nudge["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    // The plan file, in plan mode: the plan text, which says plan mode allows the fix.
    write("s1", "/home/o/.claude/plans/a-plan.md", "plan");
    let text = said("s1");
    assert!(
        text.contains("a plan was written") && text.contains("plan mode doesn't block it"),
        "{text}"
    );
    assert!(mac.stop("s1", false, false).is_none(), "said once");
    // A plan file is known by its folder too, whatever the mode says.
    write("s2", "C:/Users/o/.claude/plans/b.md", "default");
    assert!(said("s2").contains("a plan was written"));
    // In plan mode the plan file is the only file the app lets a session change.
    write("s3", "/elsewhere/plan.md", "plan");
    assert!(said("s3").contains("a plan was written"));
    // A plan and then real work: the edit text, as before.
    write("s4", "/home/o/.claude/plans/c.md", "plan");
    write("s4", "/repo/src/main.rs", "acceptEdits");
    let text = said("s4");
    assert!(text.contains("files were edited"), "{text}");
    // Real work first stays real work after a plan write.
    write("s5", "/repo/src/main.rs", "default");
    write("s5", "/home/o/.claude/plans/d.md", "plan");
    assert!(said("s5").contains("files were edited"));
    // An app that names no mode, editing a file outside any plans folder: as before.
    mac.edit("s6", "Edit");
    assert!(said("s6").contains("files were edited"));
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
fn many_untracked_files_still_move_the_print_without_reading_them_all() {
    let dir = TempDir::new();
    let repo = dir.0.join("proj");
    std::fs::create_dir_all(repo.join("node_modules")).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
    // More files than the print reads whole: the last ones count by size and time.
    for i in 0..400 {
        write(&repo.join(format!("node_modules/{i:03}.js")), "x");
    }
    let print = || {
        astrolabe::notes::RepoFacts::read(&repo)
            .unwrap()
            .fingerprint
    };
    let first = print();
    assert_eq!(first, print(), "the same tree, the same print");
    write(&repo.join("node_modules/399.js"), "xy");
    assert_ne!(first, print(), "an edit past the files read whole");
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

/// Catches: a Codex agent asked again, on loading its new effort once more, to undo a rename
/// the owner made after the first.
#[test]
fn a_new_effort_asks_a_codex_agent_to_rename_its_thread_once() {
    let world = World::new();
    let mac = world.machine("mac");
    let (text, ok) = mac.bash_codex("c1", NEW);
    assert!(ok, "{text}");
    let number = text
        .strip_prefix("Effort ")
        .and_then(|t| t.split_whitespace().next())
        .unwrap()
        .to_owned();
    let wanted = format!("Rename this thread to \"{number} Smooth ink\"");
    assert!(text.contains(&wanted), "{text}");
    let (again, ok) = mac.bash_codex("c1", &format!("astrolabe activate --effort {number}"));
    assert!(ok && !again.contains("set_thread_title"), "{again}");
    // A Claude agent is told to use its own app's tool.
    let (text, ok) = mac.bash("s1", NEW);
    assert!(
        ok && text.contains("set_session_title") && !text.contains("set_thread_title"),
        "{text}"
    );
}

/// Catches: a session renamed for an effort it only filed for later, not one it took up.
#[test]
fn filing_an_effort_for_later_keeps_the_session_name_until_it_is_picked_up() {
    let world = World::new();
    let mac = world.machine("mac");
    let (text, ok) = mac.bash("s1", NEW);
    assert!(ok && text.contains("set_session_title"), "{text}");
    let named = |current: &str| {
        let input =
            json!({"session_id": "s1", "cwd": mac.demo, "prompt": "hi", "session_title": current});
        mac.hook_json("prompt", input)
    };
    let first = text
        .strip_prefix("Effort ")
        .and_then(|t| t.split_whitespace().next())
        .unwrap()
        .to_owned();
    let mine = format!("{first} Smooth ink");
    assert_eq!(named(&mine), None, "already called that by its agent");
    let later = "astrolabe effort new --title \"Later\" --status proposed <<'ASTROLABE'\nfor later\nASTROLABE\n";
    let (text, ok) = mac.bash("s1", later);
    assert!(ok && !text.contains("Rename this session"), "{text}");
    assert_eq!(named(&mine), None, "the name it had stays");
    let resumed =
        json!({"session_id": "s1", "cwd": mac.demo, "source": "resume", "session_title": mine});
    let resumed = mac.hook_json("session-start", resumed).unwrap();
    assert!(
        resumed["hookSpecificOutput"].get("sessionTitle").is_none(),
        "{resumed}"
    );
    // Picked up by name: now it takes the effort's name.
    let number = text
        .strip_prefix("Effort ")
        .and_then(|t| t.split_whitespace().next())
        .unwrap()
        .to_owned();
    let (text, ok) = mac.bash("s1", &format!("astrolabe activate --effort {number}"));
    let wanted = format!("Rename this session to \"{number} Later\"");
    assert!(ok && text.contains(&wanted), "{text}");
}

/// Every place the hook decides a command is astrolabe's asks the one question, so the
/// program by its `.exe` name or a full path is handled in the hook like the bare name,
/// not let through to run.
#[test]
fn the_hook_takes_astrolabe_by_its_exe_name_or_a_path() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let hook = |tool: &str, command: String| {
        let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": tool, "tool_use_id": "call-9", "tool_input": {"command": command}});
        let answer = mac
            .hook_json("pre-command", input)
            .unwrap_or_else(|| panic!("the hook let it through to run: {command}"));
        answer["hookSpecificOutput"].clone()
    };
    // What the rewritten command reads.
    let shown = |answer: &Value| {
        let read = answer["updatedInput"]["command"].as_str().unwrap();
        let path = (read.strip_prefix("cat '"))
            .or_else(|| read.strip_prefix("Get-Content -Raw -Encoding UTF8 -LiteralPath '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{answer}"));
        std::fs::read_to_string(path).unwrap()
    };
    for (tool, program) in [
        ("Bash", "astrolabe"),
        ("Bash", "astrolabe.exe"),
        ("Bash", "/Users/x/.local/bin/astrolabe"),
        ("Bash", "C:/Users/x/.local/bin/astrolabe.exe"),
        ("PowerShell", "C:\\Users\\x\\.local\\bin\\astrolabe.exe"),
        ("PowerShell", "& 'C:\\Users\\x\\.local\\bin\\astrolabe.exe'"),
    ] {
        // The machine switch is refused.
        let answer = hook(tool, format!("{program} off"));
        let why = answer["permissionDecisionReason"].as_str();
        assert!(
            why.is_some_and(|why| why.starts_with("The machine switch is the owner's")),
            "{program}: {answer}"
        );
        // The session's status is shown.
        let answer = hook(tool, format!("{program} activate --status"));
        let status = shown(&answer);
        assert!(
            status.starts_with("Astrolabe is on for effort 1"),
            "{program}: {status}"
        );
        // An approval is recorded, or refused in the hook, never run as a program.
        let allow =
            format!("{program} allow --once \"git push --force\" --said \"Yes, run that.\"");
        let answer = hook(tool, allow);
        let result = shown(&answer);
        if program.starts_with('&') {
            assert!(
                result.starts_with("Approval refused:"),
                "{program}: {result}"
            );
        } else {
            assert!(
                result.starts_with("Recorded the claimed owner approval."),
                "{program}: {result}"
            );
        }
    }
    // Another program with a like name is not astrolabe.
    for program in ["notastrolabe", "astrolabe.exe.bak", "/x/astrolabe2"] {
        let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": "Bash", "tool_use_id": "call-9", "tool_input": {"command": format!("{program} activate --status")}});
        assert_eq!(mac.hook_json("pre-command", input), None, "{program}");
    }
}

/// The hook finds an astrolabe command the way it finds an effort command: wherever it
/// sits in the line, behind an assignment or `time`, with its words quoted or not. So the
/// machine switch, an approval and the session switch never reach the shell by a spelling
/// the effort commands are already caught in.
#[test]
fn the_hook_takes_an_astrolabe_command_wherever_it_sits_in_the_line() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let hook = |command: &str| {
        let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": "Bash", "tool_use_id": "call-9", "tool_input": {"command": command}});
        mac.hook_json("pre-command", input)
            .map(|answer| answer["hookSpecificOutput"].clone())
    };
    let shown = |answer: &Value| {
        let read = answer["updatedInput"]["command"].as_str().unwrap();
        let path = (read.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{answer}"));
        std::fs::read_to_string(path).unwrap()
    };
    for off in [
        "FOO=1 astrolabe off",
        "time astrolabe off",
        "cd x && astrolabe off",
        "astrolabe \"off\"",
        "astrolabe 'on'",
        "true; /Users/x/.local/bin/astrolabe.exe off",
    ] {
        let answer = hook(off).unwrap_or_else(|| panic!("the hook let it through: {off}"));
        let why = answer["permissionDecisionReason"].as_str();
        assert!(
            why.is_some_and(|why| why.starts_with("The machine switch is the owner's")),
            "{off}: {answer}"
        );
    }
    // A session switch is taken only when it's run alone, as before: with shell around it,
    // it runs bare and the program refuses it. Its words may be quoted.
    for switch in [
        "cd x && astrolabe activate --effort 1",
        "FOO=1 astrolabe activate --status",
        "astrolabe activate --status; ls",
    ] {
        assert_eq!(hook(switch), None, "{switch}");
    }
    let answer = hook("astrolabe \"activate\" \"--status\"").unwrap();
    assert!(shown(&answer).starts_with("Astrolabe is on for effort 1"));
    // An approval that isn't run alone is refused in the hook.
    let allow = "cd x && astrolabe allow --once \"git push --force\" --said \"Yes, run that.\"";
    let answer = hook(allow).unwrap_or_else(|| panic!("the hook let it through: {allow}"));
    let text = shown(&answer);
    assert!(text.starts_with("Approval refused:"), "{text}");
    // Text that only names a command is not one.
    for text in [
        "echo \"astrolabe off\"",
        "echo 'run astrolabe activate --off alone'",
        "grep -n \"astrolabe allow\" README.md # astrolabe off",
    ] {
        assert_eq!(hook(text), None, "{text}");
    }
}
