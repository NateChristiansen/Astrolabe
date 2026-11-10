use super::*;

fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn allow(f: &Fixture, session: &str, command: &str) {
    let (text, ok) = f.bash(
        session,
        &f.demo,
        &format!(
            "astrolabe allow --once {} --said {}",
            quoted(command),
            quoted("Yes, run that exact command.")
        ),
    );
    assert!(
        ok && text.contains("Recorded the claimed owner approval"),
        "{text}"
    );
}

fn loaded() -> Fixture {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    f
}

fn input(f: &Fixture, command: &str) -> Value {
    json!({"session_id":"s1", "cwd":f.demo, "tool_name":"exec_command", "tool_input":{"cmd":command}})
}

#[test]
fn approval_is_exact_bound_to_the_session_and_directory_and_consumed_once() {
    let f = loaded();
    let command = "git push --force origin HEAD:main";
    assert!(f.denies("s1", command), "no approval");
    allow(&f, "s1", command);
    assert!(f.denies("s2", command), "another session");
    // Even a copied record can't transfer a lease to another session.
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(f.state().join("s1.json")).unwrap()).unwrap();
    state["session"] = json!("s2");
    write(&f.state().join("s2.json"), &state.to_string());
    std::fs::copy(
        f.state().join("s1.allow.json"),
        f.state().join("s2.allow.json"),
    )
    .unwrap();
    assert!(f.denies("s2", command));
    assert!(
        f.denies("s1", &format!("{command} ")),
        "one extra character"
    );
    let mut other = input(&f, command);
    other["tool_input"]["workdir"] = json!(f.other);
    assert!(f.hook(other).is_some(), "tool's own directory counts");
    let mut child = input(&f, command);
    child["agent_id"] = json!("child");
    assert!(f.hook(child).is_some());
    assert_eq!(f.hook(input(&f, command)), None);
    assert!(f.denies("s1", command), "second use is denied");
    let log = std::fs::read_to_string(f.state().join("guard-denials.log")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for event in ["lease-minted", "lease-used"] {
        let records: Vec<_> = events.iter().filter(|e| e["event"] == event).collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["command"], command);
        assert_eq!(records[0]["said"], "Yes, run that exact command.");
    }
}

#[test]
fn expired_or_corrupt_approval_keeps_the_denial() {
    let f = loaded();
    let command = "git push --force";
    allow(&f, "s1", command);
    let path = f.state().join("s1.allow.json");
    let mut state: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    state[0]["expires_ms"] = json!(0);
    write(&path, &state.to_string());
    assert!(f.denies("s1", command));
    state = json!({"command":command});
    write(&path, &state.to_string());
    assert!(f.denies("s1", command));
}

#[test]
fn several_explicit_approvals_can_wait_without_replacing_each_other() {
    let f = loaded();
    for command in ["git push --force", "gh issue create"] {
        allow(&f, "s1", command);
    }
    for command in ["git push --force", "gh issue create"] {
        assert_eq!(f.hook(input(&f, command)), None);
        assert!(f.denies("s1", command));
    }
}

#[test]
fn stale_session_writes_cannot_erase_or_restore_an_approval() {
    let f = loaded();
    let path = f.state().join("s1.json");
    let before = std::fs::read(&path).unwrap();
    let command = "git push --force";
    allow(&f, "s1", command);
    let after_mint = std::fs::read(&path).unwrap();
    // An overlapping hook may commit a snapshot read before the approval was minted.
    std::fs::write(&path, &before).unwrap();
    assert_eq!(f.hook(input(&f, command)), None, "the approval survives");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "consumption leaves session state alone"
    );
    // A snapshot from before consumption must not resurrect the consumed permission.
    std::fs::write(&path, &after_mint).unwrap();
    assert!(f.denies("s1", command));
}

#[test]
fn a_post_tool_hook_can_write_while_an_approval_hook_is_waiting() {
    use fs4::fs_std::FileExt;
    let f = loaded();
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.state().join("s1.allow.lock"))
        .unwrap();
    FileExt::lock_exclusive(&lock).unwrap();
    let command = "git push --force";
    let mint = input(&f, "astrolabe allow --once 'git push --force' --said yes");
    let mut pending = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["hook", "pre-command", "--if-active"])
        .current_dir(&f.demo)
        .env(STATE_VAR, f.state())
        .env("CLAUDE_PLUGIN_ROOT", f.dir.0.join("plugin"))
        .env("ASTROLABE_NO_DETACHED_SYNC", "1")
        .env_remove(EFFORT_VAR)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    pending
        .stdin
        .take()
        .unwrap()
        .write_all(mint.to_string().as_bytes())
        .unwrap();
    let post = json!({"session_id":"s1", "cwd":f.demo, "tool_name":"Edit", "tool_input":{}});
    let (_, ok) = f.run(
        &f.demo,
        &["hook", "post-tool", "--if-active"],
        &post.to_string(),
        &[],
    );
    assert!(ok);
    let state = || {
        serde_json::from_slice::<Value>(&std::fs::read(f.state().join("s1.json")).unwrap()).unwrap()
    };
    assert_eq!(state()["mark"], true);
    assert!(
        pending.try_wait().unwrap().is_none(),
        "the approval hook is still running"
    );
    drop(lock);
    let output = pending.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(f.hook(input(&f, command)), None);
    assert_eq!(
        state()["mark"],
        true,
        "lease writes preserve the post-tool update"
    );
    assert!(f.denies("s1", command));
}

