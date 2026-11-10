//! The bypass table, then the hook as a real process: its one output form and its
//! failure rule.
use astrolabe::guard::{Scope, decide, names_denied_program};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const TABLE: &str = include_str!("fixtures/guard-bypass.tsv");

#[test]
fn the_bypass_table_holds() {
    let mut wrong = Vec::new();
    let mut rows = 0;
    for line in TABLE.lines().filter(|l| !l.starts_with('#')) {
        let [expect, machine, tool, command] = line.splitn(4, '\t').collect::<Vec<_>>()[..] else {
            panic!("bad row: {line}");
        };
        rows += 1;
        let root = match machine {
            "win" => r"C:\work\repo",
            "unc" => r"\\server\share\repo",
            _ => "/work/repo",
        };
        let scope = Scope {
            root: Some(root.to_owned()),
            cwd: Some(root.to_owned()),
        };
        let command = command.replace('⏎', "\n");
        let verdict = decide(tool, &json!({ "command": command }), &scope);
        let denied = verdict.is_some();
        if denied != (expect == "deny") {
            wrong.push(format!(
                "{expect} {machine} {tool}: {command:?} got {verdict:?}"
            ));
        }
    }
    assert!(rows > 200, "the table went missing: {rows}");
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_command_given_as_a_list_is_read_too() {
    let scope = Scope::default();
    let input = json!({ "command": ["bash", "-lc", "git push --force"] });
    assert!(decide("shell", &input, &scope).is_some());
    let input = json!({ "command": ["ls", "-la"] });
    assert!(decide("shell", &input, &scope).is_none());
}

#[test]
fn a_tools_own_working_directory_counts() {
    let scope = Scope {
        root: Some("/work/repo".to_owned()),
        cwd: Some("/work/repo".to_owned()),
    };
    let outside = json!({ "cmd": "rm -rf target", "workdir": "/tmp" });
    assert!(decide("exec_command", &outside, &scope).is_some());
    let inside = json!({ "cmd": "rm -rf target", "workdir": "sub" });
    assert!(decide("exec_command", &inside, &scope).is_none());
}

#[test]
fn no_worktree_known_means_a_recursive_delete_is_denied() {
    let input = json!({ "command": "rm -rf target" });
    assert!(decide("Bash", &input, &Scope::default()).is_some());
}

#[test]
fn nesting_past_the_limit_is_denied_when_it_names_a_denied_program() {
    let mut command = "pkill node".to_owned();
    let mut harmless = "true".to_owned();
    for _ in 0..40 {
        command = format!("eval {command}");
        harmless = format!("eval {harmless}");
    }
    let scope = Scope::default();
    assert!(decide("Bash", &json!({ "command": command }), &scope).is_some());
    assert!(decide("Bash", &json!({ "command": harmless }), &scope).is_none());
    assert!(names_denied_program("x; /usr/bin/GIT y"));
    assert!(!names_denied_program("digit rmdirs"));
}

fn run_hook(data: &Path, input: &[u8]) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["hook", "pre-command", "--if-active"])
        .env(astrolabe::activate::STATE_VAR, data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8(output.stdout).unwrap(),
        output.status.code().unwrap(),
    )
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "astrolabe-guard-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("on"), "").unwrap();
    dir
}

