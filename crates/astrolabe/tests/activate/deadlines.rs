//! Each hook event answers inside its own time, whatever holds it up.
use super::*;
use std::time::Duration;

const OUT_OF_TIME: &str = "Astrolabe's hook ran out of time before it finished with this command";

impl Fixture {
    /// One hook event run with extra environment: what it printed and how long it took. A
    /// hook still running at `limit` fails the test.
    fn timed(
        &self,
        event: &str,
        input: &Value,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> (Option<Value>, Duration) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(["hook", event, "--if-active"])
            .current_dir(&self.demo)
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            .env("ASTROLABE_NO_DETACHED_SYNC", "1")
            .env_remove(EFFORT_VAR)
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
        assert!(output.stderr.is_empty(), "a hook says nothing on stderr");
        let out = String::from_utf8(output.stdout).unwrap();
        assert!(out.lines().count() <= 1, "one answer at most: {out}");
        let answer =
            (!out.trim().is_empty()).then(|| serde_json::from_str(out.trim()).expect("JSON"));
        (answer, took)
    }
}

fn shell(f: &Fixture, command: &str) -> Value {
    json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": command}})
}

#[test]
fn time_running_out_before_a_verdict_is_a_denial() {
    let f = Fixture::new();
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "1500"),
        ("ASTROLABE_HOOK_TEST_STALL", "read:20000"),
    ];
    let limit = Duration::from_secs(8);
    let (answer, took) = f.timed("pre-command", &shell(&f, "echo hi"), &env, limit);
    let answer = answer.expect("a denial, not silence");
    assert!(reason(&answer).starts_with(OUT_OF_TIME), "{answer}");
    assert!(
        took >= Duration::from_millis(1400) && took < Duration::from_secs(5),
        "{took:?}"
    );
    // It judged nothing, so it proves nothing for any session.
    assert!(!f.state().join("guard-denials.log").exists());
}

#[test]
fn time_running_out_after_a_verdict_keeps_the_verdict() {
    let f = Fixture::new();
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "1500"),
        ("ASTROLABE_HOOK_TEST_STALL", "verdict:20000"),
    ];
    let limit = Duration::from_secs(8);
    let (answer, took) = f.timed("pre-command", &shell(&f, "echo hi"), &env, limit);
    assert_eq!(answer, None, "an allowed command still runs");
    assert!(
        took >= Duration::from_millis(1400) && took < Duration::from_secs(5),
        "{took:?}"
    );
    let (answer, took) = f.timed("pre-command", &shell(&f, "git push --force"), &env, limit);
    let answer = answer.expect("a denied command stays denied");
    assert!(
        reason(&answer).starts_with("Astrolabe denied force-push"),
        "{answer}"
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
}

