use super::*;

#[test]
fn an_unfinished_write_is_finished_by_the_next_one_and_a_retry_commits_once() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A crash after the file write, before the commit: the file is there, nothing committed,
    // the intent remains.
    let intent = json!({
        "key": "crashed-key", "files": [[shelf("0001-smooth-ink.md"), std::fs::read_to_string(mac.notes.join(shelf("0001-smooth-ink.md"))).unwrap().replace("smooth the ink", "CRASHED smooth the ink")]],
        // The first journal shape, as an intent written by the build before this one.
        "renames": [], "journal": [{"id": "crashed-key0", "kind": "decision", "effort": "0001", "title": "Smooth ink", "text": "crash line", "machine": "mac", "provider": "claude", "time": "01:00", "day": "2026-01-01"}],
        "message": "0001: checkpoint", "base": null, "states": []
    });
    write(
        &mac.state().join("ops/crashed-key.json"),
        &intent.to_string(),
    );
    std::fs::write(
        mac.notes.join(shelf("0001-smooth-ink.md")),
        intent["files"][0][1].as_str().unwrap(),
    )
    .unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"after the crash\"");
    // The delivered hash no longer matches, because the crashed write landed: refused, and
    // the leftover was still finished first.
    assert!(
        !ok && out.contains("changed since it was delivered"),
        "{out}"
    );
    let finished: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("ops/crashed-key.json")).unwrap())
            .unwrap();
    assert!(
        finished["committed"] == true && finished["result"].is_string(),
        "intent finished: {finished}"
    );
    let log = mac.log();
    assert!(
        log.starts_with("0001: checkpoint\n0001: started\n"),
        "{log}"
    );
    assert!(
        std::fs::read_to_string(mac.notes.join(astrolabe::notes::journal_file("2026-01-01")))
            .unwrap()
            .contains("crash line")
    );
    // The same tool call again (a lost reply) commits nothing new.
    let before = mac.log();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"after the crash\"");
    assert!(ok, "{out}");
    let after = mac.log();
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    let (_, ok) = mac.bash("s1", "astrolabe effort save --journal \"after the crash\"");
    assert!(ok);
    assert_eq!(
        mac.log().lines().count(),
        after.lines().count(),
        "same tool_use_id, same key: no second commit"
    );
    assert_eq!(mac.journal_today().matches("after the crash").count(), 1);
}

