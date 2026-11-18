use super::*;

#[test]
fn two_machines_number_offline_and_the_newer_is_renumbered_after_sync() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    // The PC pulls on create, so it takes 2.
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"On the PC\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    // Both go offline (an unreachable remote) and create: both take 3.
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", "/nonexistent/notes.git"],
        );
    }
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Mac three\" <<'ASTROLABE'\nmac goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    let (out, ok) = pc.bash(
        "p2",
        "astrolabe effort new --title \"PC three\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    assert!(
        mac.bash("s2", "astrolabe effort save --journal \"Mac three's line\"")
            .1
    );
    assert!(
        pc.bash("p2", "astrolabe effort save --journal \"PC three's line\"")
            .1
    );
    let origin = world.origin.0.join("notes.git");
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", origin.to_str().unwrap()],
        );
    }
    let (out, ok) = mac.sync();
    assert!(
        ok,
        "{out}\n{}\n{}",
        git(&mac.notes, &["log", "--oneline", "--all", "--graph"]),
        git(&mac.notes, &["status"])
    );
    // The PC's pull rebases its 3 onto the Mac's 3: two files, one number; the newer (the
    // PC's, created later) is renumbered to 4 and its session follows.
    let (out, ok) = pc.sync();
    assert!(
        ok,
        "{out}\n{}\n{}",
        git(&pc.notes, &["log", "--oneline", "--all", "--graph"]),
        git(&pc.notes, &["status"])
    );
    assert!(
        pc.notes.join(shelf("0004-pc-three.md")).exists(),
        "{:?}",
        std::fs::read_dir(pc.notes.join("efforts"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    assert!(!pc.notes.join(shelf("0003-pc-three.md")).exists());
    assert!(pc.effort_text(4).contains("# 0004 PC three"));
    assert!(
        pc.effort_text(4).contains("renumbered from 0003"),
        "{}",
        pc.effort_text(4)
    );
    // The PC's own journal line for its 3 follows it to 4; the Mac's line for 3 stays.
    let journal = pc.journal_today();
    assert!(
        journal.contains("PC three's line <!-- 0004 pc ")
            && journal.contains("Mac three's line <!-- 0003 mac "),
        "{journal}"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(pc.state().join("p2.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "4");
    assert_eq!(state["file"], shelf("0004-pc-three.md"));
    // And the Mac sees it after its own pull, with the journal's day file merged by union.
    assert!(mac.sync().1);
    assert!(mac.notes.join(shelf("0004-pc-three.md")).exists());
    let journal = mac.journal_today();
    assert!(
        journal.contains("Mac three's line <!-- 0003 mac ")
            && journal.contains("PC three's line <!-- 0004 pc ")
            && journal.lines().count() == 2,
        "{journal}"
    );
}

/// The Mac goes offline, `act` runs on it, and it comes back.
fn offline_on(world: &World, m: &Machine, act: impl FnOnce()) {
    git(
        &m.notes,
        &["remote", "set-url", "origin", "/nonexistent/notes.git"],
    );
    act();
    git(
        &m.notes,
        &[
            "remote",
            "set-url",
            "origin",
            world.origin.0.join("notes.git").to_str().unwrap(),
        ],
    );
}

/// Effort 1 made on the Mac and loaded on the PC, both synced.
fn both_on_one_effort(world: &World) -> (Machine, Machine) {
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync().1);
    let (out, ok) = pc.bash("p1", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    (mac, pc)
}

fn save(m: &Machine, session: &str, flags: &str, body: &str) {
    let (out, ok) = m.bash(
        session,
        &format!("astrolabe effort save {flags} <<'ASTROLABE'\n{body}\nASTROLABE\n"),
    );
    assert!(ok, "{out}");
}

fn status(m: &Machine) -> String {
    m.run(&m.notes, &["notes", "status"], "").0
}

#[test]
fn two_machines_saving_one_effort_apart_merge_without_diverging() {
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    // A checkout set to the backend that skips merge drivers still gets the driver.
    git(&mac.notes, &["config", "rebase.backend", "apply"]);
    save(
        &pc,
        "p1",
        "--decision \"chose pc: reason\"",
        "PC view.\n\nPC next.",
    );
    assert!(pc.sync().1);
    // A worker binds to the same effort on the Mac without having seen the PC's save: both
    // rewrote the stamp lines, which used to conflict and strand the Mac.
    offline_on(&world, &mac, || {
        let (out, ok) = mac.bash(
            "w1",
            "astrolabe activate --effort 1 --worker --model opus --reasoning medium",
        );
        assert!(ok, "{out}");
    });
    let (out, ok) = mac.sync();
    assert!(
        ok,
        "{out}\n{}",
        git(&mac.notes, &["log", "--oneline", "--all", "--graph"])
    );
    let text = mac.effort_text(1);
    assert!(
        text.contains("chose pc: reason") && text.contains("PC view."),
        "{text}"
    );
    // What only the binding changed (the worker's model) is kept beside the PC's save.
    assert!(
        text.contains("runs_on: "),
        "the binding's own property: {text}"
    );
    assert!(
        !text.contains("<<<<<<<") && !text.contains("unseen") && !text.contains("Unmerged"),
        "{text}"
    );
    assert!(status(&mac).contains("nothing pending"), "{}", status(&mac));
    assert!(pc.sync().1);
    assert_eq!(pc.effort_text(1), text);
}

/// The sync test's same-note case: `first` saves state, a decision and a tick and publishes;
/// `second`, offline, saves the same note seconds later, then comes back and syncs. Returns
/// the note both machines end with.
fn same_note_from_both(world: &World, first: &Machine, second: &Machine) -> String {
    let (f, s) = (session_of(first), session_of(second));
    write(
        &first.demo.join("pieces.md"),
        "- [ ] first piece\n- [ ] second piece\n",
    );
    let (out, ok) = first.bash(f, "astrolabe effort save --pieces-file pieces.md");
    assert!(ok, "{out}");
    assert!(first.sync().1);
    let (out, ok) = second.bash(s, "astrolabe effort read 1");
    assert!(ok, "{out}");
    save(
        first,
        f,
        "--decision \"chose first: its reason\" --tick \"first piece\"",
        "FIRST-STATE.\n\nFirst next.",
    );
    assert!(first.sync().1);
    // Seconds apart, as in the sync test: within one second the stamps are equal and the
    // machine name decides.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    offline_on(world, second, || {
        save(
            second,
            s,
            "--decision \"chose second: its reason\" --tick \"second piece\"",
            "SECOND-STATE.\n\nSecond next.",
        )
    });
    let (out, ok) = second.sync();
    assert!(ok, "{out}");
    assert!(first.sync().1);
    let text = second.effort_text(1);
    assert_eq!(
        first.effort_text(1),
        text,
        "both machines hold the same bytes"
    );
    text
}

fn session_of(m: &Machine) -> &'static str {
    if m.name == "pc" { "p1" } else { "s1" }
}

#[test]
fn the_same_note_saved_on_both_machines_keeps_both_sides() {
    // Defects: the later save won a whole block, so the earlier save's decision and tick
    // went with its state text; within a minute "later" was the larger machine name, so the
    // PC's earlier save beat the Mac's; and the losing state text was only in git.
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    let text = same_note_from_both(&world, &pc, &mac);
    let file = astrolabe::effort::EffortFile::parse(&text).unwrap();
    assert!(file.current.contains("SECOND-STATE."), "{text}");
    assert!(
        file.decisions.contains("chose first: its reason")
            && file.decisions.contains("chose second: its reason"),
        "{text}"
    );
    let pieces = file.pieces().unwrap();
    assert!(
        pieces.contains("- [x] first piece") && pieces.contains("- [x] second piece"),
        "{text}"
    );
    let unmerged = file
        .unmerged()
        .expect("the earlier state is kept in the note");
    assert!(
        unmerged.contains("pc · ") && unmerged.contains("> FIRST-STATE."),
        "{text}"
    );
    assert!(
        !text.contains("<<<<<<<") && !text.contains("unseen:"),
        "{text}"
    );
    // Every machine's status and the board's warnings say so, and so does what a session is
    // given when it loads the effort.
    for m in [&mac, &pc] {
        let status = status(m);
        assert!(
            status.contains("Two machines saved the same record unseen")
                && status.contains("Unmerged state")
                && !status.contains("diverged"),
            "{status}"
        );
    }
    let (out, ok) = pc.bash("p1", "astrolabe activate --effort 1");
    assert!(
        ok && out.contains("Unmerged state") && out.contains("> FIRST-STATE."),
        "{out}"
    );
    // A save without a Current state leaves it; one with a Current state replaces it, on
    // both machines.
    let (out, ok) = pc.bash("p1", "astrolabe effort read 1");
    assert!(ok, "{out}");
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort save --decision \"chose to wait: its reason\"",
    );
    assert!(ok, "{out}");
    assert!(pc.effort_text(1).contains("## Unmerged state"));
    save(&pc, "p1", "", "Folded both.\n\nNext.");
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    for m in [&mac, &pc] {
        assert!(status(m).contains("nothing pending"), "{}", status(m));
        assert!(!m.effort_text(1).contains("## Unmerged state"));
    }
}

#[test]
fn the_same_note_merges_alike_whichever_machine_merges() {
    // Defect: what both sides added came out in the merging machine's order.
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    let text = same_note_from_both(&world, &mac, &pc);
    let file = astrolabe::effort::EffortFile::parse(&text).unwrap();
    assert!(file.current.contains("SECOND-STATE."), "{text}");
    assert!(
        file.decisions
            .find("chose first")
            .zip(file.decisions.find("chose second"))
            .is_some_and(|(a, b)| a < b),
        "earlier save's first: {text}"
    );
    let pieces = file.pieces().unwrap();
    assert!(
        pieces.contains("- [x] first piece") && pieces.contains("- [x] second piece"),
        "{text}"
    );
    assert!(file.unmerged().unwrap().contains("mac · "), "{text}");
}

#[test]
fn hand_edits_on_both_machines_beside_saves_are_both_kept_and_raise_nothing() {
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nother goal\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    // Each machine adds a notice to effort 2 by hand, and a save of effort 1 sweeps it into
    // its commit: one Base for effort 1, which used to flag effort 2 as saved unseen.
    let notice = |m: &Machine, text: &str| {
        let (_, rel) = astrolabe::notes::effort_files(&m.notes)
            .into_iter()
            .find(|(n, _)| *n == 2)
            .unwrap();
        let path = m.notes.join(rel);
        let mut body = std::fs::read_to_string(&path).unwrap();
        body.push_str(&format!("\n{text}\n"));
        std::fs::write(&path, body).unwrap();
    };
    notice(&pc, "PC notice.");
    save(&pc, "p1", "", "PC view.\n\nPC next.");
    assert!(pc.sync().1);
    offline_on(&world, &mac, || {
        notice(&mac, "Mac notice.");
        save(&mac, "s1", "", "PC view.\n\nPC next.");
    });
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let other = mac.effort_text(2);
    assert!(
        other.contains("PC notice.") && other.contains("Mac notice."),
        "{other}"
    );
    assert!(status(&mac).contains("nothing pending"), "{}", status(&mac));
}

#[test]
fn a_hand_edit_swept_into_another_records_save_never_loses_to_a_save() {
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    let (out, ok) = pc.bash(
        "p2",
        "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nother goal\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    // The PC saves effort 2 from its session.
    save(&pc, "p2", "", "PC's own view of two.\n\nNext.");
    assert!(pc.sync().1);
    // On the Mac, effort 2's Current state is rewritten by hand, and a save of effort 1
    // sweeps it into a program commit: that commit is effort 1's save, not effort 2's.
    offline_on(&world, &mac, || {
        let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
            .into_iter()
            .find(|(n, _)| *n == 2)
            .unwrap();
        let path = mac.notes.join(rel);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace(
                "other goal\n\n## Current state",
                "other goal\n\n## Current state\nHand view of two.\n",
            ),
        )
        .unwrap();
        save(&mac, "s1", "", "Mac view of one.\n\nNext.");
    });
    let (out, ok) = mac.sync();
    // Not decided against the hand edit: left to git, so the pull diverges and keeps it.
    assert!(!ok && out.contains("diverged"), "{out}");
    let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 2)
        .unwrap();
    let two = std::fs::read_to_string(mac.notes.join(rel)).unwrap();
    assert!(two.contains("Hand view of two."), "{two}");
    assert!(!two.contains("unseen"), "{two}");
}

