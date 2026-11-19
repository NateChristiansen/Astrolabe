//! Each effort states, when it's filed, the event that finishes it: the merge by default, the
//! merge and an owner's look at something named, or with nothing to land, the findings
//! written or accepted. The line reaches the session with the note and the board's card.
use super::*;

fn new_with(title: &str, finish: &str) -> String {
    format!(
        "astrolabe effort new --title \"{title}\" --finish \"{finish}\" <<'ASTROLABE'\na goal\nASTROLABE\n"
    )
}

/// Catches a filing that leaves the finish line unstated, so nothing on the effort says what
/// done means, or a form that's stored as something other than the event it names.
#[test]
fn effort_new_writes_the_finish_line_and_defaults_to_the_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash("s1", NEW);
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("authority: edit\nfinish: done when merged\n"),
        "{}",
        mac.effort_text(1)
    );
    for (n, form, line) in [
        (
            2,
            "look: the board on the second machine",
            "finish: done when merged and the owner has looked at the board on the second machine\n",
        ),
        (
            3,
            "findings",
            "finish: nothing to land; done when the findings are written\n",
        ),
        (
            4,
            "accepted",
            "finish: nothing to land; done when the owner accepts the findings\n",
        ),
    ] {
        let (out, ok) = mac.bash("s1", &new_with(&format!("Effort {n}"), form));
        assert!(ok, "{out}");
        assert!(mac.effort_text(n).contains(line), "{}", mac.effort_text(n));
    }
}

/// Catches an unknown or empty form being stored as a finish line nobody can check.
#[test]
fn an_unknown_finish_form_is_refused_and_files_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    let before = mac.log().lines().count();
    for form in ["when it feels done", "look:", "look:   "] {
        let (out, ok) = mac.bash("s1", &new_with("Vague", form));
        assert!(!ok, "{form}: {out}");
        assert!(
            out.contains("isn't a finish line") && out.contains("look: <what"),
            "{form}: {out}"
        );
    }
    assert_eq!(mac.log().lines().count(), before);
    assert!(astrolabe::notes::effort_files(&mac.notes).is_empty());
}

/// Catches an initiative's members all getting one finish line when the plan says they
/// finish differently, a `finish:` line leaking into the goal, or a bad one filing anything.
#[test]
fn an_initiative_member_states_its_own_finish_line() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Ink\" <<'ASTROLABE'\nSmooth ink.\n\n## Efforts\nTether cap\nCap the tether.\n\nMeasure the goo\nTime goo at speed.\nfinish: accepted\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let cap = mac.effort_text(1);
    assert!(cap.contains("finish: done when merged\n"), "{cap}");
    let goo = mac.effort_text(2);
    assert!(
        goo.contains("finish: nothing to land; done when the owner accepts the findings\n")
            && goo.contains("## Current state\nTime goo at speed.\n")
            && !goo.contains("finish: accepted"),
        "{goo}"
    );
    let page = std::fs::read_to_string(mac.notes.join("initiatives/ink.md")).unwrap();
    assert!(!page.contains("finish:"), "{page}");
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Paint\" <<'ASTROLABE'\n## Efforts\nMix\nMix paint.\nfinish: soon\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("effort \"Mix\": finish \"soon\" isn't a finish line"),
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before);
}

/// Catches the line not reaching the session that works the effort, and a note filed before
/// the property reading as having no finish line.
#[test]
fn a_session_reads_the_finish_line_with_the_current_state() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        &new_with("Look after", "look: the export on a large file"),
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    assert!(
        out.contains("Current state\n\na goal\n\nFinish line: done when merged and the owner has looked at the export on a large file.\n"),
        "{out}"
    );
    // An older note, filed before the property: the default, as filing would have written.
    let rel = astrolabe::notes::effort_files(&mac.notes)[0].1.clone();
    let path = mac.notes.join(&rel);
    let text = std::fs::read_to_string(&path).unwrap();
    let older: String = text
        .lines()
        .filter(|l| !l.starts_with("finish:"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&path, older).unwrap();
    git(&mac.notes, &["commit", "-qam", "an older note"]);
    let (out, ok) = mac.bash("s3", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    assert!(out.contains("Finish line: done when merged.\n"), "{out}");
}

/// Catches the board dropping the line: the row carries it, and a card shows it when it says
/// more than the merge.
#[test]
fn the_board_shows_a_finish_line_other_than_the_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash(
            "s1",
            &new_with("Look after", "look: the deck on the second machine")
        )
        .1
    );
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--repo", "demo"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let finish = |n: u64| {
        v["efforts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["number"] == n)
            .map(|e| e["finish"].clone())
            .unwrap()
    };
    assert_eq!(finish(1), "done when merged");
    assert_eq!(
        finish(2),
        "done when merged and the owner has looked at the deck on the second machine"
    );
    let board = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    let active = column(&board, "active");
    assert!(
        active.contains("<p class=\"finish\">done when merged and the owner has looked at the deck on the second machine</p>"),
        "{active}"
    );
    // The default is every card's, so it isn't drawn: one finish line on the board.
    assert_eq!(active.matches("class=\"finish\"").count(), 1, "{active}");
}

/// Catches a `look:` text that ends in a quote losing it on the way back out, on the board
/// and wherever else the note is read.
#[test]
fn a_finish_line_ending_in_a_quote_reads_back_as_filed() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Ink\" <<'ASTROLABE'\n## Efforts\nNotes\nWrite notes.\nfinish: look: the 'release notes'\n\nCard\nDraw the card.\nfinish: look: \"the board card\"\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--repo", "demo"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let finish = |n: u64| {
        (v["efforts"].as_array().unwrap().iter())
            .find(|e| e["number"] == n)
            .map(|e| e["finish"].clone())
            .unwrap()
    };
    assert_eq!(
        finish(1),
        "done when merged and the owner has looked at the 'release notes'"
    );
    assert_eq!(
        finish(2),
        "done when merged and the owner has looked at \"the board card\""
    );
}

/// Catches a worker reopening an effort another session closed under it: its save from the
/// copy it had before the close, and its retry once the refusal showed it the closed note,
/// must both leave the note done. Any session may close on the finish line now, so a close
/// can land while the worker is still live.
#[test]
fn a_save_from_a_copy_older_than_the_close_cant_reopen_it() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe activate --effort 1 --worker --model opus --reasoning medium",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe effort save <<'ASTROLABE'\nBuilding.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    // Another session sees the merge and closes it.
    assert!(
        mac.bash(
            "lead",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nlead the work\nASTROLABE\n"
        )
        .1
    );
    assert!(mac.bash("lead", "astrolabe effort read 1").1);
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --effort 1 --status done <<'ASTROLABE'\nDone and merged.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let closed = mac.effort_text(1);
    let before = mac.log().lines().count();
    for save in [
        "astrolabe effort save --status review --waits-for \"dispatcher: review and land\" <<'ASTROLABE'\nIn review: built.\nASTROLABE\n",
        "astrolabe effort save --status active <<'ASTROLABE'\nActive again.\nASTROLABE\n",
        "astrolabe effort save <<'ASTROLABE'\nStill building.\nASTROLABE\n",
    ] {
        let (out, ok) = mac.bash("w1", save);
        assert!(!ok && out.contains("Nothing was written"), "{save}: {out}");
        assert!(out.contains("is done"), "{save}: {out}");
        assert!(!out.contains("Save again from this"), "{save}: {out}");
        assert_eq!(mac.effort_text(1), closed, "{save}");
    }
    assert_eq!(mac.log().lines().count(), before);
}
