use super::*;
use astrolabe::effort::EffortFile;

fn session(m: &Machine) -> Value {
    serde_json::from_slice(&std::fs::read(m.state().join("s1.json")).unwrap()).unwrap()
}
fn save_session(m: &Machine, state: &Value) {
    write(&m.state().join("s1.json"), &state.to_string());
}
fn waiting(m: &Machine, codex: bool) -> Value {
    let run = |cmd| {
        if codex {
            m.bash_codex("s1", cmd)
        } else {
            m.bash("s1", cmd)
        }
    };
    assert!(run(NEW).1);
    let (out, ok) = run(
        "astrolabe effort save --status blocked --waits-for \"owner: land it?\" <<'ASTROLABE'\nBlocked on landing.\nASTROLABE",
    );
    assert!(ok, "{out}");
    let mut state = session(m);
    state["delivered_ms"] = (Machine::now_ms() - 2_000).into();
    save_session(m, &state);
    state
}
fn input(m: &Machine, codex: bool) -> Value {
    let at = Machine::now_ms() - 1_000;
    let path = m.dir.0.join("transcript.jsonl");
    let text = if codex {
        format!(
            "{}\n{}\n",
            json!({"type":"session_meta", "payload":{"id":"s1"}}),
            json!({"type":"response_item", "payload":{"type":"message", "role":"user",
            "internal_chat_message_metadata_passthrough":{"create_time":at as f64/1000.0,
                "content_item_metadata":[{"provenance":{"type":"user"}}]}}})
        )
    } else {
        format!(
            "{}\n",
            json!({"type":"user", "sessionId":"s1", "origin":{"kind":"human"},
            "timestamp":chrono::DateTime::from_timestamp_millis(at).unwrap().to_rfc3339()})
        )
    };
    write(&path, &text);
    let mut input = json!({"session_id":"s1", "cwd":m.demo, "transcript_path":path,
        "tool_name":"Bash", "tool_input":{"command":"echo working"}, "stop_hook_active":true});
    if codex {
        input["turn_id"] = "t1".into();
    }
    input
}

#[test]
fn reply_clears_the_board_once_without_counting_as_a_progress_save() {
    for codex in [false, true] {
        for hook in ["pre-command", "stop"] {
            let world = World::new();
            let m = world.machine("pc");
            let mut before = waiting(&m, codex);
            before["mark"] = true.into();
            before["future"] = json!({"preserve":true});
            save_session(&m, &before);
            let original = EffortFile::parse(&m.effort_text(1)).unwrap();
            let commits = m.log().lines().count();
            m.sync_requested();
            let input = input(&m, codex);
            assert!(m.hook_json(hook, input.clone()).is_none());
            let current = EffortFile::parse(&m.effort_text(1)).unwrap();
            assert_eq!(current.prop("status"), Some("active"));
            assert_eq!(current.prop("waits_for"), None);
            assert_eq!(current.current, original.current);
            for field in ["saved_by", "session", "head", "branch", "dirty"] {
                assert_eq!(current.prop(field), original.prop(field));
            }
            let after = session(&m);
            for field in [
                "baseline",
                "last_save_ms",
                "saves",
                "mark",
                "acknowledged",
                "future",
            ] {
                assert_eq!(after[field], before[field], "{field}");
            }
            assert_eq!(
                after["delivered"],
                astrolabe::notes::content_hash(&m.effort_text(1))
            );
            assert_eq!(m.log().lines().count(), commits + 1);
            assert!(m.sync_requested());
            assert!(m.hook_json(hook, input.clone()).is_none());
            assert_eq!(m.log().lines().count(), commits + 1);
            let (out, ok) = m.run(&m.demo, &["efforts", "--repo", "demo"], "");
            assert!(ok, "{out}");
            let board: Value = serde_json::from_str(&out).unwrap();
            assert_eq!(board["efforts"][0]["needs_owner"], false);
            let page = std::fs::read_to_string(m.notes.join("home.html")).unwrap();
            assert!(!column(&page, "needsyou").contains("Smooth ink"));
            // The new version was acknowledged, so the next normal save is not stale.
            let (out, ok) = if codex {
                m.bash_codex("s1", "astrolabe effort save --status active")
            } else {
                m.bash("s1", "astrolabe effort save --status active")
            };
            assert!(ok, "{out}");
            // Asking again must not be cleared by the reply that answered the previous wait.
            let command = "astrolabe effort save --status blocked --waits-for \"owner: land it?\" <<'ASTROLABE'\nBlocked on landing.\nASTROLABE";
            let mut reask = input.clone();
            reask["tool_use_id"] = "new-question".into();
            reask["tool_input"]["command"] = command.into();
            let (out, ok) = m.shell(reask, &m.demo, command);
            assert!(ok, "{out}");
            assert!(m.hook_json(hook, input).is_none());
            assert!(m.effort_text(1).contains("owner: land it?"));
        }
    }
}

