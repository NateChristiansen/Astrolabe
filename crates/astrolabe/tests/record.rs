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
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
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

#[test]
fn a_new_effort_is_numbered_stamped_journaled_committed_and_bound() {
    let world = World::new();
    let mac = world.machine("mac");
    let start = mac.session_start("s1", "startup");
    assert!(start.contains("astrolabe effort new"), "{start}");
    // A labelled goal is refused before the lock and pull: no file, no commit.
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Nope\" <<'ASTROLABE'\n**Goal:** g.\nASTROLABE\n",
    );
    assert!(!ok && out.contains("no **Label:** lines"), "{out}");
    assert_eq!(mac.log().lines().count(), before);
    assert!(!mac.notes.join(shelf("0001-nope.md")).exists());
    let (out, ok) = mac.bash("s1", NEW);
    assert!(ok, "{out}");
    assert!(
        out.contains(&format!(
            "Effort 1 created as {}, status active, bound to this session",
            shelf("0001-smooth-ink.md")
        )),
        "{out}"
    );
    assert!(
        out.contains("involvement check in at plan · authority edit"),
        "{out}"
    );
    assert!(
        out.contains("Astrolabe is on in this session for effort 1"),
        "{out}"
    );
    let text = mac.effort_text(1);
    for line in [
        "status: active",
        "repo: demo",
        "involvement: check in at plan",
        "authority: edit",
        "saved_by: mac · claude",
        "branch: main",
        "dirty: false",
        "# 0001 Smooth ink",
        "## Current state\nsmooth the ink across frames, with café in it\n",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    // Starting is state, not history: no journal line until the agent writes one.
    assert!(
        !mac.notes.join("journal").exists(),
        "{}",
        mac.journal_today()
    );
    assert!(mac.log().starts_with("0001: started\n"), "{}", mac.log());
    assert!(mac.notes.join(".gitattributes").exists());
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    assert!(state["delivered"].is_string() && state["baseline"].is_string());
    // The same command typed again with no hook explains itself from the ticket.
    let (plain, ok) = mac.plain(NEW);
    assert!(ok, "{plain}");
    assert!(plain.contains("Effort 1 created"), "{plain}");
    // A different plain command with no hook is NOT recorded.
    let (plain, ok) =
        mac.plain("astrolabe effort new --title \"Other\" <<'ASTROLABE'\ngoal\nASTROLABE\n");
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
    // A subagent can't.
    let (out, ok) = mac.bash_as("s1", NEW, Some("child-1"));
    assert!(!ok && out.contains("subagent"), "{out}");
    // Resume re-injects the effort with its freshness.
    let resumed = mac.session_start("s1", "resume");
    assert!(
        resumed.contains("Last saved by mac · claude at"),
        "{resumed}"
    );
}

#[test]
fn save_replaces_current_state_journals_states_and_refuses_what_it_should() {
    let world = World::new();
    let mac = world.machine("mac");
    let (_, ok) = mac.bash("s1", NEW);
    assert!(ok);
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n**Goal:** g.\n**Done:** half.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("no **Label:** lines (**Goal:** g.)"),
        "{out}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive, half done.\n\nNext: the rest.\nASTROLABE\n",
    );
    assert!(ok && out.contains("Current state replaced"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status blocked");
    assert!(!ok && out.contains("--waits-for"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status blocked --waits-for \"owner: should it be X?\"",
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("waits_for: \"owner: should it be X?\""),
        "{}",
        mac.effort_text(1)
    );
    // Blocked is state: nothing in the journal.
    assert!(!mac.notes.join("journal").exists());
    // A long block and a long journal line are refused before anything is written.
    let long = (0..151).map(|_| "word").collect::<Vec<_>>().join(" ");
    let (out, ok) = mac.bash(
        "s1",
        &format!("astrolabe effort save <<'ASTROLABE'\n{long}\nASTROLABE\n"),
    );
    assert!(
        !ok && out.contains("151 words") && out.contains("under 150"),
        "{out}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"one two three four five six seven eight nine ten one two three four five six seven eight nine ten one two three four five six seven eight nine ten one two three four five six seven eight nine ten one\"");
    assert!(
        !ok && out.contains("41 words") && out.contains("aim for 25"),
        "{out}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status done --decision \"X over Y: cheaper\" --journal \"Started smoothing the ink; the first frame still jumps.\" <<'ASTROLABE'\nDone: merged the ink MR.\nASTROLABE\n");
    assert!(
        ok && out.contains("status done") && out.contains("journal line written"),
        "{out}"
    );
    let text = mac.effort_text(1);
    assert!(
        text.contains("status: done")
            && text.contains("waits_for: \"\"")
            && text.contains("- 20")
            && text.contains(": X over Y: cheaper"),
        "{text}"
    );
    // The journal holds only the agent's sentence, with a hidden tail.
    let journal = mac.journal_today();
    assert!(
        journal.starts_with(
            "- Started smoothing the ink; the first frame still jumps. <!-- 0001 mac "
        ) && journal.lines().count() == 1,
        "{journal}"
    );
    assert_eq!(mac.log().lines().next(), Some("0001: done"));
    // A later line the same day replaces this writer's earlier one: one per effort per day.
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"Finished the ink smoothing: strokes no longer split across frames.\"");
    assert!(ok, "{out}");
    let journal = mac.journal_today();
    assert!(
        journal.starts_with("- Finished the ink smoothing") && journal.lines().count() == 1,
        "{journal}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"two\nlines\"");
    assert!(!ok && out.contains("one line"), "{out}");
    // The plan file is read as the user, relative to the session's folder.
    write(&mac.demo.join("plan.md"), "the plan");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status active --plan-file plan.md",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("## Plan\nthe plan\n"));
    assert_eq!(mac.journal_today().lines().count(), 1);
    // Codex keeps no plan file: the plan rides in the body under a `## Plan` line, and the
    // fields above it are the Current state.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\n\n## Plan\n1. first\n2. second\nASTROLABE",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(text.contains("## Plan\n1. first\n2. second\n"), "{text}");
    assert!(
        text.contains("## Current state\nActive: g.\n\nNext: n.\n\n## "),
        "{text}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --plan-file plan.md <<'ASTROLABE'\n## Plan\nagain\nASTROLABE",
    );
    assert!(!ok && out.contains("one way"), "{out}");
    assert!(mac.effort_text(1).contains("## Plan\n1. first\n"));
    // A heading inside a section would split it on the next read. The plan is the app's
    // file, so its headings are demoted and said so; an indented one counts, as it's stored
    // trimmed. Current state and Pieces are refused.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n## Plan\n## Plan\nsecond\nASTROLABE",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    assert!(mac.effort_text(1).contains("## Plan\n### Plan\nsecond\n"));
    write(
        &mac.demo.join("plan.md"),
        "  ## Current state\n**Goal:** stolen",
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --plan-file plan.md");
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Current state\n**Goal:** stolen\n"),
        "{text}"
    );
    assert!(text.contains("## Current state\nActive: g.\n"), "{text}");
    // The plan sits below Decisions, and a plan file's own `# ` title and its `## ` parts
    // shift together so the top lands at `###`.
    write(
        &mac.demo.join("plan.md"),
        "# Plan: ink\n\n## Context\nwhy\n### Step 1\n#### sub\n#not a heading\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status proposed --plan-file plan.md --decision \"a over b: c\"",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Plan: ink\n\n#### Context\nwhy\n##### Step 1\n###### sub\n#not a heading\n"),
        "{text}"
    );
    assert!(
        text.find("## Decisions").unwrap() < text.find("## Plan").unwrap(),
        "{text}"
    );
    // A plan written only at `####` lifts to `###`; bold prose in Current state is not a
    // label.
    write(&mac.demo.join("plan.md"), "#### Only\n##### deep\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status active --plan-file plan.md <<'ASTROLABE'\n**Done.** Merged the **parser:** cleanup.\n\nNext: n.\nASTROLABE");
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(text.contains("## Plan\n### Only\n#### deep\n"), "{text}");
    assert!(
        text.contains("## Current state\n**Done.** Merged"),
        "{text}"
    );
    // Only headings change, all by the same step: line endings and an indented line (no
    // heading to the reader) stay as they were.
    write(
        &mac.demo.join("plan.md"),
        "## Steps\r\n  ## not a heading\r\n### sub\r\n",
    );
    // A different command from the one above, or it reads as that call's retry.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status ready --plan-file plan.md",
    );
    assert!(ok && out.contains("top level is `###`"), "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Steps\r\n  ## not a heading\r\n#### sub\n"),
        "{text:?}"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\ng.\n## Decisions\nASTROLABE",
    );
    assert!(!ok && out.contains("can't hold a `## ` heading"), "{out}");
    write(&mac.demo.join("pieces.md"), "## Pieces\n- [ ] x\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --pieces-file pieces.md");
    assert!(!ok && out.contains("can't hold a `## ` heading"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\n## Plan\n### Step one\nfirst\n### Step two\nsecond\nASTROLABE",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --status active");
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Plan\n### Step one\nfirst\n### Step two\nsecond\n"),
        "{text}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save");
    assert!(!ok && out.contains("nothing to save"), "{out}");
    // Pieces: a checklist the owner reads, ticked by name, counted for the home view.
    write(
        &mac.demo.join("pieces.md"),
        "- [ ] Tether cap\n- [ ] Goo at speed\n- [ ] Goo splats\n",
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --pieces-file pieces.md --tick \"Tether\"",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert!(
        text.contains("## Pieces\n- [x] Tether cap\n- [ ] Goo at speed\n- [ ] Goo splats\n"),
        "{text}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Goo\"");
    assert!(!ok && out.contains("more than one piece"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --tick \"Nothing\"");
    assert!(!ok && out.contains("no piece starts with"), "{out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --tick \"Goo at\" --untick \"Goo at speed\"",
    );
    assert!(!ok && out.contains("both ticked and unticked"), "{out}");
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --tick \"Goo at\" --untick \"Tether\""
        )
        .1
    );
    assert!(
        mac.effort_text(1)
            .contains("- [ ] Tether cap\n- [x] Goo at speed\n")
    );
    // Pieces survive a Current state save.
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\nASTROLABE\n"
        )
        .1
    );
    assert!(mac.effort_text(1).contains("- [x] Goo at speed"));
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["efforts"][0]["pieces"], json!({"done": 1, "total": 3}));
}