#[test]
fn input_trickling_past_the_events_read_time_is_blocked_as_unreadable() {
    let f = Fixture::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
        .args(["hook", "pre-command", "--if-active"])
        .current_dir(&f.demo)
        .env(STATE_VAR, f.state())
        .env("ASTROLABE_HOOK_BUDGET_MS", "2000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    // A byte at a time, each well inside the idle wait, for longer than the hook's whole time.
    let writer = std::thread::spawn(move || {
        for byte in
            br#"{"session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "echo hi"}}"#
        {
            if pipe.write_all(&[*byte]).and_then(|_| pipe.flush()).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    let started = std::time::Instant::now();
    let output = child.wait_with_output().unwrap();
    let took = started.elapsed();
    let answer: Value = serde_json::from_slice(&output.stdout).expect("a denial, not silence");
    assert!(
        reason(&answer)
            .starts_with("Astrolabe couldn't read what the app sent its hook (not a JSON object)"),
        "{answer}"
    );
    assert!(took < Duration::from_secs(2), "{took:?}");
    let _ = writer.join();
}

#[test]
fn a_held_approval_lock_denies_inside_the_budget() {
    use fs4::fs_std::FileExt;
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    let command = "git push --force";
    let mint = "astrolabe allow --once 'git push --force' --said 'Yes, run that exact command.'";
    let (text, ok) = f.bash("s1", &f.demo, mint);
    assert!(ok, "{text}");
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(f.state().join("s1.allow.lock"))
        .unwrap();
    FileExt::lock_exclusive(&lock).unwrap();
    // The lock wait ends a little past half the hook's time; the rest is room for a slow
    // machine to start the program and its git calls.
    let env = [("ASTROLABE_HOOK_BUDGET_MS", "8000")];
    let limit = Duration::from_secs(25);
    let (answer, took) = f.timed("pre-command", &shell(&f, command), &env, limit);
    let answer = answer.expect("a denial while the approval can't be read");
    assert!(
        reason(&answer).starts_with("Astrolabe denied force-push"),
        "{answer}"
    );
    assert!(took < Duration::from_secs(8), "{took:?}");
    // Recording one waits the same bounded time and says why it gave up.
    let (answer, took) = f.timed("pre-command", &shell(&f, mint), &env, limit);
    let shown = answer.unwrap()["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = shown
        .strip_prefix("cat '")
        .and_then(|p| p.strip_suffix('\''));
    let text = std::fs::read_to_string(path.expect("a read of the ticket")).unwrap();
    assert!(
        text.starts_with("Approval refused: this session's approval file was busy for the "),
        "{text}"
    );
    assert!(took < Duration::from_secs(8), "{took:?}");
    drop(lock);
    // The approval was never spent: with the lock free it lets the command through once.
    assert_eq!(f.hook(shell(&f, command)), None);
    assert!(f.denies("s1", command));
}

#[test]
fn a_slow_stop_hook_exits_silently_inside_its_budget() {
    let f = Fixture::new();
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "1000"),
        ("ASTROLABE_HOOK_TEST_STALL", "read:20000"),
    ];
    let input = json!({"session_id": "s1", "cwd": f.demo, "stop_hook_active": false});
    let (answer, took) = f.timed("stop", &input, &env, Duration::from_secs(8));
    assert_eq!(answer, None);
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(4),
        "{took:?}"
    );
}

#[test]
fn a_slow_session_start_says_it_ran_out_of_time() {
    let f = Fixture::new();
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "1000"),
        ("ASTROLABE_HOOK_TEST_STALL", "read:20000"),
    ];
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "startup"});
    let (answer, took) = f.timed("session-start", &input, &env, Duration::from_secs(8));
    let answer = answer.expect("a line of context, not silence");
    assert_eq!(
        answer["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let text = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("Astrolabe's session-start hook ran out of time"),
        "{text}"
    );
    assert!(took < Duration::from_secs(4), "{took:?}");
}

#[test]
fn a_machine_thats_off_gets_no_clock_and_no_answer() {
    let f = Fixture::new();
    f.machine("off");
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "300"),
        ("ASTROLABE_HOOK_TEST_STALL", "read:20000"),
    ];
    let limit = Duration::from_secs(8);
    let (answer, took) = f.timed("pre-command", &shell(&f, "echo hi"), &env, limit);
    assert_eq!(answer, None);
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn a_session_start_that_runs_out_of_time_blocks_nothing() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    // Held up in its first look at a repo until its time for reading repos is gone: the
    // looks after it aren't started, which reads like a folder that isn't a repo.
    let env = [
        ("ASTROLABE_HOOK_BUDGET_MS", "2000"),
        ("ASTROLABE_HOOK_TEST_STALL", "rev-parse:1800"),
    ];
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "resume"});
    let (answer, took) = f.timed("session-start", &input, &env, Duration::from_secs(10));
    let answer = answer.expect("a line of context, not silence");
    let text = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.starts_with("Astrolabe's session-start hook ran out of time"),
        "{text}"
    );
    assert!(took < Duration::from_secs(5), "{took:?}");
    // Being slow once is no reason to block the session's commands until someone repairs it.
    let state = std::fs::read_to_string(f.state().join("s1.json")).unwrap();
    assert!(!state.contains("reload_failed"), "{state}");
    assert_eq!(f.hook(shell(&f, "echo hi")), None);
}
