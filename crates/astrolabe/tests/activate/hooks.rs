use super::*;

#[test]
fn the_hook_does_the_work_and_the_command_only_prints_it() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --show 0123");
    assert!(!ok && text.contains("only from its hook"), "{text}");
    let (text, ok) = f.run(&f.demo, &["activate", "--effort", "70"], "", &[]);
    assert!(
        !ok && text.contains("No effort loaded")
            && text.contains("/hooks")
            && text.contains("claim no protection"),
        "{text}"
    );
    let made_up = ["activate", "--show", "0123456789abcdef0123456789abcdef"];
    let (text, ok) = f.run(&f.demo, &made_up, "", &[]);
    assert!(
        !ok && text.contains("isn't one the hook just issued"),
        "{text}"
    );
    // Shell around the command: not rewritten, so it runs bare and is refused.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "cd x && astrolabe activate --effort 70"}});
    assert_eq!(f.hook(input), None);
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let answer = f.hook(input).unwrap();
    assert!(
        f.state().join("s1.json").exists(),
        "on before the command runs"
    );
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let words = rewritten(command);
    assert_eq!(words[..2], ["activate", "--show"]);
    // The command writes nothing, so a read-only sandbox can run it.
    let (text, ok) = f.run(&f.demo, &words, "", &[]);
    assert!(ok && text.contains("effort 70"), "{text}");
}

#[test]
fn a_subagent_inherits_the_guard_only_and_cant_switch() {
    let f = Fixture::new();
    let child = |session: &str, command: &str| json!({"session_id": session, "agent_id": "a1", "agent_type": "general-purpose", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": command}});
    assert!(
        f.hook(child("s1", "pkill x")).is_some(),
        "guarded while the machine is on, effort or not"
    );
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let answer = f.hook(child("s1", "astrolabe activate --off")).unwrap();
    assert!(answer.to_string().contains("subagent can't switch"));
    let answer = f
        .hook(child("s1", "astrolabe activate --effort 72"))
        .unwrap();
    assert!(answer.to_string().contains("subagent can't switch"));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("effort 70"),
        "the parent's switch stands"
    );
    // Guard, from the parent: a denial, and an allowed control.
    let answer = f.hook(child("s1", "pkill x")).unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(f.hook(child("s1", "git status")), None);
    assert!(
        f.hook(child("s2", "pkill x")).is_some(),
        "any session's child"
    );
    assert!(
        f.bash("s2", &f.demo, "astrolabe activate --status")
            .0
            .contains("No effort is set"),
        "a child switched nothing"
    );
    // The denial keeps the child's identity, and doesn't prove the parent's guard.
    let log = std::fs::read_to_string(f.state().join("guard-denials.log")).unwrap();
    assert!(log.lines().any(|l| l.contains("\"agent_id\":\"a1\"")));
    assert!(f.hook(child("s1", &f.probe(&text))).is_some());
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("not proven yet")
    );
    f.control(&text);
    assert!(f.denies("s1", &f.probe(&text)));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("Guard proven")
    );
    f.machine("off");
    assert_eq!(
        f.hook(child("s1", "pkill x")),
        None,
        "off for the machine: children too"
    );
}

