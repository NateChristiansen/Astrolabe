use super::*;

#[test]
fn off_for_the_machine_nothing_runs_and_on_needs_no_step() {
    let f = Fixture::new();
    assert!(f.machine("off").contains("off for this machine"));
    assert!(f.machine("status").contains("off for this machine"));
    assert!(!f.denies("s1", "pkill -f anything"));
    assert_eq!(f.session_start("s1", &f.demo, &[]), None);
    assert_eq!(f.session_start("w1", &f.demo, &[(EFFORT_VAR, "70")]), None);
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    assert_eq!(f.hook(input), None, "not taken, so it runs bare");
    let (text, ok) = f.run(&f.demo, &["activate", "--effort", "70"], "", &[]);
    assert!(!ok && text.contains("astrolabe on"), "{text}");
    assert!(f.machine("on").contains("on for this machine"));
    // On: a fresh session in a repo with no active effort gets owner, voice's rules and the
    // guard; voice's samples wait for a session that records.
    let text = f.session_start("s2", &f.other, &[]).unwrap();
    assert!(
        text.contains("OWNER-TEXT") && text.contains("VOICE-RULE") && text.contains("guard runs"),
        "{text}"
    );
    assert!(!text.contains("VOICE-SAMPLE"), "{text}");
    // Its repo's page too, since how work lands there doesn't wait for an effort; no other's.
    assert!(
        text.contains("--- knowledge/repos/other.md\n\nREPO-OTHER-TEXT")
            && !text.contains("REPO-DEMO-TEXT"),
        "{text}"
    );
    assert!(
        text.find("OWNER-TEXT") < text.find("VOICE-RULE")
            && text.find("VOICE-RULE") < text.find("REPO-OTHER-TEXT"),
        "owner, voice's rules, then the repo's page, so a long page can't push voice out: {text}"
    );
    assert!(
        text.contains("No active effort names this repo (other)") && !text.contains("EFFORT-72"),
        "{text}"
    );
    assert!(f.denies("s2", "pkill -f anything") && !f.denies("s2", "git status"));
    assert!(
        f.denies("s9", "pkill -f anything"),
        "every session, even one never started here"
    );
    // The machine switch is the owner's, from a terminal.
    for verb in ["off", "on"] {
        let (why, ok) = f.bash("s2", &f.demo, &format!("astrolabe {verb}"));
        assert!(!ok && why.contains("owner's"), "{why}");
    }
    assert!(f.machine("status").contains("on for this machine"));
}

#[test]
fn the_no_effort_context_keeps_under_the_cut_counting_everything() {
    let f = Fixture::new();
    let big = "word ".repeat(940);
    write(&f.dir.0.join("notes/knowledge/owner.md"), &big);
    write(
        &f.dir.0.join("notes/knowledge/voice.md"),
        &format!("VOICE\n\n## Rules\n\n{big}"),
    );
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(
        text.contains("Read these now")
            && text.contains("knowledge/owner.md")
            && text.contains("knowledge/voice.md"),
        "{text}"
    );
    assert!(text.contains("pick up 70") && text.contains("guard runs"));
}

#[test]
fn a_resumed_effort_with_a_head_too_long_for_the_hook_is_cut_and_names_its_file() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    // A title long enough that the head alone is over the cut, in text that takes more
    // bytes than characters.
    let title = "題名 ".repeat(900);
    let effort = f.dir.0.join("notes/efforts/0070-demo.md");
    std::fs::write(
        &effort,
        EFFORT.replace("title: 0070 Demo", &format!("title: {title}")),
    )
    .unwrap();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    let size = text.encode_utf16().count();
    assert!(size <= 9800, "{size}");
    assert!(
        text.contains("Astrolabe is on in this session for effort 70, \"題名 ")
            && text.contains("- authority: edit: ")
            && text.contains("Prove the guard for this session")
            && text.contains("Report the effort and what the status says."),
        "the head's opening lines stay: {text}"
    );
    let close = "\n\nAstrolabe cut this text here to stay under the app's limit for a hook, so rules or notes are missing: tell the owner, and read now:\n- ";
    let (_, files) = text.split_once(close).expect(&text);
    let files = files.split("\n\n").next().unwrap();
    // Every note the uncut text names, so a cut session still reads the owner's notes.
    for file in [
        "0070-demo.md: only its `## Current state` section",
        "voice.md",
        "owner.md",
        "repos/demo.md",
        "machines.md",
    ] {
        assert!(files.contains(file), "{file} isn't named: {files}");
    }
}

