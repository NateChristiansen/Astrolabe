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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

const HASH_A: &str = "sha256:aaaa";
const HASH_B: &str = "sha256:bbbb";

struct Sandbox(PathBuf);

/// The homes as they were when this process started, then every git this process or the
/// library starts gets none of the owner's config or hooks.
fn real_homes() -> &'static Vec<(&'static str, std::ffi::OsString)> {
    static REAL: OnceLock<Vec<(&'static str, std::ffi::OsString)>> = OnceLock::new();
    REAL.get_or_init(|| {
        let names = ["HOME", "USERPROFILE", "CODEX_HOME", "CLAUDE_CONFIG_DIR"];
        let real = names
            .iter()
            .filter_map(|n| Some((*n, std::env::var_os(n)?)))
            .collect();
        let home = std::env::temp_dir().join(format!("astrolabe-setup-git-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        // SAFETY: under the OnceLock, before this process starts any child, and every other
        // reader goes through std's environment lock.
        unsafe {
            for name in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME"] {
                std::env::set_var(name, &home);
            }
            let temp = std::env::temp_dir();
            let ceiling = [temp.clone(), temp.canonicalize().unwrap_or(temp)];
            std::env::set_var(
                "GIT_CEILING_DIRECTORIES",
                std::env::join_paths(ceiling).unwrap(),
            );
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            std::env::set_var("GIT_CONFIG_COUNT", "1");
            std::env::set_var("GIT_CONFIG_KEY_0", "core.hooksPath");
            std::env::set_var("GIT_CONFIG_VALUE_0", "/dev/null");
            for name in ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"] {
                std::env::set_var(name, "T");
            }
            for name in ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"] {
                std::env::set_var(name, "t@example.com");
            }
        }
        real
    })
}

