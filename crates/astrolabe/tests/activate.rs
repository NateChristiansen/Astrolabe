//! The opt-in plugin's contract, run as the real binary: hook in, command out, as a client
//! would run them. Notes and repos are fixtures in a temp folder.
mod common;

use astrolabe::activate::{EFFORT_VAR, STATE_VAR};
use astrolabe::layer::DATA_DIR_VAR;
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

/// The arguments of a command the hook rewrote, which calls this program by its full path,
/// with PowerShell's call operator in front where the command runs in PowerShell.
fn rewritten(command: &str) -> Vec<&str> {
    let exe = format!("'{}' ", env!("CARGO_BIN_EXE_astrolabe"));
    let command = command.strip_prefix("& ").unwrap_or(command);
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

#[test]
fn off_for_the_machine_nothing_runs_and_on_needs_no_step() {
    let f = Fixture::new();
    assert!(f.machine("off").contains("off for this machine"));
    assert!(f.machine("status").contains("off for this machine"));
    assert!(!f.denies("s1", "pkill -f anything"));
    assert_eq!(f.session_start("s1", &f.demo, &[]), None);
    assert_eq!(f.session_start("w1", &f.demo, &[(EFFORT_VAR, "70")]), None);
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "astrolabe activate --effort 70"}});
    assert_eq!(f.hook(input), None, "not taken, so it runs bare");
    let (text, ok) = f.run(&f.demo, &["activate", "--effort", "70"], "", &[]);
    assert!(!ok && text.contains("astrolabe on"), "{text}");
    assert!(f.machine("on").contains("on for this machine"));
    // On: a fresh session in a repo with no active effort gets owner, voice and the guard.
    let text = f.session_start("s2", &f.other, &[]).unwrap();
    assert!(
        text.contains("OWNER-TEXT") && text.contains("VOICE-TEXT") && text.contains("guard runs"),
        "{text}"
    );
    assert!(
        text.contains("No active effort names this repo (other)") && !text.contains("EFFORT-72"),
        "{text}"
    );
    assert!(f.denies("s2", "pkill -f anything") && !f.denies("s2", "git status"));
    assert!(
        f.denies("s9", "pkill -f anything"),
        "every session, even one never started here"
    );
    // The machine switch is the owner's, from a terminal.
    for verb in ["off", "on"] {
        let (why, ok) = f.bash("s2", &f.demo, &format!("astrolabe {verb}"));
        assert!(!ok && why.contains("owner's"), "{why}");
    }
    assert!(f.machine("status").contains("on for this machine"));
}

#[test]
fn a_repos_active_efforts_are_suggested_and_never_loaded_unasked() {
    let f = Fixture::new();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("one active effort is 70, \"0070 Demo\"; say \"pick up 70\" to load it"),
        "{text}"
    );
    assert!(
        text.contains("No effort is loaded")
            && !text.contains("EFFORT-70")
            && !text.contains("check in at plan"),
        "{text}"
    );
    assert!(text.contains("OWNER-TEXT") && !f.state().join("s1.json").exists());
    assert!(f.denies("s1", "pkill x"), "the guard all the same");
    let also = "---\nstatus: active\nrepo: [other, demo]\ninvolvement: autonomous\nauthority: read\n---\n# 0077 Also\n\n## Current state\n\nEFFORT-77\n";
    let also_path = f.dir.0.join("notes/efforts/0077-also.md");
    write(&also_path, also);
    let text = f.session_start("s2", &f.demo, &[]).unwrap();
    assert!(text.contains("active efforts are 70, \"0070 Demo\"; 77, \"0077 Also\"; say \"pick up <n>\" to load one"), "{text}");
    assert!(!f.state().join("s2.json").exists());
    // Until the owner names one.
    assert!(f.bash("s2", &f.demo, "astrolabe activate --effort 77").1);
    // A candidate that can't be read: nothing listed as if the set were known.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&also_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let text = f.session_start("s4", &f.demo, &[]).unwrap();
        assert!(
            text.contains("Can't list efforts")
                && text.contains("0077-also.md")
                && !text.contains("say \"pick up 70\" to load it"),
            "{text}"
        );
        assert!(f.denies("s4", "pkill x"));
        std::fs::set_permissions(&also_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    // A folder in no registered repo: no effort, guard on.
    let loose = f.dir.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    git(&loose, &["init", "-q"]);
    let text = f.session_start("s3", &loose, &[]).unwrap();
    assert!(
        text.contains("isn't registered") && f.denies("s3", "pkill x"),
        "{text}"
    );
}