#[test]
fn voices_rules_reach_a_session_from_a_checkout_with_windows_line_endings() {
    let f = Fixture::new();
    write(
        &f.dir.0.join("notes/knowledge/voice.md"),
        "VOICE-TEXT\r\n\r\n## Rules\r\n\r\n- VOICE-RULE\r\n\r\n## Chat\r\n\r\nVOICE-SAMPLE\r\n",
    );
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("VOICE-RULE") && !text.contains("VOICE-SAMPLE") && !text.contains('\r'),
        "{text:?}"
    );
}

#[test]
fn a_repos_page_past_the_cut_goes_as_a_file_to_read_and_a_missing_one_adds_nothing() {
    let f = Fixture::new();
    let page = f.dir.0.join("notes/knowledge/repos/demo.md");
    write(&page, &format!("REPO-DEMO-TEXT {}", "rule ".repeat(2000)));
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(
        text.contains("OWNER-TEXT")
            && text.contains("VOICE-RULE")
            && !text.contains("REPO-DEMO-TEXT")
            && text.contains("Read these now")
            && text.contains("knowledge/repos/demo.md"),
        "{text}"
    );
    std::fs::remove_file(&page).unwrap();
    let text = f.session_start("s2", &f.demo, &[]).unwrap();
    assert!(
        text.contains("OWNER-TEXT")
            && text.contains("pick up 70")
            && !text.contains("knowledge/repos/demo.md"),
        "{text}"
    );
}

#[test]
fn the_notes_checkout_is_the_owners_seat_where_any_effort_loads() {
    let f = Fixture::new();
    let notes = f.dir.0.join("notes");
    let tree = f.dir.0.join("notes-tree");
    git(&notes, &["worktree", "add", "-q", tree.to_str().unwrap()]);
    for (i, cwd) in [&notes, &tree].into_iter().enumerate() {
        let session = format!("n{i}");
        let text = f.session_start(&session, cwd, &[]).unwrap();
        assert!(
            text.contains("OWNER-TEXT"),
            "the plugin runs here too: {text}"
        );
        assert!(text.contains("the owner's seat"), "{text}");
        assert!(
            !text.contains("REPO-"),
            "the seat has no repo of its own: {text}"
        );
        let input = json!({"session_id": session, "cwd": cwd, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
        assert!(f.hook(input).is_some(), "and guards here");
        // Effort 70 is demo's; from the seat it loads all the same, with demo's page, which
        // the seat's start didn't give.
        let (text, ok) = f.bash(&session, cwd, "astrolabe activate --effort 70");
        assert!(ok && text.contains("REPO-DEMO-TEXT"), "{text}");
    }
    // A product repo is still matched to the effort's repo.
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    assert!(f.denies("s1", "pkill x"));
}

#[test]
fn the_machine_switch_reaches_running_sessions_at_once() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    f.machine("off");
    assert!(!f.denies("s1", "pkill x"));
    f.machine("on");
    assert!(f.denies("s1", "pkill x"));
}

#[test]
fn switching_on_binds_the_hooks_session_and_only_that_one() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    for wanted in [
        "effort 70",
        "check in at plan",
        "authority: edit",
        "EFFORT-70",
        "NEXT-70",
        "VOICE-TEXT",
        "VOICE-SAMPLE",
        // Again, though a start in demo sent it: nothing records what a start sent, and a
        // session may have started elsewhere, so a repeat is safer than a gap.
        "REPO-DEMO-TEXT",
        "MACHINE-TEXT",
    ] {
        assert!(text.contains(wanted), "missing {wanted}: {text}");
    }
    for given in ["OWNER-TEXT", "doesn't see file edits"] {
        assert!(
            !text.contains(given),
            "session start gave {given} already: {text}"
        );
    }
    assert!(!text.contains("NOT-DELIVERED") && !text.contains("REPO-OTHER"));
    assert!(f.denies("s1", "pkill -f anything"));
    assert!(
        !f.state().join("s2.json").exists(),
        "another session has no effort"
    );
    assert!(!f.denies("s1", "git status"), "an allowed command runs");
    let text = f.session_start("s2", &f.other, &[]).unwrap();
    assert!(
        !text.contains("EFFORT-70"),
        "a fresh session inherits nothing: {text}"
    );
}

#[test]
fn unreadable_notes_refuse_with_the_file_named() {
    let f = Fixture::new();
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(
        !ok && text.contains("knowledge/machines.md")
            && text.contains("Only this effort's context and settings are missing"),
        "{text}"
    );
    assert!(!f.state().join("s1.json").exists());
}

