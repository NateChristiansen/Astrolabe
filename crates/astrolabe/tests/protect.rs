//! Receipts, the guard's self-test, the Codex mode and worktrees. Each test names the
//! failure it stops.
mod common;

use astrolabe::acp::{AcpClient, ClientEvent, Launch, ReceiptAt, adapter_entry};
use astrolabe::driver::{claude, codex};
use astrolabe::layer::{Layer, RepoSupplies, Target, TargetKind, compose, guard_rules};
use astrolabe::model::RunState;
use astrolabe::protect::unsafe_folder;
use astrolabe::worktree::{self, Closed, Progress};
use astrolabe::{Error, Result};
use common::{Harness, TempDir};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn fake_agent() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    let name = format!("fake_agent{}", std::env::consts::EXE_SUFFIX);
    profile.join("examples").join(name)
}

fn program_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_astrolabe"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn spec(h: &Harness, flags: &[&str], at: ReceiptAt) -> Launch {
    let mut launch = Launch {
        command: fake_agent().into(),
        args: flags.iter().map(|flag| (*flag).into()).collect(),
        ..Launch::default()
    };
    launch
        .protect(at, &program_dir(), &h.dir.0, std::env::var_os("PATH"))
        .unwrap();
    launch.protection.as_mut().unwrap().wait = Duration::from_millis(400);
    launch
}

fn until(client: &mut AcpClient, stop: impl Fn(&ClientEvent) -> bool) -> Vec<ClientEvent> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = client.next_event(Duration::from_millis(50)).unwrap() {
            let done = stop(&event);
            events.push(event);
            if done {
                return events;
            }
        }
    }
    panic!("no end in sight: {events:?}");
}

fn ended(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::Exited { .. })
}

fn ready(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::Ready)
}

fn failure(events: &[ClientEvent]) -> Option<&str> {
    events.iter().find_map(|event| match event {
        ClientEvent::LaunchFailed { message } => Some(message.as_str()),
        _ => None,
    })
}

#[test]
fn a_session_start_before_ready_lets_the_launch_through() {
    let h = Harness::new();
    let launch = spec(&h, &["--receipt-new"], ReceiptAt::BeforeReady);
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    until(&mut client, ready);
    client.shutdown().unwrap();
}

/// Stops: a session running with no hooks, which no client would mention.
#[test]
fn a_missing_receipt_before_ready_fails_the_launch_loudly() {
    let h = Harness::new();
    let launch = spec(&h, &[], ReceiptAt::BeforeReady);
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    let session = client.session_id();
    let events = until(&mut client, ended);
    assert!(!events.iter().any(ready), "{events:?}");
    let message = failure(&events).expect("a launch failure");
    assert!(message.contains("no session-start receipt"), "{message}");
    assert!(message.contains("ASTROLABE_RECEIPT_WAIT_MS"), "{message}");
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Failed
    );
}

/// Stops: a resume counting the report its earlier process left.
#[test]
fn an_older_report_is_not_a_receipt() {
    let h = Harness::new();
    let dir = astrolabe::hook::reports_dir_in(&h.dir.0);
    astrolabe::hook::write_report(&dir, "session-start", Some("fake-session"), &h.dir.0).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let launch = spec(&h, &[], ReceiptAt::BeforeReady);
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    let events = until(&mut client, ended);
    assert!(failure(&events).is_some(), "{events:?}");
}

/// Codex's hook fires only after a prompt, so a launch can't be judged before one.
#[test]
fn a_receipt_after_the_first_prompt_is_waited_for_from_that_prompt() {
    let h = Harness::new();
    let launch = spec(&h, &["--receipt-prompt"], ReceiptAt::AfterFirstPrompt);
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    until(&mut client, ready);
    // Longer than the wait: an idle session before any prompt is no failure.
    let idle = Instant::now();
    while idle.elapsed() < Duration::from_millis(700) {
        let event = client.next_event(Duration::from_millis(50)).unwrap();
        assert!(!matches!(event, Some(ClientEvent::LaunchFailed { .. })));
    }
    client.prompt("hello").unwrap();
    until(&mut client, |event| {
        matches!(event, ClientEvent::TurnEnded { .. })
    });
    let wait = Instant::now();
    while wait.elapsed() < Duration::from_millis(600) {
        let event = client.next_event(Duration::from_millis(50)).unwrap();
        assert!(!matches!(event, Some(ClientEvent::LaunchFailed { .. })));
    }
    client.shutdown().unwrap();
}