#[test]
fn the_no_effort_context_keeps_under_the_cut_counting_everything() {
    let f = Fixture::new();
    let big = "word ".repeat(940);
    write(&f.dir.0.join("notes/knowledge/owner.md"), &big);
    write(&f.dir.0.join("notes/knowledge/voice.md"), &big);
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(text.len() <= 9800, "{}", text.len());
    assert!(
        text.contains("Read these now")
            && text.contains("knowledge/owner.md")
            && text.contains("knowledge/voice.md"),
        "{text}"
    );
    assert!(text.contains("pick up 70") && text.contains("guard runs"));
}

#[test]
fn a_session_with_no_effort_proves_its_own_guard() {
    let f = Fixture::new();
    let text = f.session_start("s1", &f.other, &[]).unwrap();
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.other, "astrolabe activate --status").0;
    assert!(
        status().contains("No effort is set") && status().contains("not proven yet"),
        "{}",
        status()
    );
    let child = json!({"session_id": "s1", "agent_id": "a1", "cwd": f.other, "tool_name": "Bash", "tool_input": {"command": probe}});
    assert!(f.hook(child).is_some());
    assert!(f.denies("s2", &probe));
    assert!(
        status().contains("not proven yet"),
        "a child's or another session's denial doesn't count"
    );
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"), "{}", status());
    // Switched off and on again: proof starts over.
    f.machine("off");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.machine("on");
    assert!(status().contains("not proven yet"));
}

#[test]
fn the_notes_checkout_and_its_worktrees_get_nothing_from_the_plugin() {
    let f = Fixture::new();
    let notes = f.dir.0.join("notes");
    let tree = f.dir.0.join("notes-tree");
    git(&notes, &["worktree", "add", "-q", tree.to_str().unwrap()]);
    for cwd in [&notes, &tree] {
        assert_eq!(
            f.session_start("n1", cwd, &[]),
            None,
            "its own layer delivers there"
        );
        let input = json!({"session_id": "n1", "cwd": cwd, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
        assert_eq!(f.hook(input), None, "its own layer guards there");
    }
    // A product repo still gets the plugin.
    assert!(
        f.session_start("n2", &f.demo, &[])
            .unwrap()
            .contains("OWNER-TEXT")
    );
    assert!(f.denies("n2", "pkill x"));
}

#[test]
fn the_machine_switch_reaches_running_sessions_at_once() {
    let f = Fixture::new();
    assert!(f.bash("s1", &f.demo, "astrolabe activate --effort 70").1);
    f.machine("off");
    assert!(!f.denies("s1", "pkill x"));
    f.machine("on");
    assert!(f.denies("s1", "pkill x"));
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
        !f.state().join("s2.json").exists(),
        "another session has no effort"
    );
    assert!(!f.denies("s1", "git status"), "an allowed command runs");
    let text = f.session_start("s2", &f.other, &[]).unwrap();
    assert!(
        !text.contains("EFFORT-70"),
        "a fresh session inherits nothing: {text}"
    );
}

#[test]
fn the_hook_does_the_work_and_the_command_only_prints_it() {
    let f = Fixture::new();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --show 0123");
    assert!(!ok && text.contains("only from its hook"), "{text}");
    let (text, ok) = f.run(&f.demo, &["activate", "--effort", "70"], "", &[]);
    assert!(
        !ok && text.contains("No effort loaded")
            && text.contains("/hooks")
            && text.contains("claim no protection"),
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
        !ok && text.contains("no `authority` property")
            && text.contains("No effort is loaded")
            && text.contains("the guard still runs"),
        "{text}"
    );
    assert!(!f.state().join("s1.json").exists(), "no effort set");
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
            !ok && text.contains(why) && text.contains("Effort 70 stays loaded"),
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
        !ok && text.contains("knowledge/machines.md")
            && text.contains("Only this effort's context and settings are missing"),
        "{text}"
    );
    assert!(!f.state().join("s1.json").exists());
}

#[test]
fn the_guard_is_proven_only_by_a_hook_denial_of_this_activations_probe() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(status().contains("not proven yet"));
    assert!(f.denies("s2", &probe), "another session's denial");
    // ... doesn't prove this one's guard.
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
    // Or the effort dropped: unblocked, and still guarded while the machine is on.
    std::fs::remove_file(&machines).unwrap();
    f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(f.denies("s1", "git status"));
    assert!(f.bash("s1", &f.demo, "astrolabe activate --off").1);
    assert!(!f.denies("s1", "git status") && f.denies("s1", "pkill x"));
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
    assert!(!f.state().join("w2.json").exists());
    let text = f
        .session_start("w3", &f.demo, &[(EFFORT_VAR, "71")])
        .unwrap();
    assert!(
        text.contains("NOT loaded")
            && text.contains(EFFORT_VAR)
            && text.contains("the guard still runs"),
        "{text}"
    );
    assert!(!f.state().join("w3.json").exists() && f.denies("w3", "pkill x"));
}