#[test]
fn ambiguous_busy_or_newer_waits_are_left_alone() {
    let world = World::new();
    let m = world.machine("pc");
    let before = waiting(&m, false);
    let original = m.effort_text(1);
    let path = m.notes.join(before["file"].as_str().unwrap());
    let commits = m.log().lines().count();
    for case in [
        "worker",
        "child",
        "missing cutoff",
        "old reply",
        "missing transcript",
        "provider",
        "reload failed",
        "stale version",
        "unresolved sync",
        "other session",
        "done",
        "external wait",
        "pending",
        "broken",
        "locked",
    ] {
        let mut state = before.clone();
        let mut input = input(&m, false);
        let mut effort = EffortFile::parse(&original).unwrap();
        let mut held = None;
        let notes = astrolabe::notes::Notes::new(m.notes.clone(), m.state());
        let pending = m.state().join("ops/test-reply.json");
        match case {
            "worker" => state["worker"] = true.into(),
            "child" => input["agent_id"] = "child".into(),
            "missing cutoff" => {
                state.as_object_mut().unwrap().remove("delivered_ms");
            }
            "old reply" => state["delivered_ms"] = Machine::now_ms().into(),
            "missing transcript" => input["transcript_path"] = "missing-file".into(),
            "provider" => input["turn_id"] = "codex".into(),
            "reload failed" => state["reload_failed"] = "unavailable".into(),
            "stale version" => effort.set("waits_for", "owner: a newer question"),
            "unresolved sync" => m.set_sync("stale", json!({"files":[before["file"]]})),
            "other session" => effort.set("session", "other"),
            "done" => effort.set("status", "done"),
            "external wait" => effort.set("waits_for", "CI"),
            "broken" => write(&pending, "broken"),
            "pending" => write(
                &pending,
                &json!({"key":"test-reply", "files":[], "renames":[],
                "journal":[], "message":"pending", "base":null})
                .to_string(),
            ),
            "locked" => held = Some(notes.lock().unwrap()),
            _ => unreachable!(),
        }
        let text = effort.render();
        write(&path, &text);
        if case != "stale version" {
            state["delivered"] = astrolabe::notes::content_hash(&text).into();
        }
        save_session(&m, &state);
        m.hook_json("pre-command", input);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{case}");
        assert_eq!(m.log().lines().count(), commits, "{case}");
        if case == "unresolved sync" {
            assert!(notes.warnings().iter().any(|w| w.contains("Two machines")));
            m.set_sync("stale", Value::Null);
        }
        drop(held);
        let _ = std::fs::remove_file(&pending);
    }
}