#[test]
fn a_save_over_a_changed_file_is_refused_then_goes_through_from_the_new_text() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The owner edits the note by hand.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("smooth the ink", "OWNER EDIT smooth the ink");
    std::fs::write(&path, edited).unwrap();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nMine.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("changed since it was delivered") && out.contains("OWNER EDIT"),
        "{out}"
    );
    assert!(
        mac.effort_text(1).contains("OWNER EDIT"),
        "nothing overwritten"
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nOWNER EDIT kept, mine too.\n\nNext: n.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("OWNER EDIT kept"));
}

#[test]
fn two_machines_number_offline_and_the_newer_is_renumbered_after_sync() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    // The PC pulls on create, so it takes 2.
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"On the PC\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    // Both go offline (an unreachable remote) and create: both take 3.
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", "/nonexistent/notes.git"],
        );
    }
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Mac three\" <<'ASTROLABE'\nmac goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    let (out, ok) = pc.bash(
        "p2",
        "astrolabe effort new --title \"PC three\" <<'ASTROLABE'\npc goal\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    assert!(
        mac.bash("s2", "astrolabe effort save --journal \"Mac three's line\"")
            .1
    );
    assert!(
        pc.bash("p2", "astrolabe effort save --journal \"PC three's line\"")
            .1
    );
    let origin = world.origin.0.join("notes.git");
    for m in [&mac, &pc] {
        git(
            &m.notes,
            &["remote", "set-url", "origin", origin.to_str().unwrap()],
        );
    }
    let (out, ok) = mac.sync();
    assert!(
        ok,
        "{out}\n{}\n{}",
        git(&mac.notes, &["log", "--oneline", "--all", "--graph"]),
        git(&mac.notes, &["status"])
    );
    // The PC's pull rebases its 3 onto the Mac's 3: two files, one number; the newer (the
    // PC's, created later) is renumbered to 4 and its session follows.
    let (out, ok) = pc.sync();
    assert!(
        ok,
        "{out}\n{}\n{}",
        git(&pc.notes, &["log", "--oneline", "--all", "--graph"]),
        git(&pc.notes, &["status"])
    );
    assert!(
        pc.notes.join(shelf("0004-pc-three.md")).exists(),
        "{:?}",
        std::fs::read_dir(pc.notes.join("efforts"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    assert!(!pc.notes.join(shelf("0003-pc-three.md")).exists());
    assert!(pc.effort_text(4).contains("# 0004 PC three"));
    assert!(
        pc.effort_text(4).contains("renumbered from 0003"),
        "{}",
        pc.effort_text(4)
    );
    // The PC's own journal line for its 3 follows it to 4; the Mac's line for 3 stays.
    let journal = pc.journal_today();
    assert!(
        journal.contains("PC three's line <!-- 0004 pc ")
            && journal.contains("Mac three's line <!-- 0003 mac "),
        "{journal}"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(pc.state().join("p2.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "4");
    assert_eq!(state["file"], shelf("0004-pc-three.md"));
    // And the Mac sees it after its own pull, with the journal's day file merged by union.
    assert!(mac.sync().1);
    assert!(mac.notes.join(shelf("0004-pc-three.md")).exists());
    let journal = mac.journal_today();
    assert!(
        journal.contains("Mac three's line <!-- 0003 mac ")
            && journal.contains("PC three's line <!-- 0004 pc ")
            && journal.lines().count() == 2,
        "{journal}"
    );
}

#[test]
fn a_save_blind_to_the_other_machines_save_is_reported_after_sync() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.sync().1);
    // The PC loads 1 (pulling it), saves, pushes.
    let (out, ok) = pc.bash("p1", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort save <<'ASTROLABE'\nPC view.\n\nPC next.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(pc.sync().1);
    // The Mac, offline, saves the same effort from its older delivered text.
    git(
        &mac.notes,
        &["remote", "set-url", "origin", "/nonexistent/notes.git"],
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save <<'ASTROLABE'\nMac view.\n\nMac next.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    git(
        &mac.notes,
        &[
            "remote",
            "set-url",
            "origin",
            world.origin.0.join("notes.git").to_str().unwrap(),
        ],
    );
    // Both rewrote the same stamp and Current state, so the rebase conflicts: aborted, the
    // Mac's commit kept, and every session hears it until the owner resolves it.
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("diverged"), "{out}");
    assert!(
        git(&mac.notes, &["status", "--porcelain"])
            .trim()
            .is_empty(),
        "rebase aborted cleanly"
    );
    assert!(mac.effort_text(1).contains("Mac view"), "local save kept");
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(status.contains("Notes diverged from origin"), "{status}");
    let resumed = mac.session_start("s1", "resume");
    assert!(resumed.contains("Notes diverged from origin"), "{resumed}");
    // The owner resolves it (here: keep theirs), and the next sync is clean.
    git(&mac.notes, &["fetch", "-q"]);
    git(&mac.notes, &["reset", "-q", "--hard", "origin/main"]);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (status, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(status.contains("nothing pending"), "{status}");
}

#[test]
fn an_unfinished_write_is_finished_by_the_next_one_and_a_retry_commits_once() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A crash after the file write, before the commit: the file is there, nothing committed,
    // the intent remains.
    let intent = json!({
        "key": "crashed-key", "files": [[shelf("0001-smooth-ink.md"), std::fs::read_to_string(mac.notes.join(shelf("0001-smooth-ink.md"))).unwrap().replace("smooth the ink", "CRASHED smooth the ink")]],
        // The first journal shape, as an intent written by the build before this one.
        "renames": [], "journal": [{"id": "crashed-key0", "kind": "decision", "effort": "0001", "title": "Smooth ink", "text": "crash line", "machine": "mac", "provider": "claude", "time": "01:00", "day": "2026-01-01"}],
        "message": "0001: checkpoint", "base": null, "states": []
    });
    write(
        &mac.state().join("ops/crashed-key.json"),
        &intent.to_string(),
    );
    std::fs::write(
        mac.notes.join(shelf("0001-smooth-ink.md")),
        intent["files"][0][1].as_str().unwrap(),
    )
    .unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"after the crash\"");
    // The delivered hash no longer matches, because the crashed write landed: refused, and
    // the leftover was still finished first.
    assert!(
        !ok && out.contains("changed since it was delivered"),
        "{out}"
    );
    let finished: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("ops/crashed-key.json")).unwrap())
            .unwrap();
    assert!(
        finished["committed"] == true && finished["result"].is_string(),
        "intent finished: {finished}"
    );
    let log = mac.log();
    assert!(
        log.starts_with("0001: checkpoint\n0001: started\n"),
        "{log}"
    );
    assert!(
        std::fs::read_to_string(mac.notes.join(astrolabe::notes::journal_file("2026-01-01")))
            .unwrap()
            .contains("crash line")
    );
    // The same tool call again (a lost reply) commits nothing new.
    let before = mac.log();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --journal \"after the crash\"");
    assert!(ok, "{out}");
    let after = mac.log();
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    let (_, ok) = mac.bash("s1", "astrolabe effort save --journal \"after the crash\"");
    assert!(ok);
    assert_eq!(
        mac.log().lines().count(),
        after.lines().count(),
        "same tool_use_id, same key: no second commit"
    );
    assert_eq!(mac.journal_today().matches("after the crash").count(), 1);
}

