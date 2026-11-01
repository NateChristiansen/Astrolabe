//! Registry, launch contract, init and doctor, all against temporary homes. `Sandbox::new`
//! refuses to run if any path it hands out could be a real home.
use astrolabe::doctor;
use astrolabe::init;
use astrolabe::launch::{self, DEFAULT_DIALS, Machine, Provider, Request};
use astrolabe::layer::{Hook, HookEvent, Layer, guard_rules};
use astrolabe::registry::{AGENT, Registry};
use astrolabe::trust::{self, CodexPlan, Homes};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const HASH_A: &str = "sha256:aaaa";
const HASH_B: &str = "sha256:bbbb";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        // Not the system temp folder on Windows, where folder checks refuse.
        let base = if cfg!(windows) {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/setup-tests")
        } else {
            std::env::temp_dir()
        };
        let path = base.join(format!("astrolabe-setup-{}-{id}", std::process::id()));
        std::fs::create_dir_all(path.join("home/.codex")).unwrap();
        let path = PathBuf::from(trust::native(&path));
        let sandbox = Self(path);
        sandbox.refuse_real_home();
        sandbox
    }

    /// A test that could reach a real home is a defect.
    fn refuse_real_home(&self) {
        for name in ["HOME", "USERPROFILE", "CODEX_HOME", "CLAUDE_CONFIG_DIR"] {
            if let Some(real) = std::env::var_os(name) {
                let real = PathBuf::from(real);
                assert_ne!(self.home(), real, "{name} is the sandbox");
            }
        }
        assert!(self.0.to_string_lossy().contains("astrolabe-setup-"));
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn homes(&self) -> Homes {
        Homes {
            codex_config: self.home().join(".codex/config.toml"),
            claude_json: self.home().join(".claude.json"),
        }
    }

    fn machine(&self) -> Machine {
        let data_dir = self.0.join("data");
        Machine {
            registry: Registry::open(&data_dir.join("astrolabe.db")).unwrap(),
            program_dir: PathBuf::from(env!("CARGO_BIN_EXE_astrolabe"))
                .parent()
                .unwrap()
                .to_path_buf(),
            adapters: self.0.join("adapters"),
            homes: self.homes(),
            data_dir,
        }
    }

    /// The real binary with every home pointed here.
    fn cli(&self, args: &[&str]) -> Command {
        self.refuse_real_home();
        let mut command = Command::new(env!("CARGO_BIN_EXE_astrolabe"));
        command
            .args(args)
            .current_dir(&self.0)
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env("CODEX_HOME", self.home().join(".codex"))
            .env("CLAUDE_CONFIG_DIR", self.home())
            .env("ASTROLABE_DATA_DIR", self.0.join("data"))
            .env("ASTROLABE_ADAPTERS", self.0.join("adapters"))
            .stdin(Stdio::null());
        command
    }

    /// A repo with a commit, and no remote.
    fn repo(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q", "-b", "main"]);
        git(&path, &["commit", "-q", "--allow-empty", "-m", "first"]);
        path
    }

    /// A package the Claude driver accepts. Nothing here starts it.
    fn fake_adapter(&self) {
        let root = self
            .0
            .join("adapters/node_modules/@agentclientprotocol/claude-agent-acp");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("package.json"), r#"{"bin":"run.js"}"#).unwrap();
        std::fs::write(root.join("run.js"), "").unwrap();
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=T"])
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn plan() -> CodexPlan {
    CodexPlan {
        project: "/r/agent".to_owned(),
        hooks: vec![
            (
                "/r/agent/.codex/config.toml:session_start:0:0".to_owned(),
                HASH_A.to_owned(),
            ),
            (
                "/r/agent/.codex/config.toml:pre_tool_use:0:0".to_owned(),
                HASH_B.to_owned(),
            ),
        ],
    }
}

fn windows_plan() -> CodexPlan {
    let repo = r"C:\Users\Me\workspace\Agent";
    let (project, claude) = trust::keys(repo, true);
    assert_eq!(project, r"c:\users\me\workspace\agent");
    assert_eq!(claude, "C:/Users/Me/workspace/Agent");
    let key = format!(r"{repo}\.codex\config.toml:session_start:0:0");
    CodexPlan {
        project,
        hooks: vec![(key, HASH_A.to_owned())],
    }
}

#[test]
fn checked_in_layer_is_what_the_code_says() {
    let layer = Layer::protected("");
    let file = |name: &str| init::LAYER.iter().find(|(n, _)| *n == name).unwrap().1;
    let settings: Value = serde_json::from_str(file(".claude/settings.json")).unwrap();
    assert_eq!(settings["permissions"]["deny"], json!(guard_rules()));
    assert_eq!(
        settings["permissions"]["allow"],
        json!(["mcp__astrolabe__layer_check"])
    );
    assert_eq!(settings["autoMemoryEnabled"], false);
    for (event, hook) in [
        ("SessionStart", HookEvent::SessionStart),
        ("PreToolUse", HookEvent::PreCommand),
        ("ConfigChange", HookEvent::ConfigChange),
    ] {
        let hook = Hook::astrolabe(hook);
        assert!(layer.hooks.contains(&hook));
        assert_eq!(
            settings["hooks"][event][0]["hooks"][0]["command"],
            hook.command.as_str()
        );
        assert!(
            settings["hooks"][event][0].get("matcher").is_none(),
            "match-all"
        );
    }
    let mcp: Value = serde_json::from_str(file(".mcp.json")).unwrap();
    assert_eq!(
        mcp["mcpServers"]["astrolabe"],
        json!({"command":"astrolabe","args":["mcp"]})
    );
    let codex = file(".codex/config.toml");
    for line in [
        "command = \"astrolabe hook session-start\"",
        "command = \"astrolabe hook pre-command\"",
        "generate_memories = false",
        "use_memories = false",
        "memories = false",
        "default_tools_approval_mode = \"approve\"",
    ] {
        assert!(codex.contains(line), "{line}");
    }
    assert!(
        !codex.contains('/') && !codex.contains('\\'),
        "no path in a hook string"
    );
}

#[test]
fn windows_forms() {
    let lines = windows_plan().lines();
    assert_eq!(lines[0], r"[projects.'c:\users\me\workspace\agent']");
    assert_eq!(
        lines[2],
        r"[hooks.state.'C:\Users\Me\workspace\Agent\.codex\config.toml:session_start:0:0']"
    );
    let (projects, hooks) = trust::scan(&lines.join("\n"));
    assert_eq!(projects, [windows_plan().project]);
    assert_eq!(hooks, windows_plan().hooks);
}

#[test]
fn scan_reads_mixed_quoting_and_ignores_the_rest() {
    let text = r#"
model = "x"
[projects."/a"]
trust_level = "trusted"
[projects.'/b']
trust_level = "untrusted"
[projects."C:\\Users\\Me"]
trust_level = 'trusted' # comment
[hooks.state.'c:\k:session_start:0:0']
trusted_hash = "sha256:1"
[hooks.state."/k:pre_tool_use:0:0"]
trusted_hash = 'sha256:2'
[other."/a"]
trust_level = "trusted"
trusted_hash = "sha256:3"
"#;
    let (projects, hooks) = trust::scan(text);
    assert_eq!(projects, ["/a", r"C:\Users\Me"]);
    assert_eq!(
        hooks,
        [
            (r"c:\k:session_start:0:0".to_owned(), "sha256:1".to_owned()),
            ("/k:pre_tool_use:0:0".to_owned(), "sha256:2".to_owned()),
        ]
    );
}

/// Every starting form: removal gives back the same bytes.
#[test]
fn removal_is_byte_identical() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    for before in [
        "",
        "model = \"x\"",
        "model = \"x\"\n",
        "model = \"x\"\n\n",
        "model = \"x\"\r\n[a]\r\nb = 1\r\n",
        "[projects.'/other']\ntrust_level = \"trusted\"\n",
    ] {
        std::fs::write(&path, before).unwrap();
        let hash = trust::file_hash(&path);
        let said = trust::apply_codex(&path, &plan(), &mut |text| {
            for line in plan().lines() {
                assert!(text.contains(&line), "each line is shown: {line}");
            }
            true
        })
        .unwrap();
        assert!(said.contains("written"), "{said}");
        let after = std::fs::read(&path).unwrap();
        assert!(after.starts_with(before.as_bytes()), "appended only");
        if before.contains("\r\n") {
            let text = String::from_utf8(after).unwrap();
            assert_eq!(text.matches('\n').count(), text.matches("\r\n").count());
        }
        let said = trust::remove_codex(&path, &mut |_| true).unwrap();
        assert!(said.contains("Byte-identical to before: true"), "{said}");
        assert_eq!(std::fs::read(&path).unwrap(), before.as_bytes());
        assert_eq!(trust::file_hash(&path), hash);
    }
}

#[test]
fn a_no_writes_nothing_and_a_later_edit_is_reported() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    std::fs::write(&path, "model = \"x\"\n").unwrap();
    let said = trust::apply_codex(&path, &plan(), &mut |_| false).unwrap();
    assert!(said.contains("nothing written"));
    assert_eq!(std::fs::read(&path).unwrap(), b"model = \"x\"\n");

    trust::apply_codex(&path, &plan(), &mut |_| true).unwrap();
    let again = trust::apply_codex(&path, &plan(), &mut |_| panic!("nothing to ask")).unwrap();
    assert!(again.contains("already trusted"));
    let said = trust::remove_codex(&path, &mut |_| false).unwrap();
    assert!(said.contains("nothing removed"));
    assert!(trust::find_block(&std::fs::read_to_string(&path).unwrap()).is_some());

    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("added = 1\n");
    std::fs::write(&path, &text).unwrap();
    let said = trust::remove_codex(&path, &mut |_| true).unwrap();
    assert!(said.contains("Byte-identical to before: false"), "{said}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "model = \"x\"\nadded = 1\n"
    );
}