#[test]
fn a_reply_in_the_notes_repo_does_not_manufacture_or_hide_a_save_nudge() {
    for dirty in [false, true] {
        let world = World::new();
        let m = world.machine("pc");
        waiting(&m, false);
        let command =
            "astrolabe effort save --status blocked --waits-for \"owner: review the notes?\"";
        let (out, ok) = m.bash_from("s1", &m.notes, command, None);
        assert!(ok, "{out}");
        let mut before = session(&m);
        before["delivered_ms"] = (Machine::now_ms() - 2_000).into();
        save_session(&m, &before);
        if dirty {
            write(&m.notes.join("knowledge/pending.md"), "Unsaved findings");
        }
        let mut input = input(&m, false);
        input["cwd"] = m.notes.to_str().unwrap().into();
        input["stop_hook_active"] = false.into();
        let answer = m.hook_json("stop", input);
        assert_eq!(answer.is_some(), dirty, "{answer:?}");
        assert!(!m.effort_text(1).contains("owner: review the notes?"));
        let after = session(&m);
        for field in ["last_save_ms", "saves"] {
            assert_eq!(after[field], before[field]);
        }
        // RepoFacts already excludes the notes repo's HEAD: no special rebaseline needed.
        assert_eq!(after["baseline"], before["baseline"]);
    }
}