#[test]
fn codex_input_is_told_apart_and_bound_the_same_way() {
    let f = Fixture::new();
    let input = json!({"session_id": "t1", "turn_id": "u1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let answer = f.hook(input).unwrap();
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let words = rewritten(command);
    let (text, ok) = f.run(&f.demo, &words, "", &[]);
    assert!(ok && text.contains("codex session t1"), "{text}");
}

/// Catches: a rewrite or probe that PowerShell can't parse, which Codex on Windows runs.
#[test]
fn a_command_that_runs_in_powershell_is_called_and_probed_in_its_form() {
    let f = Fixture::new();
    let rewrite = |input: Value| {
        let answer = f.hook(input).unwrap();
        let command = &answer["hookSpecificOutput"]["updatedInput"]["command"];
        command.as_str().unwrap().to_owned()
    };
    let claude_ps = json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let command = rewrite(claude_ps);
    assert!(
        command.starts_with("Get-Content -Raw -Encoding UTF8 -LiteralPath '"),
        "{command}"
    );
    let (text, _) = f.run(&f.demo, &rewritten(&command), "", &[]);
    assert!(
        f.probe(&text).starts_with("New-Item"),
        "the tool's own shell"
    );
    let codex = json!({"session_id": "c1", "turn_id": "u1", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let command = rewrite(codex);
    assert_eq!(
        command.starts_with("Get-Content -Raw"),
        cfg!(windows),
        "{command}"
    );
    let (text, ok) = f.run(&f.demo, &rewritten(&command), "", &[]);
    assert!(ok, "{text}");
    let probe = f.probe(&text);
    f.control(&text);
    let powershell = probe.starts_with("New-Item -ItemType File -Force '")
        && probe.ends_with("'; Stop-Process -Name astrolabe-guard-probe");
    assert_eq!(powershell, cfg!(windows), "{probe}");
    // Denied in either form, but only the form for the shell that ran it proves the guard.
    let codex_runs = |command: &str| json!({"session_id": "c1", "turn_id": "u1", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": command}});
    let other = if powershell {
        probe
            .replace("New-Item -ItemType File -Force", "touch")
            .replace("Stop-Process -Name", "pkill -0")
    } else {
        probe
            .replace("touch", "New-Item -ItemType File -Force")
            .replace("pkill -0", "Stop-Process -Name")
    };
    let denied = |form: &str| {
        let answer = f.hook(codex_runs(form)).unwrap();
        answer["hookSpecificOutput"]["permissionDecision"] == "deny"
    };
    let status = || {
        let status = rewrite(codex_runs("astrolabe activate --status"));
        f.run(&f.demo, &rewritten(&status), "", &[]).0
    };
    assert!(status().contains("not proven yet"));
    assert!(denied(&other), "{other}");
    assert!(
        status().contains("not proven yet"),
        "the wrong shell's form: {}",
        status()
    );
    assert!(denied(&probe), "{probe}");
    let shell = if powershell {
        "PowerShell"
    } else {
        "the POSIX shell"
    };
    assert!(
        status().contains("Guard proven") && status().contains(shell),
        "{}",
        status()
    );
    // Claude's PowerShell tool: its own form, its own control, named as the path proven.
    let (text, _) = f.run(&f.demo, &rewritten(&rewrite(json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --status"}}))), "", &[]);
    let ps_probe = f.probe(&text);
    assert!(ps_probe.starts_with("New-Item"), "{text}");
    f.control(&text);
    let ps = json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": ps_probe}});
    assert_eq!(
        f.hook(ps).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let (text, _) = f.run(&f.demo, &rewritten(&rewrite(json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --status"}}))), "", &[]);
    assert!(
        text.contains("Guard proven for this session in PowerShell"),
        "{text}"
    );
}

#[test]
fn the_plugin_folder_calls_this_program_and_names_one_plugin() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin");
    let read = |file: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(file)).unwrap()).unwrap()
    };
    let hooks = read("astrolabe/hooks/hooks.json");
    for (event, slug) in [
        ("SessionStart", "session-start"),
        ("PreToolUse", "pre-command"),
    ] {
        let command = &hooks["hooks"][event][0]["hooks"][0]["command"];
        // Unquoted from `~`: Codex on Windows runs hooks in PowerShell, Claude in Git Bash or
        // sh, and all of them expand it, with no PATH, which a Mac app from the Dock may lack.
        let wanted = format!("~/.local/bin/astrolabe hook {slug} --if-active");
        assert_eq!(command, &wanted, "the installed build, by its home path");
        assert!(
            hooks["hooks"][event][0].get("matcher").is_none(),
            "every tool, as the guard decides"
        );
    }
    let name = &read("astrolabe/.claude-plugin/plugin.json")["name"];
    assert_eq!(
        &read(".claude-plugin/marketplace.json")["plugins"][0]["name"],
        name
    );
    assert_eq!(
        &read(".agents/plugins/marketplace.json")["plugins"][0]["name"],
        name
    );
    let skill = std::fs::read_to_string(root.join("astrolabe/skills/effort/SKILL.md")).unwrap();
    // Plain words name an effort, so the model may use the skill, but only on the owner's number.
    for wanted in [
        "pick up 70",
        "work on effort 70",
        "/astrolabe:effort 70",
        "Not for a number that only comes up in discussion",
        "proves which session this is, not what the owner meant",
        "astrolabe activate --effort <n>",
    ] {
        assert!(skill.contains(wanted), "{wanted}");
    }
    assert!(!skill.contains("disable-model-invocation"));
    let codex =
        std::fs::read_to_string(root.join("astrolabe/skills/effort/agents/openai.yaml")).unwrap();
    assert!(codex.contains("allow_implicit_invocation: true"));
}

#[test]
fn hook_input_with_a_byte_order_mark_is_read_as_without() {
    let f = Fixture::new();
    let bom = |input: Value| format!("\u{feff}{input}");
    // Session start, as a PowerShell pipe would send it.
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "startup"});
    let (out, ok) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &bom(input),
        &[],
    );
    assert!(ok);
    let out: Value = serde_json::from_str(out.trim()).unwrap_or_else(|_| panic!("{out}"));
    let text = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("pick up 70") && text.contains("OWNER-TEXT"),
        "{text}"
    );
    // Session start wants the home up, and the harness keeps it to a marker, not a server.
    assert!(f.state().join("home-requested").exists());
    // Pre-command: the probe from loading an effort is denied by the real guard, for this
    // session, so it proves it.
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    f.control(&text);
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": f.probe(&text)}});
    let (out, ok) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &bom(input),
        &[],
    );
    assert!(ok);
    let out: Value = serde_json::from_str(out.trim()).unwrap_or_else(|_| panic!("{out}"));
    let reason = out["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(
        out["hookSpecificOutput"]["permissionDecision"] == "deny"
            && !reason.contains("could not read"),
        "{out}"
    );
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status.contains("Guard proven"), "{status}");
    // The switch command is taken too.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &bom(input),
        &[],
    );
    assert!(out.contains("cat '"), "{out}");
}

/// Catches: a rewrite that isn't a plain read. Claude's plan mode ignores a hook's allow for
/// anything but a read-only command, and the hook has written by the time the app decides.
#[test]
fn the_rewritten_command_is_a_read_of_the_tickets_text() {
    let f = Fixture::new();
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let answer = f.hook(input).unwrap();
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    let shown = f.state().join("shown");
    let path = command
        .strip_prefix("cat '")
        .unwrap_or_else(|| panic!("{command}"))
        .strip_suffix('\'')
        .unwrap();
    assert_eq!(Path::new(path).parent(), Some(shown.as_path()), "{command}");
    let printed = std::fs::read_to_string(path).unwrap();
    let (shown, ok) = f.run(&f.demo, &rewritten(&command), "", &[]);
    assert!(ok && printed.contains("effort 70"), "{printed}");
    assert_eq!(printed, shown, "the file says what --show says");
    let ticket = read_ticket(&command).unwrap();
    assert_eq!(ticket.len(), 32, "the text file sits beside the ticket");
    // An effort command's result too.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe effort save --status done"}});
    let answer = f.hook(input).unwrap();
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    assert!(
        command.starts_with("Get-Content -Raw -Encoding UTF8 -LiteralPath '"),
        "{command}"
    );
}

#[test]
fn hook_input_that_cant_be_read_is_blocked_while_on_and_ignored_while_off() {
    let f = Fixture::new();
    let raw = |stdin: &str| {
        let (out, ok) = f.run(&f.demo, &["hook", "pre-command", "--if-active"], stdin, &[]);
        assert!(ok, "a hook always exits 0");
        (!out.trim().is_empty()).then(|| serde_json::from_str::<Value>(out.trim()).unwrap())
    };
    let unreadable = [
        "not json at all".to_owned(),
        r#"{"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "echo hi"#.to_owned(),
        String::new(),
        json!({"session_id": "s1", "tool_name": "Bash", "tool_input": {}}).to_string(),
        json!({"session_id": "s1", "tool_name": "shell_command", "tool_input": {"description": "x"}}).to_string(),
        json!({"session_id": "s1", "tool_input": {"command": "echo hi"}}).to_string(),
    ];
    for stdin in &unreadable {
        let answer = raw(stdin).unwrap_or_else(|| panic!("allowed: {stdin:?}"));
        let reason = answer["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap();
        assert!(
            answer["hookSpecificOutput"]["permissionDecision"] == "deny"
                && reason.starts_with("Astrolabe couldn't read what the app sent its hook (")
                && reason.contains("can't tell which session this is or check the command")
                && reason.contains("`astrolabe off` in a terminal"),
            "{stdin:?}: {answer}"
        );
    }
    // Readable input for a tool the guard doesn't check passes, as before.
    for tool in [
        json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Read", "tool_input": {"file_path": "/x"}}),
        json!({"session_id": "s1", "cwd": f.demo, "tool_name": "mcp__x__y", "tool_input": {}}),
        json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "git status"}}),
    ] {
        assert_eq!(raw(&tool.to_string()), None, "{tool}");
    }
    // These denials are no session's proof.
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status.contains("Guard not proven yet"), "{status}");
    // Off for the machine: the plugin stays idle, whatever arrives.
    f.machine("off");
    for stdin in &unreadable {
        assert_eq!(raw(stdin), None, "{stdin:?}");
    }
}

