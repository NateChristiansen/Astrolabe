use super::*;

#[test]
fn notes_init_seeds_a_checkout_whose_first_effort_is_offered_at_the_seat() {
    let (f, (out, ok)) = Fixture::seeded();
    assert!(ok && out.contains("registered on this machine"), "{out}");
    assert!(
        out.contains("pick up 1") && out.contains("no remote"),
        "{out}"
    );
    let notes = f.dir.0.join("notes");
    for file in [
        "knowledge/owner.md",
        "knowledge/voice.md",
        "knowledge/machines.md",
        "knowledge/repos/agent.md",
        "AGENTS.md",
    ] {
        assert!(notes.join(file).is_file(), "{file}");
    }
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&notes)
        .output()
        .unwrap();
    assert!(
        status.status.success() && status.stdout.is_empty(),
        "all committed"
    );
    // Registered, so the machine switches on with no further step.
    assert!(f.machine("on").contains("on for this machine"));
    let text = f.session_start("n1", &notes, &[]).unwrap();
    assert!(text.contains("the owner's seat"), "{text}");
    assert!(
        text.contains("one active effort is 1, \"0001 Fill in your notes\"; say \"pick up 1\""),
        "{text}"
    );
    let (text, ok) = f.bash("n1", &notes, "astrolabe activate --effort 1");
    assert!(ok && text.contains("involvement: each step"), "{text}");
    assert!(
        text.contains("- [ ] Owner") || text.contains("walks the owner through"),
        "{text}"
    );
    // A product repo never offers the notes' own effort.
    let (out, ok) = f.run(
        &f.demo,
        &["repo", "add", "demo", f.demo.to_str().unwrap()],
        "",
        &[],
    );
    assert!(ok, "{out}");
    let text = f.session_start("d1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("No active effort names this repo (demo)"),
        "{text}"
    );
    // Never twice, and never over someone's folder.
    let again = f.dir.0.join("notes2");
    let (_, ok) = f.run(
        &f.demo,
        &["notes", "init", again.to_str().unwrap()],
        "",
        IDENT,
    );
    assert!(!ok && !again.exists(), "a machine with notes keeps them");
}

#[test]
fn notes_init_with_no_git_identity_leaves_nothing_and_a_rerun_works() {
    let dir = TempDir::new();
    let state = dir.0.join("state");
    let run = |args: &[&str], env: &[(&str, &str)]| {
        Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(args)
            .current_dir(&dir.0)
            .env(STATE_VAR, &state)
            .env_remove("EMAIL")
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let nobody = [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "user.useConfigOnly"),
        ("GIT_CONFIG_VALUE_0", "true"),
    ];
    let notes = dir.0.join("notes");
    let out = run(&["notes", "init", notes.to_str().unwrap()], &nobody);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no name and email"));
    assert!(!notes.exists(), "a folder it made is taken away again");
    // An empty folder made by hand stays, empty; a plain `git init` one has no remote.
    std::fs::create_dir_all(&notes).unwrap();
    git(&notes, &["init", "-q"]);
    let out = run(&["notes", "init", notes.to_str().unwrap()], &nobody);
    assert!(!out.status.success());
    let left: Vec<_> = std::fs::read_dir(&notes)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, [".git"], "only what was there");
    let out = run(&["notes", "init", notes.to_str().unwrap()], IDENT);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && said.contains("no remote"), "{said}");
}
