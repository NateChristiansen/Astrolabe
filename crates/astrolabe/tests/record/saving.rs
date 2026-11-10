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
        "astrolabe effort save --status blocked --waits-for \"owner: should it be X?\" <<'ASTROLABE'\nBlocked on the choice.\nASTROLABE",
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
        &format!("astrolabe effort save --waits-for \"owner: go on the plan?\" {body}"),
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("waits_for: \"owner: go on the plan?\"")
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
        "astrolabe effort save --status blocked --waits-for \"owner: should it be X?\" <<'ASTROLABE'\nBlocked on the choice.\nASTROLABE",
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
        "astrolabe effort save --status active --waits-for \"\" <<'ASTROLABE'\nActive again.\nASTROLABE",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("waits_for: \"\""));
}

/// A refused journal line or Current state leaves everything as it was: the note, the owner's
/// staging, the history, pending operations and the session's state.
#[test]
fn a_refused_journal_or_current_state_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    write(&mac.notes.join("drafts/post.md"), "half a post");
    git(&mac.notes, &["add", "drafts/post.md"]);
    let session = mac.state().join("s1.json");
    let snapshot = || {
        (
            mac.effort_text(1),
            git(&mac.notes, &["diff", "--cached", "--name-only"]),
            git(&mac.notes, &["rev-list", "--count", "HEAD"]),
            std::fs::read_dir(mac.state().join("ops"))
                .map(|d| d.count())
                .unwrap_or(0),
            std::fs::read(&session).unwrap(),
        )
    };
    let before = snapshot();
    assert_eq!(before.3, 0, "no pending operations to start with");
    let long = (0..151).map(|_| "word").collect::<Vec<_>>().join(" ");
    for (command, why) in [
        (
            "astrolabe effort save --journal \"Landed the ink smoothing helper. Merged.\""
                .to_owned(),
            "where the work stands",
        ),
        (
            format!("astrolabe effort save <<'ASTROLABE'\n{long}\nASTROLABE\n"),
            "151 words",
        ),
    ] {
        let (out, ok) = mac.bash("s1", &command);
        assert!(
            !ok && out.contains(why) && out.contains("Nothing was written"),
            "{out}"
        );
        assert!(snapshot() == before, "{command} changed something");
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
            "astrolabe effort save --status blocked --waits-for waiting <<'ASTROLABE'\nBlocked, waiting.\nASTROLABE"
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
    // The section's own heading leading the file is dropped; any other is refused.
    write(&mac.demo.join("pieces.md"), "- [ ] x\n## Decisions\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file pieces.md");
    assert!(!ok && out.contains("can't hold a `## ` heading"), "{out}");
    write(&mac.demo.join("headed.md"), "## Pieces\n- [ ] x\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file headed.md");
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("## Pieces\n- [ ] x\n"));
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n## Plan\n### Step one\nfirst\n### Step two\nsecond\nASTROLABE",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status active <<'ASTROLABE'\nActive again.\nASTROLABE",
    );
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
    let day = today();
    assert!(
        text.contains(&format!(
            "## Pieces\n- [x] Tether cap ({day})\n- [ ] Goo at speed\n- [ ] Goo splats\n"
        )),
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
            .contains(&format!("- [ ] Tether cap\n- [x] Goo at speed ({day})\n"))
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
fn pieces_are_a_living_list_that_never_loses_a_ticked_outcome() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Tether cap\n- [ ] Goo at speed\n- [ ] Goo splats\n",
    );
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --pieces-file pieces.md --tick Tether"
        )
        .1
    );
    let day = today();
    // A revision rewords, adds and strikes unticked lines; the ticked one is kept with its
    // stamp, and a ticked line given back without its stamp is still the same outcome.
    write(
        &mac.demo.join("pieces.md"),
        "- [x] tether cap\n- [ ] Goo at any speed\n- ~~Goo splats~~ dropped 2026-11-08: art covers it\n- [ ] Landed on main\n",
    );
    // Headed with the section's own line, as agents often write it: the heading is dropped.
    write(
        &mac.demo.join("pieces.md"),
        &format!(
            "## Pieces\n{}",
            std::fs::read_to_string(mac.demo.join("pieces.md")).unwrap()
        ),
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file pieces.md");
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(!text.contains("## Pieces\n## Pieces"), "{text}");
    assert!(
        text.contains("- [x] tether cap\n- [ ] Goo at any speed\n- ~~Goo splats~~ dropped"),
        "{text}"
    );
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["efforts"][0]["pieces"],
        json!({"done": 1, "total": 3}),
        "a struck line isn't counted"
    );
    assert!(mac.bash("s1", "astrolabe effort save --tick \"Goo at\"").1);
    let before = mac.effort_text(1);
    assert!(
        before.contains(&format!("- [x] Goo at any speed ({day})")),
        "{before}"
    );
    // Dropping a ticked line, or giving it back unticked, is refused and writes nothing.
    for (name, list) in [
        ("dropped.md", "- [x] tether cap\n- [ ] Landed on main\n"),
        (
            "unticked.md",
            "- [x] tether cap\n- [ ] Goo at any speed\n- [ ] Landed on main\n",
        ),
    ] {
        write(&mac.demo.join(name), list);
        let (out, ok) = mac.bash("s1", &format!("astrolabe effort save --pieces-file {name}"));
        assert!(
            !ok && out.contains("loses ticked outcomes")
                && out.contains("- [x] Goo at any speed")
                && !out.contains("- [x] tether cap")
                && out.contains("Nothing was written"),
            "{out}"
        );
        assert_eq!(mac.effort_text(1), before);
    }
    // --untick stays the way back, and drops the stamp; ticking again stamps anew.
    assert!(
        mac.bash("s1", "astrolabe effort save --untick \"Goo at\"")
            .1
    );
    assert!(mac.effort_text(1).contains("- [ ] Goo at any speed\n"));
}