#[test]
fn a_committed_hand_edit_against_a_save_still_diverges_and_keeps_local_work() {
    let world = World::new();
    let (mac, pc) = both_on_one_effort(&world);
    save(&pc, "p1", "", "PC view.\n\nPC next.");
    assert!(pc.sync().1);
    // On the Mac, Current state is rewritten by hand and committed with git, stamp and all
    // as they were: the merge never decides against a hand edit.
    let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 1)
        .unwrap();
    let path = mac.notes.join(&rel);
    let text = std::fs::read_to_string(&path).unwrap();
    let at = text.find("## Current state\n").unwrap() + "## Current state\n".len();
    let end = text[at..].find("\n## ").map_or(text.len(), |e| at + e);
    std::fs::write(
        &path,
        format!("{}Hand view.\n{}", &text[..at], &text[end..]),
    )
    .unwrap();
    git(&mac.notes, &["commit", "-qam", "by hand"]);
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("diverged"), "{out}");
    assert!(
        git(&mac.notes, &["status", "--porcelain"])
            .trim()
            .is_empty(),
        "rebase aborted cleanly"
    );
    assert!(mac.effort_text(1).contains("Hand view."), "local edit kept");
    assert!(status(&mac).contains("Notes diverged from origin"));
    let resumed = mac.session_start("s1", "resume");
    assert!(resumed.contains("Notes diverged from origin"), "{resumed}");
    // The owner resolves it (here: keep theirs), and the next sync is clean.
    git(&mac.notes, &["fetch", "-q"]);
    git(&mac.notes, &["reset", "-q", "--hard", "origin/main"]);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    assert!(status(&mac).contains("nothing pending"), "{}", status(&mac));
}

