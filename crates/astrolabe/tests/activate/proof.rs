use super::*;

#[test]
fn a_session_proves_its_guard_when_it_loads_an_effort() {
    let f = Fixture::new();
    // With no effort the session only reads: no probe, and no protection claimed.
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(!text.contains("astrolabe-guard-probe"), "{text}");
    assert!(text.contains("claim no protection"), "{text}");
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(
        status().contains("No effort is set") && status().contains("not proven yet"),
        "{}",
        status()
    );
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    let probe = f.probe(&text);
    f.control(&text);
    assert!(status().contains("not proven yet"), "{}", status());
    let child = json!({"session_id": "s1", "agent_id": "a1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": probe}});
    assert!(f.hook(child).is_some());
    assert!(f.denies("s2", &probe));
    assert!(
        status().contains("not proven yet"),
        "a child's or another session's denial doesn't count"
    );
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"), "{}", status());
    // Switched off and on again: proof starts over.
    f.machine("off");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.machine("on");
    assert!(status().contains("not proven yet"));
}

#[test]
fn the_guard_is_proven_only_by_a_hook_denial_of_this_activations_probe() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status().contains("not proven yet"));
    assert!(f.denies("s2", &probe), "another session's denial");
    // ... doesn't prove this one's guard.
    assert!(status().contains("not proven yet"));
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"), "{}", status());
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --effort 70").1,
        "same effort keeps it"
    );
    assert!(status().contains("Guard proven"));
    let marker = probe.split('\'').nth(1).unwrap().to_owned();
    // Switching effort revalidates the context but keeps the proof: the guard is the same
    // hook in the same session whatever effort is loaded, and a probe named in tool output
    // after a create was declined by the model (PC, 2026-11-03).
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 75");
    assert!(
        text.contains("authority: read") && status().contains("Guard proven"),
        "{}",
        status()
    );
    assert_eq!(f.probe(&text), probe, "one probe per session");
    // A probe that did run is a failure whenever it's found.
    std::fs::write(&marker, "").unwrap();
    assert!(status().contains("Guard FAILED"));
    std::fs::remove_file(&marker).unwrap();
    assert!(status().contains("Guard proven"));
}

#[test]
fn proof_needs_the_allowed_control_in_the_same_shell() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    assert!(
        text.contains("touch '") && text.contains("-control-posix'"),
        "marker only, no kill: {text}"
    );
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(f.denies("s1", &probe));
    let half = status();
    assert!(
        half.contains("not proven yet") && half.contains("control's marker isn't there"),
        "{half}"
    );
    f.control(&text);
    assert!(status().contains("Guard proven"));
}

#[test]
fn a_changed_hook_needs_its_own_proof() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
    write(&f.dir.0.join("plugin/hooks/hooks.json"), "hooks v2");
    assert!(status().contains("not proven yet"), "{}", status());
    // Resumed under the new hook: same switch, still unproven until its own denial.
    f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(status().contains("not proven yet"));
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
}

#[test]
fn a_loaded_effort_proves_again_after_the_switch_goes_off_and_on() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
    f.machine("off");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.machine("on");
    assert!(
        status().contains("effort 70") && status().contains("not proven yet"),
        "{}",
        status()
    );
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
}

#[test]
fn markers_live_where_a_sandboxed_shell_can_write_and_an_effort_load_names_its_shell() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    // The folder is in the temp folder, named for this state folder, and already made.
    let probe = f.probe(&text);
    let marker = probe.split('\'').nth(1).unwrap();
    let probe_dir = Path::new(marker).parent().unwrap();
    let temp = std::env::temp_dir();
    assert!(probe_dir.starts_with(&temp), "{probe}");
    let name = probe_dir.file_name().unwrap().to_string_lossy();
    assert!(name.starts_with("astrolabe-probe-"), "{probe}");
    assert!(probe_dir.is_dir(), "made, so the shell's touch works");
    // Loading an effort names the shell it ran in, so it offers that one's steps only.
    let both = text.contains("For PowerShell:") && text.contains("For the POSIX shell");
    assert!(!both, "{text}");
    f.control(&text);
    assert!(f.denies("s1", &f.probe(&text)));
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(
        status.contains("Guard proven") && status.contains("POSIX"),
        "{status}"
    );
}

#[test]
fn a_probe_sent_in_the_other_command_field_proves() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    // The guard reads `cmd` when there is no `command`, so the log has to as well.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"cmd": probe}});
    let answer = f.hook(input).unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status.contains("Guard proven"), "{status}");
    // A command given as a list is logged as the words the guard judged.
    let list = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": ["pkill", "node"]}});
    assert!(f.hook(list).is_some());
    let log = std::fs::read_to_string(f.state().join(astrolabe::hook::DENIAL_LOG)).unwrap();
    let last: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["command"], "pkill node");
}
