//! Notes writes through a hook finish, or are refused with a reply, inside the hook's time.
use super::*;
use std::time::Duration;

const SAVE: &str = "astrolabe effort save --journal \"a line for the day\"";

/// What one hook run came to.
enum Answer {
    /// Nothing printed.
    Silent,
    Denied(String),
    /// The text of the ticket the rewritten command reads, and whether it worked.
    Shown(String, bool),
}

impl Machine {
    /// A Bash command through the pre-command hook with extra environment: its answer and
    /// how long the hook took. A hook still running at `limit` fails the test.
    fn timed(
        &self,
        session: &str,
        call: &str,
        command: &str,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (Answer, Duration) {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let (out, took) = self.timed_event("pre-command", &input, env, limit);
        if out.trim().is_empty() {
            return (Answer::Silent, took);
        }
        let answer: Value = serde_json::from_str(out.trim()).unwrap();
        let inner = &answer["hookSpecificOutput"];
        if inner["permissionDecision"] == "deny" {
            let why = inner["permissionDecisionReason"].as_str().unwrap();
            return (Answer::Denied(why.to_owned()), took);
        }
        let shown = inner["updatedInput"]["command"].as_str().unwrap();
        let path = (shown.strip_prefix("cat '"))
            .or_else(|| shown.strip_prefix("Get-Content -Raw -Encoding UTF8 -LiteralPath '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        let ticket: Value =
            serde_json::from_str(&std::fs::read_to_string(path.replace(".txt", ".json")).unwrap())
                .unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        (Answer::Shown(text, ticket["ok"] == true), took)
    }

    fn timed_event(
        &self,
        event: &str,
        input: &Value,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (String, Duration) {
        let mut command = self.command(&self.demo, &["hook", event, "--if-active"]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        let (output, took) = common::within(child, input.to_string().into_bytes(), limit)
            .unwrap_or_else(|| panic!("the {event} hook was still running after {limit:?}"));
        assert!(output.status.success(), "a hook always exits 0");
        (String::from_utf8(output.stdout).unwrap(), took)
    }

    fn unfinished(&self) -> Vec<Value> {
        std::fs::read_dir(self.state().join("ops"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|x| x == "json"))
            .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
            .collect()
    }

    /// A folder to put first on PATH, holding a `git` that runs `script` and then the real
    /// one. The script sees git's arguments as `$@`.
    #[cfg(unix)]
    fn git_shim(&self, script: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let real = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        let real = String::from_utf8(real.stdout).unwrap();
        let bin = self.dir.0.join("shim");
        let shim = bin.join("git");
        write(
            &shim,
            &format!("#!/bin/sh\n{script}\nexec '{}' \"$@\"\n", real.trim()),
        );
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
    }
}

#[test]
fn a_held_notes_lock_refuses_a_save_with_a_reply() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (before, log) = (mac.effort_text(1), mac.log());
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let held = notes.lock().unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "4000")];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(12));
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.starts_with("Effort NOT saved: the notes are busy: another session's write or a sync held them for the "),
        "{text}"
    );
    assert!(
        text.contains("so nothing was written. Run the same command again"),
        "{text}"
    );
    assert!(took < Duration::from_secs(4), "{took:?}");
    assert_eq!(mac.effort_text(1), before);
    assert_eq!(mac.log(), log);
    assert!(mac.unfinished().is_empty(), "nothing was planned or left");
    drop(held);
    let (out, ok) = mac.bash("s1", SAVE);
    assert!(ok, "{out}");
    assert!(mac.journal_today().contains("a line for the day"));
}

#[test]
fn a_held_notes_lock_at_session_start_still_delivers_context() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let notes = astrolabe::notes::Notes::new(mac.notes.clone(), mac.state());
    let _held = notes.lock().unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "4000")];
    let input = json!({"session_id": "s1", "cwd": mac.demo, "source": "resume"});
    let (out, took) = mac.timed_event("session-start", &input, &env, Duration::from_secs(12));
    let answer: Value = serde_json::from_str(out.trim()).expect("context, not silence");
    let text = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("Smooth ink") && text.contains("Notes not refreshed from origin (no lock)"),
        "{text}"
    );
    assert!(took < Duration::from_secs(4), "{took:?}");
}