#[test]
fn two_saves_at_once_both_land_under_the_lock() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Second\" <<'ASTROLABE'\ng2\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let mac = std::sync::Arc::new(mac);
    let handles: Vec<_> = ["s1", "s2"].into_iter().map(|s| {
        let mac = mac.clone();
        std::thread::spawn(move || {
            let mut input = json!({"session_id": s, "cwd": mac.demo, "tool_name": "Bash", "tool_use_id": format!("c-{s}"), "tool_input": {"command": format!("astrolabe effort save --journal \"from {s}\"")}});
            input["x"] = Value::Null;
            let (out, ok) = mac.run(&mac.demo, &["hook", "pre-command", "--if-active"], &input.to_string());
            assert!(ok && out.contains("permissionDecision"), "{out}");
        })
    }).collect();
    for h in handles {
        h.join().unwrap();
    }
    let journal = mac.journal_today();
    assert!(
        journal.contains("from s1") && journal.contains("from s2"),
        "{journal}"
    );
    assert_eq!(
        mac.log()
            .lines()
            .filter(|l| *l == "0001: checkpoint" || *l == "0002: checkpoint")
            .count(),
        2,
        "{}",
        mac.log()
    );
    assert!(
        std::fs::read_dir(mac.state().join("ops"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0
    );
}

#[test]
fn efforts_json_is_what_the_home_view_reads() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which?\" <<'ASTROLABE'\nActive: g.\n\nNext: ask.\nASTROLABE\n").1);
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--repo", "demo"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let e = &v["efforts"][0];
    assert_eq!(e["number"], 1);
    assert_eq!(e["status"], "blocked");
    assert_eq!(e["needs_owner"], true);
    assert_eq!(e["state"], "Active: g.");
    assert_eq!(e["next"], "Next: ask.");
    assert_eq!(e["repo"], "demo");
    assert_eq!(v["journal"], "");
    // The reader takes both journal shapes.
    let mixed = "- 14:02 done 0012 Smooth ink: merged the ink MR <!-- mac claude e:7f3a -->\n- Finished the ink. <!-- 0013 mac 5 e:9b -->\n- 12:30 is when it broke: fixed. <!-- 0014 pc 6 e:9c -->\n- Found the Deck preset. <!-- demo mac 7 e:aa -->\n- Fixed. <!-- 0015 mac e:9d -->\n- 12:30 done fixing it. <!-- 0016 mac e:9e -->\n";
    let lines = astrolabe::notes::parse_journal(mixed);
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.effort.as_str(), l.text.as_str(), l.id.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("0012", "merged the ink MR", "7f3a"),
            ("0013", "Finished the ink.", "9b"),
            ("0014", "12:30 is when it broke: fixed.", "9c"),
            ("demo", "Found the Deck preset.", "aa"),
            ("0015", "Fixed.", "9d"),
            ("0016", "12:30 done fixing it.", "9e")
        ]
    );
    // Tidy keeps the latest line per effort, as a union merge of two replacements leaves
    // both; findings and first-shape lines stay.
    let doubled = "- 09:00 started 0001 A: a <!-- mac claude e:l1 -->\n- 09:05 done 0001 A: a <!-- mac claude e:l2 -->\n- Started A. <!-- 0001 mac 10 e:a1 -->\n- Started B. <!-- 0002 pc 11 e:b1 -->\n- Unstamped C. <!-- 0003 mac e:c0 -->\n- Finished A. <!-- 0001 mac 20 e:a2 -->\n- Finished B. <!-- 0002 pc 21 e:b2 -->\n- Finished C. <!-- 0003 mac 22 e:c1 -->\n- Found x. <!-- demo mac 30 e:f1 -->\n- Found y. <!-- demo pc 31 e:f2 -->\n";
    assert_eq!(
        astrolabe::notes::tidy_journal(doubled),
        "- 09:00 started 0001 A: a <!-- mac claude e:l1 -->\n- 09:05 done 0001 A: a <!-- mac claude e:l2 -->\n- Finished A. <!-- 0001 mac 20 e:a2 -->\n- Finished B. <!-- 0002 pc 21 e:b2 -->\n- Finished C. <!-- 0003 mac 22 e:c1 -->\n- Found x. <!-- demo mac 30 e:f1 -->\n- Found y. <!-- demo pc 31 e:f2 -->\n"
    );
}

#[test]
fn two_machines_replacing_their_lines_the_same_day_leave_one_line_each_after_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.bash("s1", "astrolabe effort save --journal \"Started A.\"")
            .1
    );
    assert!(mac.sync().1);
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort new --title \"Two\" <<'ASTROLABE'\ng2\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 2 created"), "{out}");
    assert!(
        pc.bash("p1", "astrolabe effort save --journal \"Started B.\"")
            .1
    );
    assert!(pc.sync().1);
    assert!(mac.sync().1);
    assert_eq!(mac.journal_today().lines().count(), 2);
    // Both replace their own line without seeing the other's replacement: a union merge
    // would keep all four; the tidy after the pull keeps the latest per effort.
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(
        mac.bash("s1", "astrolabe effort save --journal \"Finished A.\"")
            .1
    );
    assert!(
        pc.bash("p1", "astrolabe effort save --journal \"Finished B.\"")
            .1
    );
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    for m in [&mac, &pc] {
        let journal = m.journal_today();
        assert!(
            journal.contains("- Finished A. <!-- 0001 mac ")
                && journal.contains("- Finished B. <!-- 0002 pc ")
                && journal.lines().count() == 2,
            "{journal}"
        );
    }
    assert!(
        pc.log().contains("journal tidied after a merge"),
        "{}",
        pc.log()
    );
}

#[test]
fn a_timed_out_network_call_backs_syncs_off_for_a_while() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let far = astrolabe::notes::now_ms() + 60_000;
    write(
        &mac.state().join("sync.json"),
        &json!({"backoff_until_ms": far}).to_string(),
    );
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("no fetch for another"), "{out}");
    // Over, a sync goes through and clears it.
    write(
        &mac.state().join("sync.json"),
        &json!({"backoff_until_ms": astrolabe::notes::now_ms() - 1}).to_string(),
    );
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let record: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("sync.json")).unwrap()).unwrap();
    assert!(record.get("backoff_until_ms").is_none(), "{record}");
}