#[test]
fn a_subagent_inherits_the_guard_only_and_cant_switch() {
    let f = Fixture::new();
    let child = |session: &str, command: &str| json!({"session_id": session, "agent_id": "a1", "agent_type": "general-purpose", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": command}});
    assert!(
        f.hook(child("s1", "pkill x")).is_some(),
        "guarded while the machine is on, effort or not"
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
    assert!(
        f.hook(child("s2", "pkill x")).is_some(),
        "any session's child"
    );
    assert!(
        f.bash("s2", &f.demo, "astrolabe activate --status")
            .0
            .contains("No effort is set"),
        "a child switched nothing"
    );
    // The denial keeps the child's identity, and doesn't prove the parent's guard.
    let log = std::fs::read_to_string(f.state().join("guard-denials.log")).unwrap();
    assert!(log.lines().any(|l| l.contains("\"agent_id\":\"a1\"")));
    assert!(f.hook(child("s1", &f.probe(&text))).is_some());
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("not proven yet")
    );
    f.control(&text);
    assert!(f.denies("s1", &f.probe(&text)));
    assert!(
        f.bash("s1", &f.demo, "astrolabe activate --status")
            .0
            .contains("Guard proven")
    );
    f.machine("off");
    assert_eq!(
        f.hook(child("s1", "pkill x")),
        None,
        "off for the machine: children too"
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

/// Catches: a rewrite or probe that PowerShell can't parse, which Codex on Windows runs.
#[test]
fn a_command_that_runs_in_powershell_is_called_and_probed_in_its_form() {
    let f = Fixture::new();
    let rewrite = |input: Value| {
        let answer = f.hook(input).unwrap();
        let command = &answer["hookSpecificOutput"]["updatedInput"]["command"];
        command.as_str().unwrap().to_owned()
    };
    let claude_ps = json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let command = rewrite(claude_ps);
    assert!(command.starts_with("& '"), "{command}");
    let (text, _) = f.run(&f.demo, &rewritten(&command), "", &[]);
    assert!(
        f.probe(&text).starts_with("New-Item"),
        "the tool's own shell"
    );
    let codex = json!({"session_id": "c1", "turn_id": "u1", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": "astrolabe activate --effort 70"}});
    let command = rewrite(codex);
    assert_eq!(command.starts_with("& '"), cfg!(windows), "{command}");
    let (text, ok) = f.run(&f.demo, &rewritten(&command), "", &[]);
    assert!(ok, "{text}");
    let probe = f.probe(&text);
    f.control(&text);
    let powershell = probe.starts_with("New-Item -ItemType File -Force '")
        && probe.ends_with("'; Stop-Process -Name astrolabe-guard-probe");
    assert_eq!(powershell, cfg!(windows), "{probe}");
    // Denied in either form, but only the form for the shell that ran it proves the guard.
    let codex_runs = |command: &str| json!({"session_id": "c1", "turn_id": "u1", "cwd": f.demo, "tool_name": "shell_command", "tool_input": {"command": command}});
    let other = if powershell {
        probe
            .replace("New-Item -ItemType File -Force", "touch")
            .replace("Stop-Process -Name", "pkill -0")
    } else {
        probe
            .replace("touch", "New-Item -ItemType File -Force")
            .replace("pkill -0", "Stop-Process -Name")
    };
    let denied = |form: &str| {
        let answer = f.hook(codex_runs(form)).unwrap();
        answer["hookSpecificOutput"]["permissionDecision"] == "deny"
    };
    let status = || {
        let status = rewrite(codex_runs("astrolabe activate --status"));
        f.run(&f.demo, &rewritten(&status), "", &[]).0
    };
    assert!(status().contains("not proven yet"));
    assert!(denied(&other), "{other}");
    assert!(
        status().contains("not proven yet"),
        "the wrong shell's form: {}",
        status()
    );
    assert!(denied(&probe), "{probe}");
    let shell = if powershell {
        "PowerShell"
    } else {
        "the POSIX shell"
    };
    assert!(
        status().contains("Guard proven") && status().contains(shell),
        "{}",
        status()
    );
    // Claude's PowerShell tool: its own form, its own control, named as the path proven.
    let (text, _) = f.run(&f.demo, &rewritten(&rewrite(json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --status"}}))), "", &[]);
    let ps_probe = f.probe(&text);
    assert!(ps_probe.starts_with("New-Item"), "{text}");
    f.control(&text);
    let ps = json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": ps_probe}});
    assert_eq!(
        f.hook(ps).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    let (text, _) = f.run(&f.demo, &rewritten(&rewrite(json!({"session_id": "p1", "cwd": f.demo, "tool_name": "PowerShell", "tool_input": {"command": "astrolabe activate --status"}}))), "", &[]);
    assert!(
        text.contains("Guard proven for this session in PowerShell"),
        "{text}"
    );
}

#[test]
fn proof_needs_the_allowed_control_in_the_same_shell() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    assert!(
        text.contains("touch '") && text.contains("-control-posix'"),
        "marker only, no kill: {text}"
    );
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(f.denies("s1", &probe));
    let half = status();
    assert!(
        half.contains("not proven yet") && half.contains("control's marker isn't there"),
        "{half}"
    );
    f.control(&text);
    assert!(status().contains("Guard proven"));
}

/// Catches: the plugin reading AppData, which a packaged Windows app sees as its own private
/// copy, so a repo added from a terminal never reached it.
#[test]
fn the_plugin_keeps_its_repos_and_denials_in_its_state_folder() {
    let f = Fixture::new();
    for name in ["demo", "agent"] {
        let record = std::fs::read_to_string(f.state().join(format!("repos/{name}.json"))).unwrap();
        assert!(record.contains(&format!("\"name\":\"{name}\"")), "{record}");
    }
    let elsewhere = f.dir.0.join("empty-data");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let away = [(DATA_DIR_VAR, elsewhere.to_str().unwrap())];
    let input = json!({"session_id": "s1", "cwd": f.demo, "source": "startup"});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "session-start", "--if-active"],
        &input.to_string(),
        &away,
    );
    assert!(
        out.contains("OWNER-TEXT") && out.contains("one active effort is 70"),
        "{out}"
    );
    let input = json!({"session_id": "s1", "cwd": f.demo, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
    let (out, _) = f.run(
        &f.demo,
        &["hook", "pre-command", "--if-active"],
        &input.to_string(),
        &away,
    );
    assert!(out.contains("deny"), "{out}");
    assert!(f.state().join("guard-denials.log").is_file());
    assert!(!elsewhere.join("guard-denials.log").exists());
    assert!(!f.dir.0.join("data/guard-denials.log").exists());
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
        // Unquoted from `~`: Codex on Windows runs hooks in PowerShell, Claude in Git Bash or
        // sh, and all of them expand it, with no PATH, which a Mac app from the Dock may lack.
        let wanted = format!("~/.local/bin/astrolabe-trial hook {slug} --if-active");
        assert_eq!(command, &wanted, "the trial build, by its home path");
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
    // Plain words name an effort, so the model may use the skill, but only on the owner's number.
    for wanted in [
        "pick up 70",
        "work on effort 70",
        "/astrolabe:effort 70",
        "Not for a number that only comes up in discussion",
        "proves which session this is, not what the owner meant",
        "astrolabe activate --effort <n>",
    ] {
        assert!(skill.contains(wanted), "{wanted}");
    }
    assert!(!skill.contains("disable-model-invocation"));
    let codex =
        std::fs::read_to_string(root.join("astrolabe/skills/effort/agents/openai.yaml")).unwrap();
    assert!(codex.contains("allow_implicit_invocation: true"));
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
    f.control(&text);
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

#[test]
fn a_loaded_effort_proves_again_after_the_switch_goes_off_and_on() {
    let f = Fixture::new();
    let (text, _) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    let probe = f.probe(&text);
    f.control(&text);
    let status = || f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
    f.machine("off");
    std::thread::sleep(std::time::Duration::from_millis(5));
    f.machine("on");
    assert!(
        status().contains("effort 70") && status().contains("not proven yet"),
        "{}",
        status()
    );
    assert!(f.denies("s1", &probe));
    assert!(status().contains("Guard proven"));
}

#[test]
fn a_refused_load_says_the_guard_still_runs_and_it_does() {
    let f = Fixture::new();
    let loose = f.dir.0.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    git(&loose, &["init", "-q"]);
    let (text, ok) = f.bash("s1", &loose, "astrolabe activate --effort 70");
    assert!(!ok && text.contains("NOT loaded"), "{text}");
    assert!(
        text.contains("the guard still runs")
            && text.contains("owner and voice notes")
            && text.contains("Only this effort's context and settings are missing"),
        "{text}"
    );
    assert!(
        !text.contains("stays off") && !text.contains("No new protection"),
        "{text}"
    );
    let input = json!({"session_id": "s1", "cwd": loose, "tool_name": "Bash", "tool_input": {"command": "pkill x"}});
    assert!(f.hook(input).is_some(), "and it does");
}

#[test]
fn simultaneous_repo_adds_both_land() {
    let f = Fixture::new();
    std::fs::remove_dir_all(f.state().join("repos")).unwrap();
    let notes = f.dir.0.join("notes");
    let start = |name: &str, checkout: &Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(["repo", "add", name, checkout.to_str().unwrap()])
            .env(DATA_DIR_VAR, f.dir.0.join("data"))
            .env(STATE_VAR, f.state())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    };
    // Both started before either is waited on.
    let mut children = [
        start("agent", &notes),
        start("demo", &f.demo),
        start("other", &f.other),
    ];
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    for name in ["agent", "demo", "other"] {
        assert!(
            f.state().join(format!("repos/{name}.json")).exists(),
            "{name} lost"
        );
    }
    assert!(
        f.session_start("s1", &f.demo, &[])
            .unwrap()
            .contains("pick up 70")
    );
}

#[test]
fn on_refuses_without_the_notes_checkout_and_sessions_say_why() {
    let f = Fixture::new();
    f.machine("off");
    std::fs::remove_file(f.state().join("repos/agent.json")).unwrap();
    let (text, ok) = f.run(&f.demo, &["on"], "", &[]);
    assert!(
        !ok && text.contains("repo add agent") && text.contains("stays out of the notes checkout"),
        "{text}"
    );
    assert!(f.machine("status").contains("off for this machine"));
    write(&f.state().join("repos/agent.json"), "not json");
    let (text, ok) = f.run(&f.demo, &["on"], "", &[]);
    assert!(!ok && text.contains("isn't a repo record"), "{text}");
    // A map lost after switching on: sessions say plainly what that means.
    let (out, ok) = f.run(
        &f.dir.0.join("notes"),
        &[
            "repo",
            "add",
            "agent",
            f.dir.0.join("notes").to_str().unwrap(),
        ],
        "",
        &[],
    );
    assert!(ok, "{out}");
    assert!(f.machine("on").contains("on for this machine"));
    std::fs::remove_file(f.state().join("repos/agent.json")).unwrap();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(
        text.contains("can't tell whether this is the notes repo")
            && text.contains("repo add agent"),
        "{text}"
    );
    assert!(f.denies("s1", "pkill x"));
}

#[cfg(unix)]
#[test]
fn a_repo_add_the_plugin_cant_record_says_so_and_rerunning_fixes_it() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let repos = f.state().join("repos");
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
    let out = command
        .args(["repo", "add", "late", f.other.to_str().unwrap()])
        .env(DATA_DIR_VAR, f.dir.0.join("data"))
        .env(STATE_VAR, f.state())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && said.contains("plugin doesn't have it")
            && said.contains("rerunning is safe"),
        "{said}"
    );
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (_, ok) = f.run(
        &f.demo,
        &["repo", "add", "late", f.other.to_str().unwrap()],
        "",
        &[],
    );
    assert!(ok && repos.join("late.json").exists());
}

#[test]
fn a_bad_repo_name_is_refused_before_anything_is_written() {
    let f = Fixture::new();
    let (_, ok) = f.run(
        &f.demo,
        &["repo", "add", "../x", f.other.to_str().unwrap()],
        "",
        &[],
    );
    assert!(!ok);
    let registry = astrolabe::registry::Registry::open(&f.dir.0.join("data/astrolabe.db")).unwrap();
    assert!(
        registry.checkout("../x").is_err(),
        "not in the database either"
    );
    assert!(!f.state().join("x.json").exists() && !f.state().join("repos/../x.json").exists());
}

#[test]
fn markers_live_where_a_sandboxed_shell_can_write_and_session_start_names_the_shells() {
    let f = Fixture::new();
    let text = f.session_start("s1", &f.demo, &[]).unwrap();
    let probe_dir = std::env::temp_dir().join("astrolabe-probe");
    assert!(
        f.probe(&text).contains(&probe_dir.display().to_string()),
        "{text}"
    );
    assert!(probe_dir.is_dir(), "made, so the shell's touch works");
    // Claude on Windows may use Bash or PowerShell, so session start offers both.
    let both = text.contains("For PowerShell:") && text.contains("For the POSIX shell");
    assert_eq!(both, cfg!(windows), "{text}");
    f.control(&text);
    assert!(f.denies("s1", &f.probe(&text)));
    let status = f.bash("s1", &f.demo, "astrolabe activate --status").0;
    assert!(
        status.contains("Guard proven") && status.contains("POSIX"),
        "{status}"
    );
}
