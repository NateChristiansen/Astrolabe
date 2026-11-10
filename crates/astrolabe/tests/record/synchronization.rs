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

#[test]
fn a_save_blind_to_the_other_machines_save_is_reported_after_sync() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync().1);
    // The PC loads 1 (pulling it), saves, pushes.
    let (out, ok) = pc.bash("p1", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort save <<'ASTROLABE'\nPC view.\n\nPC next.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(pc.sync().1);
    // The Mac, offline, saves the same effort from its older delivered text.
    git(
        &mac.notes,
        &["remote", "set-url", "origin", "/nonexistent/notes.git"],
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nMac view.\n\nMac next.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    git(
        &mac.notes,
        &[
            "remote",
            "set-url",
            "origin",
            world.origin.0.join("notes.git").to_str().unwrap(),
        ],
    );
    // Both rewrote the same stamp and Current state, so the rebase conflicts: aborted, the
    // Mac's commit kept, and every session hears it until the owner resolves it.
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("diverged"), "{out}");
    assert!(
        git(&mac.notes, &["status", "--porcelain"])
            .trim()
            .is_empty(),
        "rebase aborted cleanly"
    );
    assert!(mac.effort_text(1).contains("Mac view"), "local save kept");
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(status.contains("Notes diverged from origin"), "{status}");
    let resumed = mac.session_start("s1", "resume");
    assert!(resumed.contains("Notes diverged from origin"), "{resumed}");
    // The owner resolves it (here: keep theirs), and the next sync is clean.
    git(&mac.notes, &["fetch", "-q"]);
    git(&mac.notes, &["reset", "-q", "--hard", "origin/main"]);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(status.contains("nothing pending"), "{status}");
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
