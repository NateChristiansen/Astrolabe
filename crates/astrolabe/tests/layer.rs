//! The composer, Claude's form of its output, and that the layer goes out on every route.
//! Codex's fixtures join these when its delivery lands.
mod common;

use astrolabe::acp::{AcpClient, ClientEvent};
use astrolabe::driver::claude;
use astrolabe::layer::*;
use common::{Harness, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const TEXT: &str = "Layer text.";

fn layer() -> Layer {
    Layer::standard(TEXT, &["Bash(touch:*)"])
}

fn product(supplies: RepoSupplies) -> Target {
    Target {
        kind: TargetKind::Product,
        supplies,
    }
}

/// An agent repo holding the whole layer, in its own config.
fn agent() -> Target {
    let layer = layer();
    Target {
        kind: TargetKind::Agent,
        supplies: RepoSupplies {
            instructions: Some("Repo text.".to_owned()),
            hooks: layer.hooks,
            mcp: layer.mcp,
            denies: layer.denies,
            memory_off: true,
        },
    }
}

#[test]
fn agent_repo_adds_nothing_it_supplies() {
    let delivery = compose(&agent(), &layer()).unwrap();
    assert_eq!(delivery, Delivery::default());
    let claude = claude::deliver(&delivery, Path::new("dir"));
    assert_eq!(claude.options, None);
    assert_eq!(claude.instructions, None);
}

#[test]
fn agent_repo_still_gets_what_it_lacks() {
    let mut target = agent();
    target.supplies.hooks.clear();
    target.supplies.denies.clear();
    let delivery = compose(&target, &layer()).unwrap();
    assert_eq!(delivery.hooks, layer().hooks);
    assert_eq!(delivery.denies, layer().denies);
    assert_eq!(delivery.instructions, None);
    assert_eq!(delivery.mcp, vec![]);
}

#[test]
fn product_repo_gets_the_whole_layer() {
    let delivery = compose(&product(RepoSupplies::default()), &layer()).unwrap();
    let layer = layer();
    assert_eq!(delivery.hooks, layer.hooks);
    assert_eq!(delivery.mcp, layer.mcp);
    assert_eq!(delivery.denies, layer.denies);
    assert!(delivery.memory_off);
    assert_eq!(delivery.instructions.unwrap().merged(), TEXT);
}

#[test]
fn no_duplicate_hooks() {
    // The repo has the same hook: a Codex launch would otherwise run it twice.
    let supplies = RepoSupplies {
        hooks: layer().hooks,
        ..RepoSupplies::default()
    };
    assert_eq!(compose(&product(supplies), &layer()).unwrap().hooks, vec![]);

    // The layer repeats itself.
    let mut twice = layer();
    twice.hooks.extend(layer().hooks);
    let delivery = compose(&product(RepoSupplies::default()), &twice).unwrap();
    assert_eq!(delivery.hooks, layer().hooks);
    let options = claude::deliver(&delivery, Path::new("dir"))
        .options
        .unwrap();
    assert_eq!(
        options["settings"]["hooks"]["SessionStart"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // A different command is another hook, which the repo keeps and the layer adds.
    let supplies = RepoSupplies {
        hooks: vec![Hook {
            event: HookEvent::SessionStart,
            command: "their-own-hook".to_owned(),
        }],
        ..RepoSupplies::default()
    };
    assert_eq!(
        compose(&product(supplies), &layer()).unwrap().hooks,
        layer().hooks
    );
}

#[test]
fn a_malformed_deny_fails_the_composition_on_every_target() {
    for rule in [
        "",
        " ",
        "Bash()",
        "Bash( )",
        "()",
        "(x)",
        "bash",
        "Bash(touch",
        "Bash(touch) ",
        "Bash(touch)x",
        "Bash(:*)",
        "Read(x:*)",
        "mcp__s__t(x)",
        r"Read(C:\\(x\\)\\y)",
    ] {
        let layer = Layer::standard(TEXT, &[rule]);
        for target in [agent(), product(RepoSupplies::default())] {
            let error = compose(&target, &layer).unwrap_err();
            assert!(error.to_string().contains("deny rule"), "{rule:?}: {error}");
        }
    }
    // The repo holding the same bad rule must not hide it.
    let bad = Layer::standard(TEXT, &["Bash()"]);
    let mut holding = agent();
    holding.supplies.denies = bad.denies.clone();
    assert!(compose(&holding, &bad).is_err());

    for rule in [
        "Bash",
        "Bash(touch:*)",
        "Bash(git push --force:*)",
        "Bash(npm *)",
        "Read(./secret.txt)",
        "Read(./a(b).txt)",
        "mcp__server",
        "mcp__server__tool",
    ] {
        let layer = Layer::standard(TEXT, &[rule]);
        assert!(
            compose(&product(RepoSupplies::default()), &layer).is_ok(),
            "{rule}"
        );
    }
}

#[test]
fn this_server_under_another_name_is_not_added_again() {
    let server = |name: &str, command: &str| McpServer {
        name: name.to_owned(),
        command: command.to_owned(),
        args: vec!["mcp".to_owned()],
        tools: vec![],
    };
    let mcp = |kind, theirs: McpServer| {
        let target = Target {
            kind,
            supplies: RepoSupplies {
                mcp: vec![theirs],
                ..RepoSupplies::default()
            },
        };
        compose(&target, &layer()).unwrap().mcp.len()
    };
    let alias = || server("astro", "astrolabe");
    assert_eq!(mcp(TargetKind::Product, alias()), 0);
    assert_eq!(mcp(TargetKind::Agent, alias()), 0);
    // Same name: on a product repo the launch's wins, so one runs either way.
    assert_eq!(mcp(TargetKind::Product, server(MCP_NAME, "astrolabe")), 1);
    assert_eq!(mcp(TargetKind::Agent, server(MCP_NAME, "astrolabe")), 0);
    // Another program under another name is another server.
    assert_eq!(mcp(TargetKind::Product, server("other", "other")), 1);
    assert_eq!(mcp(TargetKind::Agent, server("other", "other")), 1);
}

#[test]
fn hook_and_server_commands_are_stable_strings() {
    // A repo's config must hold these exact strings. Changing one loads a second copy on
    // both clients and drops Codex's hook trust.
    let layer = layer();
    assert_eq!(layer.hooks[0].command, "astrolabe hook session-start");
    assert_eq!(layer.mcp[0].command, "astrolabe");
    assert_eq!(layer.mcp[0].args, ["mcp"]);
    assert_eq!(layer.mcp[0].tools, ["layer_check"]);
}

#[test]
fn merged_instructions_keep_the_repos_and_never_repeat() {
    let merged = |repo: Option<&str>| {
        let supplies = RepoSupplies {
            instructions: repo.map(str::to_owned),
            ..RepoSupplies::default()
        };
        let delivery = compose(&product(supplies), &layer()).unwrap();
        delivery.instructions.unwrap().merged()
    };
    assert_eq!(merged(Some("Repo text.\n")), "Repo text.\n\nLayer text.");
    assert_eq!(merged(None), TEXT);
    assert_eq!(merged(Some("  \n")), TEXT);
    let already = "Repo text.\n\nLayer text.";
    assert_eq!(merged(Some(already)), already);
}

#[test]
fn claude_gets_only_the_layers_text_in_its_added_directory() {
    let supplies = RepoSupplies {
        instructions: Some("Repo text.".to_owned()),
        ..RepoSupplies::default()
    };
    let dir = TempDir::new();
    let instructions = dir.0.join("instructions");
    let delivery = compose(&product(supplies), &layer()).unwrap();
    let claude = claude::deliver(&delivery, &instructions);
    claude.write_instructions().unwrap();
    assert_eq!(
        std::fs::read_to_string(instructions.join("CLAUDE.md")).unwrap(),
        TEXT
    );
    let options = claude.options.unwrap();
    assert_eq!(options["additionalDirectories"], json!([instructions]));
    assert_eq!(
        options["env"]["CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD"],
        "1"
    );
}

#[test]
fn claude_options_carry_the_layer() {
    let delivery = compose(&product(RepoSupplies::default()), &layer()).unwrap();
    let options = claude::deliver(&delivery, Path::new("dir"))
        .options
        .unwrap();
    assert_eq!(
        options["settings"],
        json!({
            "hooks": { "SessionStart": [
                { "hooks": [{ "type": "command", "command": "astrolabe hook session-start" }] }
            ]},
            "permissions": {
                "deny": ["Bash(touch:*)"],
                "allow": ["mcp__astrolabe__layer_check"],
            },
            "autoMemoryEnabled": false,
        })
    );
    assert_eq!(
        options["mcpServers"],
        json!({ "astrolabe": { "type": "stdio", "command": "astrolabe", "args": ["mcp"] } })
    );
}

#[test]
fn launch_env_puts_this_astrolabe_first() {
    let inherited = std::env::join_paths(["one", "two"]).unwrap();
    let env = launch_env(Path::new("mine"), Path::new("data"), Some(inherited)).unwrap();
    let path = &env.iter().find(|(name, _)| name == "PATH").unwrap().1;
    let paths: Vec<PathBuf> = std::env::split_paths(path).collect();
    let want = ["mine", "one", "two"].map(PathBuf::from);
    assert_eq!(paths, want);
    assert!(
        env.iter()
            .any(|(name, value)| name == DATA_DIR_VAR && value == "data")
    );
}

fn opened(client: &mut AcpClient) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.next_event(Duration::from_millis(50)).unwrap() != Some(ClientEvent::Ready) {
        assert!(Instant::now() < deadline, "no Ready");
    }
}

/// D12 and D13 of probe P2: a deny does not outlast its process unless sent again.
#[test]
fn resume_sends_the_layer_again() {
    let h = Harness::new();
    let delivery = compose(&product(RepoSupplies::default()), &layer()).unwrap();
    let options = claude::deliver(&delivery, Path::new("dir"))
        .options
        .unwrap();

    let package = (h.dir.0).join("node_modules/@agentclientprotocol/claude-agent-acp");
    std::fs::create_dir_all(package.join("dist")).unwrap();
    let manifest = json!({ "bin": { "claude-agent-acp": "dist/index.js" } });
    std::fs::write(package.join("package.json"), manifest.to_string()).unwrap();
    std::fs::write(package.join("dist/index.js"), "").unwrap();
    let mut spec = claude::launch(&h.dir.0, None, Some(options.clone())).unwrap();
    let profile = std::env::current_exe().unwrap();
    let profile = profile.parent().unwrap().parent().unwrap();
    let fake = format!("fake_agent{}", std::env::consts::EXE_SUFFIX);
    spec.command = profile.join("examples").join(fake).into();
    spec.args = vec!["--dump-open".into()];
    // Shared-client route the Codex driver delivers its servers by.
    spec.mcp_servers = vec![json!({ "name": "astrolabe" })];

    let mut client = AcpClient::launch(&h.engine, &spec, &h.dir.0).unwrap();
    opened(&mut client);
    let session = client.session_id();
    client.shutdown().unwrap();
    let mut client = AcpClient::resume(&h.engine, session, &spec, &h.dir.0).unwrap();
    opened(&mut client);
    client.shutdown().unwrap();

    let opens = std::fs::read_to_string(h.dir.0.join("open.jsonl")).unwrap();
    let opens: Vec<Value> = opens
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let methods: Vec<_> = opens.iter().map(|open| &open["method"]).collect();
    assert_eq!(methods, ["session/new", "session/resume"]);
    for open in &opens {
        assert_eq!(open["params"]["_meta"]["claudeCode"]["options"], options);
        assert_eq!(
            open["params"]["mcpServers"],
            json!([{ "name": "astrolabe" }])
        );
    }
}