#[test]
fn a_save_from_a_crlf_checkout_is_not_reported_stale_after_sync() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync().1);
    // The PC pushes a different effort, so the Mac's next sync rebases its save.
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nanother goal\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(pc.sync().1);
    // The Mac's checkout holds effort 1 with CRLF endings, as Git for Windows leaves it.
    let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(m, _)| *m == 1)
        .unwrap();
    let path = mac.notes.join(rel);
    let lf = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, lf.replace('\n', "\r\n")).unwrap();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nMac view.\n\nMac next.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.sync();
    assert!(ok && !out.contains("stale"), "{out}");
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(!status.contains("unseen"), "{status}");
}

#[test]
fn two_machines_replacing_their_lines_the_same_day_leave_one_line_each_after_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash("s1", "astrolabe effort save --journal \"Started A.\"")
            .1
    );
    assert!(mac.sync().1);
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"Two\" <<'ASTROLABE'\ng2\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    assert!(
        pc.bash("p1", "astrolabe effort save --journal \"Started B.\"")
            .1
    );
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    assert_eq!(mac.journal_today().lines().count(), 2);
    // Both replace their own line without seeing the other's replacement: a union merge
    // would keep all four; the tidy after the pull keeps the latest per effort.
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(
        mac.bash("s1", "astrolabe effort save --journal \"Finished A.\"")
            .1
    );
    assert!(
        pc.bash("p1", "astrolabe effort save --journal \"Finished B.\"")
            .1
    );
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    for m in [&mac, &pc] {
        let journal = m.journal_today();
        assert!(
            journal.contains("- Finished A. <!-- 0001 mac ")
                && journal.contains("- Finished B. <!-- 0002 pc ")
                && journal.lines().count() == 2,
            "{journal}"
        );
    }
    assert!(
        pc.log().contains("journal tidied after a merge"),
        "{}",
        pc.log()
    );
}