#[test]
fn partial_lines_and_a_changed_or_missing_file_are_refused() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    assert!(
        trust::apply_codex(&path, &plan(), &mut |_| true).is_err(),
        "missing file"
    );

    let partial = "[projects.'/r/agent']\ntrust_level = \"trusted\"\n";
    std::fs::write(&path, partial).unwrap();
    assert!(
        trust::apply_codex(&path, &plan(), &mut |_| true).is_err(),
        "project line there"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), partial);

    let stale = "[hooks.state.\"/r/agent/.codex/config.toml:pre_tool_use:0:0\"]\ntrusted_hash = \"sha256:old\"\n";
    std::fs::write(&path, stale).unwrap();
    assert!(
        trust::apply_codex(&path, &plan(), &mut |_| true).is_err(),
        "another hash"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), stale);

    std::fs::write(&path, "a = 1\n").unwrap();
    let raced = trust::apply_codex(&path, &plan(), &mut |_| {
        std::fs::write(&path, "a = 2\n").unwrap();
        true
    });
    assert!(raced.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2\n");
}

#[test]
fn plan_reads_only_the_repos_two_hooks() {
    let hook = |key: &str, command: &str, hash: &str| json!({"key": key, "command": command, "currentHash": hash});
    let list = |hooks: Vec<Value>| json!({"result":{"data":[{"hooks": hooks}]}});
    let session = hook(
        "/<session-flags>/config.toml:session_start:0:0",
        "astrolabe hook session-start",
        "x",
    );
    let other = hook("/u/config.toml:session_start:0:0", "mine", "y");
    let start = hook(
        "/r/.codex/config.toml:session_start:0:0",
        "astrolabe hook session-start",
        HASH_A,
    );
    let guard = hook(
        "/r/.codex/config.toml:pre_tool_use:0:0",
        "astrolabe hook pre-command",
        HASH_B,
    );
    let good = list(vec![session.clone(), other, start.clone(), guard]);
    let plan = doctor::plan_from_list(&good, "/r").unwrap();
    assert_eq!(plan.project, "/r");
    assert_eq!(plan.hooks[0].1, HASH_A);
    assert_eq!(plan.hooks.len(), 2);
    assert!(doctor::plan_from_list(&list(vec![session, start]), "/r").is_err());
}

