//! The opt-in plugin's contract, run as the real binary: hook in, command out, as a client
//! would run them. Notes and repos are fixtures in a temp folder.
mod common;

use astrolabe::activate::{EFFORT_VAR, STATE_VAR};
use astrolabe::layer::DATA_DIR_VAR;
use astrolabe::registry::Registry;
use common::TempDir;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EFFORT: &str = "---\ntitle: 0070 Demo\nstatus: active\ninitiative: \"[[testing]]\"\nrepo: demo\ninvolvement: check in at plan\nauthority: edit\nnext: NEXT-70.\n---\n\n# 0070 Demo\n\nrepo: other · authority: push\n\n## Current state\n\nEFFORT-70 goal.\n\n**Next step:** NEXT-70.\n\n## History\n\nNOT-DELIVERED\n";

struct Fixture {
    dir: TempDir,
    demo: PathBuf,
    other: PathBuf,
}

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The arguments of a command the hook rewrote, which calls this program by its full path.
fn rewritten(command: &str) -> Vec<&str> {
    let exe = format!("'{}' ", env!("CARGO_BIN_EXE_astrolabe"));
    let rest = command.strip_prefix(&exe);
    let rest = rest.unwrap_or_else(|| panic!("only the switch reaches here: {command}"));
    rest.split_whitespace().collect()
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
            ("knowledge/voice.md", "VOICE-TEXT"),
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
        let registry = Registry::open(&dir.0.join("data/astrolabe.db")).unwrap();
        registry.add("agent", &notes).unwrap();
        registry.add("demo", &demo).unwrap();
        registry.add("other", &other).unwrap();
        write(&dir.0.join("plugin/hooks/hooks.json"), "hooks v1");
        write(
            &dir.0.join("plugin/.claude-plugin/plugin.json"),
            "manifest v1",
        );
        Self { dir, demo, other }
    }

    fn state(&self) -> PathBuf {
        self.dir.0.join("state")
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> (String, bool) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(args)
            .current_dir(cwd)
            .env(DATA_DIR_VAR, self.dir.0.join("data"))
            .env(STATE_VAR, self.state())
            .env("CLAUDE_PLUGIN_ROOT", self.dir.0.join("plugin"))
            .env_remove(EFFORT_VAR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
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
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.success(),
        )
    }

    /// The pre-command hook's answer, or None when it stays silent.
    fn hook(&self, input: Value) -> Option<Value> {
        let (out, ok) = self.run(
            &self.demo,
            &["hook", "pre-command", "--if-active"],
            &input.to_string(),
            &[],
        );
        assert!(ok, "a hook always exits 0");
        (!out.trim().is_empty()).then(|| serde_json::from_str(&out).unwrap())
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
            .find(|l| l.contains("pkill -0 astrolabe-guard-probe"))
            .unwrap();
        line.split_once("alone: ").unwrap().1.to_owned()
    }
}

#[test]
fn an_off_session_is_silent_and_unguarded() {
    let f = Fixture::new();
    assert!(!f.denies("s1", "pkill -f anything"));
    assert_eq!(f.session_start("s1", &f.demo, &[]), None);
}

#[test]
fn switching_on_binds_the_hooks_session_and_only_that_one() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    for wanted in [
        "effort 70",
        "check in at plan",
        "authority: edit",
        "EFFORT-70",
        "NEXT-70",
        "OWNER-TEXT",
        "VOICE-TEXT",
        "REPO-DEMO-TEXT",
        "MACHINE-TEXT",
        "doesn't see file edits",
    ] {
        assert!(text.contains(wanted), "missing {wanted}: {text}");
    }
    assert!(!text.contains("NOT-DELIVERED") && !text.contains("REPO-OTHER"));
    assert!(f.denies("s1", "pkill -f anything"));
    assert!(
        !f.denies("s2", "pkill -f anything"),
        "another session stays off"
    );
    assert!(!f.denies("s1", "git status"), "an allowed command runs");
    assert_eq!(
        f.session_start("s2", &f.demo, &[]),
        None,
        "a fresh session inherits nothing"
    );
}

