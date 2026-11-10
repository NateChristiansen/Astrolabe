//! The opt-in plugin's contract, run as the real binary: hook in, command out, as a client
//! would run them. Notes and repos are fixtures in a temp folder.
mod common;

use astrolabe::activate::{EFFORT_VAR, STATE_VAR};
use common::TempDir;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EFFORT: &str = "---\ntitle: 0070 Demo\nstatus: active\ninitiative: \"[[testing]]\"\nrepo: demo\ninvolvement: check in at plan\nauthority: edit\nnext: NEXT-70.\n---\n\n# 0070 Demo\n\nrepo: other · authority: push\n\n## Current state\n\nEFFORT-70 goal.\n\n**Next step:** NEXT-70.\n\n## History\n\nNOT-DELIVERED\n";

/// A name and email for git, apart from the machine's own config.
const IDENT: &[(&str, &str)] = &[
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "T"),
    ("GIT_AUTHOR_EMAIL", "t@example.com"),
    ("GIT_COMMITTER_NAME", "T"),
    ("GIT_COMMITTER_EMAIL", "t@example.com"),
];

struct Fixture {
    dir: TempDir,
    demo: PathBuf,
    other: PathBuf,
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The arguments of a command the hook rewrote, which calls this program by its full path,
/// with PowerShell's call operator in front where the command runs in PowerShell.
fn rewritten(command: &str) -> Vec<&str> {
    // The read the app runs: its ticket is the file's stem, shown the same way.
    if let Some(ticket) = read_ticket(command) {
        return vec!["activate", "--show", ticket];
    }
    let exe = format!("'{}' ", env!("CARGO_BIN_EXE_astrolabe"));
    let command = command.strip_prefix("& ").unwrap_or(command);
    let rest = command.strip_prefix(&exe);
    let rest = rest.unwrap_or_else(|| panic!("only the switch reaches here: {command}"));
    rest.split_whitespace().collect()
}

/// The ticket a rewritten read names: `cat '<state>/shown/<ticket>.txt'`, or PowerShell's.
fn read_ticket(command: &str) -> Option<&str> {
    let path = command
        .strip_prefix("cat '")
        .or_else(|| command.strip_prefix("Get-Content -Raw -Encoding UTF8 -LiteralPath '"))?;
    let path = path.strip_suffix('\'')?;
    Path::new(path).file_stem()?.to_str()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new();
        let notes = dir.0.join("notes");
        for (file, text) in [
            ("knowledge/owner.md", "OWNER-TEXT"),
            (
                "knowledge/voice.md",
                "VOICE-TEXT\n\n## Rules\n\n- VOICE-RULE\n\n## Chat\n\nVOICE-SAMPLE",
            ),
            ("knowledge/machines.md", "MACHINE-TEXT"),
            ("knowledge/repos/demo.md", "REPO-DEMO-TEXT"),
            ("knowledge/repos/other.md", "REPO-OTHER-TEXT"),
            ("efforts/0070-demo.md", EFFORT),
            (
                "efforts/0071-nofield.md",
                "---\nrepo: demo\ninvolvement: autonomous\n---\n# 0071\n\n## Current state\n\nx\n",
            ),
            (
                "efforts/0072-other.md",
                "---\nrepo: [other]\ninvolvement: each step\nauthority: read\n---\n# 0072\n\n## Current state\n\nEFFORT-72\n",
            ),
            (
                "efforts/0075-also.md",
                "---\nrepo:\n  - demo\ninvolvement: \x27autonomous\x27\nauthority: read\n---\n# 0075\n\n## Current state\n\nEFFORT-75\n",
            ),
            (
                "efforts/0074-bad.md",
                "---\nrepo: demo\ninvolvement: whenever\nauthority: edit\n---\n# 0074\n\n## Current state\n\nx\n",
            ),
        ] {
            write(&notes.join(file), text);
        }
        let (demo, other) = (dir.0.join("demo"), dir.0.join("other"));
        for repo in [&notes, &demo, &other] {
            std::fs::create_dir_all(repo).unwrap();
            git(repo, &["init", "-q"]);
            git(repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        }
        write(&dir.0.join("plugin/hooks/hooks.json"), "hooks v1");
        write(
            &dir.0.join("plugin/.claude-plugin/plugin.json"),
            "manifest v1",
        );
        let f = Self { dir, demo, other };
        for (name, checkout) in [("agent", &notes), ("demo", &f.demo), ("other", &f.other)] {
            let checkout = checkout.to_str().unwrap();
            let (out, ok) = f.run(&notes, &["repo", "add", name, checkout], "", &[]);
            assert!(ok, "{out}");
        }
        assert!(f.machine("on").contains("on for this machine"));
        f
    }

    /// A machine with no notes yet: `notes init` makes them, as a new owner's setup does.
    fn seeded() -> (Self, (String, bool)) {
        let dir = TempDir::new();
        let (demo, other) = (dir.0.join("demo"), dir.0.join("other"));
        for repo in [&demo, &other] {
            std::fs::create_dir_all(repo).unwrap();
            git(repo, &["init", "-q"]);
            git(repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        }
        write(&dir.0.join("plugin/hooks/hooks.json"), "hooks v1");
        let f = Self { dir, demo, other };
        let notes = f.dir.0.join("notes");
        let made = f.run(
            &f.demo,
            &["notes", "init", notes.to_str().unwrap()],
            "",
            IDENT,
        );
        (f, made)
    }

    fn machine(&self, verb: &str) -> String {
        self.run(&self.demo, &[verb], "", &[]).0
    }

    fn state(&self) -> PathBuf {
        self.dir.0.join("state")
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> (String, bool) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(args)
            .current_dir(cwd)
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            // No detached home or sync: a session-start hook here would otherwise leave a
            // real server on the owner's default port for a day, and the owner's own home
            // couldn't start (seen 2026-11-06).
            .env("ASTROLABE_NO_DETACHED_SYNC", "1")
            .env_remove(EFFORT_VAR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.stderr.is_empty() {
            eprintln!(
                "astrolabe {args:?}, status {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if !output.status.success() {
            text += &format!(
                "\nexit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        (text, output.status.success())
    }

    /// The pre-command hook's answer, or None when it stays silent.
    fn hook(&self, input: Value) -> Option<Value> {
        let (out, ok) = self.run(
            &self.demo,
            &["hook", "pre-command", "--if-active"],
            &input.to_string(),
            &[],
        );
        assert!(ok, "a hook always exits 0: {out}");
        (!out.trim().is_empty()).then(|| {
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("hook JSON: {e}; output: {out}"))
        })
    }

    fn session_start(&self, session: &str, cwd: &Path, env: &[(&str, &str)]) -> Option<String> {
        let input = json!({"session_id": session, "cwd": cwd, "source": "startup"});
        let (out, ok) = self.run(
            cwd,
            &["hook", "session-start", "--if-active"],
            &input.to_string(),
            env,
        );
        assert!(ok);
        let out: Value = serde_json::from_str(out.trim()).ok()?;
        Some(
            out["hookSpecificOutput"]["additionalContext"]
                .as_str()?
                .to_owned(),
        )
    }

    /// A Bash command as a client runs it: through the hook, then what the hook let through.
    fn bash(&self, session: &str, cwd: &Path, command: &str) -> (String, bool) {
        let input = json!({"session_id": session, "cwd": cwd, "tool_name": "Bash", "tool_input": {"command": command, "description": "d"}});
        let answer = self.hook(input);
        let command = match &answer {
            Some(a) if a["hookSpecificOutput"]["permissionDecision"] == "deny" => {
                return (
                    a["hookSpecificOutput"]["permissionDecisionReason"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    false,
                );
            }
            Some(a) => a["hookSpecificOutput"]["updatedInput"]["command"]
                .as_str()
                .unwrap()
                .to_owned(),
            None => command.to_owned(),
        };
        if let Some(a) = &answer {
            assert_eq!(
                a["hookSpecificOutput"]["updatedInput"]["description"], "d",
                "keeps the rest of the input"
            );
        }
        let words = rewritten(&command);
        self.run(cwd, &words, "", &[])
    }

    fn denies(&self, session: &str, command: &str) -> bool {
        let input = json!({"session_id": session, "cwd": self.demo, "tool_name": "Bash", "tool_input": {"command": command}});
        self.hook(input)
            .is_some_and(|a| a["hookSpecificOutput"]["permissionDecision"] == "deny")
    }

    fn probe(&self, text: &str) -> String {
        let line = text
            .lines()
            .find(|l| l.contains("astrolabe-guard-probe") && l.contains("deny: "))
            .unwrap();
        line.split_once("deny: ").unwrap().1.to_owned()
    }

    /// The allowed control, as the session's shell would run it: it makes its marker.
    fn control(&self, text: &str) {
        let line = text
            .lines()
            .find(|l| l.contains("which must run: "))
            .unwrap();
        let command = line.split_once("which must run: ").unwrap().1;
        let path = command.split('\'').nth(1).unwrap();
        assert!(Path::new(path).is_absolute(), "{path}");
        std::fs::write(path, "").unwrap();
    }
}

const GAME: &str = "---\nstatus: active\nrepo: game\ninvolvement: autonomous\nauthority: read\n---\n# 0080 Game\n\n## Current state\n\nEFFORT-80\n";

impl Fixture {
    /// A repo no registry names, with `remote` as its origin, and an active effort for `game`.
    fn remote_repo(&self, folder: &str, remote: &str) -> PathBuf {
        let repo = self.dir.0.join(folder);
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        git(&repo, &["remote", "add", "origin", remote]);
        write(&self.notes().join("efforts/0080-game.md"), GAME);
        repo
    }

    fn notes(&self) -> PathBuf {
        self.dir.0.join("notes")
    }

    fn game_note(&self, text: &str) {
        write(&self.notes().join("knowledge/repos/game.md"), text);
    }
}

impl Fixture {
    /// The pre-command hook's answer to raw input, written aside so a hook that stops
    /// reading early doesn't fail the write.
    fn hook_raw(&self, stdin: Vec<u8>) -> Option<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_astrolabe"))
            .args(["hook", "pre-command", "--if-active"])
            .current_dir(&self.demo)
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            .env_remove(EFFORT_VAR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pipe = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = pipe.write_all(&stdin);
        });
        let output = child.wait_with_output().unwrap();
        let _ = writer.join();
        assert!(
            output.status.success(),
            "hook exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let out = String::from_utf8_lossy(&output.stdout);
        (!out.trim().is_empty()).then(|| {
            serde_json::from_str(out.trim()).unwrap_or_else(|e| {
                panic!(
                    "hook JSON: {e}; status {}; stdout: {out}; stderr: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                )
            })
        })
    }
}

fn reason(answer: &Value) -> &str {
    assert_eq!(
        answer["hookSpecificOutput"]["permissionDecision"], "deny",
        "{answer}"
    );
    answer["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
}

#[test]
fn every_authority_keeps_notes_saving_independent_of_project_pushes() {
    // The matrix of roles and authorities is in the unit tests; this is the real load and resume.
    let f = Fixture::new();
    write(
        &f.notes().join("efforts/0070-demo.md"),
        &EFFORT.replace("authority: edit", "authority: read"),
    );
    let (loaded, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{loaded}");
    let resumed = f.session_start("s1", &f.demo, &[]).unwrap();
    for text in [&loaded, &resumed] {
        assert!(
            text.contains("never ask for approval to save notes"),
            "{text}"
        );
        assert!(text.contains("authority: read:"), "{text}");
    }
    for text in [
        "`astrolabe initiative new",
        "`astrolabe activate --initiative <slug>`",
    ] {
        assert!(loaded.contains(text), "{text}\n{loaded}");
    }
}

#[test]
fn activation_delivers_pieces_and_the_linked_initiative_title_beside_current_state() {
    let f = Fixture::new();
    write(
        &f.notes().join("initiatives/testing.md"),
        "---\nrepo: demo\nstatus: active\n---\n# Testing together\n\n## Current state\n\nINITIATIVE-STATE-NOT-DELIVERED\n",
    );
    for membership in ["testing", "Testing together", "[[testing]]"] {
        let effort = EFFORT.replace("[[testing]]", membership)
            + "\n## Pieces\n\n- [x] Build the example\n- [ ] Land the example\n\n## Plan\n\nPLAN-NOT-DELIVERED\n";
        write(&f.notes().join("efforts/0070-demo.md"), &effort);
        let (loaded, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
        assert!(ok, "{loaded}");
        for text in [
            "EFFORT-70 goal.",
            "--- efforts/0070-demo.md, Pieces",
            "- [x] Build the example\n- [ ] Land the example",
            "[Testing together](initiatives/testing.md)",
        ] {
            assert!(loaded.contains(text), "{text}\n{loaded}");
        }
        for text in [
            "NOT-DELIVERED",
            "INITIATIVE-STATE-NOT-DELIVERED",
            "PLAN-NOT-DELIVERED",
        ] {
            assert!(!loaded.contains(text), "{text}\n{loaded}");
        }
    }
}

#[path = "activate/allow.rs"]
mod allow;
#[path = "activate/deadlines.rs"]
mod deadlines;
#[path = "activate/hooks.rs"]
mod hooks;
#[path = "activate/naming.rs"]
mod naming;
#[path = "activate/proof.rs"]
mod proof;
#[path = "activate/repositories.rs"]
mod repositories;
#[path = "activate/sessions.rs"]
mod sessions;
#[path = "activate/setup.rs"]
mod setup;
