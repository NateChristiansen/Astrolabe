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
