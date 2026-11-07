use super::*;

const INITIATIVE: &str = "astrolabe initiative new --title \"Smooth ink\" <<'ASTROLABE'\nMake ink smooth across frames, with café in it.\n\n## Efforts and order\nTether cap\nCap the tether so strokes stop splitting.\n\nGoo at speed\nKeep goo stable at speed,\ntwo frames at most.\n\n## What completion means\n- strokes never split\nASTROLABE\n";

#[test]
fn an_initiative_and_its_efforts_are_filed_in_one_step() {
    let world = World::new();
    let mac = world.machine("mac");
    // The spike that planned them is this session's effort, and stays so.
    assert!(mac.bash("s1", NEW).1);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", INITIATIVE);
    assert!(ok, "{out}");
    for line in [
        "Initiative \"Smooth ink\" filed as initiatives/smooth-ink.md with 2 efforts",
        "0002 Tether cap\n0003 Goo at speed",
        "This session stays on effort 1",
        "activate --initiative smooth-ink",
    ] {
        assert!(out.contains(line), "{line}\n{out}");
    }
    let tether = mac.effort_text(2);
    for line in [
        "status: proposed",
        "repo: demo",
        "initiative: smooth-ink",
        "involvement: check in at plan",
        "authority: edit",
        "saved_by: mac · claude",
        "session: s1",
        "# 0002 Tether cap",
        "## Current state\nCap the tether so strokes stop splitting.\n",
    ] {
        assert!(tether.contains(line), "{line}\n{tether}");
    }
    let goo = mac.effort_text(3);
    assert!(
        goo.contains("# 0003 Goo at speed")
            && goo.contains("## Current state\nKeep goo stable at speed,\ntwo frames at most.\n"),
        "{goo}"
    );
    let page = std::fs::read_to_string(mac.notes.join("initiatives/smooth-ink.md")).unwrap();
    assert!(
        page.starts_with("---\nstatus: ready\nrepo: demo\ninvolvement:"),
        "{page}"
    );
    let list = format!(
        "\n## Purpose\n\nMake ink smooth across frames, with café in it.\n\n## Efforts and order\n\n1. [Tether cap](../{}). Cap the tether so strokes stop splitting.\n2. [Goo at speed](../{}). Keep goo stable at speed, two frames at most.\n\n## What completion means\n- strokes never split\n",
        shelf("0002-tether-cap.md"),
        shelf("0003-goo-at-speed.md")
    );
    assert!(page.ends_with(&list), "{page}");
    // One commit for the lot, no journal line, and the session still on the spike.
    assert_eq!(mac.log().lines().count(), before + 1);
    assert_eq!(
        mac.log().lines().next(),
        Some("initiative Smooth ink: 2 efforts filed")
    );
    assert!(!mac.notes.join("journal").exists());
    // The page is in that commit, not left untracked for the sync to miss.
    assert!(
        git(&mac.notes, &["status", "--porcelain", "--", "initiatives"])
            .trim()
            .is_empty(),
        "the page is committed"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    // The board groups them under the name.
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let grouped = v["efforts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["initiative"] == "smooth-ink")
        .count();
    assert_eq!(grouped, 2, "{out}");
    // The spike joins it and closes as their record; the next effort joins by name.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done --initiative \"Smooth ink\" <<'ASTROLABE'\nDone: the plan is filed as its efforts.\n\nNothing left here.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("initiative: smooth-ink"));
    // The same command typed again with no hook explains itself from the ticket.
    let (plain, ok) = mac.plain(INITIATIVE);
    assert!(
        ok && plain.contains("filed as initiatives/smooth-ink.md"),
        "{plain}"
    );
    // Filed once: the page is the page.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Smooth ink\" <<'ASTROLABE'\nAgain.\n\n## Efforts\nMore\nmore goal\nASTROLABE\n",
    );
    assert!(!ok && out.contains("exists already"), "{out}");
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 3);
}