#[test]
fn no_receipt_after_the_first_prompt_ends_the_session_loudly() {
    let h = Harness::new();
    let launch = spec(&h, &["--hang-prompt"], ReceiptAt::AfterFirstPrompt);
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    until(&mut client, ready);
    let session = client.session_id();
    client.prompt("hello").unwrap();
    let events = until(&mut client, ended);
    let message = failure(&events).expect("a launch failure");
    assert!(message.contains("no session-start receipt"), "{message}");
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Failed
    );
}

/// Stops: a missing binary, which fails open and silent on both clients.
#[test]
fn a_guard_that_cannot_start_stops_the_launch() {
    let h = Harness::new();
    let empty = TempDir::new();
    let mut launch = spec(&h, &["--receipt-new"], ReceiptAt::BeforeReady);
    let path = launch.env.iter_mut().find(|(name, _)| name == "PATH");
    path.unwrap().1 = empty.0.clone().into();
    let error = AcpClient::launch(&h.engine, &launch, &h.dir.0).err();
    let Some(Error::GuardNotRunning(message)) = error else {
        panic!("expected the guard's failure, got {error:?}");
    };
    assert!(message.contains("could not start"), "{message}");
}

#[test]
fn landing_in_any_other_mode_fails_the_launch() {
    for (flags, why) in [
        (&["--mode-agent"][..], "landed in mode"),
        (&["--mode-full"][..], "agent-full-access"),
        (&["--mode-config-agent"][..], "landed in mode"),
        (&[][..], "did not report a mode"),
    ] {
        let h = Harness::new();
        let mut launch = spec(&h, flags, ReceiptAt::AfterFirstPrompt);
        launch.required_mode = Some(codex::MODE.to_owned());
        let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
        let events = until(&mut client, ended);
        let message = failure(&events).expect("a launch failure");
        assert!(message.contains(why), "{message}");
    }
}

#[test]
fn a_switch_away_from_the_required_mode_ends_the_session() {
    for flag in ["--switch-mode", "--switch-config"] {
        let h = Harness::new();
        let mut launch = spec(
            &h,
            &["--mode-safe", "--receipt-prompt", flag],
            ReceiptAt::AfterFirstPrompt,
        );
        launch.required_mode = Some(codex::MODE.to_owned());
        let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
        until(&mut client, ready);
        client.prompt("hello").unwrap();
        let events = until(&mut client, ended);
        assert!(
            failure(&events).unwrap().contains("landed in mode"),
            "{flag}"
        );
    }
}

/// Stops: a protected layer started by a caller who forgot the checks.
#[test]
fn a_protected_delivery_refuses_to_start_unprotected() {
    let delivery = compose(&product(), &Layer::protected("Layer")).unwrap();
    let h = Harness::new();
    let claude_dir = adapters(astrolabe::acp::CLAUDE_ADAPTER);
    let delivered = claude::deliver(&delivery, Path::new("/x"));
    let codex_dir = adapters(astrolabe::acp::CODEX_ADAPTER);
    let mut with_text = delivery.clone();
    with_text.instructions.as_mut().unwrap().repo = astrolabe::layer::RepoInstructions::None;
    let keys = [
        hook_key("session_start", "astrolabe hook session-start"),
        hook_key("pre_tool_use", "astrolabe hook pre-command"),
    ];
    let launches = [
        delivered.launch(&claude_dir.0, None).unwrap(),
        claude::launch(&claude_dir.0, None, delivered.options.clone()).unwrap(),
        codex::deliver(&with_text, &keys)
            .unwrap()
            .launch(&codex_dir.0, None)
            .unwrap(),
    ];
    for launch in launches {
        let error = AcpClient::launch(&h.engine, &launch, &h.dir.0).err();
        assert!(matches!(error, Some(Error::InvalidLayer(_))), "{error:?}");
    }
    let plain = Layer::standard("Layer", &[]);
    let delivery = compose(&product(), &plain).unwrap();
    let open = claude::deliver(&delivery, Path::new("/x"));
    assert!(
        !open
            .launch(&claude_dir.0, None)
            .unwrap()
            .protection_required
    );
}