#[test]
fn a_finding_with_no_effort_is_one_journal_line_and_a_worker_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    // A question answered now: no effort, one sentence in the day file, tagged with the repo.
    let (out, ok) = mac.bash("s1", "astrolabe journal \"Found the Deck preset in the video settings: everything Low, render scale 0.75.\"");
    assert!(ok && out.contains("Journal line written"), "{out}");
    let journal = mac.journal_today();
    assert!(
        journal.starts_with("- Found the Deck preset in the video settings: everything Low, render scale 0.75. <!-- demo mac "),
        "{journal}"
    );
    assert_eq!(mac.log().lines().next(), Some("journal: demo"));
    // Findings don't replace each other: two in a day are two lines.
    assert!(mac.bash("s1", "astrolabe journal <<'ASTROLABE'\nWalk is 3.0 and sprint 4.8, in the player motor.\nASTROLABE\n").1);
    assert_eq!(mac.journal_today().lines().count(), 2);
    let (out, ok) = mac.bash("s1", "astrolabe journal --title x");
    assert!(!ok && out.contains("only --repo"), "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe journal");
    assert!(!ok && out.contains("needs the sentence"), "{out}");
    // No hook: the plain command explains itself like an effort command does.
    let (plain, ok) = mac.run(&mac.demo, &["journal", "Something new"], "");
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
    // With an effort, the line belongs on the effort.
    assert!(mac.bash("s2", NEW).1);
    let (out, ok) = mac.bash("s2", "astrolabe journal \"x\"");
    assert!(!ok && out.contains("--journal"), "{out}");
    // A worker binds to the dispatcher's effort for context and the guard, and writes nothing.
    let (out, ok) = mac.bash("w1", "astrolabe activate --effort 1 --worker");
    assert!(ok && out.contains("worker under that effort"), "{out}");
    for cmd in [
        NEW,
        "astrolabe effort save --journal \"x\"",
        "astrolabe journal \"x\"",
    ] {
        let (out, ok) = mac.bash("w1", cmd);
        assert!(!ok && out.contains("worker under effort 1"), "{out}");
    }
    // Still a worker after a resume and after loading the effort again without the flag.
    assert!(
        mac.session_start("w1", "resume")
            .contains("worker under that effort")
    );
    let (out, ok) = mac.bash("w1", "astrolabe activate --effort 1");
    assert!(ok && out.contains("worker under that effort"), "{out}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("w1.json")).unwrap()).unwrap();
    assert_eq!(state["worker"], true);
    // Its edits never nudge.
    mac.edit("w1", "Edit");
    assert!(mac.stop("w1", false, false).is_none());
    assert!(mac.stop("w1", false, true).is_none());
}

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

#[test]
fn the_stop_nudge_fires_once_per_change_on_both_clients_and_never_loops() {
    let world = World::new();
    let mac = world.machine("mac");
    // Nothing bound, nothing edited: silence, however long the answer. A question answered
    // now is not an effort, and an ask here once turned a code review into a journal line.
    assert!(mac.stop("s1", false, false).is_none());
    let long = json!({"session_id": "s1", "cwd": mac.demo, "stop_hook_active": false, "last_assistant_message": "x".repeat(700)});
    assert!(mac.hook_json("stop", long).is_none());
    // An edit with no effort: one nudge to start one, then silence.
    mac.edit("s1", "Edit");
    let nudge = mac.stop("s1", false, false).unwrap();
    let text = nudge["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("no effort recorded") && text.contains("not the owner"),
        "{text}"
    );
    assert!(mac.stop("s1", false, false).is_none(), "said once");
    // Bound and saved: silence; a commit in the repo moves the fingerprint: one nudge.
    assert!(mac.bash("s1", NEW).1);
    assert!(
        mac.stop("s1", false, false).is_none(),
        "baseline taken at create"
    );
    // Created, never saved, long answer: one ask, then silence until a save.
    let long = json!({"session_id": "s1", "cwd": mac.demo, "stop_hook_active": false, "last_assistant_message": "y".repeat(700)});
    let asked = mac.hook_json("stop", long.clone()).unwrap();
    assert!(
        asked["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("never saved"),
        "{asked}"
    );
    assert!(mac.hook_json("stop", long.clone()).is_none());
    assert!(mac.bash("s1", "astrolabe effort save --decision \"d0\"").1);
    assert!(mac.hook_json("stop", long).is_none(), "saved: no ask");
    write(&mac.demo.join("a.txt"), "a");
    git(&mac.demo, &["add", "a.txt"]);
    git(&mac.demo, &["commit", "-q", "-m", "a"]);
    let nudge = mac.stop("s1", false, true).unwrap();
    assert_eq!(
        nudge["decision"], "block",
        "Codex continues only by a block"
    );
    let text = nudge["reason"].as_str().unwrap();
    assert!(
        text.contains("the repo changed since the last save of effort 1")
            && text.contains("not the owner"),
        "{text}"
    );
    assert!(mac.stop("s1", true, true).is_none(), "already continued");
    assert!(
        mac.stop("s1", false, true).is_none(),
        "acknowledged: no second nudge for the same state"
    );
    // A second edit to a dirty file changes the print; a save clears everything.
    write(&mac.demo.join("a.txt"), "aa");
    assert!(mac.stop("s1", false, false).is_some());
    write(&mac.demo.join("a.txt"), "aaa");
    assert!(
        mac.stop("s1", false, false).is_some(),
        "content, not file names"
    );
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save <<'ASTROLABE'\nActive: g.\n\nNext: n.\nASTROLABE\n"
        )
        .1
    );
    assert!(mac.stop("s1", false, false).is_none(), "saved");
    // The guard's view of a commit, and an apply_patch, both mark.
    let (_, ok) = mac.bash("s1", "git commit --allow-empty -q -m nothing");
    assert!(ok);
    assert!(mac.stop("s1", false, false).is_some());
    assert!(mac.stop("s1", false, false).is_none());
    mac.edit("s1", "apply_patch");
    assert!(mac.stop("s1", false, false).is_some());
    // Session end closes the state; a fresh session on the same id still reloads it.
    assert!(
        mac.hook_json(
            "session-end",
            json!({"session_id": "s1", "cwd": mac.demo, "reason": "other"})
        )
        .is_none()
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert!(state["closed_ms"].is_number());
}

