use super::*;

#[test]
fn a_new_effort_is_numbered_stamped_journaled_committed_and_bound() {
    let world = World::new();
    let mac = world.machine("mac");
    let start = mac.session_start("s1", "startup");
    assert!(start.contains("astrolabe effort new"), "{start}");
    // A labelled goal is refused before the lock and pull: no file, no commit.
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Nope\" <<'ASTROLABE'\n**Goal:** g.\nASTROLABE\n",
    );
    assert!(!ok && out.contains("no **Label:** lines"), "{out}");
    assert_eq!(mac.log().lines().count(), before);
    assert!(!mac.notes.join(shelf("0001-nope.md")).exists());
    let (out, ok) = mac.bash("s1", NEW);
    assert!(ok, "{out}");
    assert!(
        out.contains(&format!(
            "Effort 1 created as {}, status active, bound to this session",
            shelf("0001-smooth-ink.md")
        )),
        "{out}"
    );
    assert!(
        out.contains("involvement check in at plan · authority edit"),
        "{out}"
    );
    assert!(
        out.contains("Astrolabe is on in this session for effort 1"),
        "{out}"
    );
    let text = mac.effort_text(1);
    for line in [
        "status: active",
        "repo: demo",
        "involvement: check in at plan",
        "authority: edit",
        "saved_by: mac · claude",
        "branch: main",
        "dirty: false",
        "# 0001 Smooth ink",
        "## Current state\nsmooth the ink across frames, with café in it\n",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    // Starting is state, not history: no journal line until the agent writes one.
    assert!(
        !mac.notes.join("journal").exists(),
        "{}",
        mac.journal_today()
    );
    assert!(mac.log().starts_with("0001: started\n"), "{}", mac.log());
    let attributes = std::fs::read_to_string(mac.notes.join(".gitattributes")).unwrap();
    assert!(attributes.contains("* text=auto eol=lf"), "{attributes}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    assert!(state["delivered"].is_string() && state["baseline"].is_string());
    // The same command typed again with no hook explains itself from the ticket.
    let (plain, ok) = mac.plain(NEW);
    assert!(ok, "{plain}");
    assert!(plain.contains("Effort 1 created"), "{plain}");
    // A different plain command with no hook is NOT recorded.
    let (plain, ok) =
        mac.plain("astrolabe effort new --title \"Other\" <<'ASTROLABE'\ngoal\nASTROLABE\n");
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
    // A subagent can't.
    let (out, ok) = mac.bash_as("s1", NEW, Some("child-1"));
    assert!(!ok && out.contains("subagent"), "{out}");
    // Resume re-injects the effort with its freshness.
    let resumed = mac.session_start("s1", "resume");
    assert!(
        resumed.contains("Last saved by mac · claude at"),
        "{resumed}"
    );
}

#[test]
fn current_state_and_blocked_status_are_saved() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n**Goal:** g.\n**Done:** half.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("no **Label:** lines (**Goal:** g.)"),
        "{out}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive, half done.\n\nNext: the rest.\nASTROLABE\n",
    );
    assert!(ok && out.contains("Current state replaced"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status blocked");
    assert!(!ok && out.contains("--waits-for"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status blocked --waits-for \"owner: should it be X?\"",
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("waits_for: \"owner: should it be X?\""),
        "{}",
        mac.effort_text(1)
    );
    // Blocked is state: nothing in the journal.
    assert!(!mac.notes.join("journal").exists());
}

#[test]
fn a_later_save_clears_the_owner_wait_unless_still_blocked() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    let body = "<<'ASTROLABE'\nWorking on it.\nASTROLABE\n";
    // A check-in while active: the owner answers, and the next save carries no status.
    let (out, ok) = mac.bash(
        "s1",
        &format!("astrolabe effort save --waits-for \"owner: look at the plan\" {body}"),
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("waits_for: \"owner: look at the plan\"")
    );
    let (out, ok) = mac.bash("s1", &format!("astrolabe effort save {body}"));
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("waits_for: \"\"") && text.contains("status: active"),
        "{text}"
    );
    // Blocked keeps its condition across a progress save.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status blocked --waits-for \"owner: should it be X?\"",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", &format!("astrolabe effort save {body}"));
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("waits_for: \"owner: should it be X?\"")
    );
    // ...and can't be emptied while it stays blocked.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --waits-for \"\"");
    assert!(!ok && out.contains("blocked effort keeps"), "{out}");
    // An empty value clears it alongside a status that isn't blocked.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status active --waits-for \"\"",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("waits_for: \"\""));
}

