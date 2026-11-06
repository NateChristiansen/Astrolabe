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
fn a_quiet_session_still_pushes_and_a_new_wait_is_a_state_change() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let _ = mac.sync_requested();
    // Same status, a new question for the owner: the other machine must see it.
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: merge it\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a status change asks for a push");
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --waits-for \"owner: playtest it\" <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nPlaytest.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a new waits-for asks for a push");
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nStill the playtest.\nASTROLABE\n").1);
    assert!(
        !mac.sync_requested(),
        "a plain checkpoint inside the window waits"
    );
    // The session sits open with commits unpublished: a turn end inside the window does
    // nothing, one past it pushes.
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested());
    mac.set_last_push(11 * 60 * 1000);
    mac.forget_ask();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(
        mac.sync_requested(),
        "the Stop hook pushes once the window has passed"
    );
    // Asked a moment ago: that one is still on its way, so the next turn end doesn't ask.
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "one ask at a time");
    // A save's own ask counts the same.
    mac.forget_ask();
    assert!(mac.bash("s1", "astrolabe effort save --status active <<'ASTROLABE'\nBack on it.\n\nNo choice.\n\nMore.\nASTROLABE\n").1);
    assert!(mac.sync_requested());
    mac.set_last_push(11 * 60 * 1000);
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "the save just asked");
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