#[test]
fn recorded_backoff_skips_fetch_until_it_expires() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let far = astrolabe::notes::now_ms() + 60_000;
    write(
        &mac.state().join("sync.json"),
        &json!({"backoff_until_ms": far}).to_string(),
    );
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("no fetch for another"), "{out}");
    // Over, a sync goes through and clears it.
    write(
        &mac.state().join("sync.json"),
        &json!({"backoff_until_ms": astrolabe::notes::now_ms() - 1}).to_string(),
    );
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let record: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("sync.json")).unwrap()).unwrap();
    assert!(record.get("backoff_until_ms").is_none(), "{record}");
}

#[test]
fn every_save_asks_for_a_push_and_a_quiet_session_catches_up() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync_requested(), "a new effort asks for a push");
    // A status change, a new question for the owner, and a plain checkpoint all ask, however
    // recent the last push: the other machine's board follows each one.
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: merge it\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a status change asks for a push");
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --waits-for \"owner: playtest it\" <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nPlaytest.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a new waits-for asks for a push");
    mac.set_last_push(0);
    let (out, ok) = mac.bash("s1", "astrolabe effort save <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nStill the playtest.\nASTROLABE\n");
    assert!(
        ok && out.contains("committed, background sync started"),
        "{out}"
    );
    assert!(mac.sync_requested(), "a plain checkpoint asks too");
    mac.set_last_push(0);
    let (out, ok) = mac.bash("s2", "astrolabe journal \"Tried the ink at two widths.\"");
    assert!(ok, "{out}");
    assert!(mac.sync_requested(), "a journal line asks too");
    // Asked a moment ago: that one is still on its way, so the turn end doesn't ask.
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "one ask at a time");
    // The session sits open with commits unpublished (the save's push never landed): the
    // next turn end asks again, however recent the last push.
    mac.forget_ask();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(
        mac.sync_requested(),
        "the Stop hook pushes what the save's push left"
    );
    // Backing off after a timed-out network call: no ask until it ends.
    mac.forget_ask();
    mac.set_sync("backoff_until_ms", (Machine::now_ms() + 60_000).into());
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "backing off");
    mac.set_sync("backoff_until_ms", (Machine::now_ms() - 1).into());
    assert!(mac.stop("s1", false, false).is_none());
    assert!(mac.sync_requested(), "the backoff ended");
    // An ask stamped in the future never holds the next one back.
    mac.ask_from_the_future();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(mac.sync_requested(), "a future stamp counts as old");
    // Nothing unpublished: nothing to push, however old the last one.
    assert!(mac.run(&mac.notes, &["notes", "sync"], "").1);
    mac.set_last_push(11 * 60 * 1000);
    mac.forget_ask();
    let _ = mac.sync_requested();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested());
}