fn hook_key(event: &str, command: &str) -> codex::SessionFlagKey {
    codex::SessionFlagKey {
        key: format!("/<session-flags>/config.toml:{event}:0:0"),
        current_hash: format!("sha256:{}", "1".repeat(64)),
        command: command.to_owned(),
    }
}

/// Shapes from a real hooks/list on the pinned client.
#[test]
fn hooks_must_be_listed_trusted_enabled_and_match_all() {
    let hook = |event: &str, command: &str, trust: &str| {
        json!({ "eventName": event, "command": command, "trustStatus": trust,
            "enabled": true, "matcher": null })
    };
    let start = hook("sessionStart", "astrolabe hook session-start", "trusted");
    let guard = hook("preToolUse", "astrolabe hook pre-command", "trusted");
    let list =
        |hooks: Vec<serde_json::Value>| json!({ "result": { "data": [{ "hooks": hooks }] } });
    assert_eq!(
        codex::hooks_problem(&list(vec![start.clone(), guard.clone()])),
        None
    );
    for (bad, why) in [
        (list(vec![guard.clone()]), "session-start"),
        (list(vec![start.clone()]), "not listed"),
        (list(vec![]), "not listed"),
        (json!({ "error": {} }), "not listed"),
        (
            list(vec![
                start.clone(),
                hook("preToolUse", "astrolabe hook pre-command", "modified"),
            ]),
            "modified",
        ),
        (
            list(vec![
                start.clone(),
                hook("preToolUse", "astrolabe hook pre-command", "untrusted"),
            ]),
            "untrusted",
        ),
    ] {
        let problem = codex::hooks_problem(&bad).expect(why);
        assert!(problem.contains(why), "{problem}");
    }
    let mut narrow = guard.clone();
    narrow["matcher"] = json!("Bash");
    assert!(codex::hooks_problem(&list(vec![start.clone(), narrow])).is_some());
    let mut off = guard;
    off["enabled"] = json!(false);
    assert!(codex::hooks_problem(&list(vec![start, off])).is_some());
}

#[test]
fn every_launch_declares_the_subagents_capability() {
    let h = Harness::new();
    let launch = Launch {
        command: fake_agent().into(),
        args: vec!["--dump-init".into()],
        ..Launch::default()
    };
    let mut client = AcpClient::launch(&h.engine, &launch, &h.dir.0).unwrap();
    until(&mut client, ready);
    client.shutdown().unwrap();
    let opened = std::fs::read_to_string(h.dir.0.join("init.json")).unwrap();
    let initialize: serde_json::Value = serde_json::from_str(&opened).unwrap();
    assert_eq!(
        initialize["params"]["clientCapabilities"]["subagents"],
        json!({})
    );
}

fn adapters(package: &str) -> TempDir {
    let dir = TempDir::new();
    let root = package
        .split('/')
        .fold(dir.0.join("node_modules"), |p, part| p.join(part));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("package.json"), r#"{"bin":"main.js"}"#).unwrap();
    std::fs::write(root.join("main.js"), "").unwrap();
    adapter_entry(&dir.0, package).unwrap();
    dir
}

#[test]
fn the_codex_driver_requires_the_one_mode_and_checks_the_folder() {
    let dir = adapters(astrolabe::acp::CODEX_ADAPTER);
    let launch = codex::launch(&dir.0, None, &json!({})).unwrap();
    assert_eq!(launch.required_mode.as_deref(), Some("workspace-write"));
    let mode = launch
        .env
        .iter()
        .find(|(name, _)| name == "INITIAL_AGENT_MODE");
    assert_eq!(mode.unwrap().1, "workspace-write");
    assert!(launch.folder_check.is_some());
    let claude_dir = adapters(astrolabe::acp::CLAUDE_ADAPTER);
    let claude = claude::launch(&claude_dir.0, None, None).unwrap();
    assert!(
        claude.folder_check.is_none(),
        "Claude is unaffected as far as known"
    );
}

