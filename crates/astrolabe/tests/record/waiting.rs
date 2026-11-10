//! Needs you is for a decision or action the owner can take now: what a save checks about
//! a wait on them, what it says when it sets one, and what a session leaving one is told.
use super::*;
use astrolabe::effort::EffortFile;

const BODY: &str = "<<'ASTROLABE'\nWorking on it.\nASTROLABE\n";

fn page(mac: &Machine) {
    write(
        &mac.notes.join("initiatives/smooth-lines.md"),
        include_str!("../fixtures/initiatives/prose.md"),
    );
    git(&mac.notes, &["add", "initiatives"]);
    git(&mac.notes, &["commit", "-qm", "legacy page"]);
}

fn wait(mac: &Machine, n: u32) -> String {
    EffortFile::parse(&mac.effort_text(n))
        .unwrap()
        .prop("waits_for")
        .unwrap_or_default()
        .to_owned()
}

/// Catches: the wait the session text used to teach, which names nothing to decide.
#[test]
fn a_wait_on_the_owner_that_only_says_look_is_refused() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.effort_text(1);
    for ask in [
        "look at the plan",
        "Look at the branch and land it",
        "take a look at the result",
        "have a look",
        "look",
    ] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save --status review --waits-for \"owner: {ask}\" {BODY}"),
        );
        assert!(
            !ok && out.contains("Effort NOT saved")
                && out.contains(&format!(
                    "--waits-for \"owner: {ask}\" doesn't say what the owner should decide or do"
                ))
                && out.contains("Lead with the decision or action and say where to see it")
                && out.contains("Nothing was written"),
            "{ask}: {out}"
        );
        assert_eq!(mac.effort_text(1), before, "{ask}");
    }
    // A decision, a word that only starts like "look", and a wait on anyone else go through.
    for wait_for in [
        "owner: land it on main? 4 seconds instead of 30, on branch export-batch",
        "owner: lookup table or a match, both measured",
        "dispatcher: look at the branch",
    ] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save --status review --waits-for \"{wait_for}\" {BODY}"),
        );
        assert!(ok, "{wait_for}: {out}");
        assert_eq!(wait(&mac, 1), wait_for);
    }
}

/// Catches: a dispatched worker putting its effort under Needs you, where no reply clears it;
/// and the refusal reaching an effort the owner gave an agent directly.
#[test]
fn a_dispatched_worker_cannot_wait_on_the_owner_and_an_ordinary_binding_can() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("lead", NEW).1);
    assert!(mac.bash("w1", "astrolabe activate --effort 1 --worker").1);
    let before = mac.effort_text(1);
    let ask = "owner: batch the rows or stream them? both pass, batching is 4 seconds faster";
    let (out, ok) = mac.bash(
        "w1",
        &format!("astrolabe effort save --status blocked --waits-for \"{ask}\" {BODY}"),
    );
    assert!(
        !ok && out.contains("Effort NOT saved")
            && out.contains("bound with --worker")
            && out.contains("--waits-for \"dispatcher: <question>\"")
            && out.contains("Nothing was written"),
        "{out}"
    );
    assert_eq!(mac.effort_text(1), before);
    // Its wait on the dispatcher goes through.
    let (out, ok) = mac.bash(
        "w1",
        &format!("astrolabe effort save --status review --waits-for \"dispatcher: review and land\" {BODY}"),
    );
    assert!(ok, "{out}");
    // The same decision from a session the owner gave the effort to, with no --worker.
    assert!(mac.bash("solo", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash(
        "solo",
        &format!("astrolabe effort save --status blocked --waits-for \"{ask}\" {BODY}"),
    );
    assert!(ok, "{out}");
    assert_eq!(wait(&mac, 1), ask);
    assert!(!out.contains("bound with --worker"), "{out}");
}

/// Catches: an owner wait set with nothing said about what the column is for, and the note
/// repeated on every save that keeps the same wait.
#[test]
fn a_save_that_sets_an_owner_wait_says_what_needs_you_is_for() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let note = "That column is for a decision or action the owner can take now that the notes don't already settle.";
    let (out, ok) = mac.bash("s1", &format!("astrolabe effort save {BODY}"));
    assert!(ok && !out.contains(note), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        &format!(
            "astrolabe effort save --status review --waits-for \"owner: land it on main?\" {BODY}"
        ),
    );
    assert!(
        ok && out.contains("Effort 1 now shows under Needs you as \"land it on main?\".")
            && out.contains(note)
            && out.contains("save again with --waits-for \"\" or with what it really waits on"),
        "{out}"
    );
    // Kept by a later save in review, and given again unchanged: said once.
    let (out, ok) = mac.bash("s1", &format!("astrolabe effort save {BODY}"));
    assert!(ok && !out.contains(note), "{out}");
    assert_eq!(wait(&mac, 1), "owner: land it on main?");
    let (out, ok) = mac.bash(
        "s1",
        &format!("astrolabe effort save --waits-for \"owner: land it on main?\" {BODY}"),
    );
    assert!(ok && !out.contains(note), "{out}");
    // A different ask is a new one.
    let (out, ok) = mac.bash(
        "s1",
        &format!(
            "astrolabe effort save --waits-for \"owner: land it, or drop the cache first?\" {BODY}"
        ),
    );
    assert!(ok && out.contains(note), "{out}");
    // A wait on anyone else says nothing.
    let (out, ok) = mac.bash(
        "s1",
        &format!("astrolabe effort save --waits-for \"the nightly build\" {BODY}"),
    );
    assert!(ok && !out.contains("Needs you"), "{out}");
    // By number, for an orchestrator's member, and on the page itself.
    page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --effort 1 --waits-for \"owner: land it on main?\"",
    );
    assert!(
        ok && out.contains("Effort 1 now shows under Needs you as \"land it on main?\".")
            && out.contains(note),
        "{out}"
    );
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --waits-for \"owner: which effort starts first?\" <<'ASTROLABE'\nFiled.\nASTROLABE\n",
    );
    assert!(
        ok && out.contains(
            "Initiative smooth-lines now shows under Needs you as \"which effort starts first?\"."
        ) && out.contains(note),
        "{out}"
    );
}