#[test]
fn review_fixes_hold() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // A pickup in another session records what was delivered, so its save is checked too.
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s2.json")).unwrap()).unwrap();
    assert!(
        state["delivered"].is_string() && state["baseline"].is_string(),
        "{state}"
    );
    assert!(
        mac.bash("s1", "astrolabe effort save --decision \"from s1\"")
            .1
    );
    let (out, ok) = mac.bash("s2", "astrolabe effort save --decision \"from s2\"");
    assert!(
        !ok && out.contains("changed since it was delivered"),
        "{out}"
    );
    // Nested decision text survives a save.
    let path = mac.notes.join(shelf("0001-smooth-ink.md"));
    let text =
        std::fs::read_to_string(&path).unwrap() + "  nested detail under the last decision\n";
    std::fs::write(&path, &text).unwrap();
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"again\"");
    assert!(!ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"again\"");
    assert!(ok, "{out}");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("nested detail under the last decision") && saved.contains(": again"),
        "{saved}"
    );
    // A crash after the file write of a create, before its commit, then the same call again:
    // one effort, not two.
    let crashed = json!({
        "key": "k-crash", "files": [["efforts/0002-crashed.md", "---\nstatus: active\nrepo: demo\ncreated: 2026-01-01\n---\n# 0002 Crashed\n\n## Current state\n**Goal:** g.\n"]],
        "renames": [], "journal": [], "message": "0002: started", "base": null, "states": [], "committed": false,
        "digest": "d", "result": null, "cwd": "x"
    });
    write(&mac.state().join("ops/k-crash.json"), &crashed.to_string());
    std::fs::write(
        mac.notes.join("efforts/0002-crashed.md"),
        crashed["files"][0][1].as_str().unwrap(),
    )
    .unwrap();
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s3",
        "astrolabe effort new --title \"Third\" <<'ASTROLABE'\ng3\nASTROLABE\n",
    );
    assert!(ok && out.contains("Effort 3 created"), "{out}");
    assert_eq!(
        mac.log().lines().count(),
        before + 2,
        "the leftover committed, then the new one"
    );
    assert!(
        !mac.state().join("ops/k-crash.json").exists()
            || serde_json::from_slice::<Value>(
                &std::fs::read(mac.state().join("ops/k-crash.json")).unwrap()
            )
            .unwrap()["result"]
                .is_string()
    );
    // A committed intent that still has a state file to write never rewrites the note.
    let newer = std::fs::read_to_string(mac.notes.join("efforts/0002-crashed.md"))
        .unwrap()
        .replace("**Goal:** g.", "NEWER.");
    std::fs::write(mac.notes.join("efforts/0002-crashed.md"), &newer).unwrap();
    git(&mac.notes, &["add", "-A"]);
    git(&mac.notes, &["commit", "-q", "-m", "0002: newer"]);
    let mut committed = crashed.clone();
    committed["key"] = "k-committed".into();
    committed["committed"] = true.into();
    committed["states"] = json!([[mac.state().join("s9.json"), {"session": "s9", "effort": "2", "file": "efforts/0002-crashed.md"}]]);
    write(
        &mac.state().join("ops/k-committed.json"),
        &committed.to_string(),
    );
    assert!(mac.bash("s3", "astrolabe effort save --decision \"d\"").1);
    assert!(
        std::fs::read_to_string(mac.notes.join("efforts/0002-crashed.md"))
            .unwrap()
            .contains("NEWER"),
        "not rolled back"
    );
    assert!(
        mac.state().join("s9.json").exists(),
        "the state step was finished"
    );
    // The plain command finds a finished intent's result when the ticket never got written.
    let (_, ok) = mac.bash("s3", "astrolabe effort save --decision \"lost reply\"");
    assert!(ok);
    let digest =
        astrolabe::effort::EffortCommand::parse("astrolabe effort save --decision \"lost reply\"")
            .unwrap()
            .digest();
    let op = json!({"key": "k-lost", "files": [], "renames": [], "journal": [], "message": "m", "base": null, "states": [], "committed": true,
        "digest": digest, "result": "Saved effort 3: recovered text", "cwd": astrolabe::hook::comparable(&mac.demo)});
    write(&mac.state().join("ops/k-lost.json"), &op.to_string());
    for f in std::fs::read_dir(mac.state().join("shown"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    let (out, ok) = mac.plain("astrolabe effort save --decision \"lost reply\"");
    assert!(
        ok && out.contains("recovered text") && out.contains("Recovered from the hook's record"),
        "{out}"
    );
    // A PowerShell here-string arrives as one argument with line breaks: same digest.
    let (out, ok) = mac.run(
        &mac.demo,
        &["effort", "save", "--decision", "lost reply"],
        "",
    );
    assert!(ok, "{out}");
    let (out, _) = mac.run(
        &mac.demo,
        &["effort", "save", "--status", "active", "g."],
        "",
    );
    assert!(out.contains("NOT recorded"), "{out}");
    // A mistyped flag is refused, not ignored.
    let (out, ok) = mac.bash("s3", "astrolabe effort save --plan x");
    assert!(
        !ok && out.contains("isn't a flag of `effort save`"),
        "{out}"
    );
}

#[test]
fn a_retried_create_after_switching_efforts_names_the_effort_it_made() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The lost-reply retry's intent is long gone; the session has since moved to effort 2.
    for f in std::fs::read_dir(mac.state().join("ops"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort new --title \"Second\" <<'ASTROLABE'\ng2\nASTROLABE\n"
        )
        .1
    );
    for f in std::fs::read_dir(mac.state().join("ops"))
        .unwrap()
        .flatten()
    {
        std::fs::remove_file(f.path()).unwrap();
    }
    let (out, ok) = mac.bash("s1", NEW);
    assert!(
        ok && out.contains("Effort 1 was already created by this same call"),
        "{out}"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    assert_eq!(
        mac.log()
            .lines()
            .filter(|l| l.ends_with(": started"))
            .count(),
        2
    );
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

#[test]
fn the_board_follows_every_save_and_stays_out_of_git() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let board = mac.notes.join("home.html");
    let page = std::fs::read_to_string(&board).expect("the board is written at effort new");
    assert!(column(&page, "active").contains("Smooth ink"), "{page}");
    assert!(page.contains("<span class=\"pill repo\">demo</span>"));
    // Excluded in the checkout's own exclude file, so the board never reaches origin and
    // nothing is committed or left dirty for it.
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]).trim(), "");
    assert!(!mac.notes.join(".gitignore").exists());
    let exclude = git(&mac.notes, &["rev-parse", "--git-path", "info/exclude"]);
    assert!(
        std::fs::read_to_string(mac.notes.join(exclude.trim()))
            .unwrap()
            .contains("/home.html")
    );
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which cap?\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    let page = std::fs::read_to_string(&board).unwrap();
    let needs = column(&page, "needsyou");
    assert!(needs.contains("Smooth ink") && needs.contains("class=\"ask\">which cap?<"));
    assert!(!column(&page, "active").contains("Smooth ink"));
    assert!(mac.bash("s1", "astrolabe effort save --status done --journal \"Smoothed the ink.\" <<'ASTROLABE'\nDone.\n\nNothing chosen.\n\nNothing left.\nASTROLABE\n").1);
    let page = std::fs::read_to_string(&board).unwrap();
    assert!(column(&page, "done").contains("Smooth ink"));
    assert!(
        page.contains("<span>Smoothed the ink.</span></li>"),
        "{page}"
    );
    // The plain command redraws and names the file.
    std::fs::remove_file(&board).unwrap();
    let (out, ok) = mac.run(&mac.notes, &["home"], "");
    assert!(ok, "{out}");
    let file = out.lines().next().unwrap_or_default().trim();
    // The registered checkout may be the canonical path of the one the test knows.
    assert!(
        file.ends_with("home.html") && Path::new(file).exists(),
        "{out}"
    );
    // Then the address, and the server was asked for (a marker stands in for the process
    // under test).
    assert!(
        out.lines()
            .nth(1)
            .unwrap_or_default()
            .starts_with("http://localhost:"),
        "{out}"
    );
    assert!(mac.state().join("home-requested").exists());
    assert!(board.exists());
    // The other machine redraws its own copy when the saves arrive.
    assert!(mac.sync().1);
    let pc = world.machine("pc");
    assert!(!pc.notes.join("home.html").exists());
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(column(&page, "done").contains("Smooth ink"));
    assert_eq!(git(&pc.notes, &["status", "--porcelain"]).trim(), "");
    assert!(mac.bash("s1", "astrolabe effort save --status active <<'ASTROLABE'\nBack on it.\n\nNothing chosen.\n\nOne more pass.\nASTROLABE\n").1);
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(column(&page, "active").contains("Smooth ink"), "{page}");
    assert!(!column(&page, "done").contains("Smooth ink"));
    // An ignore rule that unignores it stops the draw, and says so.
    std::fs::write(pc.notes.join(".gitignore"), "!home.html\n").unwrap();
    let (out, ok) = pc.run(&pc.notes, &["home"], "");
    assert!(!ok && out.contains("overrides the exclude file"), "{out}");
    std::fs::remove_file(pc.notes.join(".gitignore")).unwrap();
    assert!(pc.run(&pc.notes, &["home"], "").1);
    // A fixed theme is kept per machine and carried by every later draw.
    let (out, ok) = pc.run(&pc.notes, &["home", "--theme", "dark"], "");
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(page.contains("content=\"dark\"") && page.contains(" data-theme=\"dark\">"));
    assert!(pc.sync().1);
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(page.contains("content=\"dark\""));
    // A palette is kept the same way, beside the theme.
    let (out, ok) = pc.run(&pc.notes, &["home", "--palette", "iris"], "");
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(pc.notes.join("home.html")).unwrap();
    assert!(
        page.contains("data-palette=\"iris\" data-theme=\"dark\">"),
        "{page}"
    );
    let (out, ok) = pc.run(&pc.notes, &["home", "--palette", "mauve"], "");
    assert!(!ok && out.contains("iris, ember or slate"), "{out}");
    let (out, ok) = pc.run(&pc.notes, &["home", "--theme", "blue"], "");
    assert!(!ok && out.contains("auto, light or dark"), "{out}");
    assert!(pc.run(&pc.notes, &["home", "--theme", "auto"], "").1);
    assert!(
        std::fs::read_to_string(pc.notes.join("home.html"))
            .unwrap()
            .contains("prefers-color-scheme")
    );
    // A tracked board is left alone and said so.
    std::fs::write(pc.notes.join("home.html"), "mine").unwrap();
    git(&pc.notes, &["add", "-f", "home.html"]);
    git(&pc.notes, &["commit", "-qm", "tracked board"]);
    assert!(pc.sync().1);
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("home.html")).unwrap(),
        "mine"
    );
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(status.contains("home.html is tracked"), "{status}");
}