#[test]
fn prose_validation_refuses_unsupported_shapes() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    // A long block and a long journal line are refused before anything is written.
    let long = (0..151).map(|_| "word").collect::<Vec<_>>().join(" ");
    let (out, ok) = mac.bash(
        "s1",
        &format!("astrolabe effort save <<'ASTROLABE'\n{long}\nASTROLABE\n"),
    );
    assert!(
        !ok && out.contains("151 words") && out.contains("under 150"),
        "{out}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"one two three four five six seven eight nine ten one two three four five six seven eight nine ten one two three four five six seven eight nine ten one two three four five six seven eight nine ten one\"");
    assert!(
        !ok && out.contains("41 words") && out.contains("aim for 25"),
        "{out}"
    );
    // The shapes sessions wrote with the rule alone are refused with what to write instead.
    for (line, why) in [
        (
            "Landed the ink smoothing helper. Merged.",
            "where the work stands",
        ),
        (
            "Strokes stop splitting across frames. Dropped the stand-in. Merged and pushed.",
            "3 sentences",
        ),
        (
            "Chat replies drift without voice.md at start.",
            "names a file (voice.md)",
        ),
        (
            "Replies lose the owner's voice without the voice note.",
            "the owner",
        ),
        (
            "Moved the hook script out of src/hooks/old and it fires again.",
            "names a file",
        ),
    ] {
        let (out, ok) = mac.bash("s1", &format!("astrolabe effort save --journal \"{line}\""));
        assert!(
            !ok && out.contains(why) && out.contains("Nothing was written"),
            "{line}: {out}"
        );
    }
    // A name after a full stop, a decimal, a slash between words and a status word inside a
    // sentence are not those shapes.
    for line in [
        "Audited the notes with Astra: five wrong facts gone. viral-studio is cloned and registered.",
        "Cut the boot to about 1.5 s. Dropped the stand-in and send-ahead.",
        "Deleted 22 landed branches on the Mac/PC pair, keeping the old Windows branch with an unmerged commit.",
    ] {
        let (out, ok) = mac.bash("s1", &format!("astrolabe effort save --journal \"{line}\""));
        assert!(ok, "{line}: {out}");
    }
}

#[test]
fn journal_replacement_keeps_one_line_per_day() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --status blocked --waits-for waiting"
        )
        .1
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status done --decision \"X over Y: cheaper\" --journal \"Started smoothing the ink; the first frame still jumps.\" <<'ASTROLABE'\nDone: merged the ink MR.\nASTROLABE\n");
    assert!(
        ok && out.contains("status done") && out.contains("journal line written"),
        "{out}"
    );
    let text = mac.effort_text(1);
    assert!(
        text.contains("status: done")
            && text.contains("waits_for: \"\"")
            && text.contains("- 20")
            && text.contains(": X over Y: cheaper"),
        "{text}"
    );
    // The journal holds only the agent's sentence, with a hidden tail.
    let journal = mac.journal_today();
    assert!(
        journal.starts_with(
            "- Started smoothing the ink; the first frame still jumps. <!-- 0001 mac "
        ) && journal.lines().count() == 1,
        "{journal}"
    );
    assert_eq!(mac.log().lines().next(), Some("0001: done"));
    // A later line the same day replaces this writer's earlier one: one per effort per day.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"Finished the ink smoothing: strokes no longer split across frames.\"");
    assert!(ok, "{out}");
    let journal = mac.journal_today();
    assert!(
        journal.starts_with("- Finished the ink smoothing") && journal.lines().count() == 1,
        "{journal}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"two\nlines\"");
    assert!(!ok && out.contains("one line"), "{out}");
}