#[test]
fn a_dirty_notes_checkout_still_pulls() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    // The owner's seat is often mid-edit: a pull must not refuse over it.
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT edited by hand",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert!(
        pc.notes.join(shelf("0001-smooth-ink.md")).exists(),
        "the Mac's effort arrived"
    );
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT edited by hand",
        "the hand edit survived the pull"
    );
    assert!(git(&pc.notes, &["stash", "list"]).trim().is_empty());
    // An edit that clashes with what arrives: the tree takes what was pulled, the edit sits
    // in a stash named in the status until it's gone, and a save commits no marker.
    write(
        &mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT from the mac",
    );
    git(&mac.notes, &["commit", "-qam", "owner"]);
    git(&mac.notes, &["push", "-q"]);
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT clashing by hand",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT from the mac"
    );
    assert!(git(&pc.notes, &["status", "--porcelain"]).trim().is_empty());
    let stash = git(&pc.notes, &["stash", "list", "--format=%H"]);
    let id = stash.trim();
    assert_eq!(id.len(), 40, "{stash}");
    assert!(git(&pc.notes, &["stash", "show", "-p", id]).contains("+OWNER-TEXT clashing by hand"));
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(
        status.contains("knowledge/owner.md")
            && status.contains(&format!("git stash apply {}", &id[..7])),
        "{status}"
    );
    assert!(pc.bash("p1", NEW).1, "a save still lands");
    assert!(!git(&pc.notes, &["show", "HEAD", "--stat"]).contains("owner.md"));
    // Two clashes before the owner takes either: both stashes named, each until it's gone.
    write(
        &mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT from the mac again",
    );
    git(&mac.notes, &["commit", "-qam", "owner again"]);
    git(&mac.notes, &["push", "-q"]);
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT clashing twice",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert_eq!(status.matches("didn't reapply").count(), 2, "{status}");
    assert!(!status.contains("git stash drop"), "{status}");
    // An index left unmerged by hand refuses every write until it's resolved.
    git(&pc.notes, &["checkout", "-q", "-b", "side"]);
    write(&pc.notes.join("knowledge/owner.md"), "OWNER-TEXT side");
    git(&pc.notes, &["commit", "-qam", "side"]);
    git(&pc.notes, &["checkout", "-q", "main"]);
    write(&pc.notes.join("knowledge/owner.md"), "OWNER-TEXT main");
    git(&pc.notes, &["commit", "-qam", "main"]);
    let _ = Command::new("git")
        .args(["merge", "side"])
        .current_dir(&pc.notes)
        .output()
        .unwrap();
    assert!(git(&pc.notes, &["diff", "--name-only", "--diff-filter=U"]).contains("owner.md"));
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("unmerged files (knowledge/owner.md)"),
        "{out}"
    );
    git(&pc.notes, &["merge", "--abort"]);
    git(&pc.notes, &["reset", "-q", "--hard", "@{upstream}"]);
    // Once the owner has taken the stashes, the status stops naming them.
    git(&pc.notes, &["stash", "clear"]);
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(!status.contains("stash"), "{status}");
}