#[test]
fn a_dispatcher_sets_another_efforts_pieces_after_reading_it() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nrun the ink work.\nASTROLABE\n"
        )
        .1
    );
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Ink pass\n- [ ] Landed\n",
    );
    let before = mac.effort_text(1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --effort 1 --pieces-file pieces.md",
    );
    assert!(
        !ok && out.contains("effort read 1") && out.contains("Nothing was written"),
        "{out}"
    );
    assert_eq!(mac.effort_text(1), before);
    assert!(mac.bash("s1", "astrolabe effort read 1").1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --effort 1 --pieces-file pieces.md --tick Ink",
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1).contains(&format!(
            "## Pieces\n- [x] Ink pass ({})\n- [ ] Landed\n",
            today()
        )),
        "{}",
        mac.effort_text(1)
    );
    assert!(
        !mac.effort_text(2).contains("## Pieces"),
        "the bound effort is untouched"
    );
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
        without_updated(&before.replace(
            &format!("- [ ] {piece}"),
            &format!("- [x] {piece} ({})", today())
        ))
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
    assert!(mac.effort_text(1).contains(&format!(
        "- [x] Repo note ({})\n- [ ] Repo note examples",
        today()
    )));
    // A different command avoids replaying the previous receipt. The exact checked label
    // still wins rather than checking the longer unfinished label.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Repo note\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains(&format!(
        "- [x] Repo note ({})\n- [ ] Repo note examples",
        today()
    )));
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
fn a_dispatcher_marks_another_effort_by_number_and_sets_an_initiative() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A second effort rebinds the session; the first is now another's record.
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nrun the ink work.\nASTROLABE\n"
        )
        .1
    );
    let worker_note = mac.effort_text(1);
    let own_note = mac.effort_text(2);
    // Text and ticks need a read first; journal and plan writes remain unavailable.
    for cmd in [
        "astrolabe effort save --effort 1 --status active <<'ASTROLABE'\nMine.\n\nNext: n.\nASTROLABE\n",
        "astrolabe effort save --effort 1 --journal \"Did the thing.\"",
        "astrolabe effort save --effort 1 --tick x",
        "astrolabe effort save --effort 1 --plan-file plan.md",
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(
            !ok && (out.contains("read 1") || out.contains("unavailable"))
                && out.contains("Nothing was written"),
            "{cmd}: {out}"
        );
    }
    let (out, ok) = mac.bash("s1", "astrolabe effort save --effort 2 --status done");
    assert!(!ok && out.contains("this session's own"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --effort 9 --status done");
    assert!(!ok && out.contains("no effort 9"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --effort one --status done");
    assert!(!ok && out.contains("effort number"), "{out}");
    assert_eq!(
        mac.effort_text(1),
        worker_note,
        "a refused save wrote nothing"
    );
    write(
        &mac.notes.join("initiatives/ink.md"),
        "---\nstatus: ready\nrepo: demo\n---\n# Ink\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --effort 1 --status active --decision \"ink over paint: cheaper\" --initiative \"Ink\"",
    );
    assert!(
        ok && out.contains("Saved effort 1: status active")
            && out.contains("effort 2 is unchanged"),
        "{out}"
    );
    let text = mac.effort_text(1);
    for line in [
        "status: active",
        "initiative: ink",
        ": ink over paint: cheaper",
        "saved_by: mac · claude",
        "session: s1",
        "## Current state\nsmooth the ink across frames, with café in it\n",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert_eq!(
        mac.effort_text(2),
        own_note,
        "the bound effort is untouched"
    );
    // Already active, so a checkpoint; the status change below names itself.
    assert_eq!(mac.log().lines().next(), Some("0001: checkpoint"));
    assert!(
        !mac.notes.join("journal").exists(),
        "no journal line by number"
    );
    // Done requires a read and text, then the bound effort can join or leave.
    assert!(mac.bash("s1", "astrolabe effort read 1").1);
    let (out, ok) = mac.bash("s1", "astrolabe effort save --effort 1 --status done <<'ASTROLABE'\nDone and checked.\nASTROLABE");
    assert!(ok && out.contains("status done"), "{out}");
    assert!(mac.effort_text(1).contains("status: done"));
    assert_eq!(mac.log().lines().next(), Some("0001: done"));
    let (out, ok) = mac.bash("s1", "astrolabe effort save --initiative \"Ink\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(2).contains("initiative: ink"));
    let (out, ok) = mac.bash("s1", "astrolabe effort save --initiative \"\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(2).contains("initiative: \"\""));
    // Another repo's effort is out of reach, as it is for activation.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let other = std::fs::read_to_string(&path)
        .unwrap()
        .replace("repo: demo", "repo: other");
    std::fs::write(&path, other).unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --effort 1 --status active");
    assert!(!ok && out.contains("repo \"other\""), "{out}");
    // A worker saves only its own effort.
    assert!(mac.bash("w1", "astrolabe activate --effort 2 --worker").1);
    let (out, ok) = mac.bash("w1", "astrolabe effort save --effort 1 --status active");
    assert!(!ok && out.contains("worker"), "{out}");
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
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and checked.\nASTROLABE",
    );
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
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and checked.\nASTROLABE",
    );
    assert!(ok && !out.contains("closes"), "{out}");
}

#[test]
fn an_effort_with_no_issue_says_nothing_about_issues() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash("s1", NEW);
    assert!(ok && !out.contains("- issue:"), "{out}");
    assert!(!mac.effort_text(1).contains("issue:"));
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and checked.\nASTROLABE",
    );
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
        "astrolabe effort save --status done <<'ASTROLABE'\nDone and checked.\nASTROLABE",
        None,
    );
    assert!(ok, "{out}");
    assert!(!out.contains("has a commit that closes #7"), "{out}");
    assert!(out.contains("#7"), "{out}");
}