#[test]
fn a_refused_initiative_files_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.log().lines().count();
    for (cmd, why) in [
        (
            "astrolabe initiative new --title \"No goal\" <<'ASTROLABE'\n## Efforts\nTitle only\nASTROLABE\n",
            "has no goal",
        ),
        (
            "astrolabe initiative new --title \"Labelled\" <<'ASTROLABE'\n## Efforts\nA\n**Goal:** g.\nASTROLABE\n",
            "no **Label:** lines",
        ),
        (
            "astrolabe initiative new --title \"Flag\" --status proposed <<'ASTROLABE'\n## Efforts\nA\ng.\nASTROLABE\n",
            "isn't a flag of `initiative new`",
        ),
        (
            "astrolabe initiative new <<'ASTROLABE'\n## Efforts\nA\ng.\nASTROLABE\n",
            "--title",
        ),
        ("astrolabe initiative new --title \"No body\"", "## Efforts"),
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(!ok && out.contains(why), "{cmd}: {out}");
    }
    // A worker files nothing; a subagent neither.
    assert!(mac.bash("w1", "astrolabe activate --effort 1 --worker").1);
    let (out, ok) = mac.bash("w1", INITIATIVE);
    assert!(!ok && out.contains("worker under effort 1"), "{out}");
    let (out, ok) = mac.bash_as("s1", INITIATIVE, Some("child-1"));
    assert!(!ok && out.contains("subagent"), "{out}");
    assert_eq!(mac.log().lines().count(), before);
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 1);
    assert!(!mac.notes.join("initiatives").exists());
    // With no hook, nothing is filed and the command says so.
    let (plain, ok) = mac.plain(INITIATIVE);
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
}

fn legacy_page(mac: &Machine) {
    write(
        &mac.notes.join("initiatives/smooth-lines.md"),
        include_str!("../fixtures/initiatives/prose.md"),
    );
    git(&mac.notes, &["add", "initiatives"]);
    git(&mac.notes, &["commit", "-qm", "legacy page"]);
}

#[test]
fn an_initiative_binding_saves_resumes_reports_and_detaches() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    let (out, ok) = mac.bash("lead", "astrolabe activate --initiative \"Smooth lines\"");
    assert!(
        ok && out.contains("initiative smooth-lines")
            && out.contains("Pieces")
            && out.contains("Status: active"),
        "{out}"
    );
    let state_path = mac.state().join("lead.json");
    let mut state: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(
        state["binding"],
        json!({"kind":"initiative", "key":"smooth-lines"})
    );
    state["future"] = json!({"nested":true});
    state["binding"]["future"] = json!({"nested":true});
    write(&state_path, &state.to_string());
    let (out, ok) = mac.bash("lead", "astrolabe effort save --decision \"chose curves over corners: smoother\" --waits-for \"owner: look at the curves\" <<'ASTROLABE'\nReady for a look.\nASTROLABE\n");
    assert!(ok && out.contains("Saved initiative smooth-lines"), "{out}");
    let page = std::fs::read_to_string(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let file = astrolabe::effort::EffortFile::parse(&page).unwrap();
    assert_eq!(file.current, "Ready for a look.");
    assert_eq!(file.decisions.lines().count(), 1);
    assert_eq!(page.matches("## Purpose").count(), 1);
    assert!(page.contains("Purpose before any section, with [a link](../guide.md)."));
    assert_eq!(
        mac.log().lines().next(),
        Some("initiative smooth-lines: checkpoint")
    );
    assert!(
        mac.session_start("lead", "resume")
            .contains("Ready for a look.")
    );
    let state: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["future"], json!({"nested":true}));
    assert_eq!(state["binding"]["future"], json!({"nested":true}));
    let (out, ok) = mac.bash("lead", "astrolabe activate --status");
    assert!(
        ok && out.contains("initiative smooth-lines in repo demo"),
        "{out}"
    );
    mac.edit("lead", "Edit");
    let nudge = mac.stop("lead", false, true).unwrap();
    assert!(
        nudge["reason"]
            .as_str()
            .unwrap()
            .contains("initiative smooth-lines")
    );
    assert!(mac.stop("lead", false, true).is_none());
    assert!(mac.stop("lead", true, true).is_none());
    assert!(mac.bash("lead", "astrolabe activate --off").1);
    assert!(
        mac.bash("lead", "astrolabe activate --status")
            .0
            .contains("No effort is set")
    );
}

