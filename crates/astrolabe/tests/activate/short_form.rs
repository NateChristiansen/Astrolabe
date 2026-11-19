//! A session that already has the standing text gets a short form on a rebind or a filing:
//! the binding's own settings, state and pieces, with what it was already sent left out.
use super::*;

/// From the effort rules every effort binding carries.
const RULES: &str = "Pieces are a living list";
/// From the worker's rules and its closing rule.
const WORKER_RULES: &str = "Needs you isn't a dispatched worker's to set";
const WORKER_CLOSING: &str = "dispatched by another agent";
const SHORT: &str = "astrolabe activate --full";

fn bind(f: &Fixture, session: &str, effort: &str) -> String {
    let (text, ok) = f.bash(
        session,
        &f.demo,
        &format!("astrolabe activate --effort {effort}"),
    );
    assert!(ok, "{text}");
    text
}

fn has_all_standing_text(text: &str) -> bool {
    [RULES, "VOICE-RULE", "REPO-DEMO-TEXT", "MACHINE-TEXT"]
        .iter()
        .all(|t| text.contains(t))
        && !text.contains(SHORT)
}

#[test]
fn a_first_bind_sends_everything_and_a_rebind_only_what_the_binding_brings() {
    // Catches a rebind that sends all 20,000 characters again, and a short form that drops
    // the new effort's own settings or state with the standing text.
    let f = Fixture::new();
    let first = bind(&f, "s1", "70");
    assert!(has_all_standing_text(&first), "{first}");
    let again = bind(&f, "s1", "75");
    for gone in [RULES, "VOICE-RULE", "REPO-DEMO-TEXT", "MACHINE-TEXT"] {
        assert!(!again.contains(gone), "{gone} sent again: {again}");
    }
    for kept in [
        "EFFORT-75",
        "involvement: autonomous",
        "authority: read",
        SHORT,
        "an effort's rules",
        "knowledge/voice.md",
    ] {
        assert!(again.contains(kept), "{kept} missing: {again}");
    }
    assert!(
        again.len() * 2 < first.len(),
        "short {} vs full {}",
        again.len(),
        first.len()
    );
}

#[test]
fn a_note_changed_since_goes_again_in_full_and_the_rest_stays_out() {
    // Catches a record keyed on names alone, which would hold back a note that changed.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    write(
        &f.notes().join("knowledge/machines.md"),
        "MACHINE-TEXT-NEWER",
    );
    let again = bind(&f, "s1", "75");
    assert!(again.contains("MACHINE-TEXT-NEWER"), "{again}");
    assert!(again.contains("Changed or new since"), "{again}");
    assert!(
        !again.contains(RULES) && !again.contains("VOICE-RULE"),
        "{again}"
    );
    // Now that it's been sent, it stays out too.
    let third = bind(&f, "s1", "70");
    assert!(!third.contains("MACHINE-TEXT-NEWER"), "{third}");
}

#[test]
fn a_note_checked_out_with_windows_line_endings_counts_as_the_same_note() {
    // Catches a hash of raw bytes, under which every rebind on Windows would differ.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    let voice = f.notes().join("knowledge/voice.md");
    let text = std::fs::read_to_string(&voice).unwrap();
    std::fs::write(&voice, text.replace('\n', "\r\n")).unwrap();
    let again = bind(&f, "s1", "75");
    assert!(!again.contains("VOICE-RULE"), "{again}");
}

#[test]
fn a_worker_bind_after_an_effort_bind_gets_the_worker_rules_whole() {
    // Catches a record that doesn't tell one role's rules from another's.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    let (text, ok) = f.bash(
        "s1",
        &f.demo,
        "astrolabe activate --effort 75 --worker --model opus --reasoning medium",
    );
    assert!(ok, "{text}");
    assert!(
        text.contains(WORKER_RULES) && text.contains(WORKER_CLOSING),
        "{text}"
    );
    assert!(!text.contains("VOICE-RULE"), "{text}");
    // The same worker binding again leaves its rules out too.
    let again = bind(&f, "s1", "75");
    assert!(
        !again.contains(WORKER_RULES) && !again.contains(WORKER_CLOSING),
        "{again}"
    );
    assert!(again.contains(SHORT), "{again}");
}

#[test]
fn a_new_session_gets_everything_whatever_another_was_sent() {
    // Catches a record kept per machine or per effort instead of per session.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    let other = bind(&f, "s2", "70");
    assert!(has_all_standing_text(&other), "{other}");
}

#[test]
fn after_a_compaction_or_any_session_start_the_next_rebind_sends_everything() {
    // Catches a compacted session that's told it already has text it lost.
    let f = Fixture::new();
    for source in ["compact", "resume", "clear", "startup"] {
        let session = format!("s-{source}");
        bind(&f, &session, "70");
        let input = json!({"session_id": session, "cwd": f.demo, "source": source});
        let (out, ok) = f.run(
            &f.demo,
            &["hook", "session-start", "--if-active"],
            &input.to_string(),
            &[],
        );
        assert!(ok && out.contains(RULES), "{source}: {out}");
        let again = bind(&f, &session, "75");
        assert!(
            again.contains(RULES) && again.contains("VOICE-RULE") && !again.contains(SHORT),
            "{source}: {again}"
        );
    }
}