#[test]
fn two_saves_at_once_both_land_under_the_lock() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Second\" <<'ASTROLABE'\ng2\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let mac = std::sync::Arc::new(mac);
    let handles: Vec<_> = ["s1", "s2"]
        .into_iter()
        .map(|s| {
            let mac = mac.clone();
            std::thread::spawn(move || {
                let (out, ok) =
                    mac.bash(s, &format!("astrolabe effort save --journal \"from {s}\""));
                assert!(ok, "{out}");
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let journal = mac.journal_today();
    assert!(
        journal.contains("from s1") && journal.contains("from s2"),
        "{journal}"
    );
    assert_eq!(
        mac.log()
            .lines()
            .filter(|l| *l == "0001: checkpoint" || *l == "0002: checkpoint")
            .count(),
        2,
        "{}",
        mac.log()
    );
    assert!(
        std::fs::read_dir(mac.state().join("ops"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
}

#[test]
fn recovery_finishes_a_create_without_rewriting_a_newer_note() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A crash after the file write of a create, before its commit, then the same call again:
    // one effort, not two.
    let crashed = json!({
        "key": "k-crash", "files": [["efforts/0002-crashed.md", "---\nstatus: active\nrepo: demo\ncreated: 2026-01-01\n---\n# 0002 Crashed\n\n## Current state\n**Goal:** g.\n"]],
        "renames": [], "journal": [], "message": "0002: started", "base": null, "states": [], "committed": false,
        "digest": "d", "result": null, "cwd": "x"
    });
    write(&mac.state().join("ops/k-crash.json"), &crashed.to_string());
    std::fs::write(
        mac.notes.join("efforts/0002-crashed.md"),
        crashed["files"][0][1].as_str().unwrap(),
    )
    .unwrap();
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s3",
        "astrolabe effort new --title \"Third\" <<'ASTROLABE'\ng3\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    assert_eq!(
        mac.log().lines().count(),
        before + 2,
        "the leftover committed, then the new one"
    );
    assert!(
        !mac.state().join("ops/k-crash.json").exists()
            || serde_json::from_slice::<Value>(
                &std::fs::read(mac.state().join("ops/k-crash.json")).unwrap()
            )
            .unwrap()["result"]
                .is_string()
    );
    // A committed intent that still has a state file to write never rewrites the note.
    let newer = std::fs::read_to_string(mac.notes.join("efforts/0002-crashed.md"))
        .unwrap()
        .replace("**Goal:** g.", "NEWER.");
    std::fs::write(mac.notes.join("efforts/0002-crashed.md"), &newer).unwrap();
    git(&mac.notes, &["add", "-A"]);
    git(&mac.notes, &["commit", "-q", "-m", "0002: newer"]);
    let mut committed = crashed.clone();
    committed["key"] = "k-committed".into();
    committed["committed"] = true.into();
    committed["states"] = json!([[mac.state().join("s9.json"), {"session": "s9", "effort": "2", "file": "efforts/0002-crashed.md"}]]);
    write(
        &mac.state().join("ops/k-committed.json"),
        &committed.to_string(),
    );
    assert!(mac.bash("s3", "astrolabe effort save --decision \"d\"").1);
    assert!(
        std::fs::read_to_string(mac.notes.join("efforts/0002-crashed.md"))
            .unwrap()
            .contains("NEWER"),
        "not rolled back"
    );
    assert!(
        mac.state().join("s9.json").exists(),
        "the state step was finished"
    );
}

#[test]
fn a_lost_ticket_recovers_the_command_reply() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The plain command finds a finished intent's result when the ticket never got written.
    let (_, ok) = mac.bash("s1", "astrolabe effort save --decision \"lost reply\"");
    assert!(ok);
    let digest =
        astrolabe::effort::EffortCommand::parse("astrolabe effort save --decision \"lost reply\"")
            .unwrap()
            .digest();
    let op = json!({"key": "k-lost", "files": [], "renames": [], "journal": [], "message": "m", "base": null, "states": [], "committed": true,
        "digest": digest, "result": "Saved effort 1: recovered text", "cwd": astrolabe::hook::comparable(&mac.demo)});
    write(&mac.state().join("ops/k-lost.json"), &op.to_string());
    for f in std::fs::read_dir(mac.state().join("shown"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    let (out, ok) = mac.plain("astrolabe effort save --decision \"lost reply\"");
    assert!(
        ok && out.contains("recovered text") && out.contains("Recovered from the hook's record"),
        "{out}"
    );
    // A PowerShell here-string arrives as one argument with line breaks: same digest.
    let (out, ok) = mac.run(
        &mac.demo,
        &["effort", "save", "--decision", "lost reply"],
        "",
    );
    assert!(ok, "{out}");
    let (out, _) = mac.run(
        &mac.demo,
        &["effort", "save", "--status", "active", "g."],
        "",
    );
    assert!(out.contains("NOT recorded"), "{out}");
    // A mistyped flag is refused, not ignored.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --plan x");
    assert!(
        !ok && out.contains("isn't a flag of `effort save`"),
        "{out}"
    );
}

#[test]
fn a_retried_create_after_switching_efforts_names_the_effort_it_made() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The lost-reply retry's intent is long gone; the session has since moved to effort 2.
    for f in std::fs::read_dir(mac.state().join("ops"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Second\" <<'ASTROLABE'\ng2\nASTROLABE\n"
        )
        .1
    );
    for f in std::fs::read_dir(mac.state().join("ops"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    let (out, ok) = mac.bash("s1", NEW);
    assert!(
        ok && out.contains("Effort 1 was already created by this same call"),
        "{out}"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    assert_eq!(
        mac.log()
            .lines()
            .filter(|l| l.ends_with(": started"))
            .count(),
        2
    );
}

#[test]
fn a_save_commits_only_the_notes_it_wrote_and_leaves_the_owners_staging_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The owner has a draft staged in the notes checkout, unrelated to the save.
    write(&mac.notes.join("drafts/post.md"), "half a post");
    git(&mac.notes, &["add", "drafts/post.md"]);
    // And a hand-edited journal line, which saves deliberately carry along.
    let day = astrolabe::notes::journal_file("2026-01-02");
    write(&mac.notes.join(&day), "- by hand\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"kept apart\"");
    assert!(ok, "{out}");
    let files = git(&mac.notes, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(!files.contains("drafts/post.md"), "{files}");
    assert!(
        files.contains(&day),
        "hand journal edits still ride along: {files}"
    );
    let staged = git(&mac.notes, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "drafts/post.md", "still staged");
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("drafts/post.md")).unwrap(),
        "half a post"
    );
}

#[test]
fn a_pending_write_that_cant_be_read_is_reported_kept_and_blocks_new_writes() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let bad = mac.state().join("ops/broken.json");
    write(&bad, "{\"key\": \"broken\", \"files\": [");
    let (out, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(!out.contains("nothing pending"), "{out}");
    assert!(out.contains("broken"), "names the record: {out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"over a broken op\"",
    );
    assert!(!ok && out.contains("broken"), "{out}");
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("broken"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&bad).unwrap(),
        "{\"key\": \"broken\", \"files\": [",
        "kept for repair"
    );
    // Once it's repaired (here, moved aside by hand), writes go through again.
    std::fs::remove_file(&bad).unwrap();
    assert!(
        mac.bash("s1", "astrolabe effort save --decision \"repaired\"")
            .1
    );
    let (out, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(!out.contains("broken"), "{out}");
}

#[test]
fn recovery_of_a_write_that_already_committed_keeps_a_newer_edit() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A crash after the commit, before the intent recorded it: the commit carries the key,
    // the intent still says uncommitted.
    let rel = shelf("0001-smooth-ink.md");
    let saved = std::fs::read_to_string(mac.notes.join(&rel))
        .unwrap()
        .replace("smooth the ink", "SAVED smooth the ink");
    std::fs::write(mac.notes.join(&rel), &saved).unwrap();
    git(&mac.notes, &["add", "-A"]);
    git(
        &mac.notes,
        &["commit", "-q", "-m", "0001: checkpoint\n\nOp-Key: k-landed"],
    );
    let intent = json!({
        "key": "k-landed", "files": [[rel, saved]], "renames": [],
        "journal": [{"day": "2026-01-01", "id": "j-landed", "effort": "0001", "machine": "mac", "text": "A line that must not appear twice."}],
        "message": "0001: checkpoint", "base": null,
        "states": [[mac.state().join("s8.json"), {"session": "s8", "effort": "1", "file": rel}]],
        "committed": false, "digest": "d", "result": null, "cwd": "x"
    });
    write(&mac.state().join("ops/k-landed.json"), &intent.to_string());
    // The owner edits the note by hand afterwards.
    let newer = saved.replace("SAVED smooth the ink", "NEWER by hand");
    std::fs::write(mac.notes.join(&rel), &newer).unwrap();
    git(&mac.notes, &["add", "-A"]);
    git(&mac.notes, &["commit", "-q", "-m", "hand edit"]);
    let before = mac.log().lines().count();

    let (out, ok) = mac.bash(
        "s3",
        "astrolabe effort new --title \"Third\" <<'ASTROLABE'\ng3\nASTROLABE\n",
    );
    assert!(ok, "{out}");

    let now = std::fs::read_to_string(mac.notes.join(&rel)).unwrap();
    assert!(
        now.contains("NEWER by hand"),
        "the hand edit survives: {now}"
    );
    assert!(!now.contains("SAVED smooth the ink"), "{now}");
    assert_eq!(
        mac.log().lines().count(),
        before + 1,
        "only the new save commits; the landed write isn't committed again"
    );
    assert!(
        !mac.journal_today().contains("must not appear twice"),
        "a landed write's journal line isn't written again"
    );
    assert!(
        mac.state().join("s8.json").exists(),
        "its session state is still finished"
    );
}

#[test]
fn a_leftover_temp_file_never_enters_a_notes_commit() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A write that died between writing aside and renaming leaves this beside the note.
    let stray = shelf("0001-smooth-ink.99999.tmp");
    std::fs::write(mac.notes.join(&stray), "half a note").unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"d\"");
    assert!(ok, "{out}");
    let tracked = git(&mac.notes, &["ls-files"]);
    assert!(!tracked.contains(".tmp"), "{tracked}");
    assert!(
        mac.notes.join(&stray).exists(),
        "left for the owner, not deleted"
    );
}