#[test]
fn plan_sources_preserve_section_boundaries() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    // The plan file is read as the user, relative to the session's folder.
    write(&mac.demo.join("plan.md"), "the plan");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status active --plan-file plan.md",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("## Plan\nthe plan\n"));
    assert!(mac.journal_today().is_empty());
    // Codex keeps no plan file: the plan rides in the body under a `## Plan` line, and the
    // fields above it are the Current state.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\n\n## Plan\n1. first\n2. second\nASTROLABE",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(text.contains("## Plan\n1. first\n2. second\n"), "{text}");
    assert!(
        text.contains("## Current state\nActive: g.\n\nNext: n.\n\n## "),
        "{text}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --plan-file plan.md <<'ASTROLABE'\n## Plan\nagain\nASTROLABE",
    );
    assert!(!ok && out.contains("one way"), "{out}");
    assert!(mac.effort_text(1).contains("## Plan\n1. first\n"));
    // A heading inside a section would split it on the next read. The plan is the app's
    // file, so its headings are demoted and said so; an indented one counts, as it's stored
    // trimmed. Current state and Pieces are refused.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n## Plan\n## Plan\nsecond\nASTROLABE",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    assert!(mac.effort_text(1).contains("## Plan\n### Plan\nsecond\n"));
    write(
        &mac.demo.join("plan.md"),
        "  ## Current state\n**Goal:** stolen",
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --plan-file plan.md");
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Current state\n**Goal:** stolen\n"),
        "{text}"
    );
    assert!(text.contains("## Current state\nActive: g.\n"), "{text}");
    // The plan sits below Decisions, and a plan file's own `# ` title and its `## ` parts
    // shift together so the top lands at `###`.
    write(
        &mac.demo.join("plan.md"),
        "# Plan: ink\n\n## Context\nwhy\n### Step 1\n#### sub\n#not a heading\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status proposed --plan-file plan.md --decision \"a over b: c\"",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Plan: ink\n\n#### Context\nwhy\n##### Step 1\n###### sub\n#not a heading\n"),
        "{text}"
    );
    assert!(
        text.find("## Decisions").unwrap() < text.find("## Plan").unwrap(),
        "{text}"
    );
    // A plan written only at `####` lifts to `###`; bold prose in Current state is not a
    // label.
    write(&mac.demo.join("plan.md"), "#### Only\n##### deep\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status active --plan-file plan.md <<'ASTROLABE'\n**Done.** Merged the **parser:** cleanup.\n\nNext: n.\nASTROLABE");
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(text.contains("## Plan\n### Only\n#### deep\n"), "{text}");
    assert!(
        text.contains("## Current state\n**Done.** Merged"),
        "{text}"
    );
    // Only headings change, all by the same step: line endings and an indented line (no
    // heading to the reader) stay as they were.
    write(
        &mac.demo.join("plan.md"),
        "## Steps\r\n  ## not a heading\r\n### sub\r\n",
    );
    // A different command from the one above, or it reads as that call's retry.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status ready --plan-file plan.md",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Steps\r\n  ## not a heading\r\n#### sub\n"),
        "{text:?}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\ng.\n## Decisions\nASTROLABE",
    );
    assert!(!ok && out.contains("can't hold a `## ` heading"), "{out}");
    write(&mac.demo.join("pieces.md"), "## Pieces\n- [ ] x\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file pieces.md");
    assert!(!ok && out.contains("can't hold a `## ` heading"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n## Plan\n### Step one\nfirst\n### Step two\nsecond\nASTROLABE",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status active");
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Step one\nfirst\n### Step two\nsecond\n"),
        "{text}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save");
    assert!(!ok && out.contains("nothing to save"), "{out}");
}

#[test]
fn pieces_are_matched_by_name_and_survive_saves() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    // Pieces: a checklist the owner reads, ticked by name, counted for the home view.
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Tether cap\n- [ ] Goo at speed\n- [ ] Goo splats\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --pieces-file pieces.md --tick \"Tether\"",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Pieces\n- [x] Tether cap\n- [ ] Goo at speed\n- [ ] Goo splats\n"),
        "{text}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Goo\"");
    assert!(!ok && out.contains("more than one piece"), "{out}");
    assert!(
        out.contains("Goo at speed") && out.contains("Goo splats"),
        "{out}"
    );
    assert_eq!(mac.effort_text(1), text, "an ambiguous tick must not save");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Nothing\"");
    assert!(!ok && out.contains("no piece starts with"), "{out}");
    for label in ["Tether cap", "Goo at speed", "Goo splats"] {
        assert!(out.contains(label), "{out}");
    }
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --tick \"Goo at\" --untick \"Goo at speed\"",
    );
    assert!(!ok && out.contains("both ticked and unticked"), "{out}");
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --tick \"Goo at\" --untick \"Tether\""
        )
        .1
    );
    assert!(
        mac.effort_text(1)
            .contains("- [ ] Tether cap\n- [x] Goo at speed\n")
    );
    // Pieces survive a Current state save.
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\nASTROLABE\n"
        )
        .1
    );
    assert!(mac.effort_text(1).contains("- [x] Goo at speed"));
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["efforts"][0]["pieces"], json!({"done": 1, "total": 3}));
}