/// Catches: a session moving to another record while the one it leaves still shows under
/// Needs you, with nothing said (the audit effort sat there for hours, 2026-11-09).
#[test]
fn a_session_leaving_a_record_that_waits_on_the_owner_is_told_at_the_bind() {
    let world = World::new();
    let mac = world.machine("mac");
    page(&mac);
    assert!(mac.bash("s1", NEW).1);
    let second = "astrolabe effort new --title \"Second thing\" --status proposed <<'ASTROLABE'\nthe second thing\nASTROLABE\n";
    assert!(mac.bash("other", second).1);
    let leaving = "You're leaving effort 1, \"Smooth ink\", and it still shows under Needs you: in review, waiting on \"owner: land it on main?\".";
    // Nothing waits on the owner yet: nothing said.
    let (out, ok) = mac.bash("s1", "astrolabe activate --effort 2");
    assert!(ok && !out.contains("You're leaving"), "{out}");
    assert!(mac.bash("s1", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash(
        "s1",
        &format!(
            "astrolabe effort save --status review --waits-for \"owner: land it on main?\" {BODY}"
        ),
    );
    assert!(ok, "{out}");
    // Loading the same record again leaves nothing.
    let (out, ok) = mac.bash("s1", "astrolabe activate --effort 1");
    assert!(ok && !out.contains("You're leaving"), "{out}");
    // To another effort: said after the opening paragraph, with the command that clears it.
    let (out, ok) = mac.bash("s1", "astrolabe activate --effort 2");
    assert!(ok, "{out}");
    let mut paragraphs = out.split("\n\n");
    assert!(
        paragraphs
            .next()
            .unwrap()
            .starts_with("Astrolabe is on in this session for effort 2"),
        "{out}"
    );
    let said = paragraphs.next().unwrap();
    assert!(
        said.starts_with(leaving)
            && said.contains("clear it before anything else with `astrolabe effort save --effort 1 --waits-for \"\"`, or close it")
            && said.ends_with("If it still needs them, leave it and tell the owner it's still open."),
        "{out}"
    );
    // The command it names works from the new binding.
    let (cleared, ok) = mac.bash("s1", "astrolabe effort save --effort 1 --waits-for \"\"");
    assert!(ok, "{cleared}");
    assert_eq!(wait(&mac, 1), "");
    // To an initiative, and by a new effort.
    for (session, bind) in [
        ("s2", "astrolabe activate --initiative smooth-lines"),
        (
            "s3",
            "astrolabe effort new --title \"Third thing\" <<'ASTROLABE'\nthe third thing\nASTROLABE\n",
        ),
    ] {
        assert!(mac.bash(session, "astrolabe activate --effort 1").1);
        let (out, ok) = mac.bash(
            session,
            &format!("astrolabe effort save --status review --waits-for \"owner: land it on main?\" {BODY}"),
        );
        assert!(ok, "{out}");
        let (out, ok) = mac.bash(session, bind);
        assert!(ok && out.contains(leaving), "{bind}: {out}");
    }
    // Blocked keeps what it waits on, so the way out is a status.
    assert!(mac.bash("s4", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash(
        "s4",
        &format!(
            "astrolabe effort save --status blocked --waits-for \"owner: batch or stream?\" {BODY}"
        ),
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s4", "astrolabe activate --effort 2");
    assert!(
        ok && out.contains("it still shows under Needs you: blocked, waiting on \"owner: batch or stream?\".")
            && out.contains("`astrolabe effort read 1`, then `astrolabe effort save --effort 1 --status active` with a Current state body"),
        "{out}"
    );
    // Dropping the effort from the session says so too.
    assert!(mac.bash("s5", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash("s5", "astrolabe activate --off");
    assert!(
        ok && out.contains(
            "Effort 1 still shows under Needs you: blocked, waiting on \"owner: batch or stream?\"."
        ) && out.contains("load it again and clear that"),
        "{out}"
    );
    // A closed record waits on nothing, whatever an old note says.
    assert!(mac.bash("s6", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash(
        "s6",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and merged.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s6", "astrolabe activate --effort 2");
    assert!(ok && !out.contains("You're leaving"), "{out}");
}

/// Catches: an orchestrator leaving its page for an effort while the page waits on the owner.
#[test]
fn a_session_leaving_an_initiative_page_that_waits_on_the_owner_is_told() {
    let world = World::new();
    let mac = world.machine("mac");
    page(&mac);
    assert!(mac.bash("other", NEW).1);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --waits-for \"owner: which effort starts first?\" <<'ASTROLABE'\nFiled.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("lead", "astrolabe activate --effort 1");
    assert!(
        ok && out.contains("You're leaving initiative smooth-lines, \"Smooth lines\", and it still shows under Needs you: active, waiting on \"owner: which effort starts first?\".")
            && out.contains("bind back with `astrolabe activate --initiative smooth-lines` and run `astrolabe effort save --waits-for \"\"` before anything else"),
        "{out}"
    );
}

/// Catches: the rename ask buried ahead of every note, where an orchestrator skipped it twice,
/// and an initiative's session told it's on an effort.
#[test]
fn the_rename_ask_is_the_last_thing_a_bind_says() {
    let world = World::new();
    let mac = world.machine("mac");
    page(&mac);
    assert!(mac.bash("other", NEW).1);
    let (out, ok) = mac.bash("s1", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let last = out.trim_end().rsplit("\n\n").next().unwrap();
    assert!(
        last.starts_with("Rename this session to \"1 Smooth ink\" now")
            && last.contains("so the owner can see which effort it's on."),
        "{out}"
    );
    assert_eq!(out.matches("Rename this session").count(), 1, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe activate --initiative smooth-lines");
    assert!(ok, "{out}");
    let last = out.trim_end().rsplit("\n\n").next().unwrap();
    assert!(
        last.starts_with("Rename this session to \"Smooth lines\" now")
            && last.contains("so the owner can see which initiative it's on."),
        "{out}"
    );
}

fn page_wait(mac: &Machine) -> (String, String) {
    let text = std::fs::read_to_string(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let file = EffortFile::parse(&text).unwrap();
    (
        file.prop("status").unwrap_or_default().to_owned(),
        file.prop("waits_for").unwrap_or_default().to_owned(),
    )
}

/// Catches: a notice whose advice leaves the wait where it was. A save with no --waits-for
/// keeps the wait of a blocked or review record, and an empty one is refused for blocked,
/// so each status needs the command that really clears it.
#[test]
fn what_the_leaving_notice_says_to_run_clears_the_wait_from_that_status() {
    let world = World::new();
    let mac = world.machine("mac");
    page(&mac);
    assert!(mac.bash("other", NEW).1);
    let ask = "owner: which effort starts first?";
    for (status, said, clear) in [
        (
            "blocked",
            "bind back with `astrolabe activate --initiative smooth-lines` and save it with `astrolabe effort save --status active` and a Current state body before anything else",
            "astrolabe effort save --status active <<'ASTROLABE'\nThe order is settled.\nASTROLABE\n",
        ),
        (
            "active",
            "bind back with `astrolabe activate --initiative smooth-lines` and run `astrolabe effort save --waits-for \"\"` before anything else",
            "astrolabe effort save --waits-for \"\"",
        ),
    ] {
        assert!(
            mac.bash("lead", "astrolabe activate --initiative smooth-lines")
                .1
        );
        let (out, ok) = mac.bash(
            "lead",
            &format!("astrolabe effort save --status {status} --waits-for \"{ask}\" <<'ASTROLABE'\nFiled.\nASTROLABE\n"),
        );
        assert!(ok, "{out}");
        assert_eq!(page_wait(&mac), (status.to_owned(), ask.to_owned()));
        let (out, ok) = mac.bash("lead", "astrolabe activate --effort 1");
        assert!(ok && out.contains(said), "{status}: {out}");
        assert!(
            !out.contains("save it without --waits-for"),
            "{status}: {out}"
        );
        // Done as told, it's gone from Needs you.
        assert!(
            mac.bash("lead", "astrolabe activate --initiative smooth-lines")
                .1
        );
        let (out, ok) = mac.bash("lead", clear);
        assert!(ok, "{status}: {out}");
        assert_eq!(
            page_wait(&mac),
            ("active".to_owned(), String::new()),
            "{status}"
        );
    }
    // The same two commands on an effort loaded again, from review and from blocked.
    for (status, clear) in [
        ("review", "astrolabe effort save --waits-for \"\""),
        (
            "blocked",
            "astrolabe effort save --status active <<'ASTROLABE'\nAnswered.\nASTROLABE\n",
        ),
    ] {
        assert!(mac.bash("s1", "astrolabe activate --effort 1").1);
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save --status {status} --waits-for \"owner: batch or stream?\" {BODY}"),
        );
        assert!(ok, "{out}");
        let (out, ok) = mac.bash("s1", clear);
        assert!(ok, "{status}: {out}");
        assert_eq!(wait(&mac, 1), "", "{status}");
    }
}

/// Catches: a session that becomes a worker told to run a save by number and a read, both
/// of which a worker is refused.
#[test]
fn a_session_leaving_for_a_worker_binding_is_told_what_a_worker_can_do() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let second = "astrolabe effort new --title \"Second thing\" --status proposed <<'ASTROLABE'\nthe second thing\nASTROLABE\n";
    assert!(mac.bash("other", second).1);
    let (out, ok) = mac.bash(
        "s1",
        &format!(
            "astrolabe effort save --status review --waits-for \"owner: land it on main?\" {BODY}"
        ),
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe activate --effort 2 --worker");
    assert!(
        ok && out.contains("You're leaving effort 1, \"Smooth ink\", and it still shows under Needs you: in review, waiting on \"owner: land it on main?\".")
            && out.contains("A worker saves only its own effort, so tell your dispatcher now that effort 1 still shows there"),
        "{out}"
    );
    for refused in ["effort save --effort 1", "effort read 1"] {
        assert!(!out.contains(refused), "{refused}: {out}");
    }
}

/// Catches: the look that hides behind a word ("please look", "have a quick look") and an
/// ask with nothing in it, while a decision that starts like one still goes through.
#[test]
fn a_look_behind_another_word_and_an_empty_ask_are_refused() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    for ask in [
        "owner: please look at the PR",
        "owner: Please take a look at the PR",
        "owner: have a quick look",
        "owner: take another look at the branch",
        "owner:",
        "owner: ",
    ] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save --status review --waits-for \"{ask}\" {BODY}"),
        );
        assert!(
            !ok && out.contains("doesn't say what the owner should decide or do"),
            "{ask:?}: {out}"
        );
    }
    for ask in [
        "owner: looks good to land on main?",
        "owner: lookup table A or B?",
        "owner: have the export take a path, or look it up from the config?",
    ] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save --status review --waits-for \"{ask}\" {BODY}"),
        );
        assert!(ok, "{ask}: {out}");
        assert_eq!(wait(&mac, 1), ask);
    }
}

/// Catches: a record that starts showing under Needs you with nothing said, because the wait
/// it carries is one an old note already held while it was done.
#[test]
fn a_reopened_record_that_newly_shows_under_needs_you_gets_the_note() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and merged.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    // As a note written before saves refused a wait on a closed record.
    let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 1)
        .unwrap();
    let text = mac.effort_text(1);
    assert!(text.contains("waits_for: \"\""), "{text}");
    write(
        &mac.notes.join(&rel),
        &text.replace("waits_for: \"\"", "waits_for: \"owner: three picks\""),
    );
    git(&mac.notes, &["commit", "-qam", "an old note"]);
    assert!(mac.bash("s2", "astrolabe activate --effort 1").1);
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort save --status active --waits-for \"owner: three picks\" <<'ASTROLABE'\nOpen again.\nASTROLABE\n",
    );
    assert!(
        ok && out.contains("Effort 1 now shows under Needs you as \"three picks\"."),
        "{out}"
    );
}