#[test]
fn review_is_accepted_and_keeps_its_wait_and_board_column() {
    let world = World::new();
    let m = world.machine("mac");
    let (out, ok) = m.bash("s1", "astrolabe effort new --title Check --status review <<'ASTROLABE'\nReady for review.\nASTROLABE");
    assert!(ok, "{out}");
    let (out, ok) = m.bash("s1", "astrolabe effort save --waits-for \"the reviewer\"");
    assert!(ok, "{out}");
    let (out, ok) = m.bash("s1", "astrolabe effort save --status review");
    assert!(ok, "{out}");
    assert!(m.effort_text(1).contains("waits_for: the reviewer"));
    let page = std::fs::read_to_string(m.notes.join("home.html")).unwrap();
    assert!(column(&page, "review").contains("Check"));
    assert!(!page.contains("class=\"col ideas\""));
}

#[test]
fn status_transitions_need_text_but_repeated_statuses_do_not() {
    let world = World::new();
    let m = world.machine("mac");
    assert!(m.bash("s1", NEW).1);
    let (out, ok) = m.bash("s1", "astrolabe effort save --status active");
    assert!(ok, "{out}");
    for flags in [
        "--status review",
        "--status blocked --waits-for reviewer",
        "--status done",
        "--status abandoned --reason superseded",
    ] {
        let before = m.effort_text(1);
        let (out, ok) = m.bash("s1", &format!("astrolabe effort save {flags}"));
        assert!(
            !ok && out.contains("needs a Current state body") && out.contains("Accepted forms"),
            "{out}"
        );
        assert_eq!(m.effort_text(1), before);
    }
    let (out, ok) = m.bash(
        "s1",
        "astrolabe effort save --status review <<'ASTROLABE'\nReady for review.\nASTROLABE",
    );
    assert!(ok, "{out}");
    assert!(m.bash("s1", "astrolabe effort save --status review").1);
}