#[test]
fn workers_and_subagents_cannot_bind_to_an_initiative() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(mac.bash("worker", NEW).1);
    assert!(
        mac.bash("worker", "astrolabe activate --effort 1 --worker")
            .1
    );
    for cmd in [
        "astrolabe activate --initiative smooth-lines",
        "astrolabe activate --initiative smooth-lines --worker",
    ] {
        let (out, ok) = mac.bash("worker", cmd);
        assert!(!ok && out.contains("worker"), "{out}");
    }
    let (out, ok) = mac.bash_as(
        "lead",
        "astrolabe activate --initiative smooth-lines",
        Some("child"),
    );
    assert!(!ok && out.contains("subagent"), "{out}");
}

#[test]
fn initiative_saves_refuse_forbidden_flags_and_stale_text() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let path = mac.notes.join("initiatives/smooth-lines.md");
    let original = std::fs::read_to_string(&path).unwrap();
    for flag in [
        "--tick x",
        "--untick x",
        "--pieces-file missing",
        "--plan-file missing",
        "--initiative other",
        "--status review",
    ] {
        let (out, ok) = mac.bash("lead", &format!("astrolabe effort save {flag}"));
        assert!(!ok, "{out}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save <<'ASTROLABE'\nState.\n\n## Plan\nSteps.\nASTROLABE\n",
    );
    assert!(!ok && out.contains("plan replacement"), "{out}");
    write(&path, &format!("{original}\nExternal purpose change.\n"));
    let cmd = "astrolabe effort save <<'ASTROLABE'\nNew state.\nASTROLABE\n";
    let (out, ok) = mac.bash("lead", cmd);
    assert!(!ok && out.contains("changed since"), "{out}");
    let (out, ok) = mac.bash("lead", cmd);
    assert!(ok, "{out}");
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("External purpose change.")
    );
}

#[test]
fn initiative_done_requires_acceptance_and_closed_members_across_repositories() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = mac.bash("lead", "astrolabe effort save --status done");
    assert!(!ok && out.contains("needs a Current state body"), "{out}");
    for alias in [
        "smooth-lines",
        "Smooth lines",
        "[[smooth-lines]]",
        "[[Smooth lines]]",
        "\"[[smooth-lines]]\"",
        "\"[[Smooth lines]]\"",
    ] {
        for status in [
            "proposed", "ready", "active", "review", "blocked", "", "unknown",
        ] {
            write(
                &mac.notes.join("efforts/0009-member.md"),
                &format!(
                    "---\nrepo: elsewhere\nstatus: {status}\ninitiative: {alias}\n---\n# 0009 Member\n\n## Current state\nStill open.\n"
                ),
            );
            let (out, ok) = mac.bash("lead", "astrolabe effort save --status done <<'ASTROLABE'\nAccepted: the whole works.\nASTROLABE\n");
            assert!(
                !ok && out.contains("members are open"),
                "{alias}, {status}: {out}"
            );
        }
    }
    write(
        &mac.notes.join("efforts/0009-member.md"),
        "---\nrepo: elsewhere\nstatus: abandoned\ninitiative: Smooth lines\n---\n# 0009 Member\n",
    );
    let (out, ok) = mac.bash("lead", "astrolabe effort save --status done <<'ASTROLABE'\nAccepted: the whole works.\nASTROLABE\n");
    assert!(ok, "{out}");
    assert_eq!(
        mac.log().lines().next(),
        Some("initiative smooth-lines: done")
    );
}

