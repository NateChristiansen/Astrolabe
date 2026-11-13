use super::*;

#[test]
fn a_repos_active_efforts_are_suggested_and_never_loaded_unasked() {
    let f = Fixture::new();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("one active effort is 70, \"0070 Demo\"; say \"pick up 70\" to load it"),
        "{text}"
    );
    assert!(
        text.contains("No effort is loaded")
            && !text.contains("EFFORT-70")
            && !text.contains("- involvement: check in at plan"),
        "{text}"
    );
    assert!(text.contains("OWNER-TEXT") && !f.state().join("s1.json").exists());
    assert!(f.denies("s1", "pkill x"), "the guard all the same");
    let also = "---\nstatus: active\nrepo: [other, demo]\ninvolvement: autonomous\nauthority: read\n---\n# 0077 Also\n\n## Current state\n\nEFFORT-77\n";
    let also_path = f.dir.0.join("notes/efforts/0077-also.md");
    write(&also_path, also);
    let text = f.session_start("s2", &f.demo, &[]).unwrap();
    assert!(text.contains("active efforts are 70, \"0070 Demo\"; 77, \"0077 Also\"; say \"pick up <n>\" to load one"), "{text}");
    assert!(!f.state().join("s2.json").exists());
    // Until the owner names one.
    assert!(f.bash("s2", &f.demo, "astrolabe activate --effort 77").1);
    // A candidate that can't be read: nothing listed as if the set were known.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&also_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let text = f.session_start("s4", &f.demo, &[]).unwrap();
        assert!(
            text.contains("Can't list efforts")
                && text.contains("0077-also.md")
                && !text.contains("say \"pick up 70\" to load it"),
            "{text}"
        );
        assert!(f.denies("s4", "pkill x"));
        std::fs::set_permissions(&also_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    // A folder in no registered repo: no effort, guard on.
    let loose = f.dir.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    git(&loose, &["init", "-q"]);
    let text = f.session_start("s3", &loose, &[]).unwrap();
    assert!(
        text.contains("isn't registered") && f.denies("s3", "pkill x"),
        "{text}"
    );
}

#[test]
fn notes_init_refuses_a_folder_with_work_in_it_and_takes_an_empty_clone() {
    let dir = TempDir::new();
    let state = dir.0.join("state");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(args)
            .current_dir(&dir.0)
            .env(STATE_VAR, &state)
            .envs(IDENT.iter().copied())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let busy = dir.0.join("busy");
    write(&busy.join("keep.md"), "mine");
    let out = run(&["notes", "init", busy.to_str().unwrap()]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("isn't empty"));
    assert_eq!(
        std::fs::read_dir(&busy).unwrap().count(),
        1,
        "nothing written"
    );
    // A fresh clone of an empty private repo: its remote stays, nothing else is asked.
    let clone = dir.0.join("clone");
    std::fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q"]);
    git(
        &clone,
        &[
            "remote",
            "add",
            "origin",
            "https://example.com/me/notes.git",
        ],
    );
    let out = run(&["notes", "init", clone.to_str().unwrap()]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && !said.contains("no remote"),
        "{said}"
    );
    assert!(clone.join("knowledge/owner.md").is_file());
}

#[test]
fn a_missing_or_bad_field_or_another_repo_is_refused_and_keeps_what_was_on() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 71");
    assert!(
        !ok && text.contains("no `authority` property")
            && text.contains("No effort is loaded")
            && text.contains("the guard still runs"),
        "{text}"
    );
    assert!(!f.state().join("s1.json").exists(), "no effort set");
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    for (effort, why) in [
        ("74", "involvement \"whenever\" isn't one of"),
        ("72", "isn't the effort's (other)"),
        ("99", "no effort 99"),
    ] {
        let (text, ok) = f.bash(
            "s1",
            &f.demo,
            &format!("astrolabe activate --effort {effort}"),
        );
        assert!(
            !ok && text.contains(why) && text.contains("Effort 70 stays loaded"),
            "{text}"
        );
    }
    assert!(f.denies("s1", "pkill x"));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("effort 70")
    );
    // The other repo's effort switches on from that repo.
    assert!(f.bash("s3", &f.other, "astrolabe activate --effort 72").1);
}