#[test]
fn done_lists_unfinished_pieces_without_refusing() {
    let world = World::new();
    let m = world.machine("mac");
    assert!(m.bash("s1", NEW).1);
    write(
        &m.demo.join("pieces.md"),
        "- [x] Built\n  - [ ] Deferred playtest\n\t- [ ] Dropped variant\n- [X] Verified\n- [ ]not a piece\n* [ ] Not counted",
    );
    assert!(
        m.bash("s1", "astrolabe effort save --pieces-file pieces.md")
            .1
    );
    let file = astrolabe::effort::EffortFile::parse(&m.effort_text(1)).unwrap();
    assert_eq!(file.pieces_count(), Some((2, 4)));
    let (out, ok) = m.bash("s1", "astrolabe effort save --status done <<'ASTROLABE'\nDone, with playtest deferred and variant dropped.\nASTROLABE");
    assert!(
        ok && out.contains("Unticked pieces:\n  - [ ] Deferred playtest\n\t- [ ] Dropped variant")
            && !out.contains("Verified")
            && !out.contains("not a piece")
            && !out.contains("Not counted"),
        "{out}"
    );
}

fn dispatcher(m: &Machine) {
    assert!(m.bash("worker", NEW).1);
    write(&m.demo.join("pieces.md"), "- [ ] Build\n- [ ] Land");
    assert!(
        m.bash("worker", "astrolabe effort save --pieces-file pieces.md")
            .1
    );
    assert!(
        m.bash(
            "lead",
            "astrolabe effort new --title Lead <<'ASTROLABE'\nCoordinate the work.\nASTROLABE"
        )
        .1
    );
}

#[test]
fn read_delivers_another_note_for_text_ticks_and_stale_write_checks() {
    let world = World::new();
    let m = world.machine("mac");
    dispatcher(&m);
    let own = m.effort_text(2);
    let command = "astrolabe effort save --effort 1 --status review --tick Build <<'ASTROLABE'\nBuilt and checked, awaiting review.\nASTROLABE";
    let (out, ok) = m.bash("lead", command);
    assert!(!ok && out.contains("effort read 1"), "{out}");
    let (out, ok) = m.bash("lead", "astrolabe effort read 1");
    assert!(ok && out.contains(&m.effort_text(1)), "{out}");
    let (out, ok) = m.bash("lead", command);
    assert!(ok, "{out}");
    assert!(m.effort_text(1).contains("- [x] Build"));
    assert!(m.effort_text(1).contains("Built and checked"));
    let (out, ok) = m.bash("lead", "astrolabe effort save --effort 1 --untick Build");
    assert!(ok, "{out}");
    assert!(m.effort_text(1).contains("- [ ] Build"));
    // Another session changes the note after the dispatcher read it.
    assert!(m.bash("other", "astrolabe activate --effort 1").1);
    assert!(
        m.bash(
            "other",
            "astrolabe effort save <<'ASTROLABE'\nNew findings from the worker.\nASTROLABE"
        )
        .1
    );
    let before = m.effort_text(1);
    let command = &command.replace("Built and checked", "Rechecked with the new findings");
    let (out, ok) = m.bash("lead", command);
    assert!(
        !ok && out.contains("changed since") && out.contains("New findings from the worker"),
        "{out}"
    );
    assert_eq!(m.effort_text(1), before);
    // The refusal delivered the latest text, allowing a deliberate retry.
    assert!(m.bash("lead", command).1);
    assert_eq!(m.effort_text(2), own);
    assert!(m.bash("lead", "astrolabe effort save --status active").1);
}