#[test]
fn page_only_filing_keeps_the_binding_and_allocates_no_effort() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("lead", NEW).1);
    let before = mac.log().lines().count();
    let command = "astrolabe initiative new --title \"Empty project\" --involvement autonomous --authority read <<'ASTROLABE'\nA purpose for later work.\nASTROLABE\n";
    let (out, ok) = mac.bash("lead", command);
    assert!(
        ok && out.contains("0 created")
            && out.contains("stays on effort 1")
            && out.contains("committed"),
        "{out}"
    );
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 1);
    let page = std::fs::read_to_string(mac.notes.join("initiatives/empty-project.md")).unwrap();
    assert!(
        page.contains("## Current state\n")
            && page.contains("## Decisions\n")
            && page.contains("## Purpose\n\nA purpose for later work."),
        "{page}"
    );
    let file = astrolabe::effort::EffortFile::parse(&page).unwrap();
    assert_eq!(file.prop("involvement"), Some("autonomous"));
    assert_eq!(file.prop("authority"), Some("read"));
    assert_eq!(mac.log().lines().count(), before + 1);
    let (out, ok) = mac.bash("lead", command);
    assert!(ok && out.contains("retry"), "{out}");
    assert_eq!(mac.log().lines().count(), before + 1);
}

#[test]
fn joins_accept_existing_aliases_and_change_only_membership_atomically() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("lead", NEW).1);
    for (number, alias) in [
        (2, "Smooth curves"),
        (3, "smooth-curves"),
        (4, "[[Smooth curves]]"),
        (5, "[[smooth-curves]]"),
    ] {
        let property = if alias.starts_with("[[") {
            alias.to_owned()
        } else {
            format!("\"{alias}\"")
        };
        write(
            &mac.notes.join(format!("efforts/{number:04}-member.md")),
            &format!(
                "---\nstatus: ready\nrepo: demo\ninitiative: {property}\n---\n# {number:04} Member {number}\n\nLegacy preamble.\n\n## Current state\nKeep this.\n\n## History\n  Keep these spaces.  \n"
            ),
        );
    }
    git(&mac.notes, &["add", "efforts"]);
    git(&mac.notes, &["commit", "-qm", "members"]);
    let original = mac.effort_text(2);
    let before = mac.log().lines().count();
    let command = "astrolabe initiative new --title \"Smooth curves\" <<'ASTROLABE'\nKeep curves smooth.\n\n## Efforts\neffort 2.\n\neffort 3, already delivers this\n\neffort 4\n\neffort 5\nASTROLABE\n";
    let (out, ok) = mac.bash("lead", command);
    assert!(
        ok && out.contains("4 joined") && out.contains("stays on effort 1"),
        "{out}"
    );
    assert_eq!(
        mac.effort_text(2),
        original.replace("initiative: \"Smooth curves\"", "initiative: smooth-curves")
    );
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 5);
    assert_eq!(mac.log().lines().count(), before + 1);
    let (out, ok) = mac.bash("lead", command);
    assert!(ok && out.contains("retry"), "{out}");
    assert_eq!(mac.log().lines().count(), before + 1);
}

#[test]
fn invalid_or_conflicting_joins_write_neither_page_nor_members() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("lead", NEW).1);
    let original = mac.effort_text(1);
    let before = mac.log().lines().count();
    for paragraphs in [
        "effort nope",
        "effort 1\n\neffort 1",
        "effort 1\n\neffort 999",
        "effort 1\n\nNew effort\nA goal.",
        "",
    ] {
        let command = format!(
            "astrolabe initiative new --title \"Join project\" <<'ASTROLABE'\nPurpose.\n\n## Efforts\n{paragraphs}\nASTROLABE\n"
        );
        let (out, ok) = mac.bash("lead", &command);
        assert!(!ok, "{paragraphs:?}: {out}");
        assert_eq!(mac.effort_text(1), original);
        assert!(!mac.notes.join("initiatives/join-project.md").exists());
        assert_eq!(mac.log().lines().count(), before);
    }
    write(&mac.notes.join("initiatives/other.md"), "# Other\n");
    let (out, ok) = mac.bash("lead", "astrolabe effort save --initiative other");
    assert!(ok, "{out}");
    let command = "astrolabe initiative new --title \"Join project\" <<'ASTROLABE'\nPurpose.\n\n## Efforts\neffort 1\nASTROLABE\n";
    let (out, ok) = mac.bash("lead", command);
    assert!(!ok && out.contains("another initiative"), "{out}");
    assert!(!mac.notes.join("initiatives/join-project.md").exists());
}