#[test]
fn a_reload_or_stale_save_starts_a_new_reply_window() {
    let world = World::new();
    let m = world.machine("pc");
    let before = waiting(&m, false);
    let input = input(&m, false);
    let path = m.notes.join(before["file"].as_str().unwrap());
    let mut effort = EffortFile::parse(&m.effort_text(1)).unwrap();
    effort.set("waits_for", "owner: newer question");
    write(&path, &effort.render());
    let (out, ok) = m.bash("s1", "astrolabe effort save --status active");
    assert!(!ok && out.contains("changed since"), "{out}");
    assert!(
        session(&m)["delivered_ms"].as_i64().unwrap() > before["delivered_ms"].as_i64().unwrap()
    );
    assert!(m.hook_json("pre-command", input.clone()).is_none());
    assert!(m.effort_text(1).contains("owner: newer question"));
    // Reloading the exact same note is a new delivery too.
    let mut state = session(&m);
    state["delivered_ms"] = before["delivered_ms"].clone();
    save_session(&m, &state);
    let (out, ok) = m.bash("s1", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    assert!(
        session(&m)["delivered_ms"].as_i64().unwrap() > before["delivered_ms"].as_i64().unwrap()
    );
    assert!(m.hook_json("pre-command", input).is_none());
    assert!(m.effort_text(1).contains("owner: newer question"));
}

#[test]
fn parallel_session_edits_survive_the_reply_commit() {
    for switch in [false, true] {
        let world = World::new();
        let m = world.machine("pc");
        let before = waiting(&m, false);
        let input = input(&m, false);
        // Pause only this fixture's commit, after the reply took its initial snapshot.
        let git_dir = m.notes.join(".git");
        write(
            &git_dir.join("hooks/pre-commit"),
            "#!/bin/sh\ntouch .git/reply-ready\ni=0\nwhile [ ! -f .git/reply-release ]; do\n i=$((i+1))\n [ $i -lt 400 ] || exit 1\n sleep 0.05\ndone\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                git_dir.join("hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let mut child = Kill(
            m.command(&m.demo, &["hook", "pre-command", "--if-active"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        child
            .0
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        let start = std::time::Instant::now();
        while !git_dir.join("reply-ready").exists() {
            assert!(start.elapsed().as_secs() < 15, "reply did not reach commit");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut concurrent = before.clone();
        concurrent["mark"] = true.into();
        concurrent["named"] = "a newer name".into();
        if switch {
            concurrent["effort"] = "2".into();
            concurrent["delivered"] = "another effort".into();
            concurrent["delivered_ms"] = Machine::now_ms().into();
        }
        save_session(&m, &concurrent);
        write(&git_dir.join("reply-release"), "");
        assert!(child.0.wait().unwrap().success());
        let after = session(&m);
        if switch {
            assert_eq!(after, concurrent);
        } else {
            assert_eq!(after["mark"], true);
            assert_eq!(after["named"], concurrent["named"]);
            assert_ne!(after["delivered"], before["delivered"]);
        }
    }
}

#[test]
fn a_reply_leaves_review_status_and_its_wait_untouched() {
    for codex in [false, true] {
        let world = World::new();
        let m = world.machine("pc");
        waiting(&m, codex);
        let command = "astrolabe effort save --status review --waits-for \"owner: review it\" <<'ASTROLABE'\nBuilt and checked, awaiting review.\nASTROLABE";
        let (out, ok) = if codex {
            m.bash_codex("s1", command)
        } else {
            m.bash("s1", command)
        };
        assert!(ok, "{out}");
        let mut state = session(&m);
        state["delivered_ms"] = (Machine::now_ms() - 2000).into();
        save_session(&m, &state);
        let before = m.effort_text(1);
        let commits = m.log().lines().count();
        assert!(m.hook_json("pre-command", input(&m, codex)).is_none());
        assert_eq!(m.effort_text(1), before);
        assert_eq!(m.log().lines().count(), commits);
    }
}

/// Catches: an orchestrator's question staying under Needs you after the owner answered it,
/// though its text says a reply clears an active or blocked wait.
#[test]
fn a_reply_clears_an_owner_wait_on_an_initiative_page() {
    let world = World::new();
    let m = world.machine("pc");
    write(
        &m.notes.join("initiatives/smooth-lines.md"),
        include_str!("../fixtures/initiatives/prose.md"),
    );
    git(&m.notes, &["add", "initiatives"]);
    git(&m.notes, &["commit", "-qm", "legacy page"]);
    assert!(
        m.bash("s1", "astrolabe activate --initiative smooth-lines")
            .1
    );
    let (out, ok) = m.bash(
        "s1",
        "astrolabe effort save --status blocked --waits-for \"owner: which effort starts first?\" <<'ASTROLABE'\nFiled, waiting on the order.\nASTROLABE",
    );
    assert!(ok, "{out}");
    let mut state = session(&m);
    state["delivered_ms"] = (Machine::now_ms() - 2_000).into();
    save_session(&m, &state);
    let read = || {
        let text = std::fs::read_to_string(m.notes.join("initiatives/smooth-lines.md")).unwrap();
        EffortFile::parse(&text).unwrap()
    };
    let original = read();
    assert_eq!(
        original.prop("waits_for"),
        Some("owner: which effort starts first?")
    );
    assert!(m.hook_json("pre-command", input(&m, false)).is_none());
    let current = read();
    assert_eq!(current.prop("status"), Some("active"));
    assert_eq!(current.prop("waits_for").unwrap_or_default(), "");
    assert_eq!(current.current, original.current);
    assert_eq!(
        m.log().lines().next(),
        Some("initiative smooth-lines: owner replied")
    );
    // Once: the next command writes nothing.
    let commits = m.log().lines().count();
    assert!(m.hook_json("pre-command", input(&m, false)).is_none());
    assert_eq!(m.log().lines().count(), commits);
}

#[test]
fn an_unknown_notes_identity_does_not_manufacture_a_save_nudge() {
    for dirty in [false, true] {
        let world = World::new();
        let m = world.machine("pc");
        waiting(&m, false);
        assert!(
            m.bash_from(
                "s1",
                &m.notes,
                "astrolabe effort save --status blocked --waits-for \"owner: review the notes?\"",
                None
            )
            .1
        );
        let mut before = session(&m);
        assert!(before["baseline"].is_string());
        before["delivered_ms"] = (Machine::now_ms() - 2_000).into();
        save_session(&m, &before);
        if dirty {
            write(&m.notes.join("knowledge/pending.md"), "Unsaved findings");
        }
        let mut input = input(&m, false);
        input["cwd"] = m.notes.to_str().unwrap().into();
        input["stop_hook_active"] = false.into();
        // Git status and diff have completed, but identity reads no longer fit. This
        // advances the existing test clock, without a sleep or changing real deadlines.
        let mut child = m
            .command(&m.demo, &["hook", "stop", "--if-active"])
            .env("ASTROLABE_HOOK_TEST_STALL", "fingerprint-identity:+3500")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(session(&m)["baseline"], before["baseline"]);
        assert!(!m.effort_text(1).contains("owner: review the notes?"));
    }
}