#[test]
fn a_write_during_a_pull_survives_a_reapply_that_leaves_no_conflict() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    write(
        &mac.notes.join("knowledge/machines.md"),
        "MACHINES from the mac",
    );
    git(&mac.notes, &["add", "-A"]);
    git(&mac.notes, &["commit", "-qm", "machines"]);
    git(&mac.notes, &["push", "-q"]);
    // A local commit for the rebase to replay, so git runs post-rewrite inside the pull,
    // after the picks and before the autostash goes back: a session writing the hand-edited
    // file at that moment makes git refuse the reapply without leaving a conflict.
    write(&pc.notes.join("knowledge/voice.md"), "VOICE local");
    git(&pc.notes, &["add", "-A"]);
    git(&pc.notes, &["commit", "-qm", "voice"]);
    write(&pc.notes.join("knowledge/owner.md"), "OWNER-TEXT by hand");
    let hook = pc.notes.join(".git/hooks/post-rewrite");
    write(
        &hook,
        "#!/bin/sh\nprintf 'OWNER-TEXT written during the pull' > knowledge/owner.md\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    std::fs::remove_file(&hook).unwrap();
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT written during the pull",
        "the write during the pull is kept"
    );
    assert!(
        std::fs::read_to_string(pc.notes.join("knowledge/machines.md"))
            .unwrap()
            .contains("MACHINES from the mac")
    );
    let stash = git(&pc.notes, &["stash", "list", "--format=%H"]);
    let id = stash.trim();
    assert_eq!(id.len(), 40, "{stash}");
    assert!(git(&pc.notes, &["stash", "show", "-p", id]).contains("+OWNER-TEXT by hand"));
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(
        status.contains("(knowledge/owner.md) didn't reapply")
            && status.contains(&format!("git stash apply {}", &id[..7])),
        "{status}"
    );
}

#[test]
fn a_pull_with_nothing_incoming_leaves_hand_edits_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    write(&mac.notes.join("knowledge/owner.md"), "OWNER-TEXT mid-edit");
    let reflog = git(&mac.notes, &["reflog"]).lines().count();
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(ok, "{out}");
    // No stash, no reset: the edit was never lifted out of the tree.
    assert_eq!(git(&mac.notes, &["reflog"]).lines().count(), reflog);
    assert!(git(&mac.notes, &["stash", "list"]).trim().is_empty());
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT mid-edit"
    );
}

#[test]
fn an_offline_pickup_still_begins_astrolabe_is_on_with_the_note_after() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    git(
        &mac.notes,
        &["remote", "set-url", "origin", "/nonexistent/notes.git"],
    );
    // The first line is what a pickup checks; the refresh note follows the opening paragraph.
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    assert!(out.starts_with("Astrolabe is on"), "{out}");
    let (opening, rest) = out.split_once("\n\n").unwrap();
    assert!(!opening.contains("Notes not refreshed"), "{out}");
    assert!(rest.starts_with("Notes not refreshed from origin"), "{out}");
    // The same at a session's reload of its effort.
    let start = mac.session_start("s2", "resume");
    assert!(start.starts_with("Astrolabe is on"), "{start}");
    assert!(
        start.contains("\n\nNotes not refreshed from origin"),
        "{start}"
    );
}

#[test]
fn a_save_without_a_remote_reports_local_success_even_when_sync_is_due() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    git(&mac.notes, &["remote", "remove", "origin"]);
    let before = git(&mac.notes, &["rev-parse", "HEAD"]);
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status ready");
    assert!(ok, "{out}");
    assert!(
        out.contains("saved locally; sync unavailable: no remote configured"),
        "{out}"
    );
    assert!(
        !out.contains("push started") && !out.contains("sync started"),
        "{out}"
    );
    assert!(mac.effort_text(1).contains("status: ready"));
    assert_ne!(git(&mac.notes, &["rev-parse", "HEAD"]), before);
    assert!(git(&mac.notes, &["status", "--porcelain"]).is_empty());
    // A plain checkpoint still explains the missing remote.
    mac.set_last_push(0);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"Kept local saves available\"",
    );
    assert!(
        ok && out.contains("saved locally; sync unavailable: no remote configured"),
        "{out}"
    );
    assert!(
        !out.contains("push started") && !out.contains("sync started"),
        "{out}"
    );
}

#[test]
fn a_save_reports_a_background_launch_without_claiming_a_finished_push() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let _ = mac.sync_requested();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status ready");
    assert!(
        ok && out.contains("committed, background sync started"),
        "{out}"
    );
    assert!(mac.sync_requested());
    assert!(
        !out.contains("pushed") && !out.contains("push started"),
        "{out}"
    );
    // The launch marker stands in for a background process: origin is unchanged until sync.
    assert!(!git(&mac.notes, &["log", "origin/main..HEAD", "--oneline"]).is_empty());
    let (out, ok) = mac.sync();
    assert!(ok && out.contains("pushed"), "{out}");
    assert!(git(&mac.notes, &["log", "origin/main..HEAD", "--oneline"]).is_empty());
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"Kept background sync automatic\"",
    );
    // A plain checkpoint right after a push starts another: every save goes out.
    assert!(
        ok && out.contains("committed, background sync started"),
        "{out}"
    );
    assert!(mac.sync_requested(), "{out}");
}