#[test]
fn page_journal_replaces_by_slug_without_colliding_with_repository_findings() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("finding", "astrolabe journal \"Found the curve settings.\"")
            .1
    );
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    for sentence in [
        "Started smoothing curves.",
        "Smoothed curves across frames.",
    ] {
        let (out, ok) = mac.bash(
            "lead",
            &format!("astrolabe effort save --journal \"{sentence}\""),
        );
        assert!(ok, "{out}");
    }
    let journal = mac.journal_today();
    assert_eq!(journal.lines().count(), 2, "{journal}");
    assert!(
        journal.contains("<!-- initiative:smooth-lines mac ")
            && journal.contains("Found the curve settings.")
            && journal.contains("Smoothed curves across frames."),
        "{journal}"
    );
    assert!(!journal.contains("Started smoothing"));
    let (out, ok) = mac.run(&mac.demo, &["home"], "");
    assert!(ok, "{out}");
    let html = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    assert!(
        html.contains("initiatives/smooth-lines.md")
            && html.contains("Smoothed curves across frames."),
        "journal links to page"
    );
}

#[test]
fn a_page_owner_wait_adds_a_linked_attention_entry_without_effort_counts() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    assert!(mac.bash("lead", "astrolabe effort save --waits-for \"owner: try the curves\" <<'ASTROLABE'\nThe curves are ready for a look.\nASTROLABE\n").1);
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let rows: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(rows["efforts"].as_array().unwrap().len(), 0);
    assert_eq!(rows["initiative_attention"].as_array().unwrap().len(), 1);
    let html = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    let attention = html
        .split("col needsyou")
        .nth(1)
        .unwrap()
        .split("</section>")
        .next()
        .unwrap();
    assert!(
        attention.contains("initiatives/smooth-lines.md")
            && attention.contains("try the curves")
            && attention.contains("The curves are ready for a look."),
        "{attention}"
    );
}

#[test]
fn unreadable_effort_records_prevent_accepting_an_initiative() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    std::fs::write(mac.notes.join("efforts/0009-unreadable.md"), [0xff]).unwrap();
    let (out, ok) = mac.bash("lead", "astrolabe effort save --status done <<'ASTROLABE'\nAccepted: everything works.\nASTROLABE\n");
    assert!(!ok && out.contains("0009-unreadable.md"), "{out}");
}

#[test]
fn title_slug_collisions_refuse_filing_before_any_write() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    for title in ["SMOOTH LINES", "smooth-lines"] {
        let command = format!(
            "astrolabe initiative new --title \"{title}\" <<'ASTROLABE'\nPurpose.\nASTROLABE\n"
        );
        let before = mac.log();
        let (out, ok) = mac.bash("lead", &command);
        assert!(!ok, "{out}");
        assert_eq!(mac.log(), before);
    }
}

#[test]
fn a_failed_initiative_reload_blocks_commands_with_the_correct_repair() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    std::fs::remove_file(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let out = mac.session_start("lead", "resume");
    assert!(
        out.contains("initiative smooth-lines") && out.contains("--initiative smooth-lines"),
        "{out}"
    );
    let (out, ok) = mac.bash("lead", "git status");
    assert!(!ok && out.contains("--initiative smooth-lines"), "{out}");
    assert!(mac.bash("lead", "astrolabe activate --off").1);
}

#[test]
fn initiative_abandonment_records_the_reason_without_closing_members() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = mac.bash("lead", "astrolabe effort save --status abandoned --reason \"No longer needed\" <<'ASTROLABE'\nAbandoned because the scope changed.\nASTROLABE\n");
    assert!(ok, "{out}");
    let text = std::fs::read_to_string(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    assert!(text.contains("status: abandoned") && text.contains("abandoned: No longer needed"));
}