impl Sandbox {
    fn new() -> Self {
        real_homes();
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
        for (name, real) in real_homes() {
            assert_ne!(self.home(), PathBuf::from(real), "{name} is the sandbox");
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
    let printed = format!("{real}\n");
    assert_eq!(doctor::judge(Some(&real), Some(&printed), cwd), None);
    assert!(doctor::judge(None, Some(&printed), cwd).is_some());
    let sys32 = doctor::judge(Some(r"C:\Windows\System32"), Some(&printed), cwd).unwrap();
    assert!(sys32.contains("System32"));
    assert!(
        doctor::judge(Some(&real), None, cwd).is_some(),
        "sh wrote nothing"
    );
    assert!(
        doctor::judge(Some(&real), Some(""), cwd).is_some(),
        "touch, not the command"
    );
    assert!(doctor::judge(Some(&real), Some("DONE"), cwd).is_some());
    assert!(doctor::judge(Some(&real), Some("/somewhere/else"), cwd).is_some());
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

#[test]
fn approval_is_an_exact_match_on_the_raw_input() {
    let token = "astrolabe-verify-1";
    let wanted = doctor::commands(token);
    assert!(doctor::allowed(
        Some(&json!({"command": wanted[0]})),
        &wanted
    ));
    assert!(doctor::allowed(
        Some(&json!({"command": wanted[1]})),
        &wanted
    ));
    for bad in [
        format!("echo {token}; rm -rf /"),
        format!("{} && curl x", wanted[1]),
        format!(" {}", wanted[0]),
        token.to_owned(),
    ] {
        assert!(
            !doctor::allowed(Some(&json!({"command": bad})), &wanted),
            "{bad}"
        );
    }
    assert!(
        !doctor::allowed(None, &wanted),
        "a title alone is never enough"
    );
    assert!(!doctor::allowed(
        Some(&json!({"command": [&wanted[1]]})),
        &wanted
    ));
    assert!(!doctor::allowed(
        Some(&json!({"title": wanted[1]})),
        &wanted
    ));
}

#[test]
fn raw_input_comes_from_the_request_or_the_tool_call_it_names() {
    use astrolabe::acp::requested_input;
    let mut known = std::collections::HashMap::new();
    known.insert("s1/c1".to_owned(), json!({"command": "from update"}));
    let other = json!({"params": {"sessionId": "s2", "toolCall": {"toolCallId": "c1"}}});
    assert_eq!(requested_input(&other, &known), None);
    let ask = |call: Value| json!({"params": {"sessionId": "s1", "toolCall": call}});
    let own = ask(json!({"toolCallId": "c1", "title": "t", "rawInput": {"command": "own"}}));
    assert_eq!(
        requested_input(&own, &known),
        Some(json!({"command": "own"}))
    );
    let named = ask(json!({"toolCallId": "c1", "title": "own", "rawInput": null}));
    assert_eq!(
        requested_input(&named, &known),
        Some(json!({"command": "from update"}))
    );
    let unknown = ask(json!({"toolCallId": "c2", "title": "own"}));
    assert_eq!(requested_input(&unknown, &known), None);
}

#[test]
fn removal_refuses_a_file_changed_during_the_prompt() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    std::fs::write(&path, "a = 1\n").unwrap();
    trust::apply_codex(&path, &plan(), &mut |_| true).unwrap();
    let edited = std::fs::read_to_string(&path).unwrap() + "edit = 1\n";
    let said = trust::remove_codex(&path, &mut |_| {
        std::fs::write(&path, &edited).unwrap();
        true
    });
    assert!(said.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}

#[test]
fn a_look_alike_marker_line_above_the_block_survives_removal() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    let text = "# BEGIN astrolabe before=old\nmodel = \"x\"\n";
    std::fs::write(&path, text).unwrap();
    trust::apply_codex(&path, &plan(), &mut |_| true).unwrap();
    let said = trust::remove_codex(&path, &mut |_| true).unwrap();
    assert!(said.contains("Byte-identical to before: true"), "{said}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn marker_text_inside_a_key_is_not_a_marker() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    std::fs::write(&path, "a = 1\n").unwrap();
    let plan = CodexPlan {
        project: "/tmp/# END astrolabe/# BEGIN astrolabe before=x/agent".to_owned(),
        hooks: vec![("/tmp/# END astrolabe/k".to_owned(), HASH_A.to_owned())],
    };
    trust::apply_codex(&path, &plan, &mut |_| true).unwrap();
    let said = trust::remove_codex(&path, &mut |_| true).unwrap();
    assert!(said.contains("Byte-identical to before: true"), "{said}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
    let open = "# BEGIN astrolabe before=x\n[a]\n#  END astrolabe\n";
    assert_eq!(trust::find_block(open), None);
}

#[test]
fn apply_refuses_any_trace_of_its_keys() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    for held in [
        "[projects.\"/r/agent\"]\ntrust_level = \"untrusted\"\n",
        "[projects.\"/r/agent\"]\n",
        "[projects.'/r/agent']\n",
        "projects = { \"/r/agent\" = { trust_level = \"trusted\" } }\n",
        "[projects]\n\"/r/agent\" = { trust_level = \"trusted\" }\n",
        "[hooks.state.\"/r/agent/.codex/config.toml:pre_tool_use:0:0\"]\n",
        "[hooks.state.\"/r/agent/.codex/config.toml:session_start:0:0\"]\ntrusted_hash = \"old\"\n",
        "\u{feff}[projects.\"/r/agent\"]\ntrust_level = \"trusted\"\n",
        "[projects.\"/r/\\u0061gent\"]\ntrust_level = \"trusted\"\n",
        "[projects.\"/r/\\U00000061gent\"]\n",
        "[PROJECTS]\n# /R/AGENT\n",
    ] {
        std::fs::write(&path, held).unwrap();
        let said = trust::apply_codex(&path, &plan(), &mut |_| panic!("asked: {held}"));
        assert!(said.is_err(), "{held}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), held);
    }
    for unrelated in [
        "[projects.\"/r/agent2\"]\n",
        "[projects.\"/r/agent-old\"]\n",
        "[projects.\"C:\\\\Users\\\\me\"]\n",
        "[projects.'c:\\users\\me']\n",
    ] {
        std::fs::write(&path, unrelated).unwrap();
        let said = trust::apply_codex(&path, &plan(), &mut |_| false).unwrap();
        assert!(said.contains("nothing written"), "{unrelated}: {said}");
    }
}

#[test]
fn removal_will_not_write_through_a_planted_temporary_file() {
    let box_ = Sandbox::new();
    let path = box_.homes().codex_config;
    std::fs::write(&path, "a = 1\n").unwrap();
    trust::apply_codex(&path, &plan(), &mut |_| true).unwrap();
    let applied = std::fs::read(&path).unwrap();
    let aside = path.with_extension(format!("astrolabe-{}.tmp", std::process::id()));
    std::fs::write(&aside, "planted").unwrap();
    assert!(trust::remove_codex(&path, &mut |_| true).is_err());
    assert_eq!(std::fs::read_to_string(&aside).unwrap(), "planted");
    assert_eq!(std::fs::read(&path).unwrap(), applied);
    std::fs::remove_file(&aside).unwrap();
    #[cfg(unix)]
    {
        let victim = box_.0.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, &aside).unwrap();
        assert!(trust::remove_codex(&path, &mut |_| true).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }
}

/// A committed `developer_instructions`, deleted from the checkout: the worktree has it, so
/// the launch must see it there. This client lists no hooks, then the driver refuses.
#[cfg(unix)]
#[test]
fn instructions_are_read_from_the_worktree() {
    use std::os::unix::fs::PermissionsExt;
    let box_ = Sandbox::new();
    box_.fake_adapter();
    let machine = box_.machine();
    let repo = box_.repo("prod");
    std::fs::create_dir_all(repo.join(".codex")).unwrap();
    std::fs::write(
        repo.join(".codex/config.toml"),
        "developer_instructions = \"x\"\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "own instructions"]);
    std::fs::remove_file(repo.join(".codex/config.toml")).unwrap();
    machine.registry.add("prod", &repo).unwrap();
    let client = box_.0.join("fake-codex");
    let script = "#!/bin/sh\nread a\necho '{\"id\":1,\"result\":{}}'\nread a\nread a\necho '{\"id\":2,\"result\":{\"data\":[{\"hooks\":[]}]}}'\nread a\n";
    std::fs::write(&client, script).unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o755)).unwrap();
    let request = Request {
        repo: "prod",
        provider: Provider::Codex,
        session: "s1",
        dials: DEFAULT_DIALS,
        client: &client,
    };
    let error = launch::prepare(&machine, &request, &mut |_| {})
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("state effective repo instructions"),
        "{error}"
    );
}

#[test]
fn a_session_name_is_checked_on_every_path() {
    let box_ = Sandbox::new();
    box_.fake_adapter();
    let machine = box_.machine();
    init::create(&machine.registry, &box_.0.join("agent")).unwrap();
    std::fs::write(box_.0.join("agent/.mcp.json"), "PLACEHOLDER_SERVER").unwrap();
    let repo = box_.repo("prod");
    machine.registry.add("prod", &repo).unwrap();
    let client = PathBuf::from("/nowhere/claude");
    for name in ["../../escape", "/abs", "a/b", "..", ""] {
        for repo in [AGENT, "prod"] {
            let request = Request {
                repo,
                provider: Provider::Claude,
                session: name,
                dials: DEFAULT_DIALS,
                client: &client,
            };
            assert!(
                launch::prepare(&machine, &request, &mut |_| {}).is_err(),
                "{name} {repo}"
            );
        }
    }
    assert!(!box_.0.join("escape").exists() && !box_.0.join("data/instructions").exists());
}

#[test]
fn init_commits_without_the_owners_hooks() {
    let box_ = Sandbox::new();
    let hooks = box_.0.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let probe = box_.0.join("probe");
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", probe.display())).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let config = format!(
        "[core]\n\thooksPath = {}\n[user]\n\tname = Owner\n\temail = o@example.com\n",
        hooks.display().to_string().replace('\\', "/")
    );
    std::fs::write(box_.home().join(".gitconfig"), config).unwrap();
    let mut init = box_.cli(&["init", "agent"]);
    for name in ["GIT_CONFIG_COUNT", "GIT_CONFIG_GLOBAL", "XDG_CONFIG_HOME"] {
        init.env_remove(name);
    }
    for name in ["AUTHOR", "COMMITTER"] {
        init.env_remove(format!("GIT_{name}_NAME"))
            .env_remove(format!("GIT_{name}_EMAIL"));
    }
    let output = init.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!probe.exists(), "the owner's hook ran");
    let log = git(&box_.0.join("agent"), &["log", "--format=%an"]);
    assert_eq!(log.trim(), "Owner", "{output:?}");
}