#[test]
fn resume_reloads_fresh_context_and_a_failed_reload_keeps_the_guard_and_blocks() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    let effort = f.dir.0.join("notes/efforts/0070-demo.md");
    std::fs::write(&effort, EFFORT.replace("EFFORT-70", "EFFORT-70-NEWER")).unwrap();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.contains("EFFORT-70-NEWER"), "{text}");
    assert!(
        text.contains("REPO-DEMO-TEXT"),
        "a resumed session starts over, so it gets the repo's page again: {text}"
    );
    let machines = f.dir.0.join("notes/knowledge/machines.md");
    std::fs::remove_file(&machines).unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "compact"});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input.to_string(),
        &[],
    );
    assert!(
        out.contains("stays on for effort 70")
            && out.contains("machines.md")
            && out.contains("blocked"),
        "{out}"
    );
    assert!(
        f.denies("s1", "pkill -0 astrolabe-guard-probe"),
        "the guard stays"
    );
    let (why, ok) = f.bash("s1", &f.demo, "git status");
    assert!(!ok && why.contains("--off"), "{why}");
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("couldn't be reloaded")
    );
    // Repaired and switched on again: unblocked.
    std::fs::write(&machines, "MACHINE-TEXT").unwrap();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    assert!(!f.denies("s1", "git status") && f.denies("s1", "pkill x"));
    // Or the effort dropped: unblocked, and still guarded while the machine is on.
    std::fs::remove_file(&machines).unwrap();
    f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(f.denies("s1", "git status"));
    assert!(f.bash("s1", &f.demo, "astrolabe activate --off").1);
    assert!(!f.denies("s1", "git status") && f.denies("s1", "pkill x"));
}

#[test]
fn a_launch_switches_on_by_env_bound_to_its_own_session() {
    let f = Fixture::new();
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(
        text.contains("effort 70")
            && text.contains("OWNER-TEXT")
            && text.contains("REPO-DEMO-TEXT"),
        "{text}"
    );
    assert!(f.denies("w1", "pkill x"));
    assert!(!f.state().join("w2.json").exists());
    let text = f
        .session_start("w3", &f.demo, &[(EFFORT_VAR, "71")])
        .unwrap();
    assert!(
        text.contains("NOT loaded")
            && text.contains(EFFORT_VAR)
            && text.contains("the guard still runs"),
        "{text}"
    );
    assert!(!f.state().join("w3.json").exists() && f.denies("w3", "pkill x"));
}

#[test]
fn unreadable_state_blocks_commands_with_a_repair_and_off_repairs_it() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::write(f.state().join("s1.json"), "garbage").unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "git status"}});
    let answer = f.hook(input).unwrap().to_string();
    assert!(
        answer.contains("can't read its state") && answer.contains("--off"),
        "{answer}"
    );
    assert!(f.bash("s1", &f.demo, "astrolabe activate --off").1);
    assert!(!f.denies("s1", "git status"));
}