#[test]
fn large_hook_input_is_read_whole_and_still_guarded() {
    let f = Fixture::new();
    let big = "rm -rf / ".repeat(250_000); // over 2 MiB
    let edit = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Edit", "tool_input": {"file_path": "/x", "old_string": big, "new_string": "y"}});
    assert_eq!(
        f.hook_raw(edit.to_string().into_bytes()),
        None,
        "an edit isn't a command"
    );
    let bash = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": format!("pkill -f x # {big}")}});
    let answer = f.hook_raw(bash.to_string().into_bytes()).expect("guarded");
    assert!(
        !reason(&answer).contains("couldn't read"),
        "judged, not unreadable: {answer}"
    );
    let fine = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": format!("echo {}", "a".repeat(2 << 20))}});
    assert_eq!(f.hook_raw(fine.to_string().into_bytes()), None);
}

#[test]
fn hook_input_past_the_cap_runs_only_for_a_tool_the_guard_doesnt_check() {
    let f = Fixture::new();
    let huge = "a".repeat(65 << 20);
    let edit = format!(
        r#"{{"session_id": "s1", "tool_name": "Write", "tool_input": {{"content": "{huge}"}}}}"#
    );
    assert_eq!(f.hook_raw(edit.into_bytes()), None);
    let bash = format!(
        r#"{{"session_id": "s1", "tool_name": "Bash", "tool_input": {{"command": "echo {huge}"}}}}"#
    );
    let answer = f.hook_raw(bash.into_bytes()).expect("blocked");
    assert!(
        reason(&answer).contains("more than the hook reads whole"),
        "{answer}"
    );
    let nameless = format!(r#"{{"session_id": "s1", "tool_input": {{"command": "echo {huge}"}}}}"#);
    assert!(f.hook_raw(nameless.into_bytes()).is_some());
}

#[test]
fn the_cap_counts_what_follows_a_byte_order_mark() {
    let f = Fixture::new();
    let template = r#"{"session_id": "s1", "tool_name": "Write", "tool_input": {"content": ""}}"#;
    let pad = (64 << 20) - template.len();
    let at = template.find(r#""}}"#).unwrap();
    let body = format!("{}{}{}", &template[..at], "a".repeat(pad), &template[at..]);
    assert_eq!(body.len(), 64 << 20, "exactly the cap");
    assert_eq!(f.hook_raw(format!("\u{feff}{body}").into_bytes()), None);
}

#[test]
fn input_trickling_past_the_deadline_is_blocked_as_unreadable() {
    let f = Fixture::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["hook", "pre-command", "--if-active"])
        .current_dir(&f.demo)
        .env(STATE_VAR, f.state())
        .env("ASTROLABE_HOOK_STDIN_DEADLINE_MS", "600")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    // A byte at a time, each well inside the idle wait, for longer than the deadline.
    let writer = std::thread::spawn(move || {
        for byte in
            br#"{"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "echo hi"}}"#
        {
            if pipe.write_all(&[*byte]).and_then(|_| pipe.flush()).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    let started = std::time::Instant::now();
    let output = child.wait_with_output().unwrap();
    let took = started.elapsed();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        reason(&answer)
            .starts_with("Astrolabe couldn't read what the app sent its hook (not a JSON object)"),
        "{answer}"
    );
    assert!(took < std::time::Duration::from_secs(4), "{took:?}");
    let _ = writer.join();
}

#[test]
fn only_gated_plugin_hooks_are_available() {
    let f = Fixture::new();
    for event in [
        "session-start",
        "pre-command",
        "post-tool",
        "stop",
        "session-end",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["hook", event])
            .current_dir(&f.demo)
            .env(STATE_VAR, f.state())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "plain hook {event} must be refused"
        );
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("hook <event> --if-active"));
    }
    let (help, ok) = f.run(&f.demo, &["--help"], "", &[]);
    assert!(ok);
    // `notes init` and `initiative new` are new; the retired one was v1's top-level `init`.
    let help = help
        .replace("notes <init <folder>|", "notes <")
        .replace("initiative new …|", "");
    for retired in ["init", "doctor", "serve [--db", "where", "mcp"] {
        assert!(
            !help.contains(retired),
            "retired command in help: {retired}"
        );
    }
}