/// A body ending in an empty `## Plan` line stores only the text above it, by number as when
/// bound: the line once landed in the other effort's Current state.
#[test]
fn a_save_by_number_keeps_an_empty_plan_heading_out_of_current_state() {
    let world = World::new();
    let m = world.machine("mac");
    dispatcher(&m);
    assert!(m.bash("lead", "astrolabe effort read 1").1);
    let plan_before = m.effort_text(1).split("## Plan").count();
    let (out, ok) = m.bash(
        "lead",
        "astrolabe effort save --effort 1 <<'ASTROLABE'\nChecked the build.\n\n## Plan\nASTROLABE",
    );
    assert!(ok, "{out}");
    let text = m.effort_text(1);
    let current = text
        .split("## Current state\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .unwrap();
    assert_eq!(current.trim(), "Checked the build.", "{text}");
    assert_eq!(text.split("## Plan").count(), plan_before, "{text}");
}

#[test]
fn cross_repo_access_requires_shared_membership_or_the_notes_seat() {
    let world = World::new();
    let m = world.machine("mac");
    dispatcher(&m);
    let path = m.notes.join(shelf("0001-smooth-ink.md"));
    write(
        &path,
        &m.effort_text(1).replace("repo: demo", "repo: elsewhere"),
    );
    for command in [
        "astrolabe effort read 1",
        "astrolabe effort save --effort 1 --status ready",
    ] {
        let (out, ok) = m.bash("lead", command);
        assert!(!ok && out.contains("share no initiative"), "{out}");
    }
    write(
        &m.notes.join("initiatives/shared.md"),
        "---\nstatus: ready\nrepo: demo\n---\n# Shared work\n\n## Current state\nCoordinating.\n",
    );
    write(
        &path,
        &m.effort_text(1)
            .replace("repo: elsewhere", "repo: elsewhere\ninitiative: shared"),
    );
    assert!(
        m.bash("lead", "astrolabe effort save --initiative \"Shared work\"")
            .1
    );
    for binding in [
        "astrolabe activate --effort 2",
        "astrolabe activate --initiative shared",
    ] {
        assert!(m.bash("lead", binding).1);
        let (out, ok) = m.bash("lead", "astrolabe effort read 1");
        assert!(ok, "{out}");
        let text = if binding.contains("--initiative") {
            "Built from the initiative page."
        } else {
            "Built from the shared initiative."
        };
        let (out, ok) = m.bash(
            "lead",
            &format!(
                "astrolabe effort save --effort 1 --tick Build <<'ASTROLABE'\n{text}\nASTROLABE"
            ),
        );
        assert!(ok, "{out}");
    }
    // In the notes checkout no shared membership is needed.
    write(
        &path,
        &m.effort_text(1)
            .replace("initiative: shared", "initiative: unrelated"),
    );
    let (out, ok) = m.bash_from("lead", &m.notes, "astrolabe effort read 1", None);
    assert!(ok, "{out}");
    let (out, ok) = m.bash_from(
        "lead",
        &m.notes,
        "astrolabe effort save --effort 1 --untick Build",
        None,
    );
    assert!(ok, "{out}");
}

#[test]
fn a_worker_cannot_read_another_effort() {
    let world = World::new();
    let m = world.machine("mac");
    dispatcher(&m);
    assert!(m.bash("worker", "astrolabe activate --effort 1 --worker").1);
    let (out, ok) = m.bash("worker", "astrolabe effort read 2");
    assert!(!ok && out.contains("worker"), "{out}");
    let (out, ok) = m.bash("worker", "astrolabe effort read 1");
    assert!(ok, "{out}");
}

#[test]
fn initiative_status_moves_need_text_and_review_is_unavailable() {
    let world = World::new();
    let m = world.machine("mac");
    write(
        &m.notes.join("initiatives/shared.md"),
        "---\nstatus: ready\nrepo: demo\n---\n# Shared work\n\n## Current state\nReady to begin.\n",
    );
    assert!(m.bash("lead", "astrolabe activate --initiative shared").1);
    let (out, ok) = m.bash("lead", "astrolabe effort save --status active");
    assert!(!ok && out.contains("needs a Current state body"), "{out}");
    let (out, ok) = m.bash("lead", "astrolabe effort save --status active <<'ASTROLABE'\nActive, coordinating the work.\nASTROLABE");
    assert!(ok, "{out}");
    assert!(m.bash("lead", "astrolabe effort save --status active").1);
    let (out, ok) = m.bash(
        "lead",
        "astrolabe effort save --status review <<'ASTROLABE'\nReady for review.\nASTROLABE",
    );
    assert!(!ok && out.contains("no review status"), "{out}");
    assert!(m.bash("lead", "astrolabe effort save --status done <<'ASTROLABE'\nAccepted, the whole works.\nASTROLABE").1);
    let (out, ok) = m.bash("lead", "astrolabe effort save --status done");
    assert!(ok, "{out}");
}

#[test]
fn reactivating_the_same_record_keeps_reads_without_refreshing_other_notes() {
    for binding in ["--effort 2", "--initiative coordination"] {
        let world = World::new();
        let m = world.machine("mac");
        dispatcher(&m);
        write(
            &m.notes.join("initiatives/coordination.md"),
            "---\nstatus: active\nrepo: demo\n---\n# Coordination\n\n## Current state\nCoordinating the work.\n",
        );
        let activate = format!("astrolabe activate {binding}");
        assert!(m.bash("lead", &activate).1);
        assert!(m.bash("lead", "astrolabe effort read 1").1);
        assert!(m.bash("lead", &activate).1);
        let (out, ok) = m.bash("lead", "astrolabe effort save --effort 1 --tick Build");
        assert!(ok, "{binding}: {out}");
        assert!(m.effort_text(1).contains("- [x] Build"));

        assert!(m.bash("other", "astrolabe activate --effort 1").1);
        assert!(m.bash("other", "astrolabe effort save <<'ASTROLABE'\nNew worker findings after the read.\nASTROLABE").1);
        let before = m.effort_text(1);
        assert!(m.bash("lead", &activate).1);
        let command = "astrolabe effort save --effort 1 --untick Build";
        let (out, ok) = m.bash("lead", command);
        assert!(
            !ok && out.contains("changed since") && out.contains("New worker findings"),
            "{binding}: {out}"
        );
        assert_eq!(m.effort_text(1), before);
        assert!(m.bash("lead", command).1);

        // A different record starts a new context and needs a fresh explicit read.
        assert!(
            m.bash(
                "lead",
                "astrolabe effort new --title Other <<'ASTROLABE'\nAnother task.\nASTROLABE"
            )
            .1
        );
        let (out, ok) = m.bash("lead", "astrolabe effort save --effort 1 --tick Land");
        assert!(!ok && out.contains("effort read 1"), "{out}");
    }
}

#[test]
fn every_decision_given_is_recorded_in_order() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // Each set of decisions lands under Decisions as given: nothing dropped, nothing reordered.
    let in_order = |text: &str, lines: &[&str]| {
        let decisions = text.split("## Decisions").nth(1).unwrap_or_default();
        let at: Vec<_> = lines.iter().map(|l| decisions.find(l)).collect();
        at.iter().all(Option::is_some) && at.windows(2).all(|w| w[0] < w[1])
    };
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"ink over paint: cheaper\" --decision \"   \" --decision \"curves over corners: smoother\"",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        in_order(
            &text,
            &[
                ": ink over paint: cheaper",
                ": curves over corners: smoother"
            ]
        ) && text
            .split("## Decisions")
            .nth(1)
            .unwrap_or_default()
            .matches("\n- ")
            .count()
            == 2,
        "a blank one is skipped, the rest kept\n{text}"
    );
    // By number, from a session bound to another effort.
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nrun the ink work.\nASTROLABE\n"
        )
        .1
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --effort 1 --decision \"one pass over two: faster\" --decision \"GPU over CPU: no readback\"",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        in_order(
            &text,
            &[
                ": curves over corners: smoother",
                ": one pass over two: faster",
                ": GPU over CPU: no readback"
            ]
        ),
        "{text}"
    );
    // On a bound initiative page.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Ink\" <<'ASTROLABE'\nMake ink smooth.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe activate --initiative ink");
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"tether first: it unblocks goo\" --decision \"one machine: Deck only\" <<'ASTROLABE'\nUnder way.\nASTROLABE\n",
    );
    assert!(ok && out.contains("Saved initiative ink"), "{out}");
    let page = std::fs::read_to_string(mac.notes.join("initiatives/ink.md")).unwrap();
    assert!(
        in_order(
            &page,
            &[
                ": tether first: it unblocks goo",
                ": one machine: Deck only"
            ]
        ),
        "{page}"
    );
}