#[test]
fn a_hook_gets_what_fits_and_the_files_to_read_for_the_rest() {
    let f = Fixture::new();
    let big = "word ".repeat(1500);
    write(&f.dir.0.join("notes/knowledge/repos/demo.md"), &big);
    write(&f.dir.0.join("notes/knowledge/machines.md"), &big);
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(text.contains("EFFORT-70") && text.contains("Read these now"));
    assert!(text.contains("knowledge/repos/demo.md") && text.contains("knowledge/machines.md"));
    let (full, ok) = f.bash("w1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && full.len() > 9800, "a command's output carries it all");
}

#[test]
fn a_switch_whose_result_cant_be_left_isnt_made() {
    let f = Fixture::new();
    let shown = f.state().join("shown");
    std::fs::create_dir_all(f.state()).unwrap();
    std::fs::write(&shown, "a file where the folder goes").unwrap();
    let (why, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(!ok && why.contains("couldn't finish"), "{why}");
    assert!(
        !f.state().join("s1.json").exists(),
        "first switch-on not made"
    );
    std::fs::remove_file(&shown).unwrap();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_dir_all(&shown).unwrap();
    std::fs::write(&shown, "").unwrap();
    assert!(!f.bash("s1", &f.demo, "astrolabe activate --effort 75").1);
    assert!(!f.bash("s1", &f.demo, "astrolabe activate --off").1);
    std::fs::remove_file(&shown).unwrap();
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(
        status.contains("effort 70"),
        "effort switch not made: {status}"
    );
    assert!(f.denies("s1", "pkill x"), "off not made");
}

#[test]
fn a_switch_that_cant_be_saved_shows_that_instead() {
    let f = Fixture::new();
    // A folder where the session's state file goes: the result is left, the write fails.
    std::fs::create_dir_all(f.state().join("s1.json")).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(!ok && text.contains("couldn't be written"), "{text}");
}

#[test]
fn a_large_effort_points_at_current_state_only() {
    let f = Fixture::new();
    let current = "Current words. ".repeat(800);
    let effort = format!(
        "---\nrepo: demo\ninvolvement: check in at plan\nauthority: edit\n---\n# 0070 Demo\n\n## Current state\n\nBIG-CURRENT {current}\n\n## History\n\nSUPERSEDED: push straight to main without review.\n"
    );
    write(&f.dir.0.join("notes/efforts/0070-demo.md"), &effort);
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(!text.contains("SUPERSEDED"), "{text}");
    assert!(
        text.contains("only its `## Current state` section"),
        "{text}"
    );
    let path = f
        .dir
        .0
        .join("notes/efforts/0070-demo.md")
        .display()
        .to_string();
    assert!(
        !text.contains(&format!("- {path}\n")) && !text.ends_with(&format!("- {path}")),
        "never the whole file"
    );
    assert!(
        text.contains("knowledge/owner.md"),
        "whole files for the rest"
    );
    let (full, ok) = f.bash("w1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && full.contains("BIG-CURRENT") && !full.contains("SUPERSEDED"));
}

#[test]
fn properties_come_from_frontmatter_only() {
    let f = Fixture::new();
    let line_only = "# 0076\n\nrepo: demo · involvement: autonomous · authority: edit\n\n## Current state\n\nx\n";
    write(&f.dir.0.join("notes/efforts/0076-line.md"), line_only);
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 76");
    assert!(
        !ok && text.contains("no `repo` property") && text.contains("properties at the top"),
        "{text}"
    );
    // Effort 70's body holds a decoy line naming another repo and push; the properties win.
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(
        ok && text.contains("authority: edit") && text.contains("\"0070 Demo\""),
        "{text}"
    );
    // A YAML list and quoted values read too.
    assert!(f.bash("s2", &f.demo, "astrolabe activate --effort 75").1);
    assert!(f.bash("s3", &f.other, "astrolabe activate --effort 72").1);
}

#[cfg(unix)]
#[test]
fn a_failed_reload_that_cant_be_saved_still_blocks_or_says_it_doesnt() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let lock =
        |mode| std::fs::set_permissions(f.state(), std::fs::Permissions::from_mode(mode)).unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "resume"}).to_string();
    let command = |cmd: &str| {
        json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": cmd}}).to_string()
    };
    // State folder read-only: the block goes to the temp folder and still holds.
    lock(0o555);
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input,
        &[],
    );
    assert!(
        out.contains("blocked until") && !out.contains("NOT blocked"),
        "{out}"
    );
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("git status"),
        &[],
    );
    assert!(
        out.contains("\"deny\"") && out.contains("couldn't be reloaded"),
        "{out}"
    );
    lock(0o755);
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --off").1,
        "off clears it"
    );
    assert!(!f.denies("s1", "git status"));
    // Neither can be written: said plainly, and the guard alone stays on.
    write(&f.dir.0.join("notes/knowledge/machines.md"), "M");
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let tmp = f.dir.0.join("locked-tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o555)).unwrap();
    let env = [("TMPDIR", tmp.to_str().unwrap())];
    lock(0o555);
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input,
        &env,
    );
    assert!(
        out.contains("NOT blocked") && out.contains("only the guard is on"),
        "{out}"
    );
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("git status"),
        &env,
    );
    assert!(out.trim().is_empty(), "as said, not blocked: {out}");
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("pkill x"),
        &env,
    );
    assert!(out.contains("\"deny\""), "the guard stays: {out}");
    lock(0o755);
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_refused_load_says_the_guard_still_runs_and_it_does() {
    let f = Fixture::new();
    let loose = f.dir.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    git(&loose, &["init", "-q"]);
    let (text, ok) = f.bash("s1", &loose, "astrolabe activate --effort 70");
    assert!(!ok && text.contains("NOT loaded"), "{text}");
    assert!(
        text.contains("the guard still runs")
            && text.contains("owner and voice notes")
            && text.contains("Only this effort's context and settings are missing"),
        "{text}"
    );
    assert!(
        !text.contains("stays off") && !text.contains("No new protection"),
        "{text}"
    );
    let input = json!({"session_id": "s1", "cwd": loose, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
    assert!(f.hook(input).is_some(), "and it does");
}

#[test]
fn on_refuses_without_the_notes_checkout_and_sessions_say_why() {
    let f = Fixture::new();
    f.machine("off");
    std::fs::remove_file(f.state().join("repos/agent.json")).unwrap();
    let (text, ok) = f.run(&f.demo, &["on"], "", &[]);
    assert!(
        !ok && text.contains("repo add agent") && text.contains("where the notes are"),
        "{text}"
    );
    assert!(f.machine("status").contains("off for this machine"));
    write(&f.state().join("repos/agent.json"), "not json");
    let (text, ok) = f.run(&f.demo, &["on"], "", &[]);
    assert!(!ok && text.contains("isn't a repo record"), "{text}");
    // A map lost after switching on: sessions say plainly what that means.
    let (out, ok) = f.run(
        &f.dir.0.join("notes"),
        &[
            "repo",
            "add",
            "agent",
            f.dir.0.join("notes").to_str().unwrap(),
        ],
        "",
        &[],
    );
    assert!(ok, "{out}");
    assert!(f.machine("on").contains("on for this machine"));
    std::fs::remove_file(f.state().join("repos/agent.json")).unwrap();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("nothing can be saved either") && text.contains("repo add agent"),
        "{text}"
    );
    assert!(f.denies("s1", "pkill x"));
}

#[test]
fn a_byte_order_mark_keeps_a_notes_properties() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    write(
        &f.notes().join("efforts/0080-game.md"),
        &format!("\u{feff}{}", GAME.replace('\n', "\r\n")),
    );
    f.game_note("\u{feff}---\r\nremote: git@example.com:team/old.git\r\ntitle: Game\r\n---\r\n\r\nREPO-GAME-TEXT\r\n");
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(
        ok && out.contains("which listed git@example.com:team/old.git"),
        "{out}"
    );
    let text = std::fs::read_to_string(f.notes().join("knowledge/repos/game.md")).unwrap();
    assert_eq!(
        text,
        "\u{feff}---\r\nremote:\r\n  - git@example.com:team/old.git\r\n  - git@example.com:team/game.git\r\ntitle: Game\r\n---\r\n\r\nREPO-GAME-TEXT\r\n"
    );
    // The effort's properties are read past the mark too.
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(
        text.contains("one active effort is 80, \"0080 Game\""),
        "{text}"
    );
    let (text, ok) = f.bash("s1", &game, "astrolabe activate --effort 80");
    assert!(
        ok && text.contains("involvement: autonomous") && text.contains("EFFORT-80"),
        "{text}"
    );
}