#[test]
fn initiative_journal_replacements_survive_a_real_union_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    let apply =
        |machine: &Machine, key: &str, tag: &str, sentence: &str, stamp: i64, replace: bool| {
            let (day, _) = astrolabe::notes::local_day_time();
            let op: astrolabe::notes::Op = serde_json::from_value(json!({
                "key":key, "files":[], "renames":[], "journal":[{
                    "id": key, "effort":tag, "text":sentence, "machine":machine.name,
                    "at_ms":stamp, "day":day, "replace":replace
                }], "message":"journal checkpoint", "base":null
            }))
            .unwrap();
            let notes = astrolabe::notes::Notes::new(machine.notes.clone(), machine.state());
            let _held = notes.lock().unwrap();
            notes.apply(&op).unwrap();
        };
    apply(
        &mac,
        "seed-page",
        "initiative:smooth-lines",
        "Old page update.",
        10,
        true,
    );
    apply(
        &mac,
        "repo-finding",
        "smooth-lines",
        "A repository finding.",
        11,
        false,
    );
    assert!(mac.sync().1);
    let pc = world.machine("pc");
    apply(
        &mac,
        "mac-page",
        "initiative:smooth-lines",
        "A newer page update.",
        20,
        true,
    );
    apply(
        &pc,
        "pc-page",
        "initiative:smooth-lines",
        "The latest page update.",
        30,
        true,
    );
    assert!(mac.sync().1);
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let journal = pc.journal_today();
    assert_eq!(
        journal.matches("initiative:smooth-lines").count(),
        1,
        "{journal}"
    );
    assert!(
        journal.contains("The latest page update.") && journal.contains("A repository finding."),
        "{journal}"
    );
}

#[test]
fn unfinished_join_recovery_finishes_all_files_in_one_commit() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("lead", NEW).1);
    let effort_path = shelf("0001-smooth-ink.md");
    let joined = astrolabe::effort::set_initiative_property(&mac.effort_text(1), "curves");
    let page = "---\nstatus: ready\nrepo: demo\n---\n# Curves\n\n## Current state\n\n## Decisions\n\n## Purpose\nKeep curves smooth.\n";
    let intent = json!({
        "key":"crashed-join", "files":[["initiatives/curves.md",page],[effort_path,joined]],
        "renames":[], "journal":[], "message":"initiative Curves: 1 efforts filed", "base":null
    });
    write(
        &mac.state().join("ops/crashed-join.json"),
        &intent.to_string(),
    );
    write(&mac.notes.join("initiatives/curves.md"), page);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --decision \"Kept the recovered membership\"",
    );
    assert!(!ok && out.contains("changed since"), "{out}");
    assert_eq!(mac.log().lines().count(), before + 1);
    assert_eq!(mac.effort_text(1), joined);
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("initiatives/curves.md")).unwrap(),
        page
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("lead.json")).unwrap()).unwrap();
    assert_eq!(state["binding"], json!({"kind":"effort","key":"1"}));
    assert_eq!(
        mac.log().lines().next(),
        Some("initiative Curves: 1 efforts filed")
    );
}

#[test]
fn an_initiative_binding_refuses_a_repository_journal_line() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let before = mac.log();
    let (out, ok) = mac.bash("lead", "astrolabe journal \"Smoothed the curves.\"");
    assert!(
        !ok && out.contains("initiative smooth-lines") && out.contains("effort save --journal"),
        "{out}"
    );
    assert_eq!(mac.log(), before);
    assert!(mac.journal_today().is_empty());
}