#[test]
fn a_save_keeps_the_title_decisions_and_plan_after_a_hand_edited_closing_line() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"X over Y: cheaper\" <<'ASTROLABE'\nActive: g.\n\nNext: n.\n\n## Plan\n1. first\n2. second\nASTROLABE",
    );
    assert!(ok, "{out}");
    // The owner's editor leaves spaces after the header's closing line.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let before = std::fs::read_to_string(&path).unwrap();
    let edited = before.replacen("\n---\n", "\n---  \n", 1);
    assert_ne!(edited, before);
    std::fs::write(&path, edited).unwrap();
    // The changed note is delivered with its Current state, not an empty one.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive: first try.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("changed since it was delivered") && out.contains("Active: g."),
        "{out}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive: more done.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    for kept in [
        "# 0001 Smooth ink\n",
        "## Current state\nActive: more done.\n\nNext: n.\n",
        ": X over Y: cheaper\n",
        "## Plan\n1. first\n2. second\n",
    ] {
        assert!(text.contains(kept), "{kept:?} is missing from:\n{text}");
    }
    assert_eq!(text.matches("\n---").count(), 1, "one header: {text}");
}

#[test]
fn a_note_whose_header_never_closes_is_not_written_over() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nrun the ink work.\nASTROLABE\n"
        )
        .1
    );
    // A hand edit breaks each closing line, so every line below the first reads as header.
    let broken = |n: u32, name: &str| {
        let path = mac.notes.join(shelf(name));
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replacen("\n---\n", "\n--- x\n", 1);
        std::fs::write(&path, &text).unwrap();
        assert_eq!(mac.effort_text(n), text);
        text
    };
    let own = broken(2, "0002-lead.md");
    let other = broken(1, "0001-smooth-ink.md");
    let refused = "header never closes";
    // The session's own effort.
    for _ in 0..2 {
        let (out, ok) = mac.bash(
            "s1",
            "astrolabe effort save <<'ASTROLABE'\nActive: more done.\nASTROLABE\n",
        );
        assert!(
            !ok && out.contains(refused) && out.contains("Nothing was written"),
            "{out}"
        );
        assert_eq!(mac.effort_text(2), own);
    }
    // Another effort by number, with and without a body.
    let (out, _) = mac.bash("s1", "astrolabe effort read 1");
    assert!(out.contains("--- x"), "{out}");
    for cmd in [
        "astrolabe effort save --effort 1 --status review <<'ASTROLABE'\nBuilt.\nASTROLABE\n",
        "astrolabe effort save --effort 1 --decision \"ink over paint: cheaper\"",
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(
            !ok && out.contains(refused) && out.contains("Nothing was written"),
            "{cmd}: {out}"
        );
        assert_eq!(mac.effort_text(1), other);
    }
    // A worker can't take up such an effort and mark it active: it has no Current state.
    let ready = "---\nstatus: ready\nrepo: demo\ninvolvement: check in at plan\nauthority: edit\n# 0009 Member\n\n## Current state\nKeep this.\n";
    write(&mac.notes.join("efforts/0009-member.md"), ready);
    let (out, ok) = mac.bash("w1", "astrolabe activate --effort 9 --worker");
    assert!(
        !ok && out.contains("no `## Current state` section"),
        "{out}"
    );
    assert_eq!(mac.effort_text(9), ready);
    // With its closing line back, the save goes through and keeps the note.
    std::fs::write(
        mac.notes.join(shelf("0002-lead.md")),
        own.replacen("\n--- x\n", "\n---\n", 1),
    )
    .unwrap();
    let save = "astrolabe effort save <<'ASTROLABE'\nActive: more done.\nASTROLABE\n";
    let (out, ok) = mac.bash("s1", save);
    assert!(ok, "{out}");
    let text = mac.effort_text(2);
    assert!(
        text.contains("# 0002 Lead\n") && text.contains("Active: more done."),
        "{text}"
    );
}