/// On the disk or not at all: what the model said doesn't reach this.
#[test]
fn verify_judges_from_the_disk() {
    let box_ = Sandbox::new();
    let cwd = box_.0.as_path();
    let real = cwd.to_string_lossy().into_owned();
    assert_eq!(doctor::judge(Some(&real), true, cwd), None);
    assert!(doctor::judge(None, true, cwd).unwrap().contains("None"));
    let sys32 = doctor::judge(Some(r"C:\Windows\System32"), true, cwd).unwrap();
    assert!(sys32.contains("System32"));
    assert!(
        doctor::judge(Some(&real), false, cwd)
            .unwrap()
            .contains("sh: false")
    );
}

#[test]
fn folder_check_catches_a_folder_that_cant_be_used() {
    let box_ = Sandbox::new();
    assert_eq!(doctor::folder_check(&box_.0), Ok(()));
    assert_eq!(
        std::fs::read_dir(&box_.0).unwrap().count(),
        1,
        "leaves only home/"
    );
    assert!(doctor::folder_check(&box_.0.join("missing")).is_err());
}

#[test]
fn daemon_lines_name_the_version_on_both_machines() {
    let list = "/usr/bin/other\n/h/.codex/packages/app-server-daemon/releases/0.166.0-aarch64-apple-darwin/bin/codex app-server daemon pid-update-loop\n/h/.codex/packages/app-server-daemon/releases/0.166.0-aarch64-apple-darwin/bin/codex-code-mode-host\nC:\\Users\\m\\.codex\\packages\\app-server-daemon\\releases\\0.164.0-x86_64-pc-windows-msvc\\bin\\codex.exe app-server --listen";
    let lines = doctor::daemon_lines(list);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"0.166.0\""));
    assert!(lines[1].contains("\"0.164.0\""));
}

