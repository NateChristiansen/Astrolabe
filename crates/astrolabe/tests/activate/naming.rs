use super::*;

/// The hook's whole answer to an event, or None when it stays silent.
fn answer(f: &Fixture, event: &str, input: Value, env: &[(&str, &str)]) -> Option<Value> {
    let (out, ok) = f.run(
        &f.demo,
        &["hook", event, "--if-active"],
        &input.to_string(),
        env,
    );
    assert!(ok, "a hook always exits 0: {out}");
    (!out.trim().is_empty()).then(|| serde_json::from_str(out.trim()).unwrap())
}

fn title(answer: &Option<Value>) -> Option<&str> {
    answer.as_ref()?["hookSpecificOutput"]["sessionTitle"].as_str()
}

fn prompt(f: &Fixture, session: &str, current: Option<&str>) -> Option<Value> {
    let mut input = json!({"session_id": session, "cwd": f.demo, "prompt": "hi"});
    if let Some(current) = current {
        input["session_title"] = current.into();
    }
    answer(f, "prompt", input, &[])
}

#[test]
fn the_name_is_the_number_then_the_title_without_its_own_number() {
    use astrolabe::activate::session_name;
    assert_eq!(session_name("70", "0070 Demo"), "70 Demo");
    assert_eq!(session_name("83", "Name sessions"), "83 Name sessions");
    assert_eq!(session_name("83", "2026 roadmap"), "83 2026 roadmap");
    assert_eq!(session_name("7", "0007"), "7");
    // A title that starts with the number as part of a word keeps it.
    assert_eq!(session_name("7", "0007 7zip support"), "7 7zip support");
    assert_eq!(session_name("7", "7zip support"), "7 7zip support");
}

/// Catches: a name given on every prompt, or over the owner's own rename.
#[test]
fn a_claude_session_is_named_at_the_next_prompt_once_and_an_owner_rename_stays() {
    let f = Fixture::new();
    assert_eq!(prompt(&f, "s1", None), None, "no effort, no name");
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    // Told to rename itself with the desktop app's tool, so the name shows at once.
    assert!(
        ok && text.contains("Rename this session to \"70 Demo\"")
            && text.contains("set_session_title")
            && !text.contains("set_thread_title"),
        "{text}"
    );
    // The desktop app's own title reads as a custom one: binding an effort wins over it.
    let first = prompt(&f, "s1", Some("Auto title"));
    assert_eq!(title(&first), Some("70 Demo"));
    assert_eq!(
        first.unwrap()["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert_eq!(prompt(&f, "s1", Some("70 Demo")), None, "once");
    // Renamed by the owner, then picked up again: their name stays, and the agent isn't
    // told to undo it.
    let (again, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && !again.contains("set_session_title"), "{again}");
    assert_eq!(prompt(&f, "s1", Some("Mine")), None);
    let resumed =
        json!({"session_id": "s1", "cwd": f.demo, "source": "resume", "session_title": "Mine"});
    assert_eq!(title(&answer(&f, "session-start", resumed, &[])), None);
}

/// Catches: a launched worker waiting for a prompt that never comes, or a title on compact.
#[test]
fn a_claude_session_on_an_effort_from_its_start_is_named_then() {
    let f = Fixture::new();
    let start = json!({"session_id": "w1", "cwd": f.demo, "source": "startup"});
    let started = answer(&f, "session-start", start, &[(EFFORT_VAR, "70")]);
    assert_eq!(title(&started), Some("70 Demo"));
    assert!(
        started.unwrap()["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some()
    );
    assert_eq!(prompt(&f, "w1", Some("70 Demo")), None);
    // Compacted: no name there, as the app takes none; the next prompt gives it.
    assert!(f.bash("w2", &f.demo, "astrolabe activate --effort 70").1);
    let compact = json!({"session_id": "w2", "cwd": f.demo, "source": "compact"});
    assert_eq!(title(&answer(&f, "session-start", compact, &[])), None);
    assert_eq!(title(&prompt(&f, "w2", None)), Some("70 Demo"));
}

/// Catches: a title field in Codex hook output, which has none, or a rename asked again on resume.
#[test]
fn a_codex_agent_is_told_to_rename_its_thread_when_it_picks_an_effort_up() {
    let f = Fixture::new();
    let input = json!({"session_id": "c1", "turn_id": "u1", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let rewritten_to = f.hook(input).unwrap();
    let command = rewritten_to["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let (text, ok) = f.run(&f.demo, &rewritten(command), "", &[]);
    assert!(
        ok && text.contains("Rename this thread to \"70 Demo\"")
            && text.contains("set_thread_title"),
        "{text}"
    );
    let (again, ok) = f.run(&f.demo, &rewritten(command), "", &[]);
    assert!(ok, "{again}");
    let input = json!({"session_id": "c1", "turn_id": "u3", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let rerun = f.hook(input).unwrap();
    let rerun = rerun["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    let (again, ok) = f.run(&f.demo, &rewritten(&rerun), "", &[]);
    assert!(
        ok && !again.contains("set_thread_title"),
        "asked once: {again}"
    );
    let codex_prompt = json!({"session_id": "c1", "turn_id": "u2", "cwd": f.demo, "prompt": "hi"});
    assert_eq!(answer(&f, "prompt", codex_prompt, &[]), None);
    let resumed = json!({"session_id": "c1", "cwd": f.demo, "source": "resume", "transcript_path": "/x/rollout-1.jsonl"});
    let resumed = answer(&f, "session-start", resumed, &[]).unwrap();
    assert!(resumed["hookSpecificOutput"].get("sessionTitle").is_none());
    let context = resumed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(!context.contains("set_thread_title"), "{context}");
}