#[test]
fn nested_shells_are_refused_and_a_log_failure_does_not_allow_execution() {
    let f = loaded();
    let nested = "sh -c 'git push --force; pkill node'";
    let (text, ok) = f.bash(
        "s1",
        &f.demo,
        &format!("astrolabe allow --once {} --said yes", quoted(nested)),
    );
    assert!(!ok && text.contains("without a wrapper"), "{text}");
    let command = "git push --force";
    allow(&f, "s1", command);
    let path = f.state().join("guard-denials.log");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        f.denies("s1", command),
        "failure to record consumption stays denied"
    );
    std::fs::remove_dir(&path).unwrap();
    assert!(f.denies("s1", command), "the lease was already consumed");
}

#[test]
fn allow_requires_the_hook_and_one_supported_denied_command() {
    let f = Fixture::new();
    let (text, ok) = f.run(
        &f.demo,
        &["allow", "--once", "git push --force", "--said", "yes"],
        "",
        &[],
    );
    assert!(!ok && text.contains("No approval recorded"), "{text}");
    assert!(!f.state().join("s1.json").exists());
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    for (command, why) in [
        (
            "astrolabe allow --once 'git status' --said yes",
            "wouldn't deny",
        ),
        (
            "astrolabe allow --once 'pkill node' --said yes",
            "cannot cover",
        ),
        (
            "astrolabe allow --once 'git reset --hard' --said yes",
            "cannot cover",
        ),
        (
            "astrolabe allow --once 'git push --force; echo hi' --said yes",
            "alone",
        ),
        (
            "astrolabe allow --once 'git push --force' --said yes && echo hi",
            "alone",
        ),
        (
            "astrolabe allow --once 'git push --force' --said ''",
            "nonempty",
        ),
    ] {
        let (text, ok) = f.bash("s1", &f.demo, command);
        assert!(!ok && text.contains(why), "{command}: {text}");
    }
    assert!(f.denies("s1", "git push --force"));
}