#[test]
fn a_save_still_succeeds_when_background_sync_cannot_start() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let _ = mac.sync_requested();
    // Make the test launch marker unwritable without breaking local note writes.
    std::fs::create_dir(mac.state().join("sync-requested")).unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status ready");
    assert!(
        ok && out.contains("committed; background sync could not start"),
        "{out}"
    );
    assert!(mac.effort_text(1).contains("status: ready"));
    assert!(git(&mac.notes, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_renumbered_note_whose_header_never_closes_keeps_every_byte() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    // Both go offline and create: both take 2.
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", "/nonexistent/notes.git"],
        );
    }
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Mac two\" <<'ASTROLABE'\nmac goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    let (out, ok) = pc.bash(
        "p2",
        "astrolabe effort new --title \"PC two\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    assert!(
        pc.bash("p2", "astrolabe effort save --journal \"PC two's line\"")
            .1
    );
    // A hand edit on the PC breaks the closing line and adds sections of the owner's own:
    // all of the note reads as header now.
    let path = pc.notes.join(shelf("0002-pc-two.md"));
    let broken = std::fs::read_to_string(&path)
        .unwrap()
        .replacen("\n---\n", "\n--- x\n", 1)
        + "\n## History\n  Keep  these spaces.  \n\n## Links\nKeep: this too.\n";
    std::fs::write(&path, &broken).unwrap();
    git(&pc.notes, &["add", "efforts"]);
    git(&pc.notes, &["commit", "-qm", "hand edit"]);
    let origin = world.origin.0.join("notes.git");
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", origin.to_str().unwrap()],
        );
    }
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    // The PC's pull finds two files numbered 2; its own, the newer, moves to 3.
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert!(!pc.notes.join(shelf("0002-pc-two.md")).exists());
    assert_eq!(
        std::fs::read_to_string(pc.notes.join(shelf("0003-pc-two.md"))).unwrap(),
        broken
    );
    // What doesn't need the note rewritten still follows: its journal line and its session.
    let journal = pc.journal_today();
    assert!(journal.contains("PC two's line <!-- 0003 pc "), "{journal}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(pc.state().join("p2.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "3");
    assert!(pc.log().contains("0002 renumbered to 0003"), "{}", pc.log());
    // The other machine receives the same bytes.
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_to_string(mac.notes.join(shelf("0003-pc-two.md"))).unwrap(),
        broken
    );
    assert!(mac.effort_text(2).contains("# 0002 Mac two"));
}

/// A page a session wrote as a plain file on the Mac, then any save there and a sync each way.
fn a_file_written_by_hand_reaches_the_pc(rel: &str) {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    write(
        &mac.notes.join(rel),
        "# Written by hand\n\nIt cost a turn.\n",
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"kept going\"");
    assert!(ok, "{out}");
    // It rides along in the save's own commit, under the save's own subject.
    let files = git(&mac.notes, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(files.lines().any(|f| f == rel), "{files}");
    let subject = git(&mac.notes, &["log", "-1", "--format=%s"]);
    assert_eq!(subject.trim(), "0001: checkpoint");
    assert!(
        git(&mac.notes, &["status", "--porcelain"])
            .trim()
            .is_empty()
    );
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_to_string(pc.notes.join(rel)).ok().as_deref(),
        Some("# Written by hand\n\nIt cost a turn.\n"),
        "the other machine has it after its own sync"
    );
}

#[test]
fn a_lesson_written_as_a_file_reaches_the_other_machine_after_a_save_and_sync() {
    a_file_written_by_hand_reaches_the_pc("lessons/2026-11-09-friction.md");
}

#[test]
fn a_draft_written_as_a_file_reaches_the_other_machine_after_a_save_and_sync() {
    a_file_written_by_hand_reaches_the_pc("drafts/2026-11-09-reply.md");
}

