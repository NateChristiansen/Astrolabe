//! Writing notes through the hook: `astrolabe effort new | save` as a client runs them, against
//! a notes checkout with a bare origin, from one machine and from two.
mod common;

use astrolabe::activate::STATE_VAR;
use common::TempDir;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// An effort file on today's month shelf, where `effort new` files it.
fn shelf(name: &str) -> String {
    let (day, _) = astrolabe::notes::local_day_time();
    format!("{}/{name}", astrolabe::notes::effort_dir(&day))
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// One machine: its own state folder and notes clone of the shared origin, and a product
/// repo `demo` whose remote the repo note names.
struct Machine {
    dir: TempDir,
    name: &'static str,
    notes: PathBuf,
    demo: PathBuf,
}

struct World {
    origin: TempDir,
}

impl World {
    fn new() -> Self {
        let origin = TempDir::new();
        let bare = origin.0.join("notes.git");
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        let seed = origin.0.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        for (file, text) in [
            ("knowledge/owner.md", "OWNER-TEXT"),
            ("knowledge/voice.md", "VOICE-TEXT"),
            ("knowledge/machines.md", "MACHINE-TEXT"),
            (
                "knowledge/repos/demo.md",
                "---\nremote: git@example.com:team/demo.git\n---\nREPO-DEMO-TEXT",
            ),
            ("efforts/.keep", ""),
        ] {
            write(&seed.join(file), text);
        }
        git(&seed, &["add", "-A"]);
        git(&seed, &["commit", "-q", "-m", "seed"]);
        git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&seed, &["push", "-q", "origin", "main"]);
        Self { origin }
    }

    fn machine(&self, name: &'static str) -> Machine {
        let dir = TempDir::new();
        let notes = dir.0.join("notes");
        git(
            &dir.0,
            &[
                "clone",
                "-q",
                self.origin.0.join("notes.git").to_str().unwrap(),
                "notes",
            ],
        );
        git(&notes, &["config", "user.email", "t@example.com"]);
        git(&notes, &["config", "user.name", "T"]);
        let demo = dir.0.join("demo");
        std::fs::create_dir_all(&demo).unwrap();
        git(&demo, &["init", "-q", "-b", "main"]);
        git(&demo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(
            &demo,
            &["remote", "add", "origin", "git@example.com:team/demo.git"],
        );
        write(&dir.0.join("plugin/hooks/hooks.json"), "hooks v1");
        write(
            &dir.0.join("plugin/.claude-plugin/plugin.json"),
            "manifest v1",
        );
        let m = Machine {
            dir,
            name,
            notes,
            demo,
        };
        let notes_str = m.notes.to_str().unwrap().to_owned();
        let (out, ok) = m.run(&m.notes, &["repo", "add", "agent", &notes_str], "");
        assert!(ok, "{out}");
        assert!(
            m.run(&m.demo, &["on"], "")
                .0
                .contains("on for this machine")
        );
        m
    }
}

impl Machine {
    fn state(&self) -> PathBuf {
        self.dir.0.join("state")
    }

    /// The binary with this machine's environment, for a caller that spawns it itself.
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        self.command_from(Path::new(env!("CARGO_BIN_EXE_astrolabe")), cwd, args)
    }

    /// `command` with the program at `exe`, for a copy a test replaces.
    fn command_from(&self, exe: &Path, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(cwd)
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            .env("ASTROLABE_MACHINE", self.name)
            .env("ASTROLABE_NO_DETACHED_SYNC", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str) -> (String, bool) {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text += &String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            text += &format!("\nexit {}", output.status);
        }
        (text, output.status.success())
    }

    /// A Bash command as a client runs it: through the hook, then what the hook let through.
    fn bash(&self, session: &str, command: &str) -> (String, bool) {
        self.bash_as(session, command, None)
    }

    fn bash_as(&self, session: &str, command: &str, agent: Option<&str>) -> (String, bool) {
        self.bash_from(session, &self.demo, command, agent)
    }

    /// The same from another folder, such as the notes checkout.
    fn bash_from(
        &self,
        session: &str,
        cwd: &Path,
        command: &str,
        agent: Option<&str>,
    ) -> (String, bool) {
        let mut input = json!({"session_id": session, "cwd": cwd, "tool_name": "Bash", "tool_use_id": "call-1", "tool_input": {"command": command}});
        if let Some(agent) = agent {
            input["agent_id"] = agent.into();
        }
        let (out, ok) = self.run(
            cwd,
            &["hook", "pre-command", "--if-active"],
            &input.to_string(),
        );
        assert!(ok, "a hook always exits 0");
        if out.trim().is_empty() {
            if !command.trim_start().starts_with("astrolabe") {
                let out = Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .current_dir(cwd)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_AUTHOR_NAME", "T")
                    .env("GIT_AUTHOR_EMAIL", "t@example.com")
                    .env("GIT_COMMITTER_NAME", "T")
                    .env("GIT_COMMITTER_EMAIL", "t@example.com")
                    .output()
                    .unwrap();
                return (
                    String::from_utf8_lossy(&out.stdout).into_owned(),
                    out.status.success(),
                );
            }
            return self.plain(command);
        }
        let answer: Value = serde_json::from_str(&out).unwrap();
        let hso = &answer["hookSpecificOutput"];
        if hso["permissionDecision"] == "deny" {
            return (
                hso["permissionDecisionReason"].as_str().unwrap().to_owned(),
                false,
            );
        }
        let command = hso["updatedInput"]["command"].as_str().unwrap();
        // The app runs a read of the ticket's text; the ticket's own `--show` says the same
        // and carries whether it worked.
        let path = command
            .strip_prefix("cat '")
            .and_then(|p| p.strip_suffix('\''))
            .unwrap_or_else(|| panic!("{command}"));
        let printed = std::fs::read_to_string(path).unwrap();
        // Whether it worked is in the ticket the plain command looks up; the app sees text.
        let ticket: Value =
            serde_json::from_str(&std::fs::read_to_string(path.replace(".txt", ".json")).unwrap())
                .unwrap();
        assert_eq!(
            ticket["text"].as_str().unwrap().trim_end(),
            printed.trim_end()
        );
        (printed, ticket["ok"] == true)
    }

    /// The command run with no hook: the shell feeds the heredoc body on stdin.
    fn plain(&self, command: &str) -> (String, bool) {
        let (head, body) = match command.split_once("<<'ASTROLABE'\n") {
            Some((h, b)) => (
                h.trim(),
                b.trim_end()
                    .trim_end_matches("ASTROLABE")
                    .trim_end()
                    .to_owned(),
            ),
            None => (command.trim(), String::new()),
        };
        let words: Vec<String> = astrolabe_words(head);
        let args: Vec<&str> = words.iter().skip(1).map(String::as_str).collect();
        self.run(&self.demo, &args, &body)
    }

    fn session_start(&self, session: &str, source: &str) -> String {
        let input = json!({"session_id": session, "cwd": self.demo, "source": source});
        let (out, ok) = self.run(
            &self.demo,
            &["hook", "session-start", "--if-active"],
            &input.to_string(),
        );
        assert!(ok);
        let out: Value = serde_json::from_str(out.trim()).unwrap();
        out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn effort_text(&self, n: u32) -> String {
        let (_, rel) = astrolabe::notes::effort_files(&self.notes)
            .into_iter()
            .find(|(m, _)| *m == n)
            .unwrap_or_else(|| panic!("no effort {n} under {}", self.notes.display()));
        std::fs::read_to_string(self.notes.join(rel)).unwrap()
    }

    fn journal_today(&self) -> String {
        astrolabe::notes::md_files(&self.notes, "journal")
            .into_iter()
            .map(|rel| std::fs::read_to_string(self.notes.join(rel)).unwrap())
            .collect()
    }

    fn log(&self) -> String {
        git(&self.notes, &["log", "--format=%s"])
    }

    fn sync(&self) -> (String, bool) {
        self.run(&self.notes, &["notes", "sync"], "")
    }
}

/// Bare words and double-quoted values, as the shell would hand them to the program.
fn astrolabe_words(head: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut rest = head.trim();
    while !rest.is_empty() {
        if let Some(q) = rest.strip_prefix('"') {
            let end = q.find('"').unwrap();
            words.push(q[..end].to_owned());
            rest = q[end + 1..].trim_start();
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            words.push(rest[..end].to_owned());
            rest = rest[end..].trim_start();
        }
    }
    words
}

const NEW: &str = "astrolabe effort new --title \"Smooth ink\" <<'ASTROLABE'\nsmooth the ink across frames, with café in it\nASTROLABE\n";

impl Machine {
    fn hook_json(&self, event: &str, input: Value) -> Option<Value> {
        let (out, ok) = self.run(
            &self.demo,
            &["hook", event, "--if-active"],
            &input.to_string(),
        );
        assert!(ok, "{out}");
        (!out.trim().is_empty())
            .then(|| serde_json::from_str(out.trim()).unwrap_or_else(|_| panic!("{out}")))
    }

    /// Whether a detached push was asked for since the last look, and forgets it.
    fn sync_requested(&self) -> bool {
        let marker = self.state().join("sync-requested");
        let was = marker.exists();
        let _ = std::fs::remove_file(marker);
        was
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    /// Edits one field of the sync record, keeping the rest.
    fn set_sync(&self, key: &str, value: Value) {
        let path = self.state().join("sync.json");
        let mut record: Value = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));
        record[key] = value;
        std::fs::write(path, record.to_string()).unwrap();
    }

    fn set_last_push(&self, ago_ms: i64) {
        self.set_sync("pushed_ms", (Self::now_ms() - ago_ms).into());
    }

    /// Stamps the last ask into the future, as a clock set back would.
    fn ask_from_the_future(&self) {
        let path = self.state().join("sync-asked");
        std::fs::write(&path, "").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    /// Forgets the last ask, so the next eligible turn end asks again.
    fn forget_ask(&self) {
        let _ = std::fs::remove_file(self.state().join("sync-asked"));
    }

    fn stop(&self, session: &str, active: bool, codex: bool) -> Option<Value> {
        let mut input = json!({"session_id": session, "cwd": self.demo, "stop_hook_active": active, "last_assistant_message": "done"});
        if codex {
            input["turn_id"] = "t1".into();
        }
        self.hook_json("stop", input)
    }

    fn edit(&self, session: &str, tool: &str) {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": tool, "tool_input": {"file_path": "x"}, "tool_response": {}});
        assert!(self.hook_json("post-tool", input).is_none());
    }
}

/// One column of the board page, from its heading to the next column.
fn column<'a>(page: &'a str, key: &str) -> &'a str {
    let start = page
        .find(&format!("class=\"col {key}\""))
        .unwrap_or_else(|| panic!("no column {key} in the board"));
    let end = page[start + 1..]
        .find("<section")
        .map_or(page.len(), |i| start + 1 + i);
    &page[start..end]
}

/// A child killed and reaped however the test ends.
struct Kill(std::process::Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[path = "record/home.rs"]
mod home;
#[path = "record/initiatives.rs"]
mod initiatives;
#[path = "record/lessons.rs"]
mod lessons;
#[path = "record/recovery.rs"]
mod recovery;
#[path = "record/saving.rs"]
mod saving;
#[path = "record/sessions.rs"]
mod sessions;
#[path = "record/synchronization.rs"]
mod synchronization;