#[test]
fn a_refused_save_keeps_the_sections_an_unclosed_header_swallowed() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Lead\" <<'ASTROLABE'\nrun the ink work.\nASTROLABE\n"
        )
        .1
    );
    // No closing line at all: the sections below the properties read as header too, so a
    // save built from the parsed note has no History to write back.
    let text = "---\nstatus: active\nrepo: demo\n## History\nKeep this.\n";
    for name in ["0001-smooth-ink.md", "0002-lead.md"] {
        std::fs::write(mac.notes.join(shelf(name)), text).unwrap();
    }
    assert!(mac.bash("s1", "astrolabe effort read 1").1);
    for cmd in [
        "astrolabe effort save --status review <<'ASTROLABE'\nBuilt.\nASTROLABE\n",
        "astrolabe effort save --status review <<'ASTROLABE'\nBuilt, again.\nASTROLABE\n",
        "astrolabe effort save --effort 1 --status review <<'ASTROLABE'\nBuilt.\nASTROLABE\n",
        "astrolabe effort save --effort 1 --status review <<'ASTROLABE'\nBuilt, again.\nASTROLABE\n",
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(!ok, "{cmd}: {out}");
        for n in [1, 2] {
            assert_eq!(mac.effort_text(n), text, "{cmd}: {out}");
        }
    }
}