#[test]
fn a_quiet_session_still_pushes_and_a_new_wait_is_a_state_change() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let _ = mac.sync_requested();
    // Same status, a new question for the owner: the other machine must see it.
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: merge it\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a status change asks for a push");
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save --waits-for \"owner: playtest it\" <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nPlaytest.\nASTROLABE\n").1);
    assert!(mac.sync_requested(), "a new waits-for asks for a push");
    mac.set_last_push(0);
    assert!(mac.bash("s1", "astrolabe effort save <<'ASTROLABE'\nMerged.\n\nNo choice.\n\nStill the playtest.\nASTROLABE\n").1);
    assert!(
        !mac.sync_requested(),
        "a plain checkpoint inside the window waits"
    );
    // The session sits open with commits unpublished: a turn end inside the window does
    // nothing, one past it pushes.
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested());
    mac.set_last_push(11 * 60 * 1000);
    mac.forget_ask();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(
        mac.sync_requested(),
        "the Stop hook pushes once the window has passed"
    );
    // Asked a moment ago: that one is still on its way, so the next turn end doesn't ask.
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "one ask at a time");
    // A save's own ask counts the same.
    mac.forget_ask();
    assert!(mac.bash("s1", "astrolabe effort save --status active <<'ASTROLABE'\nBack on it.\n\nNo choice.\n\nMore.\nASTROLABE\n").1);
    assert!(mac.sync_requested());
    mac.set_last_push(11 * 60 * 1000);
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "the save just asked");
    // Backing off after a timed-out network call: no ask until it ends.
    mac.forget_ask();
    mac.set_sync("backoff_until_ms", (Machine::now_ms() + 60_000).into());
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested(), "backing off");
    mac.set_sync("backoff_until_ms", (Machine::now_ms() - 1).into());
    assert!(mac.stop("s1", false, false).is_none());
    assert!(mac.sync_requested(), "the backoff ended");
    // An ask stamped in the future never holds the next one back.
    mac.ask_from_the_future();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(mac.sync_requested(), "a future stamp counts as old");
    // Nothing unpublished: nothing to push, however old the last one.
    assert!(mac.run(&mac.notes, &["notes", "sync"], "").1);
    mac.set_last_push(11 * 60 * 1000);
    mac.forget_ask();
    let _ = mac.sync_requested();
    assert!(mac.stop("s1", false, false).is_none());
    assert!(!mac.sync_requested());
}

/// A child killed and reaped however the test ends.
struct Kill(std::process::Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_notes_checkout_names_the_repo_a_new_effort_is_for() {
    let world = World::new();
    let mac = world.machine("mac");
    let notes = mac.notes.clone();
    let (out, ok) = mac.bash_from("n1", &notes, NEW, None);
    assert!(!ok && out.contains("--repo <name>"), "{out}");
    assert!(!notes.join(shelf("0001-smooth-ink.md")).exists());
    let (out, ok) = mac.bash_from(
        "n1",
        &notes,
        "astrolabe effort new --title \"Seat\" --repo nope <<'ASTROLABE'\ngoal\nASTROLABE\n",
        None,
    );
    assert!(!ok && out.contains("repo add nope"), "{out}");
    let (out, ok) = mac.bash_from(
        "n1",
        &notes,
        "astrolabe effort new --title \"Seat\" --repo demo <<'ASTROLABE'\ngoal\nASTROLABE\n",
        None,
    );
    assert!(ok && out.contains("Effort 1 created"), "{out}");
    assert!(
        mac.effort_text(1).contains("repo: demo"),
        "{}",
        mac.effort_text(1)
    );
    // The seat's own saves commit under the session; that isn't work moving, so no nudge.
    let stop = || json!({"session_id": "n1", "cwd": notes, "stop_hook_active": false, "last_assistant_message": "done"});
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "a create is no work"
    );
    assert!(
        mac.bash_from(
            "n1",
            &notes,
            "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree.\nASTROLABE\n",
            None
        )
        .1
    );
    assert!(mac.hook_json("stop", stop()).is_none(), "a save is no work");
    // A journal edited by hand nudges once; the save that then commits it doesn't.
    let day = notes
        .join("journal")
        .join(format!("{}.md", chrono::Local::now().format("%Y-%m-%d")));
    write(&day, "- Hand line.\n");
    assert!(
        mac.hook_json("stop", stop()).is_some(),
        "a hand edit is work"
    );
    assert!(
        mac.bash_from(
            "n1",
            &notes,
            "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree more.\nASTROLABE\n",
            None
        )
        .1
    );
    assert!(
        git(&notes, &["status", "--porcelain"]).trim().is_empty(),
        "the save committed the hand edit"
    );
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "its own commit isn't work"
    );
    // A save cut off between its commit and its state: the replay takes the baseline
    // again too, so the commit it carried never nudges.
    let state_path = mac.state().join("n1.json");
    let mut stale: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    stale["baseline"] = "stale".into();
    let intent = json!({
        "key": "cut-key", "files": [], "renames": [], "journal": [], "message": "0001: checkpoint",
        "base": null, "states": [[state_path, stale]], "committed": true, "rebaseline": notes,
    });
    write(&mac.state().join("ops/cut-key.json"), &intent.to_string());
    let (out, ok) = mac.run(&notes, &["notes", "sync"], "");
    assert!(ok, "{out}");
    let replayed: Value = serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_ne!(replayed["baseline"], "stale", "{replayed}");
    assert!(
        mac.hook_json("stop", stop()).is_none(),
        "the replayed save isn't work"
    );
    write(
        &notes.join("knowledge/owner.md"),
        "OWNER-TEXT edited by hand",
    );
    assert!(
        mac.hook_json("stop", stop()).is_some(),
        "a hand edit in the notes is"
    );
    assert!(mac.hook_json("stop", stop()).is_none(), "said once");
    // A finding with no effort names its repo the same way, from another session.
    let (out, ok) = mac.bash_from("n2", &notes, "astrolabe journal \"Found a thing.\"", None);
    assert!(!ok && out.contains("--repo <name>"), "{out}");
    let (out, ok) = mac.bash_from(
        "n2",
        &notes,
        "astrolabe journal --repo demo \"Found a thing.\"",
        None,
    );
    assert!(ok, "{out}");
    assert!(
        mac.journal_today().contains("Found a thing. <!-- demo "),
        "{}",
        mac.journal_today()
    );
    let (out, ok) = mac.bash_from("n2", &notes, "astrolabe journal --tick x \"No.\"", None);
    assert!(!ok && out.contains("only --repo"), "{out}");
    // Stamped by the work's repo, not the notes': the seat's branch and head say nothing.
    let text = mac.effort_text(1);
    assert!(
        text.contains("branch: \"\"") || text.contains("branch:\n") || text.contains("branch: ''"),
        "{text}"
    );
    assert!(
        !text.contains("head: ") || text.contains("head: \"\"") || text.contains("head: ''"),
        "{text}"
    );
    // In the repo's own folder the folder says which, so --repo is refused there.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Two\" --repo demo <<'ASTROLABE'\ngoal\nASTROLABE\n",
    );
    assert!(!ok && out.contains("drop --repo"), "{out}");
}