/// Catches: the plugin reading AppData, which a packaged Windows app sees as its own private
/// copy, so a repo added from a terminal never reached it.
#[test]
fn the_plugin_keeps_its_repos_and_denials_in_its_state_folder() {
    let f = Fixture::new();
    for name in ["demo", "agent"] {
        let record = std::fs::read_to_string(f.state().join(format!("repos/{name}.json"))).unwrap();
        assert!(record.contains(&format!("\"name\":\"{name}\"")), "{record}");
    }
    let elsewhere = f.dir.0.join("empty-data");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let away = [("ASTROLABE_DATA_DIR", elsewhere.to_str().unwrap())];
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "startup"});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input.to_string(),
        &away,
    );
    assert!(
        out.contains("OWNER-TEXT") && out.contains("one active effort is 70"),
        "{out}"
    );
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &input.to_string(),
        &away,
    );
    assert!(out.contains("deny"), "{out}");
    assert!(f.state().join("guard-denials.log").is_file());
    assert!(!elsewhere.join("guard-denials.log").exists());
    assert!(!f.dir.0.join("data/guard-denials.log").exists());
}

#[test]
fn simultaneous_repo_adds_both_land() {
    let f = Fixture::new();
    std::fs::remove_dir_all(f.state().join("repos")).unwrap();
    let notes = f.dir.0.join("notes");
    let start = |name: &str, checkout: &Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(["repo", "add", name, checkout.to_str().unwrap()])
            .env(STATE_VAR, f.state())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    };
    // Both started before either is waited on.
    let mut children = [
        start("agent", &notes),
        start("demo", &f.demo),
        start("other", &f.other),
    ];
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    for name in ["agent", "demo", "other"] {
        assert!(
            f.state().join(format!("repos/{name}.json")).exists(),
            "{name} lost"
        );
    }
    assert!(
        f.session_start("s1", &f.demo, &[])
            .unwrap()
            .contains("pick up 70")
    );
}

#[cfg(unix)]
#[test]
fn a_repo_add_the_plugin_cant_record_says_so_and_rerunning_fixes_it() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let repos = f.state().join("repos");
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
    let out = command
        .args(["repo", "add", "late", f.other.to_str().unwrap()])
        .env(STATE_VAR, f.state())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && said.contains("plugin doesn't have it")
            && said.contains("rerunning is safe"),
        "{said}"
    );
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (_, ok) = f.run(
        &f.demo,
        &["repo", "add", "late", f.other.to_str().unwrap()],
        "",
        &[],
    );
    assert!(ok && repos.join("late.json").exists());
}

#[test]
fn a_bad_repo_name_is_refused_before_anything_is_written() {
    let f = Fixture::new();
    let (_, ok) = f.run(
        &f.demo,
        &["repo", "add", "../x", f.other.to_str().unwrap()],
        "",
        &[],
    );
    assert!(!ok);
    assert!(!f.state().join("x.json").exists() && !f.state().join("repos/../x.json").exists());
}

#[test]
fn a_repo_note_matches_its_repo_by_remote_in_ssh_or_https_form_and_in_worktrees() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    f.game_note("---\nremote: https://Example.com/team/game/\n---\n\nREPO-GAME-TEXT\n");
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(
        text.contains("one active effort is 80, \"0080 Game\"; say \"pick up 80\""),
        "{text}"
    );
    assert!(
        !f.state().join("repos/game.json").exists(),
        "no registry entry"
    );
    // A worktree shares its checkout's remotes, so it's the same repo.
    let tree = f.dir.0.join("game.worktrees/wt");
    git(
        &game,
        &["worktree", "add", "-q", "-b", "wt", tree.to_str().unwrap()],
    );
    let text = f.session_start("s2", &tree, &[]).unwrap();
    assert!(
        text.contains("say \"pick up 80\"") && text.contains("REPO-GAME-TEXT"),
        "matched by remote, its page comes at the start: {text}"
    );
    let (text, ok) = f.bash("s2", &tree, "astrolabe activate --effort 80");
    assert!(
        ok && text.contains("repo game") && text.contains("REPO-GAME-TEXT"),
        "and again with the effort: {text}"
    );
    // Path case is kept: another owner's repo of the same name is another repo.
    let upper = f.remote_repo("upper", "git@example.com:Team/game.git");
    let text = f.session_start("s3", &upper, &[]).unwrap();
    assert!(text.contains("has no note in the notes"), "{text}");
}