#[test]
fn explicit_membership_requires_a_page_but_omitted_membership_preserves_legacy_values() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    for (i, name) in [
        "smooth-lines",
        "Smooth lines",
        "[[smooth-lines]]",
        "[[Smooth lines]]",
    ]
    .iter()
    .enumerate()
    {
        let cmd = format!(
            "astrolabe effort new --title \"Member {i}\" --initiative \"{name}\" <<'ASTROLABE'\nSmooth curves.\nASTROLABE\n"
        );
        let (out, ok) = mac.bash("worker", &cmd);
        assert!(ok, "{out}");
        assert!(
            mac.effort_text(i as u32 + 1)
                .contains("initiative: smooth-lines\n")
        );
    }
    let before = mac.effort_text(4);
    let log = mac.log();
    for cmd in [
        "astrolabe effort new --title Missing --initiative Unknown <<'ASTROLABE'\nPurpose.\nASTROLABE\n",
        "astrolabe effort save --initiative Unknown",
        "astrolabe effort save --effort 1 --initiative Unknown",
    ] {
        let (out, ok) = mac.bash("worker", cmd);
        assert!(
            !ok && out.contains("no initiative page") && out.contains("astrolabe initiative new"),
            "{out}"
        );
        assert_eq!(mac.log(), log);
        assert_eq!(mac.effort_text(4), before);
        assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 4);
    }
    let path = mac.notes.join(shelf("0004-member-3.md"));
    write(
        &path,
        &before.replace("initiative: smooth-lines", "initiative: No page yet"),
    );
    // Reload after the external edit, then save without asking to change membership.
    assert!(mac.bash("worker", "astrolabe activate --effort 4").1);
    let (out, ok) = mac.bash(
        "worker",
        "astrolabe effort save --decision \"Kept the existing membership\"",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(4).contains("initiative: No page yet\n"));
    let (out, ok) = mac.bash("worker", "astrolabe effort save --initiative \"\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(4).contains("initiative: \"\""));
}

#[test]
fn dispatcher_saves_from_an_initiative_report_the_binding_and_canonicalize_membership() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    assert!(mac.bash("worker", NEW).1);
    let before = std::fs::read_to_string(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --effort 1 --status ready --initiative \"[[Smooth lines]]\"",
    );
    assert!(
        ok && out.contains("This session's initiative smooth-lines is unchanged"),
        "{out}"
    );
    assert!(mac.effort_text(1).contains("initiative: smooth-lines\n"));
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("initiatives/smooth-lines.md")).unwrap(),
        before
    );
}

#[test]
fn a_refused_effort_activation_identifies_the_initiative_that_stays_loaded() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = mac.bash("lead", "astrolabe activate --effort 999");
    assert!(
        !ok && out.contains("Initiative smooth-lines stays loaded as it was"),
        "{out}"
    );
    assert!(
        mac.bash("lead", "astrolabe activate --status")
            .0
            .contains("initiative smooth-lines")
    );
}

#[test]
fn initiative_journal_titles_come_from_the_board_snapshot() {
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    assert!(
        mac.bash(
            "lead",
            "astrolabe effort save --journal \"Smoothed the curves.\""
        )
        .1
    );
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let rows = astrolabe::board::rows_in(&notes, None, true);
    assert_eq!(
        rows.initiative_titles
            .get("smooth-lines")
            .map(String::as_str),
        Some("Smooth lines")
    );
    // Rendering must keep the title from the snapshot even if the file disappears.
    std::fs::remove_file(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let html = astrolabe::home::render(
        &rows,
        &mac.notes,
        astrolabe::notes::now_ms(),
        astrolabe::home::Look::default(),
    );
    assert!(
        html.contains("Smooth lines</a></span>") && html.contains("Smoothed the curves."),
        "{html}"
    );
}

#[cfg(unix)]
#[test]
fn an_unrecordable_reload_block_still_gives_the_initiative_repair_command() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let world = World::new();
    let mac = world.machine("mac");
    legacy_page(&mac);
    assert!(
        mac.bash("lead", "astrolabe activate --initiative smooth-lines")
            .1
    );
    std::fs::remove_file(mac.notes.join("initiatives/smooth-lines.md")).unwrap();
    let hash = Sha256::digest(mac.state().display().to_string());
    let folder: String = hash.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let fallback = std::env::temp_dir().join(format!("astrolabe-blocked-{folder}-lead.json"));
    std::fs::create_dir(&fallback).unwrap();
    let permissions = std::fs::metadata(mac.state()).unwrap().permissions();
    std::fs::set_permissions(mac.state(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = mac.session_start("lead", "resume");
    std::fs::set_permissions(mac.state(), permissions).unwrap();
    std::fs::remove_dir(fallback).unwrap();
    assert!(
        out.contains("couldn't record a block") && out.contains("NOT blocked"),
        "{out}"
    );
    assert!(
        out.contains("initiative smooth-lines") && out.contains("--initiative smooth-lines"),
        "{out}"
    );
    assert!(!out.contains("--effort smooth-lines"), "{out}");
}