fn windows_roots() -> [(&'static str, String); 3] {
    [
        ("LOCALAPPDATA", r"C:\Users\me\AppData\Local".to_owned()),
        ("TEMP", r"C:\Users\me\AppData\Local\Temp".to_owned()),
        ("TMP", String::new()),
    ]
}

#[test]
fn a_session_under_local_app_data_or_temp_is_refused_with_the_reason() {
    for cwd in [
        r"C:\Users\me\AppData\Local\x",
        r"c:\users\ME\appdata\local\Temp\x",
        r"\\?\C:\Users\me\AppData\Local\x",
        "C:/Users/me/AppData/Local",
    ] {
        let message = unsafe_folder(cwd, &windows_roots()).expect(cwd);
        assert!(message.contains("%LOCALAPPDATA%"), "{message}");
        assert!(message.contains("system32"), "{message}");
    }
    let roots = [("TEMP", r"D:\t".to_owned())];
    assert!(unsafe_folder(r"D:\t\x", &roots).unwrap().contains("%TEMP%"));
}

/// Real checkouts, a sibling that only shares a prefix, and an unset variable.
#[test]
fn a_session_elsewhere_passes() {
    for cwd in [
        r"C:\Users\me\workspace\x",
        r"C:\Users\Public\x",
        r"C:\Users\me\AppData\LocalLow\x",
    ] {
        assert_eq!(unsafe_folder(cwd, &windows_roots()), None, "{cwd}");
    }
}

/// Stops: anything starting in a refused folder, on a launch or a resume.
#[test]
fn a_refused_folder_starts_nothing() {
    fn refuse(_: &Path) -> std::result::Result<(), String> {
        Err("no".to_owned())
    }
    let h = Harness::new();
    let launch = Launch {
        command: fake_agent().into(),
        args: vec!["--dump-init".into()],
        folder_check: Some(refuse),
        ..Launch::default()
    };
    let error = AcpClient::launch(&h.engine, &launch, &h.dir.0).err();
    assert!(
        matches!(error, Some(Error::UnsafeSessionFolder(_))),
        "{error:?}"
    );
    assert!(!h.dir.0.join("init.json").exists());
}

#[cfg(windows)]
#[test]
fn the_real_check_refuses_this_machines_temp_folder() {
    let dir = TempDir::new();
    let message = astrolabe::protect::refuse_unsafe_windows_folder(&dir.0).unwrap_err();
    assert!(message.contains("system32"), "{message}");
}

#[cfg(not(windows))]
#[test]
fn the_real_check_has_no_opinion_off_windows() {
    let dir = TempDir::new();
    assert_eq!(
        astrolabe::protect::refuse_unsafe_windows_folder(&dir.0),
        Ok(())
    );
}

fn product() -> Target {
    Target {
        kind: TargetKind::Product,
        supplies: RepoSupplies::default(),
    }
}

#[test]
fn the_protected_layer_reaches_claude_with_a_match_all_guard_and_both_tools_rules() {
    let delivery = compose(&product(), &Layer::protected("Layer")).unwrap();
    let options = claude::deliver(&delivery, Path::new("/x")).options.unwrap();
    let settings = &options["settings"];
    let guard = &settings["hooks"]["PreToolUse"][0];
    assert!(guard.get("matcher").is_none(), "match-all has no matcher");
    assert_eq!(guard["hooks"][0]["command"], "astrolabe hook pre-command");
    assert_eq!(
        settings["hooks"]["ConfigChange"][0]["hooks"][0]["command"],
        "astrolabe hook config-change"
    );
    let denies: Vec<_> = settings["permissions"]["deny"].as_array().unwrap().to_vec();
    assert!(denies.contains(&json!("Bash(git push --force:*)")));
    assert!(denies.contains(&json!("PowerShell(git push --force *)")));
    assert_eq!(denies.len(), guard_rules().len());
}

#[test]
fn the_protected_layer_reaches_codex_by_event_with_no_config_change() {
    let delivery = compose(&product(), &Layer::protected("Layer")).unwrap();
    let mut with_text = delivery.clone();
    with_text.instructions.as_mut().unwrap().repo = astrolabe::layer::RepoInstructions::None;
    let key = |event: &str, command: &str| codex::SessionFlagKey {
        key: format!("/<session-flags>/config.toml:{event}:0:0"),
        current_hash: format!("sha256:{}", "1".repeat(64)),
        command: command.to_owned(),
    };
    let keys = [
        key("session_start", "astrolabe hook session-start"),
        key("pre_tool_use", "astrolabe hook pre-command"),
    ];
    let config = codex::deliver(&with_text, &keys).unwrap().config;
    assert_eq!(
        config["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "astrolabe hook pre-command"
    );
    assert!(config["hooks"].get("ConfigChange").is_none());
    let state = config["hooks"]["state"].as_object().unwrap();
    assert!(state.contains_key("/<session-flags>/config.toml:pre_tool_use:0:0"));
    assert!(state.contains_key("/<session-flags>/config.toml:session_start:0:0"));
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .args([] as [&str; 0])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn repo(holder: &TempDir) -> PathBuf {
    let repo = holder.0.join("thing");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "T"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "first"]);
    repo
}

fn create(repo: &Path, name: &str) -> worktree::Worktree {
    let mut creating = worktree::create(repo, name).unwrap();
    while let Some(progress) = creating.poll() {
        match progress {
            Progress::Creating { .. } => {}
            Progress::Ready(worktree) => return worktree,
            Progress::Failed(message) => panic!("{message}"),
        }
    }
    panic!("no outcome");
}

#[test]
fn a_clean_worktree_is_named_for_its_session_and_cleaned_up() -> Result<()> {
    let holder = TempDir::new();
    let repo = repo(&holder);
    let made = create(&repo, "session-1");
    assert_eq!(
        made.path,
        holder.0.join("thing.worktrees").join("session-1")
    );
    assert!(made.path.join("a.txt").exists());
    assert_eq!(
        worktree::close(&made)?,
        Closed::Removed { branch_kept: false }
    );
    assert!(!made.path.exists());
    Ok(())
}

#[test]
fn a_dirty_worktree_is_never_removed() -> Result<()> {
    let holder = TempDir::new();
    let repo = repo(&holder);
    let made = create(&repo, "session-2");
    std::fs::write(made.path.join("a.txt"), "changed").unwrap();
    assert!(matches!(worktree::close(&made)?, Closed::KeptDirty(_)));
    assert_eq!(std::fs::read_to_string(made.path.join("a.txt"))?, "changed");
    git(&made.path, &["checkout", "--", "a.txt"]);
    std::fs::write(made.path.join("new.txt"), "untracked").unwrap();
    assert!(matches!(worktree::close(&made)?, Closed::KeptDirty(_)));
    assert!(made.path.join("new.txt").exists());
    Ok(())
}

#[test]
fn commits_nothing_else_has_keep_their_branch() -> Result<()> {
    let holder = TempDir::new();
    let repo = repo(&holder);
    let made = create(&repo, "session-3");
    std::fs::write(made.path.join("b.txt"), "b").unwrap();
    git(&made.path, &["add", "."]);
    git(&made.path, &["commit", "-qm", "work"]);
    assert_eq!(
        worktree::close(&made)?,
        Closed::Removed { branch_kept: true }
    );
    git(&repo, &["rev-parse", "--verify", "session-3"]);
    Ok(())
}

#[test]
fn creation_fails_on_a_bad_name_or_an_existing_path_and_reports_git_failing() {
    let holder = TempDir::new();
    let repo = repo(&holder);
    for name in ["", "../x", "a/b", "-x", ".hidden", "a..b", "a.lock", "a b"] {
        assert!(worktree::create(&repo, name).is_err(), "{name}");
    }
    let made = create(&repo, "session-4");
    assert!(worktree::create(&repo, "session-4").is_err());
    // Same branch, new folder name impossible through `create`: a non-repo is what's left.
    let not_repo = holder.0.join("plain");
    std::fs::create_dir_all(&not_repo).unwrap();
    let mut creating = worktree::create(&not_repo, "s").unwrap();
    let mut failed = false;
    while let Some(progress) = creating.poll() {
        failed |= matches!(progress, Progress::Failed(_));
    }
    assert!(failed);
    worktree::close(&made).unwrap();
}