#[test]
fn an_effort_keeps_its_goal_and_tags_pieces_that_come_after_its_plan() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let goal =
        "# 0001 Smooth ink\n\nsmooth the ink across frames, with café in it\n\n## Current state\n";
    assert!(mac.effort_text(1).contains(goal), "{}", mac.effort_text(1));
    // A plan file with its own Pieces list: the list is lifted out as the effort's Pieces,
    // so the note keeps one, and nothing in a plan's own save is tagged as added.
    write(
        &mac.demo.join("plan.md"),
        "# Plan\n\n## Steps\n1. Cap the tether.\n2. Goo at speed.\n\n## Pieces\n- [ ] Tether cap\n- [ ] Goo at speed\n\n## Risks\nNone.\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --plan-file plan.md <<'ASTROLABE'\nPlanned.\n\nThe tether first.\n\nNext the goo.\nASTROLABE",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains(goal),
        "the goal stays once Current state is rewritten: {text}"
    );
    assert!(
        text.contains("## Pieces\n- [ ] Tether cap\n- [ ] Goo at speed\n"),
        "{text}"
    );
    assert_eq!(text.matches("Tether cap").count(), 1, "{text}");
    assert!(
        text.contains("### Steps") && text.contains("### Risks\nNone."),
        "{text}"
    );
    assert!(!text.contains("added"), "{text}");
    // A later save that adds a piece tags it with the day; one that rewords keeps none.
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Tether cap\n- [ ] Goo at any speed\n- [ ] Landed on main\n",
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file pieces.md");
    assert!(ok, "{out}");
    let day = today();
    let text = mac.effort_text(1);
    assert!(
        text.contains(&format!(
            "- [ ] Tether cap\n- [ ] Goo at any speed\n- [ ] Landed on main (added {day})\n"
        )),
        "{text}"
    );
    assert!(mac.bash("s1", "astrolabe effort save --tick Landed").1);
    assert!(
        mac.effort_text(1)
            .contains(&format!("- [x] Landed on main (added {day}) ({day})")),
        "{}",
        mac.effort_text(1)
    );
    // A plan saved with a list that carries its own Pieces keeps the given list.
    write(
        &mac.demo.join("plan.md"),
        "# Plan\n\n## Pieces\n- [ ] Something else\n",
    );
    write(
        &mac.demo.join("pieces.md"),
        &format!(
            "- [ ] Tether cap\n- [ ] Goo at any speed\n- [x] Landed on main (added {day}) ({day})\n- [ ] Replanned\n"
        ),
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --plan-file plan.md --pieces-file pieces.md",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(!text.contains("Something else"), "{text}");
    assert!(
        text.contains("- [ ] Replanned\n"),
        "a re-plan tags nothing: {text}"
    );
}