#[test]
fn filing_an_effort_after_a_bind_sends_the_short_form() {
    // Catches filing, which binds the new effort, sending everything again: the cost that
    // left 25 approved efforts in prose.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    let (out, ok) = f.bash(
        "s1",
        &f.demo,
        "astrolabe effort new --title \"Filed later\" <<'ASTROLABE'\nA goal.\nASTROLABE\n",
    );
    assert!(ok && out.contains("created"), "{out}");
    assert!(!out.contains(RULES) && !out.contains("VOICE-RULE"), "{out}");
    assert!(out.contains(SHORT) && out.contains("involvement:"), "{out}");
    // A second filing is short too: the first one's record was kept.
    let (out, ok) = f.bash(
        "s1",
        &f.demo,
        "astrolabe effort new --title \"Filed later too\" <<'ASTROLABE'\nAnother goal.\nASTROLABE\n",
    );
    assert!(ok && out.contains(SHORT) && !out.contains(RULES), "{out}");
}

#[test]
fn filing_an_effort_as_the_first_bind_sends_everything() {
    // Catches a filing that takes an empty record for one that has everything.
    let f = Fixture::new();
    let (out, ok) = f.bash(
        "s1",
        &f.demo,
        "astrolabe effort new --title \"First thing\" <<'ASTROLABE'\nA goal.\nASTROLABE\n",
    );
    assert!(ok && out.contains(RULES) && !out.contains(SHORT), "{out}");
    let again = bind(&f, "s1", "70");
    assert!(again.contains(SHORT) && !again.contains(RULES), "{again}");
}

#[test]
fn full_prints_the_whole_text_and_changes_only_the_record() {
    // Catches a fetch command that rebinds, drops a worker, or leaves out the owner's note.
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --full");
    assert!(
        !ok && text.contains("no effort or initiative loaded"),
        "{text}"
    );
    bind(&f, "s1", "70");
    bind(&f, "s1", "75");
    let before = std::fs::read_to_string(f.state().join("s1.json")).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --full");
    assert!(ok, "{text}");
    for whole in [
        RULES,
        "VOICE-RULE",
        "REPO-DEMO-TEXT",
        "OWNER-TEXT",
        "EFFORT-75",
    ] {
        assert!(text.contains(whole), "{whole} missing: {text}");
    }
    assert!(!text.contains(SHORT), "{text}");
    let after: Value =
        serde_json::from_str(&std::fs::read_to_string(f.state().join("s1.json")).unwrap()).unwrap();
    let mut before: Value = serde_json::from_str(&before).unwrap();
    before["sent"] = after["sent"].clone();
    assert_eq!(before, after, "only what was sent is recorded");
    // And the next rebind is short again.
    assert!(bind(&f, "s1", "70").contains(SHORT));
}

#[test]
fn an_orchestrator_rebinding_its_initiative_gets_the_short_form_and_an_effort_bind_after_it_the_effort_rules()
 {
    // Catches the initiative route missed by the short form, and an orchestrator's rules
    // taken for an effort's when it then binds an effort.
    let f = Fixture::new();
    write(
        &f.notes().join("initiatives/one-repo.md"),
        "---\nstatus: active\nrepo: demo\n---\n# One repo\n\n## Current state\n\nPAGE-TEXT\n",
    );
    let bind_page = || {
        let (text, ok) = f.bash("o1", &f.demo, "astrolabe activate --initiative one-repo");
        assert!(ok, "{text}");
        text
    };
    let first = bind_page();
    assert!(
        first.contains("Start each worker as its own separate session") && !first.contains(SHORT)
    );
    let again = bind_page();
    assert!(
        again.contains("PAGE-TEXT") && again.contains(SHORT),
        "{again}"
    );
    assert!(
        !again.contains("Start each worker as its own separate session"),
        "{again}"
    );
    assert!(again.contains("an orchestrator's rules"), "{again}");
    let effort = bind(&f, "o1", "70");
    assert!(effort.contains(RULES) && effort.contains(SHORT), "{effort}");
    assert!(!effort.contains("VOICE-RULE"), "{effort}");
}

#[test]
fn a_reload_that_fails_after_a_compaction_leaves_the_repair_rebind_whole() {
    // Catches a record cleared only when the reload works: the session is told to repair and
    // bind again, and that bind would leave out what the compaction took.
    let f = Fixture::new();
    bind(&f, "s1", "70");
    bind(&f, "s1", "75");
    let machines = f.notes().join("knowledge/machines.md");
    std::fs::remove_file(&machines).unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "compact"});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input.to_string(),
        &[],
    );
    assert!(out.contains("blocked"), "the reload failed: {out}");
    std::fs::write(&machines, "MACHINE-TEXT").unwrap();
    let repaired = bind(&f, "s1", "75");
    assert!(has_all_standing_text(&repaired), "{repaired}");
}