#[test]
fn a_command_is_readable_only_as_text_or_a_list_of_text_in_the_field_the_guard_reads() {
    let f = Fixture::new();
    for tool_input in [
        json!({"command": [null, 123]}),
        json!({"command": ["echo", 1]}),
        json!({"command": null, "cmd": "echo hi"}),
        json!({"command": {"text": "echo hi"}}),
    ] {
        let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": tool_input});
        let answer = f
            .hook_raw(input.to_string().into_bytes())
            .unwrap_or_else(|| panic!("allowed: {input}"));
        assert!(reason(&answer).starts_with("Astrolabe couldn't read what the app sent its hook (a command that isn't text or a list of text)"), "{answer}");
    }
    // An unknown tool with a command is read as a shell by the guard, so it's held to the same.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "NewShell", "tool_input": {"command": [null]}});
    assert!(f.hook_raw(input.to_string().into_bytes()).is_some());
    for tool_input in [
        json!({"command": ["echo", "hi"]}),
        json!({"cmd": "echo hi"}),
    ] {
        let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "shell", "tool_input": tool_input});
        assert_eq!(f.hook_raw(input.to_string().into_bytes()), None, "{input}");
    }
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "shell", "tool_input": {"command": ["pkill", "-f", "x"]}});
    assert!(
        !reason(&f.hook_raw(input.to_string().into_bytes()).unwrap()).contains("couldn't read")
    );
}

#[test]
fn resume_preserves_unknown_fields_and_legacy_optional_values() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    let path = f.state().join("s1.json");
    let mut state: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    state["future"] = json!({"nested": [1, null, true]});
    state["worker"] = "legacy value".into();
    state["last_save_ms"] = Value::Null;
    write(&path, &state.to_string());
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.contains("EFFORT-70"), "{text}");
    let resumed: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for field in ["future", "worker", "last_save_ms"] {
        assert_eq!(resumed[field], state[field]);
    }
    assert!(!f.denies("s1", "git status"));
}
