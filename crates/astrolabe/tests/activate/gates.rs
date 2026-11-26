//! Gate for any cut to activation text: the guard proof stays an independent challenge in
//! each app's own shell tool. The allowed control runs in a real shell and makes its marker;
//! the probe is refused by the hook itself; and nothing the program says about itself, run
//! without the hook, counts as proof. Run it before and after a wording change.
use super::*;

/// Each shell tool an app sends: Claude's Bash and PowerShell, and Codex's shell command.
fn tools() -> Vec<(&'static str, Value)> {
    vec![
        ("Bash", json!({})),
        ("PowerShell", json!({})),
        ("shell_command", json!({"turn_id": "u1"})),
    ]
}

fn input(f: &Fixture, session: &str, tool: &str, extra: &Value, command: &str) -> Value {
    let mut input = json!({"session_id": session, "cwd": f.demo, "tool_name": tool, "tool_input": {"command": command}});
    for (key, value) in extra.as_object().into_iter().flatten() {
        input[key] = value.clone();
    }
    input
}

/// An Astrolabe command as the app runs it: through the hook, then the read it hands back.
fn astrolabe(f: &Fixture, session: &str, tool: &str, extra: &Value, command: &str) -> String {
    let answer = f.hook(input(f, session, tool, extra, command)).unwrap();
    let read = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    f.run(&f.demo, &rewritten(read), "", &[]).0
}

/// A command run in the shell the tool runs, as the app does once the hook lets it through.
/// Returns false where that shell isn't on this machine.
fn in_shell(f: &Fixture, command: &str) -> bool {
    let powershell = command.starts_with("New-Item");
    let mut shell = if powershell {
        if !cfg!(windows) {
            return false;
        }
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    let out = match shell.current_dir(&f.demo).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(e) => panic!("{command}: {e}"),
    };
    assert!(
        out.status.success(),
        "{command}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    true
}

#[test]
fn gate_the_guard_proof_is_a_challenge_only_the_hook_can_answer_in_each_apps_shell() {
    for (tool, extra) in tools() {
        let f = Fixture::new();
        let session = format!("g-{tool}");
        let text = astrolabe(&f, &session, tool, &extra, "astrolabe activate --effort 70");
        // The agent is given the challenge, not a claim: both commands, the check, and no
        // protection until it passes.
        assert!(text.contains("which must run: "), "{tool}: {text}");
        let probe = f.probe(&text);
        let marker = PathBuf::from(probe.split('\'').nth(1).unwrap());
        assert!(
            text.contains("astrolabe activate --status") && text.contains("claim no protection"),
            "{tool}: {text}"
        );
        let status = || astrolabe(&f, &session, tool, &extra, "astrolabe activate --status");
        assert!(status().contains("not proven yet"), "{tool}: {}", status());
        // The control goes through the hook untouched and runs in the tool's real shell.
        let control = text
            .lines()
            .find_map(|l| l.split_once("which must run: ").map(|(_, c)| c.to_owned()))
            .unwrap();
        let answer = f.hook(input(&f, &session, tool, &extra, &control));
        assert!(
            answer
                .as_ref()
                .is_none_or(|a| a["hookSpecificOutput"]["permissionDecision"] != "deny"),
            "{tool}: the control was refused: {answer:?}"
        );
        if !in_shell(&f, &control) {
            f.control(&text);
        }
        // Nothing run around the hook proves it: the program asked directly, as a session
        // whose app skipped the hook would, only reports.
        for args in [
            &["activate", "--status"][..],
            &["activate", "--effort", "70"],
            &["status"],
        ] {
            f.run(&f.demo, args, "", &[]);
        }
        assert!(
            status().contains("not proven yet"),
            "{tool}: proven without a denial: {}",
            status()
        );
        // The probe is refused by the hook, in the reply shape the app enforces, so it
        // never runs and its marker is never made.
        let answer = f.hook(input(&f, &session, tool, &extra, &probe)).unwrap();
        assert_eq!(
            answer["hookSpecificOutput"]["permissionDecision"], "deny",
            "{tool}: {answer}"
        );
        assert!(!marker.exists(), "{tool}");
        let proven = status();
        assert!(proven.contains("Guard proven"), "{tool}: {proven}");
        // Had the app run it anyway, the marker would say so, and the proof falls.
        std::fs::write(&marker, "").unwrap();
        assert!(status().contains("Guard FAILED"), "{tool}: {}", status());
    }
}