#[test]
fn the_hook_does_the_work_and_the_command_only_prints_it() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --show 0123");
    assert!(!ok && text.contains("only from its hook"), "{text}");
    let (text, ok) = f.run(&f.demo, &["activate", "--effort", "70"], "", &[]);
    assert!(
        !ok && text.contains("NOT switched on") && text.contains("/hooks"),
        "{text}"
    );
    let made_up = ["activate", "--show", "0123456789abcdef0123456789abcdef"];
    let (text, ok) = f.run(&f.demo, &made_up, "", &[]);
    assert!(
        !ok && text.contains("isn't one the hook just issued"),
        "{text}"
    );
    // Shell around the command: not rewritten, so it runs bare and is refused.
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "cd x && astrolabe activate --effort 70"}});
    assert_eq!(f.hook(input), None);
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let answer = f.hook(input).unwrap();
    assert!(
        f.state().join("s1.json").exists(),
        "on before the command runs"
    );
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let words = rewritten(command);
    assert_eq!(words[..2], ["activate", "--show"]);
    // The command writes nothing, so a read-only sandbox can run it.
    let (text, ok) = f.run(&f.demo, &words, "", &[]);
    assert!(ok && text.contains("effort 70"), "{text}");
}

#[test]
fn a_missing_or_bad_field_or_another_repo_is_refused_and_keeps_what_was_on() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 71");
    assert!(
        !ok && text.contains("no `authority` property") && text.contains("stays off"),
        "{text}"
    );
    assert!(!f.denies("s1", "pkill x"), "no protection claimed or given");
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    for (effort, why) in [
        ("74", "involvement \"whenever\" isn't one of"),
        ("72", "isn't the effort's (other)"),
        ("99", "no effort 99"),
    ] {
        let (text, ok) = f.bash(
            "s1",
            &f.demo,
            &format!("astrolabe activate --effort {effort}"),
        );
        assert!(
            !ok && text.contains(why) && text.contains("Effort 70 stays on"),
            "{text}"
        );
    }
    assert!(f.denies("s1", "pkill x"));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("effort 70")
    );
    // The other repo's effort switches on from that repo.
    assert!(f.bash("s3", &f.other, "astrolabe activate --effort 72").1);
}

#[test]
fn unreadable_notes_refuse_with_the_file_named() {
    let f = Fixture::new();
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(
        !ok && text.contains("knowledge/machines.md") && text.contains("No new protection"),
        "{text}"
    );
    assert!(!f.state().join("s1.json").exists());
}

#[test]
fn the_guard_is_proven_only_by_a_hook_denial_of_this_activations_probe() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status().contains("not proven yet"));
    assert!(
        !f.denies("s2", &probe),
        "an off session's run proves nothing"
    );
    assert!(status().contains("not proven yet"));
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"), "{}", status());
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --effort 70").1,
        "same effort keeps it"
    );
    assert!(status().contains("Guard proven"));
    let marker = probe.split('\'').nth(1).unwrap().to_owned();
    // Switching effort revalidates and needs a new proof.
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 75");
    assert!(text.contains("authority: read") && status().contains("not proven yet"));
    assert!(f.denies("s1", &f.probe(&text)));
    let second = f.probe(&text).split('\'').nth(1).unwrap().to_owned();
    std::fs::write(&second, "").unwrap();
    assert!(status().contains("Guard FAILED"));
    std::fs::remove_file(second).unwrap();
    assert!(
        !std::path::Path::new(&marker).exists(),
        "the first probe never ran"
    );
}