#[test]
fn a_dirty_notes_checkout_still_pulls() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    assert!(mac.bash("s1", NEW).1);
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    // The owner's seat is often mid-edit: a pull must not refuse over it.
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT edited by hand",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert!(
        pc.notes.join(shelf("0001-smooth-ink.md")).exists(),
        "the Mac's effort arrived"
    );
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT edited by hand",
        "the hand edit survived the pull"
    );
    assert!(git(&pc.notes, &["stash", "list"]).trim().is_empty());
    // An edit that clashes with what arrives: the tree takes what was pulled, the edit sits
    // in a stash named in the status until it's gone, and a save commits no marker.
    write(
        &mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT from the mac",
    );
    git(&mac.notes, &["commit", "-qam", "owner"]);
    git(&mac.notes, &["push", "-q"]);
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT clashing by hand",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::read_to_string(pc.notes.join("knowledge/owner.md")).unwrap(),
        "OWNER-TEXT from the mac"
    );
    assert!(git(&pc.notes, &["status", "--porcelain"]).trim().is_empty());
    let stash = git(&pc.notes, &["stash", "list", "--format=%H"]);
    let id = stash.trim();
    assert_eq!(id.len(), 40, "{stash}");
    assert!(git(&pc.notes, &["stash", "show", "-p", id]).contains("+OWNER-TEXT clashing by hand"));
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(
        status.contains("knowledge/owner.md")
            && status.contains(&format!("git stash apply {}", &id[..7])),
        "{status}"
    );
    assert!(pc.bash("p1", NEW).1, "a save still lands");
    assert!(!git(&pc.notes, &["show", "HEAD", "--stat"]).contains("owner.md"));
    // Two clashes before the owner takes either: both stashes named, each until it's gone.
    write(
        &mac.notes.join("knowledge/owner.md"),
        "OWNER-TEXT from the mac again",
    );
    git(&mac.notes, &["commit", "-qam", "owner again"]);
    git(&mac.notes, &["push", "-q"]);
    write(
        &pc.notes.join("knowledge/owner.md"),
        "OWNER-TEXT clashing twice",
    );
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert_eq!(status.matches("didn't reapply").count(), 2, "{status}");
    assert!(!status.contains("git stash drop"), "{status}");
    // An index left unmerged by hand refuses every write until it's resolved.
    git(&pc.notes, &["checkout", "-q", "-b", "side"]);
    write(&pc.notes.join("knowledge/owner.md"), "OWNER-TEXT side");
    git(&pc.notes, &["commit", "-qam", "side"]);
    git(&pc.notes, &["checkout", "-q", "main"]);
    write(&pc.notes.join("knowledge/owner.md"), "OWNER-TEXT main");
    git(&pc.notes, &["commit", "-qam", "main"]);
    let _ = Command::new("git")
        .args(["merge", "side"])
        .current_dir(&pc.notes)
        .output()
        .unwrap();
    assert!(git(&pc.notes, &["diff", "--name-only", "--diff-filter=U"]).contains("owner.md"));
    let (out, ok) = pc.bash(
        "p1",
        "astrolabe effort save <<'ASTROLABE'\nOne.\n\nTwo.\n\nThree.\nASTROLABE\n",
    );
    assert!(
        !ok && out.contains("unmerged files (knowledge/owner.md)"),
        "{out}"
    );
    git(&pc.notes, &["merge", "--abort"]);
    git(&pc.notes, &["reset", "-q", "--hard", "@{upstream}"]);
    // Once the owner has taken the stashes, the status stops naming them.
    git(&pc.notes, &["stash", "clear"]);
    let (out, ok) = pc.sync();
    assert!(ok, "{out}");
    let (status, _) = pc.run(&pc.notes, &["notes", "status"], "");
    assert!(!status.contains("stash"), "{status}");
}