/// Two machines each made an effort 1, and one published first. The other's next save
/// pulls, which renumbers its effort: the save goes to the effort where it is now, and
/// says so.
#[test]
fn a_save_whose_own_pull_renumbers_its_effort_still_lands() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"Other\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 1 created"), "{out}");
    assert!(pc.sync().1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --journal \"a line for the day\"",
    );
    assert!(ok, "{out}");
    assert!(
        out.starts_with("Saved effort 2: status active, journal line written;")
            && out.contains("renumbered the effort from 1 to 2"),
        "{out}"
    );
    assert!(
        mac.log().contains("0001 renumbered to 0002"),
        "{}",
        mac.log()
    );
    assert!(mac.notes.join(shelf("0002-smooth-ink.md")).exists());
    let journal = mac.journal_today();
    assert!(
        journal.contains("a line for the day <!-- 0002 mac "),
        "{journal}"
    );
    // The session saves on from there, with nothing refused as changed behind it.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(ok && out.starts_with("Saved effort 2"), "{out}");
    assert!(mac.effort_text(2).contains("chose a over b: c"));
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
}

/// A renumber was planned and then stopped before its rename (a file lock, a hook that
/// died), and the session saved at the old path meanwhile. The next hook must not replay
/// the plan's text from before that save over it: it renumbers the note as it is now.
#[test]
fn a_renumber_left_unbegun_is_made_again_from_the_note_as_it_is_now() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let old_rel = shelf("0001-smooth-ink.md");
    let new_rel = shelf("0002-smooth-ink.md");
    let planned_from = mac.effort_text(1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(ok, "{out}");
    // The other machine's effort 1 arrives, as a pull's rebase brings it.
    assert!(
        pc.bash(
            "p1",
            "astrolabe effort new --title \"Other\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
        )
        .1
    );
    assert!(pc.sync().1);
    git(&mac.notes, &["pull", "-q", "--rebase", "origin", "main"]);
    // The plan as it was saved before the save above, with the session following the file.
    let planned = planned_from.replace("# 0001 Smooth ink", "# 0002 Smooth ink");
    let session = mac.state().join("s1.json");
    let mut state: Value = serde_json::from_slice(&std::fs::read(&session).unwrap()).unwrap();
    state["file"] = new_rel.clone().into();
    state["effort"] = "2".into();
    state["binding"]["key"] = "2".into();
    state["delivered"] = astrolabe::notes::content_hash(&planned).into();
    let plan = json!({
        "key": "renumber-0001-0002", "files": [[new_rel, planned]], "renames": [[old_rel, new_rel]],
        "journal": [], "message": "0001 renumbered to 0002", "base": null,
        "states": [[session, state]], "committed": false, "digest": null, "result": null, "cwd": null
    });
    write(
        &mac.state().join("ops/renumber-0001-0002.json"),
        &plan.to_string(),
    );
    // The next hook finishes the renumber and saves.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose d over e: f\"",
    );
    assert!(ok && out.starts_with("Saved effort 2"), "{out}");
    let note = mac.effort_text(2);
    assert!(
        note.contains("chose a over b: c") && note.contains("chose d over e: f"),
        "the save made before the renumber is still in the note: {note}"
    );
    assert!(note.contains("# 0002 Smooth ink") && note.contains("renumbered from 0001"));
    assert!(!mac.notes.join(&old_rel).exists());
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
}

/// The usual two-machine case: the renumber is made by a sync, not by the session's own
/// save. The session had the note as it stood, so its next save isn't refused as made over
/// text it hasn't seen. One that hadn't is still refused.
#[test]
fn a_renumber_made_by_a_sync_leaves_the_sessions_next_save_unrefused() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        pc.bash(
            "p1",
            "astrolabe effort new --title \"Other\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
        )
        .1
    );
    assert!(pc.sync().1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    assert!(
        mac.log().contains("0001 renumbered to 0002"),
        "{}",
        mac.log()
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(ok && out.starts_with("Saved effort 2"), "{out}");
    assert!(mac.effort_text(2).contains("chose a over b: c"));
}

#[test]
fn a_renumber_doesnt_carry_a_session_past_text_it_hadnt_seen() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    // The note changes behind the session, as a hand edit would change it.
    let rel = shelf("0001-smooth-ink.md");
    let edited = mac
        .effort_text(1)
        .replace("smooth the ink", "SMOOTH the ink");
    write(&mac.notes.join(&rel), &edited);
    git(&mac.notes, &["commit", "-qam", "hand edit"]);
    assert!(
        pc.bash(
            "p1",
            "astrolabe effort new --title \"Other\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
        )
        .1
    );
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    assert!(
        mac.log().contains("0001 renumbered to 0002"),
        "{}",
        mac.log()
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(
        !ok && out.contains("effort 2 changed since it was delivered to this session"),
        "{out}"
    );
}