#[test]
fn resume_reloads_fresh_context_and_a_failed_reload_keeps_the_guard_and_blocks() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    let effort = f.dir.0.join("notes/efforts/0070-demo.md");
    std::fs::write(&effort, EFFORT.replace("EFFORT-70", "EFFORT-70-NEWER")).unwrap();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.contains("EFFORT-70-NEWER"), "{text}");
    let machines = f.dir.0.join("notes/knowledge/machines.md");
    std::fs::remove_file(&machines).unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "compact"});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input.to_string(),
        &[],
    );
    assert!(
        out.contains("stays on for effort 70")
            && out.contains("machines.md")
            && out.contains("blocked"),
        "{out}"
    );
    assert!(
        f.denies("s1", "pkill -0 astrolabe-guard-probe"),
        "the guard stays"
    );
    let (why, ok) = f.bash("s1", &f.demo, "git status");
    assert!(!ok && why.contains("--off"), "{why}");
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("couldn't be reloaded")
    );
    // Repaired and switched on again: unblocked.
    std::fs::write(&machines, "MACHINE-TEXT").unwrap();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    assert!(!f.denies("s1", "git status") && f.denies("s1", "pkill x"));
    // Or switched off: unblocked and unguarded, as asked.
    std::fs::remove_file(&machines).unwrap();
    f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(f.denies("s1", "git status"));
    assert!(f.bash("s1", &f.demo, "astrolabe activate --off").1);
    assert!(!f.denies("s1", "git status"));
}

#[test]
fn a_launch_switches_on_by_env_bound_to_its_own_session() {
    let f = Fixture::new();
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(
        text.contains("effort 70") && text.contains("OWNER-TEXT"),
        "{text}"
    );
    assert!(f.denies("w1", "pkill x"));
    assert!(!f.denies("w2", "pkill x"));
    let text = f
        .session_start("w3", &f.demo, &[(EFFORT_VAR, "71")])
        .unwrap();
    assert!(
        text.contains("NOT switched on") && text.contains(EFFORT_VAR),
        "{text}"
    );
    assert!(!f.denies("w3", "pkill x"));
}

#[test]
fn a_subagent_inherits_the_guard_only_and_cant_switch() {
    let f = Fixture::new();
    let child = |session: &str, command: &str| json!({"session_id": session, "agent_id": "a1", "agent_type": "general-purpose", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": command}});
    assert_eq!(
        f.hook(child("s1", "pkill x")),
        None,
        "an off parent's child stays off"
    );
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let answer = f.hook(child("s1", "astrolabe activate --off")).unwrap();
    assert!(answer.to_string().contains("subagent can't switch"));
    let answer = f
        .hook(child("s1", "astrolabe activate --effort 72"))
        .unwrap();
    assert!(answer.to_string().contains("subagent can't switch"));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("effort 70"),
        "the parent's switch stands"
    );
    // Guard, from the parent: a denial, and an allowed control.
    let answer = f.hook(child("s1", "pkill x")).unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(f.hook(child("s1", "git status")), None);
    assert_eq!(
        f.hook(child("s2", "pkill x")),
        None,
        "an unrelated parent's child stays off"
    );
    // The denial keeps the child's identity, and doesn't prove the parent's guard.
    let log = std::fs::read_to_string(f.dir.0.join("data/guard-denials.log")).unwrap();
    assert!(log.lines().any(|l| l.contains("\"agent_id\":\"a1\"")));
    assert!(f.hook(child("s1", &f.probe(&text))).is_some());
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("not proven yet")
    );
    assert!(f.denies("s1", &f.probe(&text)));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("Guard proven")
    );
}

#[test]
fn unreadable_state_blocks_commands_with_a_repair_and_off_repairs_it() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::write(f.state().join("s1.json"), "garbage").unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "git status"}});
    let answer = f.hook(input).unwrap().to_string();
    assert!(
        answer.contains("can't read its state") && answer.contains("--off"),
        "{answer}"
    );
    assert!(f.bash("s1", &f.demo, "astrolabe activate --off").1);
    assert!(!f.denies("s1", "git status"));
}