#[test]
fn powershell_literal_quotes_are_preserved_and_only_one_hook_can_consume() {
    let f = loaded();
    let command = "gh issue create --title 'Owner''s issue'";
    let text = format!(
        "astrolabe allow --once '{}' --said 'Yes, post it.'",
        command.replace('\'', "''")
    );
    let mut mint = input(&f, &text);
    mint["tool_name"] = json!("PowerShell");
    let answer = f.hook(mint).unwrap();
    assert_ne!(
        answer["hookSpecificOutput"]["permissionDecision"], "deny",
        "{answer}"
    );
    assert!(
        std::fs::read_to_string(f.state().join("s1.allow.json"))
            .unwrap()
            .contains("Owner''s issue")
    );
    let answers = std::thread::scope(|scope| {
        let a = scope.spawn(|| f.hook(input(&f, command)));
        let b = scope.spawn(|| f.hook(input(&f, command)));
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(answers.iter().filter(|a| a.is_none()).count(), 1);
}

#[cfg(unix)]
#[test]
fn a_force_push_to_a_local_bare_remote_runs_once() {
    let f = loaded();
    let remote = f.dir.0.join("remote.git");
    git(&f.demo, &["init", "--bare", "-q", remote.to_str().unwrap()]);
    git(
        &f.demo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let command = "git push --force origin HEAD:refs/heads/approved";
    allow(&f, "s1", command);
    assert_eq!(f.hook(input(&f, command)), None);
    let output = Command::new("sh")
        .args(["-c", command])
        .current_dir(&f.demo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    git(&remote, &["rev-parse", "--verify", "refs/heads/approved"]);
    assert!(f.denies("s1", command));
}

#[cfg(unix)]
#[test]
fn an_approved_github_write_runs_a_stub_once() {
    use std::os::unix::fs::PermissionsExt;
    let f = loaded();
    let bin = f.dir.0.join("bin");
    let stub = bin.join("gh");
    write(&stub, "#!/usr/bin/env sh\nprintf '%s\\n' \"$*\" >> calls\n");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let command = "gh issue create --title approved";
    allow(&f, "s1", command);
    assert_eq!(f.hook(input(&f, command)), None);
    let output = Command::new("sh")
        .args(["-c", command])
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .current_dir(&f.demo)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(f.denies("s1", command));
    assert_eq!(
        std::fs::read_to_string(f.demo.join("calls")).unwrap(),
        "issue create --title approved\n"
    );
}

#[test]
fn an_approval_with_no_effort_loaded_runs_once_and_a_second_run_is_denied() {
    let f = Fixture::new();
    let command = "gh issue create --title approved";
    assert!(f.denies("s1", command), "no approval");
    allow(&f, "s1", command);
    assert!(!f.state().join("s1.json").exists(), "no effort was loaded");
    assert_eq!(f.hook(input(&f, command)), None);
    assert!(f.denies("s1", command), "second use is denied");
    let log = std::fs::read_to_string(f.state().join("guard-denials.log")).unwrap();
    let events: Vec<Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for event in ["lease-minted", "lease-used"] {
        let records: Vec<_> = events.iter().filter(|e| e["event"] == event).collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["command"], command);
        assert_eq!(records[0]["session_id"], "s1");
    }
}

#[test]
fn an_approval_with_no_effort_is_exact_and_bound_to_the_session_and_directory() {
    let f = Fixture::new();
    let command = "git push --force origin HEAD:main";
    let mut mint = input(
        &f,
        &format!("astrolabe allow --once {} --said yes", quoted(command)),
    );
    mint["agent_id"] = json!("child");
    assert!(
        reason(&f.hook(mint).unwrap()).contains("A subagent can't record an owner approval"),
        "a subagent can't record one"
    );
    assert!(!f.state().join("s1.allow.json").exists());
    allow(&f, "s1", command);
    assert!(f.denies("s2", command), "another session");
    // A copied lease file can't transfer a lease to another session.
    std::fs::copy(
        f.state().join("s1.allow.json"),
        f.state().join("s2.allow.json"),
    )
    .unwrap();
    assert!(f.denies("s2", command));
    assert!(
        f.denies("s1", &format!("{command} ")),
        "one extra character"
    );
    let mut other = input(&f, command);
    other["tool_input"]["workdir"] = json!(f.other);
    assert!(f.hook(other).is_some(), "tool's own directory counts");
    let mut child = input(&f, command);
    child["agent_id"] = json!("child");
    assert!(f.hook(child).is_some(), "a subagent can't use one");
    assert_eq!(f.hook(input(&f, command)), None, "it was still there");
    assert!(f.denies("s1", command));
}

#[test]
fn an_expired_or_corrupt_approval_with_no_effort_keeps_the_denial() {
    let f = Fixture::new();
    let command = "git push --force";
    allow(&f, "s1", command);
    let path = f.state().join("s1.allow.json");
    let minted = std::fs::read(&path).unwrap();
    let mut leases: Value = serde_json::from_slice(&minted).unwrap();
    leases[0]["expires_ms"] = json!(0);
    write(&path, &leases.to_string());
    assert!(f.denies("s1", command), "expired");
    write(&path, &json!({"command":command}).to_string());
    assert!(f.denies("s1", command), "a corrupt lease file");
    let mint = format!("astrolabe allow --once {} --said yes", quoted(command));
    let (text, ok) = f.bash("s1", &f.demo, &mint);
    assert!(!ok && text.contains("Approval refused"), "{text}");
    // Session state that's there but unreadable isn't the same as no effort.
    std::fs::write(&path, &minted).unwrap();
    write(&f.state().join("s1.json"), "not a state file");
    // The hook denies every command of a session whose state it can't read, before it looks
    // for a lease, so the lease code's own read of that state is a second line never reached here.
    let answer = f.hook(input(&f, command)).unwrap();
    assert!(reason(&answer).contains("can't read its state"), "{answer}");
    let (text, ok) = f.bash("s1", &f.demo, &mint);
    assert!(
        !ok && text.contains("isn't a state file this program wrote"),
        "{text}"
    );
    std::fs::remove_file(f.state().join("s1.json")).unwrap();
    assert_eq!(f.hook(input(&f, command)), None, "the lease was intact");
}

#[test]
fn with_no_effort_nested_shells_are_refused_and_a_log_failure_does_not_allow_execution() {
    let f = Fixture::new();
    let nested = "sh -c 'git push --force; pkill node'";
    let (text, ok) = f.bash(
        "s1",
        &f.demo,
        &format!("astrolabe allow --once {} --said yes", quoted(nested)),
    );
    assert!(!ok && text.contains("without a wrapper"), "{text}");
    let command = "git push --force";
    allow(&f, "s1", command);
    let path = f.state().join("guard-denials.log");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        f.denies("s1", command),
        "failure to record consumption stays denied"
    );
    std::fs::remove_dir(&path).unwrap();
    assert!(f.denies("s1", command), "the lease was already consumed");
}

#[test]
fn dropping_the_effort_cancels_an_approval_that_was_waiting() {
    let f = loaded();
    let command = "git push --force";
    allow(&f, "s1", command);
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --off");
    assert!(ok, "{text}");
    assert!(!f.state().join("s1.allow.json").exists());
    assert!(f.denies("s1", command), "the approval went with the effort");
}
