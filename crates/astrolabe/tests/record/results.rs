//! A finished command's result: kept for the session that asked for it however late it
//! reads, and a retry of the same command is that request again, not a second write.
use super::*;
use std::time::{Duration, SystemTime};

impl Machine {
    /// A Bash command through the pre-command hook, and the file its rewritten command
    /// reads: what the app would print, whenever it gets to it.
    fn result_file(&self, session: &str, call: &str, command: &str) -> PathBuf {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let answer = self.hook_json("pre-command", input).unwrap();
        let shown = answer["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{answer}"));
        let path = (shown.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        PathBuf::from(path)
    }
}

/// A result's files, as if the hook had left them `ago` earlier.
fn age(text: &Path, ago: Duration) {
    for path in [text.to_path_buf(), text.with_extension("json")] {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - ago).unwrap();
    }
}

#[test]
fn a_result_read_after_a_minute_and_another_sessions_write_is_still_there() {
    let world = World::new();
    let mac = world.machine("mac");
    // The hook wrote the effort; the app hasn't run the read yet, say behind a prompt.
    let slow = mac.result_file("s1", "call-1", NEW);
    age(&slow, Duration::from_secs(5 * 60));
    // Another session's command leaves its own result meanwhile.
    let other = "astrolabe effort new --title \"Other\" <<'ASTROLABE'\nanother goal\nASTROLABE\n";
    assert!(mac.bash("s2", other).1);
    let late = std::fs::read_to_string(&slow)
        .unwrap_or_else(|why| panic!("the first session's result was removed: {why}"));
    assert!(late.starts_with("Effort 1 created"), "{late}");
    // Only age removes one: past its week it goes when the next is written.
    age(&slow, Duration::from_secs(8 * 24 * 3600));
    assert!(
        mac.bash(
            "s2",
            "astrolabe effort save --decision \"chose a over b: c\""
        )
        .1
    );
    assert!(!slow.exists());
}

const OUT_OF_TIME: &str = "Astrolabe's hook ran out of time before it finished with this command";
/// Held up between writing the files and committing them until the hook's time is up: it
/// answers at 8 s and is gone soon after, with the write left planned.
const HELD_UP: [(&str, &str); 2] = [
    ("ASTROLABE_HOOK_TEST_STALL", "commit:60000"),
    ("ASTROLABE_HOOK_BUDGET_MS", "8000"),
];

impl Machine {
    /// A Bash command through the pre-command hook as its own tool call: the denial's
    /// reason, or the result the rewritten command reads.
    fn call(
        &self,
        session: &str,
        call: &str,
        command: &str,
        env: &[(&str, &str)],
    ) -> Result<String, String> {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_use_id": call, "tool_input": {"command": command}});
        let mut hook = self.command(&self.demo, &["hook", "pre-command", "--if-active"]);
        hook.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            hook.env(key, value);
        }
        let child = hook.spawn().unwrap();
        let limit = Duration::from_secs(60);
        let (output, _) = common::within(child, input.to_string().into_bytes(), limit)
            .expect("the hook answers inside its time");
        let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
        let inner = &answer["hookSpecificOutput"];
        if inner["permissionDecision"] == "deny" {
            return Err(inner["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .to_owned());
        }
        let shown = inner["updatedInput"]["command"].as_str().unwrap();
        let path = (shown.strip_prefix("cat '"))
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{shown}"));
        Ok(std::fs::read_to_string(path).unwrap())
    }

    /// The recovery records of finished writes, as if they had finished `ago` earlier.
    fn age_finished_writes(&self, ago: Duration) {
        let folders = ["ops", "answered"].map(|name| std::fs::read_dir(self.state().join(name)));
        for entry in folders.into_iter().flatten().flatten().flatten() {
            let file = std::fs::File::options()
                .write(true)
                .open(entry.path())
                .unwrap();
            file.set_modified(SystemTime::now() - ago).unwrap();
        }
    }
}

const FOUND: &str = "astrolabe journal \"Found the Deck preset in the video settings.\"";

/// The hook ran out of time while committing, so the session was denied and ran the same
/// command again: a new tool call. Recovery finishes the first write, and the retry is that
/// request, not a second line.
#[test]
fn a_journal_line_retried_after_the_out_of_time_denial_is_written_once() {
    let world = World::new();
    let mac = world.machine("mac");
    let denied = mac.call("s1", "call-1", FOUND, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    let retried = mac.call("s1", "call-2", FOUND, &[]).unwrap();
    assert!(
        retried.contains("nothing new went in the journal"),
        "{retried}"
    );
    let journal = mac.journal_today();
    assert_eq!(
        journal.matches("Found the Deck preset").count(),
        1,
        "{journal}"
    );
    assert_eq!(
        mac.log().matches("journal: demo").count(),
        1,
        "{}",
        mac.log()
    );
}

#[test]
fn a_new_effort_retried_after_the_out_of_time_denial_is_created_once() {
    let world = World::new();
    let mac = world.machine("mac");
    let twice = "astrolabe effort new --title \"Twice\" <<'ASTROLABE'\none goal\nASTROLABE\n";
    let denied = mac.call("s1", "call-1", twice, &HELD_UP).unwrap_err();
    assert!(denied.starts_with(OUT_OF_TIME), "{denied}");
    let retried = mac.call("s1", "call-2", twice, &[]).unwrap();
    assert!(
        retried.starts_with("Effort 1 was already created")
            && retried.contains("this session is bound to it"),
        "{retried}"
    );
    let efforts = astrolabe::notes::effort_files(&mac.notes);
    assert_eq!(efforts.len(), 1, "{efforts:?}");
    assert!(mac.notes.join(shelf("0001-twice.md")).exists());
    // The session works on from the one effort.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose a over b: c\"",
    );
    assert!(ok && out.starts_with("Saved effort 1"), "{out}");
}

/// The first attempt finished and its result was left, but the session ran the command
/// again anyway: it gets that result, and nothing is written twice.
#[test]
fn a_command_retried_after_it_committed_gets_its_first_result_and_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    let first = mac.call("s1", "call-1", FOUND, &[]).unwrap();
    assert!(first.starts_with("Journal line written for"), "{first}");
    assert_eq!(mac.call("s1", "call-2", FOUND, &[]).unwrap(), first);
    let lines = || mac.journal_today().matches("Found the Deck preset").count();
    assert_eq!(lines(), 1);
    // Another session saying the same is its own request, and so is this one in another
    // folder or after the first attempt is no longer recent.
    assert!(mac.call("s2", "call-1", FOUND, &[]).is_ok());
    assert_eq!(lines(), 2);
    mac.age_finished_writes(Duration::from_secs(2 * 60));
    assert!(mac.call("s1", "call-3", FOUND, &[]).is_ok());
    assert_eq!(lines(), 3);
    // A save too: the same decision isn't recorded twice.
    assert!(mac.bash("s3", NEW).1);
    let chose = "astrolabe effort save --decision \"chose a over b: c\"";
    let saved = mac.call("s3", "call-2", chose, &[]).unwrap();
    assert!(saved.starts_with("Saved effort 1"), "{saved}");
    assert_eq!(mac.call("s3", "call-3", chose, &[]).unwrap(), saved);
    assert_eq!(mac.effort_text(1).matches("chose a over b: c").count(), 1);
    // Once the effort has been saved again, the same words are a new request.
    let other = "astrolabe effort save --decision \"chose d over e: f\"";
    assert!(mac.call("s3", "call-4", other, &[]).is_ok());
    assert!(mac.call("s3", "call-5", chose, &[]).is_ok());
    assert_eq!(mac.effort_text(1).matches("chose a over b: c").count(), 2);
}