#[test]
fn inline_code_pieces_can_be_ticked_without_changing_their_text() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let piece = "First repo note: `astrolabe repo add <name>` from that repo";
    write(&mac.demo.join("pieces.md"), &format!("- [ ] {piece}\n"));
    assert!(
        mac.bash("s1", "astrolabe effort save --pieces-file pieces.md")
            .1
    );
    let before = mac.effort_text(1);
    // The hook refuses shell-sensitive values and tells the agent a usable alternative.
    for value in [piece, "First $repo note", "First \"repo\" note"] {
        let (out, ok) = mac.bash("s1", &format!("astrolabe effort save --tick \"{value}\""));
        assert!(!ok, "{out}");
        assert!(
            out.contains("no $, backtick or inner double quote"),
            "{out}"
        );
        assert!(
            out.contains("unambiguous prefix") && out.contains("--tick \"First repo note\""),
            "{out}"
        );
        assert_eq!(mac.effort_text(1), before);
    }
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"first repo note\"");
    assert!(ok, "{out}");
    // Every save restamps `updated` to the minute, so the minute may roll over between
    // the two saves; only the tick is what this test is about.
    let without_updated = |text: &str| -> String {
        text.lines()
            .filter(|line| !line.starts_with("updated:"))
            .map(|line| format!("{line}\n"))
            .collect()
    };
    assert_eq!(
        without_updated(&mac.effort_text(1)),
        without_updated(&before.replace(&format!("- [ ] {piece}"), &format!("- [x] {piece}")))
    );
}

#[test]
fn an_exact_piece_label_wins_over_longer_labels_even_when_already_checked() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Repo note\n- [ ] Repo note examples\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --pieces-file pieces.md --tick \"repo note\"",
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("- [x] Repo note\n- [ ] Repo note examples")
    );
    // A different command avoids replaying the previous receipt. The exact checked label
    // still wins rather than checking the longer unfinished label.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Repo note\"");
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("- [x] Repo note\n- [ ] Repo note examples")
    );
    // Unticking is outside this change: its existing prefix rule still considers the
    // longer label even when the value is an exact label.
    let before = mac.effort_text(1);
    let (out, ok) = mac.bash("s1", "astrolabe effort save --untick \"Repo note\"");
    assert!(!ok && out.contains("more than one piece"), "{out}");
    assert_eq!(mac.effort_text(1), before);
}

#[test]
fn tick_prefixes_consider_only_unfinished_pieces_and_resolve_before_any_ticks() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    write(
        &mac.demo.join("pieces.md"),
        "- [x] Repo setup\n- [ ] Repo note\n",
    );
    // Both prefixes select the same unfinished piece. Resolve them once so the second
    // cannot fail or select another piece after the first tick changes the checklist.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --pieces-file pieces.md --tick Repo --tick \"Repo n\"",
    );
    assert!(ok, "{out}");
    let before = mac.effort_text(1);
    assert!(before.contains("- [x] Repo setup\n- [x] Repo note"));
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick Repo");
    assert!(
        !ok && out.contains("no piece starts with") && out.contains("unfinished"),
        "{out}"
    );
    assert!(
        out.contains("Repo setup") && out.contains("Repo note"),
        "{out}"
    );
    assert_eq!(mac.effort_text(1), before);
}

#[test]
fn a_save_over_a_changed_file_is_refused_then_goes_through_from_the_new_text() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The owner edits the note by hand.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("smooth the ink", "OWNER EDIT smooth the ink");
    std::fs::write(&path, edited).unwrap();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nMine.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("changed since it was delivered") && out.contains("OWNER EDIT"),
        "{out}"
    );
    assert!(
        mac.effort_text(1).contains("OWNER EDIT"),
        "nothing overwritten"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nOWNER EDIT kept, mine too.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("OWNER EDIT kept"));
}

#[test]
fn a_crlf_copy_of_the_effort_between_saves_is_not_a_change() {
    // Git for Windows checks text out with CRLF (its system `core.autocrlf=true`), so a
    // pull can rewrite the file with nothing in it changed.
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nFirst.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let lf = std::fs::read_to_string(&path).unwrap();
    assert!(!lf.contains('\r'));
    std::fs::write(&path, lf.replace('\n', "\r\n")).unwrap();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nSecond.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("Second."));
}