#[test]
fn hooks_list_failure_says_retry_not_untrusted() {
    let box_ = Sandbox::new();
    let client = std::env::current_exe().unwrap();
    let error = astrolabe::driver::codex::list_in_home(&client, &box_.0, &box_.home());
    let error = error.unwrap_err().to_string();
    assert!(
        error.contains("says nothing about trust") && error.contains("retry"),
        "{error}"
    );
}

#[test]
fn registry_maps_a_name_to_this_machines_checkout() {
    let box_ = Sandbox::new();
    let machine = box_.machine();
    let registry = &machine.registry;
    assert!(registry.checkout("x").is_err());
    assert_eq!(registry.agent_repo().unwrap(), None);
    assert!(registry.add("x", &box_.0).is_err(), "not a repo");
    let (one, two) = (box_.repo("one"), box_.repo("two"));
    registry.add("x", &one).unwrap();
    assert_eq!(registry.checkout("x").unwrap(), one);
    registry.add("x", &two).unwrap();
    assert_eq!(registry.checkout("x").unwrap(), two);
    assert!(registry.provider_on("codex").unwrap());
    registry.set_provider("codex", false).unwrap();
    assert!(!registry.provider_on("codex").unwrap());
    registry.set_provider("codex", true).unwrap();
    assert!(registry.provider_on("codex").unwrap());
    registry.set_provider("codex", false).unwrap();
    assert!(registry.provider_on("claude").unwrap());
}

#[test]
fn the_registry_shares_a_database_with_the_engine() {
    let box_ = Sandbox::new();
    let machine = box_.machine();
    machine.registry.add("x", &box_.repo("one")).unwrap();
    let probe = astrolabe::OsProcessProbe;
    astrolabe::Engine::open(&machine.db(), astrolabe::SystemClock, probe).unwrap();
    let again = Registry::open(&machine.db()).unwrap();
    assert!(again.checkout("x").is_ok());
}

#[test]
fn a_product_launch_gets_a_worktree_the_whole_layer_and_its_profile() {
    let box_ = Sandbox::new();
    box_.fake_adapter();
    let machine = box_.machine();
    let agent = box_.0.join("agent");
    init::create(&machine.registry, &agent).unwrap();
    std::fs::create_dir_all(agent.join("knowledge/repos")).unwrap();
    std::fs::write(agent.join("knowledge/repos/prod.md"), "PROFILE-MARK").unwrap();
    let repo = box_.repo("prod");
    machine.registry.add("prod", &repo).unwrap();
    let client = PathBuf::from("/nowhere/claude");
    let request = Request {
        repo: "prod",
        provider: Provider::Claude,
        session: "s1",
        dials: DEFAULT_DIALS,
        client: &client,
    };
    let mut said = Vec::new();
    let ready = launch::prepare(&machine, &request, &mut |line| said.push(line)).unwrap();
    assert!(
        said.iter().any(|line| line.contains("not synced")),
        "no remote: {said:?}"
    );
    assert_eq!(ready.cwd, box_.0.join("prod.worktrees").join("s1"));
    assert!(ready.cwd.join(".git").exists());
    assert!(ready.launch.protection.is_some() && ready.launch.protection_required);
    assert!(ready.launch.folder_check.is_some());
    let options = &ready.launch.meta.as_ref().unwrap()["claudeCode"]["options"];
    assert!(options["settings"]["hooks"]["PreToolUse"].is_array());
    let text = machine.data_dir.join("instructions/s1/CLAUDE.md");
    let text = std::fs::read_to_string(text).unwrap();
    assert!(text.contains("PROFILE-MARK") && text.contains(DEFAULT_DIALS));

    assert!(
        launch::prepare(&machine, &request, &mut |_| {}).is_err(),
        "name taken"
    );
    machine.registry.set_provider("claude", false).unwrap();
    let error = launch::prepare(&machine, &request, &mut |_| {}).unwrap_err();
    assert!(error.to_string().contains("off"));
}