#[test]
fn a_hook_gets_what_fits_and_the_files_to_read_for_the_rest() {
    let f = Fixture::new();
    let big = "word ".repeat(1500);
    write(&f.dir.0.join("notes/knowledge/repos/demo.md"), &big);
    write(&f.dir.0.join("notes/knowledge/machines.md"), &big);
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(text.contains("EFFORT-70") && text.contains("Read these now"));
    assert!(text.contains("knowledge/repos/demo.md") && text.contains("knowledge/machines.md"));
    let (full, ok) = f.bash("w1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && full.len() > 9800, "a command's output carries it all");
}

#[test]
fn codex_input_is_told_apart_and_bound_the_same_way() {
    let f = Fixture::new();
    let input = json!({"session_id": "t1", "turn_id": "u1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let answer = f.hook(input).unwrap();
    let command = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let words = rewritten(command);
    let (text, ok) = f.run(&f.demo, &words, "", &[]);
    assert!(ok && text.contains("codex session t1"), "{text}");
}

#[test]
fn the_plugin_folder_calls_this_program_and_names_one_plugin() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin");
    let read = |file: &str| -> Value {
        serde_json::from_str(&std::fs::read_to_string(root.join(file)).unwrap()).unwrap()
    };
    let hooks = read("astrolabe/hooks/hooks.json");
    for (event, slug) in [
        ("SessionStart", "session-start"),
        ("PreToolUse", "pre-command"),
    ] {
        let command = &hooks["hooks"][event][0]["hooks"][0]["command"];
        let wanted = format!(
            "\"${{ASTROLABE_TRIAL_BIN:-$HOME/.local/bin/astrolabe-trial}}\" hook {slug} --if-active"
        );
        assert_eq!(command, &wanted, "the trial build, by path");
        assert!(
            hooks["hooks"][event][0].get("matcher").is_none(),
            "every tool, as the guard decides"
        );
    }
    let name = &read("astrolabe/.claude-plugin/plugin.json")["name"];
    assert_eq!(
        &read(".claude-plugin/marketplace.json")["plugins"][0]["name"],
        name
    );
    assert_eq!(
        &read(".agents/plugins/marketplace.json")["plugins"][0]["name"],
        name
    );
    let skill = std::fs::read_to_string(root.join("astrolabe/skills/effort/SKILL.md")).unwrap();
    assert!(
        skill.contains("disable-model-invocation: true")
            && skill.contains("astrolabe activate --effort <n>")
    );
    let codex =
        std::fs::read_to_string(root.join("astrolabe/skills/effort/agents/openai.yaml")).unwrap();
    assert!(
        codex.contains("allow_implicit_invocation: false"),
        "never by automatic discovery"
    );
}

#[test]
fn a_switch_whose_result_cant_be_left_isnt_made() {
    let f = Fixture::new();
    let shown = f.state().join("shown");
    std::fs::create_dir_all(f.state()).unwrap();
    std::fs::write(&shown, "a file where the folder goes").unwrap();
    let (why, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(!ok && why.contains("couldn't finish"), "{why}");
    assert!(
        !f.state().join("s1.json").exists(),
        "first switch-on not made"
    );
    std::fs::remove_file(&shown).unwrap();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_dir_all(&shown).unwrap();
    std::fs::write(&shown, "").unwrap();
    assert!(!f.bash("s1", &f.demo, "astrolabe activate --effort 75").1);
    assert!(!f.bash("s1", &f.demo, "astrolabe activate --off").1);
    std::fs::remove_file(&shown).unwrap();
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(
        status.contains("effort 70"),
        "effort switch not made: {status}"
    );
    assert!(f.denies("s1", "pkill x"), "off not made");
}

#[test]
fn a_switch_that_cant_be_saved_shows_that_instead() {
    let f = Fixture::new();
    // A folder where the session's state file goes: the result is left, the write fails.
    std::fs::create_dir_all(f.state().join("s1.json")).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(!ok && text.contains("couldn't be written"), "{text}");
}

#[test]
fn a_changed_hook_needs_its_own_proof() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
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
fn a_large_effort_points_at_current_state_only() {
    let f = Fixture::new();
    let current = "Current words. ".repeat(800);
    let effort = format!(
        "---\nrepo: demo\ninvolvement: check in at plan\nauthority: edit\n---\n# 0070 Demo\n\n## Current state\n\nBIG-CURRENT {current}\n\n## History\n\nSUPERSEDED: push straight to main without review.\n"
    );
    write(&f.dir.0.join("notes/efforts/0070-demo.md"), &effort);
    let text = f
        .session_start("w1", &f.demo, &[(EFFORT_VAR, "70")])
        .unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(!text.contains("SUPERSEDED"), "{text}");
    assert!(
        text.contains("only its `## Current state` section"),
        "{text}"
    );
    let path = f
        .dir
        .0
        .join("notes/efforts/0070-demo.md")
        .display()
        .to_string();
    assert!(
        !text.contains(&format!("- {path}\n")) && !text.ends_with(&format!("- {path}")),
        "never the whole file"
    );
    assert!(
        text.contains("knowledge/owner.md"),
        "whole files for the rest"
    );
    let (full, ok) = f.bash("w1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && full.contains("BIG-CURRENT") && !full.contains("SUPERSEDED"));
}

#[test]
fn properties_come_from_frontmatter_only() {
    let f = Fixture::new();
    let line_only = "# 0076\n\nrepo: demo · involvement: autonomous · authority: edit\n\n## Current state\n\nx\n";
    write(&f.dir.0.join("notes/efforts/0076-line.md"), line_only);
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 76");
    assert!(
        !ok && text.contains("no `repo` property") && text.contains("properties at the top"),
        "{text}"
    );
    // Effort 70's body holds a decoy line naming another repo and push; the properties win.
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(
        ok && text.contains("authority: edit") && text.contains("\"0070 Demo\""),
        "{text}"
    );
    // A YAML list and quoted values read too.
    assert!(f.bash("s2", &f.demo, "astrolabe activate --effort 75").1);
    assert!(f.bash("s3", &f.other, "astrolabe activate --effort 72").1);
}

#[cfg(unix)]
#[test]
fn a_failed_reload_that_cant_be_saved_still_blocks_or_says_it_doesnt() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let lock =
        |mode| std::fs::set_permissions(f.state(), std::fs::Permissions::from_mode(mode)).unwrap();
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "resume"}).to_string();
    let command = |cmd: &str| {
        json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": cmd}}).to_string()
    };
    // State folder read-only: the block goes to the temp folder and still holds.
    lock(0o555);
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input,
        &[],
    );
    assert!(
        out.contains("blocked until") && !out.contains("NOT blocked"),
        "{out}"
    );
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("git status"),
        &[],
    );
    assert!(
        out.contains("\"deny\"") && out.contains("couldn't be reloaded"),
        "{out}"
    );
    lock(0o755);
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --off").1,
        "off clears it"
    );
    assert!(!f.denies("s1", "git status"));
    // Neither can be written: said plainly, and the guard alone stays on.
    write(&f.dir.0.join("notes/knowledge/machines.md"), "M");
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    std::fs::remove_file(f.dir.0.join("notes/knowledge/machines.md")).unwrap();
    let tmp = f.dir.0.join("locked-tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o555)).unwrap();
    let env = [("TMPDIR", tmp.to_str().unwrap())];
    lock(0o555);
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input,
        &env,
    );
    assert!(
        out.contains("NOT blocked") && out.contains("only the guard is on"),
        "{out}"
    );
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("git status"),
        &env,
    );
    assert!(out.trim().is_empty(), "as said, not blocked: {out}");
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &command("pkill x"),
        &env,
    );
    assert!(out.contains("\"deny\""), "the guard stays: {out}");
    lock(0o755);
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
}