#[test]
fn verbatim_prefixes_keep_a_real_path() {
    assert_eq!(trust::strip_verbatim(r"\\?\C:\x"), r"C:\x");
    assert_eq!(
        trust::strip_verbatim(r"\\?\UNC\host\share\x"),
        r"\\host\share\x"
    );
    assert_eq!(trust::strip_verbatim("/x"), "/x");
}

#[test]
fn the_native_binary_is_found_behind_a_shim() {
    let box_ = Sandbox::new();
    let file = |path: &Path, text: &str| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    // As npm on Windows lays it out, and as the effort note names it.
    let npm = box_.0.join("npm");
    let shim = npm.join("codex.cmd");
    file(&shim, "@echo off");
    let exe = npm.join("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex/codex.exe");
    file(&exe, "MZ native");
    let found = doctor::native_behind(&shim, "codex.exe").unwrap();
    assert_eq!(
        std::fs::canonicalize(found).unwrap(),
        std::fs::canonicalize(&exe).unwrap()
    );

    // No match, or a script under that name, isn't a binary.
    let bare = box_.0.join("bare");
    file(&bare.join("codex.cmd"), "@echo off");
    assert_eq!(
        doctor::native_behind(&bare.join("codex.cmd"), "codex.exe"),
        None
    );

    #[cfg(unix)]
    for platform in [
        "codex/node_modules/@openai/codex-darwin-arm64",
        "codex-darwin-arm64",
    ] {
        // As Homebrew's npm on the Mac: a symlink to the script, the binary nested or beside.
        let lib = box_.0.join(format!("lib-{}", platform.len()));
        let script = lib.join("node_modules/@openai/codex/bin/codex.js");
        file(&script, "#!/usr/bin/env node");
        let native = lib.join("node_modules/@openai").join(platform);
        let native = native.join("vendor/aarch64-apple-darwin/bin/codex");
        file(&native, "\u{7f}ELF");
        let link = lib.join("bin/codex");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&script, &link).unwrap();
        let found = doctor::native_behind(&link, "codex").unwrap();
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(&native).unwrap()
        );
    }
}