#[test]
fn the_board_is_served_fresh_on_localhost() {
    use std::io::{BufRead, BufReader, Read};
    use std::time::Duration;
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let mut child = Kill(
        mac.command(&mac.notes, &["home", "--serve", "--port", "0"])
            .env("ASTROLABE_HOME_IDLE_SECS", "6")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut url = String::new();
        let _ = BufReader::new(stdout).read_line(&mut url);
        let _ = tx.send(url);
    });
    let url = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the server names its URL on start");
    let addr = url
        .trim()
        .strip_prefix("http://")
        .and_then(|u| u.strip_suffix('/'))
        .unwrap_or_else(|| panic!("{url}"))
        .to_owned();
    // A second server on this machine leaves at once, naming the address.
    let (out, ok) = mac.run(&mac.notes, &["home", "--serve", "--port", "0"], "");
    assert!(
        ok && out.contains("already serving") && out.contains("http://localhost:"),
        "{out}"
    );
    // One request; a connection the runner's stack resets before any byte arrives (seen
    // once on Windows CI) is tried again, since that's the runner, not the server.
    let get = |target: &str, host: &str| {
        let mut last = String::new();
        for _ in 0..3 {
            let mut s = std::net::TcpStream::connect(&addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
            s.write_all(format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
                .unwrap();
            let mut out = String::new();
            match s.read_to_string(&mut out) {
                Ok(_) => return out,
                // A saturated listener can close with the request still unread.
                // Keep its status for the recovery poll even if Windows resets the connection.
                Err(e)
                    if out.starts_with("HTTP/1.1 503 Service Unavailable\r\n")
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    return out;
                }
                Err(e)
                    if out.is_empty()
                        && matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    last = format!("{e}");
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => panic!("{e}: {out}"),
            }
        }
        panic!("reset three times: {last}")
    };
    let page = get("/", &addr);
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    assert!(page.contains("Cache-Control: no-store"));
    assert!(
        !page.contains("http-equiv=\"refresh\"") && page.contains("new EventSource('/events')"),
        "a served page follows the stream instead of reloading: {page}"
    );
    assert!(column(&page, "active").contains("Smooth ink"));
    // The card links through the server, since a browser refuses file links from a page.
    assert!(page.contains("href=\"/efforts/"), "{page}");
    // The card's own link, past the style sheet and the tree.
    let link = page[page.find("class=\"card").unwrap()..]
        .split("href=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_owned();
    let note = get(&link, &addr);
    assert!(
        note.starts_with("HTTP/1.1 200 OK\r\n") && note.contains("text/html"),
        "{note}"
    );
    assert!(
        note.contains("<h1 id=\"0001-smooth-ink\">0001 Smooth ink</h1>"),
        "{note}"
    );
    // A page holding the stream hears of a save within a few seconds; meanwhile the stream
    // says nothing but that it's alive.
    let mut events = std::net::TcpStream::connect(&addr).unwrap();
    events
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    events
        .write_all(format!("GET /events HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut heard = String::new();
    let mut buf = [0u8; 1024];
    let n = events.read(&mut buf).unwrap();
    heard.push_str(&String::from_utf8_lossy(&buf[..n]));
    assert!(
        heard.starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream"),
        "{heard}"
    );
    std::thread::sleep(Duration::from_millis(1500));
    // Drawn per request: a save shows on the next fetch with no redraw of the file.
    assert!(mac.bash("s1", "astrolabe effort save --status blocked --waits-for \"owner: which cap?\" <<'ASTROLABE'\nWaiting.\n\nNo choice yet.\n\nAsk.\nASTROLABE\n").1);
    let page = get("/", &addr);
    assert!(column(&page, "needsyou").contains("which cap?"), "{page}");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !heard.contains("event: changed") && std::time::Instant::now() < deadline {
        match events.read(&mut buf) {
            Ok(n) if n > 0 => heard.push_str(&String::from_utf8_lossy(&buf[..n])),
            _ => break,
        }
    }
    assert!(heard.contains("event: changed\ndata: "), "{heard}");
    drop(events);
    // Another origin can't hold the stream either.
    let mut other = std::net::TcpStream::connect(&addr).unwrap();
    other
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    other
        .write_all(b"GET /events HTTP/1.1\r\nHost: evil.example:80\r\n\r\n")
        .unwrap();
    let mut out = String::new();
    other.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 421"), "{out}");
    // A lesson in the notes' inbox is counted, listed and served as a page.
    assert!(!page.contains("to triage"), "{page}");
    std::fs::create_dir_all(mac.notes.join("lessons")).unwrap();
    std::fs::write(
        mac.notes.join("lessons/2026-11-05-gates.md"),
        "# Gates did not stop\n\nWhat happened.\n",
    )
    .unwrap();
    let page = get("/", &addr);
    assert!(page.contains("<b>1</b> lesson to triage"), "{page}");
    assert!(
        page.contains("href=\"/lessons/2026-11-05-gates.md\">Gates did not stop</a>"),
        "{page}"
    );
    let lesson = get("/lessons/2026-11-05-gates.md", &addr);
    assert!(
        lesson.starts_with("HTTP/1.1 200 OK\r\n") && lesson.contains("Gates did not stop"),
        "{lesson}"
    );
    assert!(get("/lessons/../knowledge/owner.md", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/favicon.ico", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/efforts/C:owner.md", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/efforts/..%2Fknowledge%2Fowner.md", &addr).starts_with("HTTP/1.1 404"));
    // HEAD gets the headers and no body.
    let mut s = std::net::TcpStream::connect(&addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(format!("HEAD / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    assert!(
        out.starts_with("HTTP/1.1 200 OK") && out.ends_with("\r\n\r\n"),
        "{out}"
    );
    assert!(get("/efforts/../knowledge/owner.md", &addr).starts_with("HTTP/1.1 404"));
    // Any note by its path is a page, since the board is where the owner reads them; a
    // dot folder (the plugin's own state, an app's config) is not.
    assert!(get("/knowledge/owner.md", &addr).starts_with("HTTP/1.1 200"));
    assert!(get("/.git/config", &addr).starts_with("HTTP/1.1 404"));
    assert!(get("/.agents/x.md", &addr).starts_with("HTTP/1.1 404"));
    assert!(
        get("/", "evil.example:80").starts_with("HTTP/1.1 421"),
        "another host gets nothing"
    );
    // A connection that says nothing gets a bad-request reply, not a hung server.
    let mut quiet = std::net::TcpStream::connect(&addr).unwrap();
    quiet
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    quiet.write_all(b"GET / HTTP/1.1\r\n").unwrap();
    quiet.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = String::new();
    quiet.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    assert!(
        get("/", &addr).starts_with("HTTP/1.1 200"),
        "still serving after a bad request"
    );
    // A head that never ends and outgrows the bound is refused, not buffered.
    let mut big = std::net::TcpStream::connect(&addr).unwrap();
    big.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let _ = big.write_all(format!("GET / HTTP/1.1\r\nX: {}\r\n", "y".repeat(20_000)).as_bytes());
    let mut out = String::new();
    let _ = big.read_to_string(&mut out);
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    // Past the handler cap an idle connection is told to come back, and the rest still serve
    // once the idle ones go.
    let idle: Vec<_> = (0..32)
        .map(|_| std::net::TcpStream::connect(&addr).unwrap())
        .collect();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut out = String::new();
    while !out.starts_with("HTTP/1.1 503") {
        assert!(
            std::time::Instant::now() < deadline,
            "no 503 past the cap: {out}"
        );
        let mut extra = std::net::TcpStream::connect(&addr).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        out.clear();
        let _ = extra.read_to_string(&mut out);
    }
    drop(idle);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !get("/", &addr).starts_with("HTTP/1.1 200") {
        assert!(
            std::time::Instant::now() < deadline,
            "not serving again after the idle ones left"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // With nothing asking for a while, the server stops on its own and the lock goes with
    // it; the next change to the notes starts one again.
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let exited = loop {
        if let Ok(Some(status)) = child.0.try_wait() {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(
        exited.is_some_and(|s| s.success()),
        "the server stops when idle: {exited:?}"
    );
}

#[test]
fn a_save_commits_only_the_notes_it_wrote_and_leaves_the_owners_staging_alone() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    // The owner has a draft staged in the notes checkout, unrelated to the save.
    write(&mac.notes.join("drafts/post.md"), "half a post");
    git(&mac.notes, &["add", "drafts/post.md"]);
    // And a hand-edited journal line, which saves deliberately carry along.
    let day = astrolabe::notes::journal_file("2026-01-02");
    write(&mac.notes.join(&day), "- by hand\n");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --decision \"kept apart\"");
    assert!(ok, "{out}");
    let files = git(&mac.notes, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(!files.contains("drafts/post.md"), "{files}");
    assert!(
        files.contains(&day),
        "hand journal edits still ride along: {files}"
    );
    let staged = git(&mac.notes, &["diff", "--cached", "--name-only"]);
    assert_eq!(staged.trim(), "drafts/post.md", "still staged");
    assert_eq!(
        std::fs::read_to_string(mac.notes.join("drafts/post.md")).unwrap(),
        "half a post"
    );
}

#[test]
fn a_shell_edit_to_an_untracked_file_moves_the_print_with_spaces_and_from_a_subfolder() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    assert!(mac.stop("s1", false, false).is_none(), "baseline at create");
    // A file with a space in its name, edited twice by shell, no edit tool marking it.
    let (_, ok) = mac.bash("s1", "printf a > 'b c.txt'");
    assert!(ok);
    assert!(mac.stop("s1", false, false).is_some(), "new file");
    let (_, ok) = mac.bash("s1", "printf aa > 'b c.txt'");
    assert!(ok);
    assert!(
        mac.stop("s1", false, false).is_some(),
        "second edit, same name"
    );
    // From a subfolder: paths come from the repo root, not the session's folder.
    let sub = mac.demo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let stop_in = |dir: &Path| {
        mac.hook_json(
            "stop",
            json!({"session_id": "s1", "cwd": dir, "stop_hook_active": false, "last_assistant_message": "done"}),
        )
    };
    write(&sub.join("x.txt"), "x");
    assert!(stop_in(&sub).is_some(), "new file in a subfolder");
    write(&sub.join("x.txt"), "xx");
    assert!(stop_in(&sub).is_some(), "second edit from the subfolder");
    assert!(stop_in(&sub).is_none(), "said once per change");
}

#[test]
fn a_pending_write_that_cant_be_read_is_reported_kept_and_blocks_new_writes() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let bad = mac.state().join("ops/broken.json");
    write(&bad, "{\"key\": \"broken\", \"files\": [");
    let (out, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(!out.contains("nothing pending"), "{out}");
    assert!(out.contains("broken"), "names the record: {out}");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"over a broken op\"",
    );
    assert!(!ok && out.contains("broken"), "{out}");
    let (out, ok) = mac.sync();
    assert!(!ok && out.contains("broken"), "{out}");
    assert_eq!(
        std::fs::read_to_string(&bad).unwrap(),
        "{\"key\": \"broken\", \"files\": [",
        "kept for repair"
    );
    // Once it's repaired (here, moved aside by hand), writes go through again.
    std::fs::remove_file(&bad).unwrap();
    assert!(
        mac.bash("s1", "astrolabe effort save --decision \"repaired\"")
            .1
    );
    let (out, _) = mac.run(&mac.notes, &["notes", "status"], "");
    assert!(!out.contains("broken"), "{out}");
}

#[test]
fn activation_reads_the_same_current_state_a_save_does() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let rel = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 1)
        .unwrap()
        .1;
    let text = mac.effort_text(1);
    let (head, rest) = text.split_once("\n## Current state\n").unwrap();
    let edited = format!(
        "{head}\n\nHow a note looks:\n\n```md\n## Current state\nFENCED EXAMPLE\n```\n\n## Current state\n{rest}"
    );
    write(&mac.notes.join(&rel), &edited);
    git(&mac.notes, &["commit", "-qam", "hand edit"]);
    let (out, ok) = mac.bash("s2", "astrolabe activate --effort 1");
    assert!(ok, "{out}");
    let shown = out.split(", Current state\n").nth(1).unwrap_or_default();
    assert!(shown.contains("smooth the ink"), "{out}");
    assert!(!shown.contains("FENCED EXAMPLE"), "{out}");
    // A save from that delivery agrees, and keeps the example as text.
    let (out, ok) = mac.bash("s2", "astrolabe effort save --decision \"agreed\"");
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("smooth the ink"));
}

#[test]
fn the_print_reads_untracked_files_under_a_root_ending_in_a_space_and_odd_byte_names() {
    let dir = TempDir::new();
    // Windows drops a trailing space from a folder name, so only Unix can have such a root.
    let repo = dir.0.join(if cfg!(unix) { "proj " } else { "proj" });
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
    let print = || {
        astrolabe::notes::RepoFacts::read(&repo)
            .unwrap()
            .fingerprint
    };
    write(&repo.join("f.txt"), "a");
    let first = print();
    write(&repo.join("f.txt"), "b");
    assert_ne!(first, print(), "content read from the real root");
    // Names that aren't UTF-8, where the filesystem allows them (not APFS).
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let name = |b: &[u8]| repo.join(std::ffi::OsStr::from_bytes(b));
        if std::fs::write(name(b"n-\xff"), "x").is_ok() {
            let before = print();
            std::fs::rename(name(b"n-\xff"), name(b"n-\xfe")).unwrap();
            assert_ne!(before, print(), "two names, two prints");
            let before = print();
            std::fs::write(name(b"n-\xfe"), "xy").unwrap();
            assert_ne!(before, print(), "its content counts");
        }
    }
}