#[test]
fn nested_decision_details_survive_a_save() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.bash("s1", "astrolabe effort save --decision first").1);
    // Nested decision text survives a save.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let text =
        std::fs::read_to_string(&path).unwrap() + "  nested detail under the last decision\n";
    std::fs::write(&path, &text).unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"again\"");
    assert!(!ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"again\"");
    assert!(ok, "{out}");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("nested detail under the last decision") && saved.contains(": again"),
        "{saved}"
    );
}

#[test]
fn an_abandoned_effort_keeps_its_reason_in_decisions() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.effort_text(1);
    // No reason, or a reason with no abandon to explain, is refused and writes nothing.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status abandoned");
    assert!(!ok && out.contains("--reason"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done --reason \"stale\"",
    );
    assert!(
        !ok && out.contains("--status abandoned") && out.contains("Nothing was written"),
        "{out}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --reason \"stale\"");
    assert!(!ok && out.contains("--status abandoned"), "{out}");
    assert_eq!(mac.effort_text(1), before, "a refused save wrote nothing");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status abandoned --reason \"the brush rewrite replaced it\" <<'ASTROLABE'\nStopped.\n\nNothing left.\nASTROLABE\n",
    );
    assert!(ok && out.contains("status abandoned"), "{out}");
    // Read back from the parsed note, so it's in the section a fresh session reads.
    let file = astrolabe::effort::EffortFile::parse(&mac.effort_text(1)).unwrap();
    assert_eq!(file.prop("status"), Some("abandoned"));
    assert!(
        file.decisions
            .lines()
            .any(|l| l.starts_with("- 20")
                && l.ends_with(": abandoned: the brush rewrite replaced it")),
        "{}",
        file.decisions
    );
    // A later save replaces Current state but keeps the reason.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status active <<'ASTROLABE'\nRevived.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("Revived.") && text.contains(": abandoned: the brush rewrite replaced it"),
        "{text}"
    );
}

#[test]
fn an_effort_for_an_issue_says_how_to_close_it_and_reports_it_when_done() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Smooth ink\" --issue five <<'ASTROLABE'\ngoal\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("isn't a list of issue numbers"),
        "{out}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Smooth ink\" --issue \"#5, 6\" <<'ASTROLABE'\ngoal\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(
        out.contains("- issue: #5, #6: this work answers it on GitHub. The commit that lands it says `Fixes #5` and `Fixes #6`"),
        "{out}"
    );
    assert!(
        mac.effort_text(1).contains("issue: 5, 6\n"),
        "{}",
        mac.effort_text(1)
    );
    // A reload carries the line too, so a fresh session that lands the work sees it.
    assert!(mac.session_start("s1", "resume").contains("`Fixes #5`"));
    // Main here, as last fetched, closes 5 but not 6.
    git(
        &mac.demo,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Smooth ink\n\nFixes #5",
        ],
    );
    git(
        &mac.demo,
        &["update-ref", "refs/remotes/origin/main", "HEAD"],
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status done");
    assert!(ok, "{out}");
    assert!(
        out.contains("Main here has a commit that closes #5."),
        "{out}"
    );
    assert!(
        out.contains("No commit on main here, as of the last fetch, closes #6")
            && out.contains("`Fixes #6`"),
        "{out}"
    );
    // Only the save that marks it done reports.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status done");
    assert!(ok && !out.contains("closes"), "{out}");
}

#[test]
fn an_effort_with_no_issue_says_nothing_about_issues() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash("s1", NEW);
    assert!(ok && !out.contains("- issue:"), "{out}");
    assert!(!mac.effort_text(1).contains("issue:"));
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status done");
    assert!(ok && !out.contains("issue"), "{out}");
}

#[test]
fn the_done_report_never_reads_another_repo_s_history() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Smooth ink\" --issue 7 <<'ASTROLABE'\ngoal\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    // The notes checkout's own main says Fixes #7; that's not the project's main.
    git(
        &mac.notes,
        &["commit", "-q", "--allow-empty", "-m", "Fixes #7"],
    );
    git(
        &mac.notes,
        &["update-ref", "refs/remotes/origin/HEAD", "HEAD"],
    );
    let (out, ok) = mac.bash_from(
        "s1",
        &mac.notes,
        "astrolabe effort save --status done",
        None,
    );
    assert!(ok, "{out}");
    assert!(!out.contains("has a commit that closes #7"), "{out}");
    assert!(out.contains("#7"), "{out}");
}