#[test]
fn a_repo_note_may_list_several_remotes() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "ssh://git@example.com/team/game.git");
    f.game_note("---\nremote:\n  - git@example.com:team/old.git\n  - https://example.com/team/game\n---\n\nREPO-GAME-TEXT\n");
    assert!(
        f.session_start("s1", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
    f.game_note(
        "---\nremote: [\"git@example.com:team/old.git\", git@example.com:team/game.git]\n---\n",
    );
    assert!(
        f.session_start("s2", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
    f.game_note("---\nremote: git@example.com:team/old.git\n---\n");
    assert!(
        !f.session_start("s3", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
}

#[test]
fn a_repo_with_no_note_says_so_and_how_the_owner_adds_one() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(
        text.contains("This repo has no note in the notes: no `knowledge/repos/<name>.md` lists its remote (git@example.com:team/game.git)")
            && text.contains("Only if the owner asks: `astrolabe repo add <name>`, run from this folder, writes that note")
            && !text.contains("pick up 80"),
        "{text}"
    );
    assert!(f.denies("s1", "pkill x"), "the guard all the same");
    // No remote another machine could match: registering here is what's offered.
    let loose = f.dir.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    git(&loose, &["init", "-q"]);
    git(&loose, &["remote", "add", "origin", "/srv/local/game.git"]);
    let text = f.session_start("s2", &loose, &[]).unwrap();
    assert!(
        text.contains("no remote another machine could match and isn't registered")
            && text.contains("registers it on this machine"),
        "{text}"
    );
    // Loading an effort whose repo nothing locates says how to add it.
    let (text, ok) = f.bash("s1", &game, "astrolabe activate --effort 80");
    assert!(
        !ok && text.contains("isn't the effort's (game)")
            && text.contains("`astrolabe repo add game`, run from a folder of that repo, adds it")
            && !text.contains("<checkout>"),
        "{text}"
    );
}

#[test]
fn repo_add_from_a_folder_with_a_remote_writes_a_note_and_no_record() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    git(
        &game,
        &[
            "remote",
            "add",
            "upstream",
            "https://example.com/up/game.git",
        ],
    );
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(ok, "{out}");
    let note = f.notes().join("knowledge/repos/game.md");
    assert!(
        out.starts_with("Wrote ")
            && out.contains("game.md, a new note for repo game")
            && out.contains("with remote git@example.com:team/game.git")
            && out.contains("Not committed or pushed"),
        "{out}"
    );
    let text = std::fs::read_to_string(&note).unwrap();
    assert!(
        text.starts_with("---\nremote: git@example.com:team/game.git\n---\n\n# game\n\nCreated by `astrolabe repo add`"),
        "origin, not upstream: {text}"
    );
    assert!(
        !f.state().join("repos/game.json").exists(),
        "no registry entry"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(f.notes())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("knowledge/"),
        "left uncommitted"
    );
    assert!(
        f.session_start("s1", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
    // Naming a checkout explicitly still uses the remote note, with no local record.
    let (out, ok) = f.run(
        &f.demo,
        &["repo", "add", "game", game.to_str().unwrap()],
        "",
        &[],
    );
    assert!(
        ok && out.contains("already lists git@example.com:team/game.git")
            && out.contains("nothing was written"),
        "{out}"
    );
    assert!(!f.state().join("repos/game.json").exists());
    assert!(!f.dir.0.join("data").exists(), "no retired app database");
}

#[test]
fn repo_add_adds_its_remote_to_a_note_that_lists_another() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "https://example.com/team/game.git");
    f.game_note("---\ntitle: Game\nremote: git@example.com:team/old.git\nstatus: x\n---\n\nREPO-GAME-TEXT\n");
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(
        ok && out.contains("Added remote https://example.com/team/game.git")
            && out.contains("which listed git@example.com:team/old.git and still does"),
        "{out}"
    );
    let text = std::fs::read_to_string(f.notes().join("knowledge/repos/game.md")).unwrap();
    assert_eq!(
        text,
        "---\ntitle: Game\nremote:\n  - git@example.com:team/old.git\n  - https://example.com/team/game.git\nstatus: x\n---\n\nREPO-GAME-TEXT\n"
    );
    assert!(
        f.session_start("s1", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
    // Again: already there, nothing written.
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(ok && out.contains("nothing was written"), "{out}");
    // A note with no properties gets them, its text kept.
    f.game_note("REPO-GAME-TEXT\r\n");
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(ok && out.contains("which listed none"), "{out}");
    let text = std::fs::read_to_string(f.notes().join("knowledge/repos/game.md")).unwrap();
    assert_eq!(
        text,
        "---\r\nremote: https://example.com/team/game.git\r\n---\r\n\r\nREPO-GAME-TEXT\r\n"
    );
}

#[test]
fn the_registry_still_matches_and_unions_with_remote_matches() {
    let f = Fixture::new();
    // A note's remote that isn't this folder's doesn't undo its registry entry.
    write(
        &f.notes().join("knowledge/repos/demo.md"),
        "---\nremote: git@example.com:team/elsewhere.git\n---\n\nREPO-DEMO-TEXT\n",
    );
    assert!(
        f.session_start("s1", &f.demo, &[])
            .unwrap()
            .contains("pick up 70")
    );
    // A registered folder whose remote a note lists is both names.
    git(
        &f.demo,
        &["remote", "add", "origin", "git@example.com:team/game.git"],
    );
    write(&f.notes().join("efforts/0080-game.md"), GAME);
    f.game_note("---\nremote: git@example.com:team/game.git\n---\n\nREPO-GAME-TEXT\n");
    let text = f.session_start("s2", &f.demo, &[]).unwrap();
    assert!(
        text.contains("active efforts are 70, \"0070 Demo\"; 80, \"0080 Game\""),
        "{text}"
    );
    assert!(f.bash("s2", &f.demo, "astrolabe activate --effort 70").1);
    // A repo without a remote is added to the registry, from its own folder.
    let local = f.dir.0.join("local");
    std::fs::create_dir_all(local.join("sub")).unwrap();
    git(&local, &["init", "-q"]);
    let (out, ok) = f.run(&local.join("sub"), &["repo", "add", "local"], "", &[]);
    assert!(
        ok && out.contains("has no remote another machine could match"),
        "{out}"
    );
    assert!(f.state().join("repos/local.json").exists());
    assert!(!f.notes().join("knowledge/repos/local.md").exists());
    // The notes checkout stays a record even with a remote: the plugin finds the notes by it.
    git(
        &f.notes(),
        &["remote", "add", "origin", "git@example.com:team/notes.git"],
    );
    std::fs::remove_file(f.state().join("repos/agent.json")).unwrap();
    let notes = f.notes();
    let (out, ok) = f.run(
        &notes,
        &["repo", "add", "agent", notes.to_str().unwrap()],
        "",
        &[],
    );
    assert!(ok, "{out}");
    assert!(f.state().join("repos/agent.json").exists());
    assert!(!f.notes().join("knowledge/repos/agent.md").exists());
}

#[test]
fn a_repo_notes_remote_that_cant_be_read_refuses_naming_the_file() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    for bad in [
        "remote: not a url",
        "remote:",
        "remote: [git@example.com:team/game.git, C:/x]",
    ] {
        f.game_note(&format!("---\n{bad}\n---\n"));
        let text = f.session_start("s1", &game, &[]).unwrap();
        assert!(
            text.contains("Can't list efforts: knowledge/repos/game.md: ")
                && text.contains("the owner fixes it in the note's properties"),
            "{bad}: {text}"
        );
        let (text, ok) = f.bash("s1", &game, "astrolabe activate --effort 80");
        assert!(!ok && text.contains("knowledge/repos/game.md: "), "{text}");
        let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
        assert!(!ok && !out.contains("Added"), "{out}");
    }
}

#[test]
fn an_scp_path_from_the_root_is_another_repo_than_one_from_home() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    f.game_note("---\nremote: git@example.com:/team/game.git\n---\n");
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(text.contains("has no note in the notes"), "{text}");
    // The same rooted path, in any spelling of it, still matches.
    let rooted = f.remote_repo("rooted", "git@example.com:/team/game.git/");
    assert!(
        f.session_start("s2", &rooted, &[])
            .unwrap()
            .contains("pick up 80")
    );
    // scp from home, ssh:// and https of a hosted repo are one repo.
    f.game_note(
        "---\nremote: [ssh://git@example.com/team/game.git, https://example.com/team/game]\n---\n",
    );
    assert!(
        f.session_start("s3", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
}

#[test]
fn a_partial_clone_is_matched_by_its_remote() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    // `git remote -v` prints a filter after a partial clone's URL.
    git(&game, &["config", "remote.origin.promisor", "true"]);
    git(
        &game,
        &["config", "remote.origin.partialclonefilter", "blob:none"],
    );
    f.game_note("---\nremote: git@example.com:team/game.git\n---\n");
    assert!(
        f.session_start("s1", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
}

#[test]
fn a_file_url_or_unknown_scheme_isnt_a_portable_remote() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "file://localhost/srv/game.git");
    write(
        &f.notes().join("knowledge/repos/web.md"),
        "---\nremote: https://localhost/srv/game.git\n---\n",
    );
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(
        text.contains("no remote another machine could match") && !text.contains("pick up 80"),
        "{text}"
    );
    // So `repo add` registers it on this machine instead of writing a note.
    let (out, ok) = f.run(&game, &["repo", "add", "game"], "", &[]);
    assert!(ok && out.contains("registered on this machine"), "{out}");
    assert!(!f.notes().join("knowledge/repos/game.md").exists());
    // A note naming one refuses, as it could never match.
    let hosted = f.remote_repo("hosted", "git@example.com:team/x.git");
    for bad in ["file:///srv/game.git", "ftp://example.com/team/game.git"] {
        f.game_note(&format!("---\nremote: {bad}\n---\n"));
        let text = f.session_start("s2", &hosted, &[]).unwrap();
        let named = format!("knowledge/repos/game.md: remote \"{bad}\" isn't a git remote URL");
        assert!(text.contains(&named), "{text}");
    }
}

#[test]
fn only_the_url_a_remote_fetches_from_matches() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    // A second `url` is push-only: git fetches from the first alone.
    git(
        &game,
        &[
            "config",
            "--add",
            "remote.origin.url",
            "git@example.com:team/other.git",
        ],
    );
    f.game_note("---\nremote: git@example.com:team/other.git\n---\n");
    let text = f.session_start("s1", &game, &[]).unwrap();
    assert!(
        text.contains("has no note in the notes") && !text.contains("pick up 80"),
        "{text}"
    );
    let (text, ok) = f.bash("s1", &game, "astrolabe activate --effort 80");
    assert!(!ok && text.contains("isn't the effort's (game)"), "{text}");
    f.game_note("---\nremote: git@example.com:team/game.git\n---\n");
    assert!(
        f.session_start("s2", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
}

#[test]
fn a_remote_with_only_a_push_url_is_skipped_and_the_rest_still_match() {
    let f = Fixture::new();
    let game = f.remote_repo("game", "git@example.com:team/game.git");
    git(
        &game,
        &[
            "config",
            "remote.mirror.pushurl",
            "git@example.com:team/mirror.git",
        ],
    );
    f.game_note("---\nremote: git@example.com:team/game.git\n---\n");
    assert!(
        f.session_start("s1", &game, &[])
            .unwrap()
            .contains("pick up 80")
    );
    // Its push URL is nothing this folder fetches, so it matches no note.
    f.game_note("---\nremote: git@example.com:team/mirror.git\n---\n");
    let text = f.session_start("s2", &game, &[]).unwrap();
    assert!(
        text.contains("has no note in the notes") && !text.contains("pick up 80"),
        "{text}"
    );
}

#[test]
fn another_repos_broken_note_doesnt_stop_this_one_matching() {
    let f = Fixture::new();
    let hosted = f.remote_repo("hosted", "git@example.com:team/hosted.git");
    write(
        &f.notes().join("knowledge/repos/hosted.md"),
        "---\nremote: git@example.com:team/hosted.git\n---\n",
    );
    // The game's note has an empty remote: unreadable, but plainly not this folder's repo.
    f.game_note("---\nremote:\n---\n");
    let text = f.session_start("s1", &hosted, &[]).unwrap();
    assert!(!text.contains("knowledge/repos/game.md"), "{text}");
    let (out, ok) = f.bash(
        "s1",
        &hosted,
        "astrolabe effort new --title \"Hosted work\" <<'ASTROLABE'\nA goal.\nASTROLABE\n",
    );
    assert!(ok && out.contains("created"), "{out}");
    // A folder that matches no note may be the broken one's, so it's still refused naming it.
    let other = f.remote_repo("loose", "git@example.com:team/loose.git");
    let text = f.session_start("s2", &other, &[]).unwrap();
    assert!(
        text.contains("knowledge/repos/game.md: `remote` has no URL"),
        "{text}"
    );
}