#[test]
fn a_hand_set_client_path_wins_and_inspect_says_a_shim_is_one() {
    let box_ = Sandbox::new();
    let shim = box_.0.join("my-codex");
    std::fs::write(&shim, "#!/bin/sh\necho no\n").unwrap();
    let mut inspect = box_.cli(&["doctor", "inspect"]);
    let output = inspect.env("CODEX_PATH", &shim).output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(&format!("codex: {}", shim.display())),
        "{text}"
    );
    assert!(text.contains("not native"), "{text}");
}

#[test]
fn remove_prints_the_exact_claude_key() {
    let box_ = Sandbox::new();
    let machine = box_.machine();
    let agent = box_.0.join("agent");
    init::create(&machine.registry, &agent).unwrap();
    std::fs::write(box_.homes().codex_config, "a = 1\n").unwrap();
    let key = trust::keys(&trust::native(&agent), cfg!(windows)).1;
    let text = json!({"projects": {key.clone(): {"hasTrustDialogAccepted": true}}}).to_string();
    std::fs::write(box_.homes().claude_json, &text).unwrap();
    let lines = doctor::remove(&machine, &mut |_| true).unwrap();
    assert!(lines[1].contains(&format!("{key:?}")), "{lines:?}");
    assert_eq!(
        std::fs::read_to_string(box_.homes().claude_json).unwrap(),
        text
    );
}