#[cfg(unix)]
#[test]
fn a_hanging_git_refuses_a_save_inside_the_budget() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let path = mac.git_shim("for a in \"$@\"; do [ \"$a\" = commit ] && exec sleep 60; done");
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "6000"),
        ("PATH", path.as_str()),
    ];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(20));
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.starts_with("Effort NOT saved: git commit was stopped after ")
            && text.contains(" s, when this hook's time ran out"),
        "{text}"
    );
    assert!(took < Duration::from_secs(6), "{took:?}");
    // The write is planned and unfinished: the next hook that writes finishes it.
    let left = mac.unfinished();
    assert!(left.len() == 1 && left[0]["committed"] != true, "{left:?}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"the next line\"");
    assert!(ok, "{out}");
    // Both landed, each once: the day keeps an effort's latest line, history has both.
    let history = git(&mac.notes, &["log", "-p", "--format=%s"]);
    assert_eq!(
        history.matches("+- a line for the day").count(),
        1,
        "{history}"
    );
    assert!(mac.journal_today().contains("the next line"));
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
    assert!(mac.unfinished().iter().all(|op| op["result"].is_string()));
}

#[cfg(unix)]
#[test]
fn a_commit_stopped_at_its_limit_leaves_no_index_lock_and_no_children() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The real git, holding its index lock while a commit hook of the checkout's hangs.
    let hook = mac.notes.join(".git/hooks/pre-commit");
    let pid = mac.dir.0.join("hook.pid");
    write(
        &hook,
        &format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n", pid.display()),
    );
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "6000")];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(20));
    let Answer::Shown(text, ok) = answer else {
        panic!("a reply through the ticket, not a denial or silence");
    };
    assert!(
        !ok && text.starts_with("Effort NOT saved: git commit was stopped after "),
        "{text}"
    );
    assert!(took < Duration::from_secs(6), "{took:?}");
    // Git was asked to stop, so it took its lock files with it; killed outright it leaves
    // them, and every later save fails until someone removes them by hand.
    let locks: Vec<_> = std::fs::read_dir(mac.notes.join(".git"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".lock"))
        .collect();
    assert!(locks.is_empty(), "{locks:?}");
    // And what git started went with it, so nothing touches the index after the lock.
    let pid = std::fs::read_to_string(&pid).unwrap();
    let alive = Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !alive.success(),
        "the commit hook's process is still running"
    );
    std::fs::remove_file(&hook).unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"the next line\"");
    assert!(ok, "{out}");
    let history = git(&mac.notes, &["log", "-p", "--format=%s"]);
    assert_eq!(
        history.matches("+- a line for the day").count(),
        1,
        "{history}"
    );
}

#[test]
fn a_hook_that_dies_mid_write_leaves_a_write_the_next_hook_finishes() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // Held up between writing the files and committing them, past the hook's whole time.
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "10000"),
        ("ASTROLABE_HOOK_TEST_STALL", "commit:60000"),
    ];
    let (answer, took) = mac.timed("s1", "call-2", SAVE, &env, Duration::from_secs(30));
    let Answer::Denied(why) = answer else {
        panic!("the backstop's denial");
    };
    assert!(
        why.starts_with("Astrolabe's hook ran out of time before it finished with this command")
            && why.contains("run `astrolabe notes status` to see whether its write landed"),
        "{why}"
    );
    assert!(
        took >= Duration::from_millis(9900) && took < Duration::from_secs(14),
        "{took:?}"
    );
    let left = mac.unfinished();
    assert!(left.len() == 1 && left[0]["committed"] != true, "{left:?}");
    assert_ne!(git(&mac.notes, &["status", "--porcelain"]), "");
    // The lock died with the process, and the next hook's recovery finishes that write.
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(
        status.contains("1 notes write(s) left unfinished"),
        "{status}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"the next line\"");
    assert!(ok, "{out}");
    // Both landed, each once: the day keeps an effort's latest line, history has both.
    let history = git(&mac.notes, &["log", "-p", "--format=%s"]);
    assert_eq!(
        history.matches("+- a line for the day").count(),
        1,
        "{history}"
    );
    assert!(mac.journal_today().contains("the next line"));
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "");
    assert!(mac.unfinished().iter().all(|op| op["result"].is_string()));
    assert_eq!(
        mac.log().matches("0001: checkpoint").count(),
        2,
        "{}",
        mac.log()
    );
}

#[cfg(unix)]
#[test]
fn a_flooding_fetch_is_not_a_timeout() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // More than a pipe holds, on the stream git reports progress on.
    let path = mac.git_shim(
        "for a in \"$@\"; do [ \"$a\" = fetch ] && head -c 1048576 /dev/zero | tr '\\0' x >&2; done",
    );
    let mut command = mac.command(&mac.notes, &["notes", "sync"]);
    command
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (output, took) = common::within(
        command.spawn().unwrap(),
        Vec::new(),
        Duration::from_secs(40),
    )
    .expect("the sync ends");
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && out.contains("pushed"), "{out}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    let record = std::fs::read_to_string(mac.state().join("sync.json")).unwrap();
    assert!(!record.contains("backoff_until_ms"), "{record}");
}