#[test]
fn the_agent_repo_runs_in_place_and_the_launch_adds_nothing() {
    let box_ = Sandbox::new();
    box_.fake_adapter();
    let machine = box_.machine();
    let agent = box_.0.join("agent");
    init::create(&machine.registry, &agent).unwrap();
    let client = PathBuf::from("/nowhere/claude");
    let request = Request {
        repo: AGENT,
        provider: Provider::Claude,
        session: "s1",
        dials: DEFAULT_DIALS,
        client: &client,
    };
    let ready = launch::prepare(&machine, &request, &mut |_| {}).unwrap();
    assert_eq!(ready.cwd, agent);
    assert!(ready.worktree.is_none());
    assert!(ready.launch.meta.is_none(), "{:?}", ready.launch.meta);
    assert!(ready.launch.protection.is_some(), "still checked");

    // Not the real layer, so it can't be counted on: the whole layer goes per launch.
    std::fs::write(agent.join(".mcp.json"), "PLACEHOLDER_SERVER").unwrap();
    let ready = launch::prepare(&machine, &request, &mut |_| {}).unwrap();
    assert!(ready.launch.meta.is_some());
}

#[test]
fn init_creates_and_clone_replaces_only_placeholders() {
    let box_ = Sandbox::new();
    let machine = box_.machine();
    let agent = box_.0.join("agent");
    init::create(&machine.registry, &agent).unwrap();
    assert!(
        init::layer_state(&agent)
            .iter()
            .all(|(_, state)| *state == Some(true))
    );
    assert!(agent.join("efforts/.gitkeep").is_file() && agent.join("AGENTS.md").is_file());
    assert_eq!(machine.registry.agent_repo().unwrap(), Some(agent.clone()));
    assert!(
        init::create(&machine.registry, &agent).is_err(),
        "not empty"
    );
    git(&agent, &["add", "."]);

    std::fs::write(agent.join(".mcp.json"), "PLACEHOLDER_SERVER").unwrap();
    std::fs::write(agent.join(".codex/config.toml"), "mine = true\n").unwrap();
    std::fs::remove_file(agent.join(".claude/settings.json")).unwrap();
    git(&agent, &["add", "."]);
    git(&agent, &["commit", "-qm", "as a second device finds it"]);
    let second = Sandbox::new();
    let machine = second.machine();
    let url = agent.to_string_lossy().into_owned();
    let path = second.0.join("copy").to_string_lossy().into_owned();
    init::clone(&machine.registry, &url, Some(&path)).unwrap();
    let copy = PathBuf::from(&path);
    let states = init::layer_state(&copy);
    assert_eq!(states[0], (".claude/settings.json", Some(true)));
    assert_eq!(states[1], (".mcp.json", Some(true)));
    assert_eq!(states[2], (".codex/config.toml", None));
    assert_eq!(
        std::fs::read_to_string(copy.join(".codex/config.toml")).unwrap(),
        "mine = true\n"
    );
    assert_eq!(machine.registry.agent_repo().unwrap(), Some(copy));
}

fn tree(root: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            tree(&path, out);
        } else {
            out.push((path.clone(), trust::file_hash(&path)));
        }
    }
    out.sort();
}

/// The binary, end to end: inspect leaves the homes as they were, and with no client apply
/// fails before it asks anything.
#[test]
fn the_cli_inspects_without_writing() {
    let box_ = Sandbox::new();
    std::fs::write(box_.homes().codex_config, "a = 1\n").unwrap();
    std::fs::write(box_.homes().claude_json, "{\"projects\":{}}").unwrap();
    let output = box_.cli(&["init", "agent"]).output().unwrap();
    assert!(output.status.success());
    let mut before = Vec::new();
    tree(&box_.home(), &mut before);
    let output = box_.cli(&["doctor", "inspect"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("codex project line: false, 0 hook lines"),
        "{text}"
    );
    assert!(text.contains("claude entry: None"), "{text}");
    let mut after = Vec::new();
    tree(&box_.home(), &mut after);
    assert_eq!(before, after);

    let empty = box_.0.join("no-path");
    std::fs::create_dir_all(&empty).unwrap();
    let mut apply = box_.cli(&["doctor", "apply"]);
    apply.env("PATH", &empty).env_remove("CODEX_PATH");
    let output = apply.output().unwrap();
    assert!(!output.status.success());
    let mut after = Vec::new();
    tree(&box_.home(), &mut after);
    assert_eq!(before, after);
}