#[test]
fn a_denial_is_json_with_exit_0_and_logged() {
    let data = temp("deny");
    let input = json!({
        "session_id": "s1", "cwd": data, "tool_name": "Bash", "agent_id": "child",
        "tool_input": { "command": "sh -c 'git push --force'" },
    });
    let (stdout, code) = run_hook(&data, input.to_string().as_bytes());
    assert_eq!(code, 0);
    let answer: Value = serde_json::from_str(&stdout).unwrap();
    let specific = &answer["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "PreToolUse");
    assert_eq!(specific["permissionDecision"], "deny");
    assert!(
        specific["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("force-push")
    );

    let log = std::fs::read_to_string(data.join(astrolabe::hook::DENIAL_LOG)).unwrap();
    let entry: Value = serde_json::from_str(log.trim()).unwrap();
    assert_eq!(entry["command"], "sh -c 'git push --force'");
    assert_eq!(entry["agent_id"], "child");
}

#[test]
fn approval_guidance_is_only_given_for_force_pushes_and_github_writes() {
    let data = temp("approval-hint");
    let hint = "If the owner approved this exact command, run `astrolabe allow --once \"<command>\" --said \"<their words>\"` alone, then run it again within five minutes.";
    for (command, covered) in [
        ("git push --force", true),
        ("gh issue create", true),
        ("pkill node", false),
        ("git reset --hard", false),
    ] {
        let input = json!({"session_id":"s1", "cwd":data, "tool_name":"Bash", "tool_input":{"command":command}});
        let (stdout, code) = run_hook(&data, input.to_string().as_bytes());
        assert_eq!(code, 0);
        let answer: Value = serde_json::from_str(&stdout).unwrap();
        let reason = answer["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap();
        assert_eq!(reason.contains(hint), covered, "{reason}");
    }
}

/// An allow must be silence: a hook's explicit allow removes the client's prompt.
#[test]
fn anything_else_is_silence_with_exit_0() {
    let data = temp("allow");
    let input = json!({ "session_id": "s1", "cwd": data, "tool_name": "Bash",
        "tool_input": { "command": "ls" } });
    assert_eq!(
        run_hook(&data, input.to_string().as_bytes()),
        (String::new(), 0)
    );
    let input = json!({ "session_id": "s1", "cwd": data, "tool_name": "apply_patch",
        "tool_input": { "input": "git push --force" } });
    assert_eq!(
        run_hook(&data, input.to_string().as_bytes()),
        (String::new(), 0)
    );
    assert!(!data.join(astrolabe::hook::DENIAL_LOG).exists());
}

#[test]
fn input_the_plugin_cannot_read_is_denied_without_claiming_proof() {
    let data = temp("broken");
    let (stdout, code) = run_hook(&data, b"{\"tool_input\": {\"command\": \"git push --fo");
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "{stdout}"
    );
    let (stdout, code) = run_hook(&data, b"{not json, ls");
    assert_eq!(code, 0);
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "{stdout}"
    );
    assert!(
        !data.join(astrolabe::hook::DENIAL_LOG).exists(),
        "not proof of interception"
    );
}

#[test]
fn a_panic_inside_is_caught_and_follows_the_same_rule() {
    let data = temp("panic");
    let run = |input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["hook", "pre-command", "--if-active"])
            .env(astrolabe::activate::STATE_VAR, &data)
            .env("ASTROLABE_GUARD_TEST_PANIC", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let named =
        run(&json!({ "session_id": "s1", "cwd": data, "tool_name": "Bash", "tool_input": { "command": "git status" } }).to_string());
    assert!(named.status.success());
    let stdout = String::from_utf8(named.stdout).unwrap();
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "{stdout}"
    );
    assert!(named.stderr.is_empty());
    let plain = run(&json!({ "session_id": "s1", "cwd": data, "tool_name": "Bash", "tool_input": { "command": "ls" } }).to_string());
    assert!(plain.status.success());
    assert!(plain.stdout.is_empty());
}

#[test]
fn a_panic_that_escapes_the_hook_denies_a_command_and_is_silent_elsewhere() {
    let data = temp("escaped-panic");
    let run = |event: &str, input: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["hook", event, "--if-active"])
            .env(astrolabe::activate::STATE_VAR, &data)
            .env("ASTROLABE_HOOK_TEST_PANIC", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    // Nothing was judged, so even a harmless command is blocked rather than run unguarded.
    let input = json!({ "session_id": "s1", "cwd": data, "tool_name": "Bash", "tool_input": { "command": "ls" } }).to_string();
    let command = run("pre-command", &input);
    assert!(command.status.success());
    assert!(command.stderr.is_empty());
    let answer: serde_json::Value = serde_json::from_slice(&command.stdout).expect("a denial");
    let inner = &answer["hookSpecificOutput"];
    assert_eq!(inner["permissionDecision"], "deny", "{answer}");
    assert!(
        inner["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .starts_with("Astrolabe's hook failed before it finished with this command"),
        "{answer}"
    );
    assert!(!data.join("guard-denials.log").exists());
    for event in [
        "stop",
        "session-start",
        "prompt",
        "post-tool",
        "session-end",
    ] {
        let other = run(
            event,
            &json!({ "session_id": "s1", "cwd": data }).to_string(),
        );
        assert!(other.status.success(), "{event}");
        assert!(other.stdout.is_empty(), "{event}");
        assert!(other.stderr.is_empty(), "{event}");
    }
}
