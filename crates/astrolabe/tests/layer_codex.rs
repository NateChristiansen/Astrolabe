//! Synthetic hooks/list fixtures: platform key shapes from the probes, dummy hashes.
mod common;

use astrolabe::acp::{AcpClient, ClientEvent};
use astrolabe::driver::codex::{self, SessionFlagKey};
use astrolabe::layer::*;
use common::{Harness, TempDir};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn keys(windows: bool) -> Vec<SessionFlagKey> {
    let text = if windows {
        include_str!("fixtures/codex/hooks-windows.json")
    } else {
        include_str!("fixtures/codex/hooks-mac.json")
    };
    SessionFlagKey::from_hooks_list(&serde_json::from_str(text).unwrap()).unwrap()
}

fn product() -> Delivery {
    compose(
        &Target {
            kind: TargetKind::Product,
            supplies: RepoSupplies {
                instructions: Some("Repo word REPO-BEE.".into()),
                ..Default::default()
            },
        },
        &Layer::standard("Layer word LAYER-OWL.", &[]),
    )
    .unwrap()
}

#[test]
fn both_platform_keys_are_preserved_with_inline_trust_and_merged_instructions() {
    for windows in [false, true] {
        let keys = keys(windows);
        let delivered = codex::deliver(&product(), &keys).unwrap();
        let config = delivered.config;
        assert_eq!(
            config["developer_instructions"],
            "Repo word REPO-BEE.\n\nLayer word LAYER-OWL."
        );
        assert_eq!(
            config["hooks"]["state"],
            json!({ &keys[0].key: { "trusted_hash": keys[0].current_hash } })
        );
        assert_eq!(
            config["hooks"]["SessionStart"],
            json!([{ "hooks": [{ "type":"command", "command":"astrolabe hook session-start" }] }])
        );
        assert_eq!(config["features.hooks"], true);
        for name in [
            "features.memories",
            "memories.generate_memories",
            "memories.use_memories",
        ] {
            assert_eq!(config[name], false);
        }
        assert_eq!(
            config["mcp_servers.astrolabe.tools"]["layer_check"]["approval_mode"],
            "approve"
        );
        assert_eq!(
            delivered.mcp_servers,
            [json!({"name":"astrolabe", "command":"astrolabe", "args":["mcp"], "env":[]})]
        );
    }
}

#[test]
fn empty_and_fully_supplied_agent_deliver_nothing() {
    let layer = Layer::standard("Layer", &[]);
    let target = Target {
        kind: TargetKind::Agent,
        supplies: RepoSupplies {
            instructions: Some("Layer".into()),
            hooks: layer.hooks.clone(),
            mcp: layer.mcp.clone(),
            memory_off: true,
            ..Default::default()
        },
    };
    for delivery in [Delivery::default(), compose(&target, &layer).unwrap()] {
        let output = codex::deliver(&delivery, &[]).unwrap();
        assert_eq!(output.config, json!({}));
        assert!(output.mcp_servers.is_empty());
    }
}

#[test]
fn identical_repo_hook_is_not_delivered_or_trusted_twice() {
    let mut layer = Layer::standard("Layer", &[]);
    layer.hooks.extend(layer.hooks.clone());
    let target = Target {
        kind: TargetKind::Product,
        supplies: RepoSupplies {
            hooks: layer.hooks.clone(),
            ..Default::default()
        },
    };
    let output = codex::deliver(&compose(&target, &layer).unwrap(), &[]).unwrap();
    assert!(output.config.get("hooks").is_none());
    assert!(output.config.get("features.hooks").is_none());
}

#[test]
fn missing_ambiguous_or_wrong_declaration_cannot_silently_skip_trust() {
    assert!(codex::deliver(&product(), &[]).is_err());
    let mut keys = keys(false);
    keys.push(keys[0].clone());
    assert!(codex::deliver(&product(), &keys).is_err());
    keys.pop();
    keys[0].command.push_str(" different");
    assert!(codex::deliver(&product(), &keys).is_err());
    assert!(SessionFlagKey::from_hooks_list(&json!({"error": {"message":"failed"}})).is_err());
    let invalid = json!({"result":{"data":[{"hooks":[{"key":"/<session-flags>/config.toml:session_start:0:0", "command":"astrolabe hook session-start", "currentHash":""}]}]}});
    assert!(SessionFlagKey::from_hooks_list(&invalid).is_err());
}

#[test]
fn memory_keys_are_only_added_when_requested_and_tool_names_are_preserved() {
    let mut delivery = product();
    delivery.hooks.clear();
    delivery.memory_off = false;
    delivery.mcp[0].name = "server-name".into();
    delivery.mcp[0].tools = vec!["tool.with.dots".into()];
    let output = codex::deliver(&delivery, &[]).unwrap();
    assert!(
        !output
            .config
            .as_object()
            .unwrap()
            .keys()
            .any(|key| key.contains("memories"))
    );
    assert_eq!(
        output.config["mcp_servers.server-name.tools"]["tool.with.dots"]["approval_mode"],
        "approve"
    );
}

fn opened(client: &mut AcpClient) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while client.next_event(Duration::from_millis(50)).unwrap() != Some(ClientEvent::Ready) {
        assert!(Instant::now() < deadline, "no Ready");
    }
}

#[test]
fn codex_launch_and_resume_send_mcp_with_the_same_layer_config() {
    let h = Harness::new();
    let adapters = TempDir::new();
    let package = adapters
        .0
        .join("node_modules/@agentclientprotocol/codex-acp");
    std::fs::create_dir_all(package.join("dist")).unwrap();
    std::fs::write(package.join("package.json"), r#"{"bin":"dist/index.js"}"#).unwrap();
    std::fs::write(package.join("dist/index.js"), "").unwrap();
    let delivered = codex::deliver(&product(), &keys(false)).unwrap();
    let mut spec = delivered.launch(&adapters.0, None).unwrap();
    let config = spec
        .env
        .iter()
        .find(|(name, _)| name == "CODEX_CONFIG")
        .unwrap();
    let config: Value = serde_json::from_str(config.1.to_str().unwrap()).unwrap();
    for (key, value) in delivered.config.as_object().unwrap() {
        assert_eq!(&config[key], value);
    }
    let profile = std::env::current_exe().unwrap();
    spec.command = profile
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join(format!("fake_agent{}", std::env::consts::EXE_SUFFIX))
        .into();
    spec.args = vec!["--dump-open".into()];
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
    assert_eq!(
        opens
            .iter()
            .map(|open| open["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["session/new", "session/resume"]
    );
    for open in opens {
        assert_eq!(open["params"]["mcpServers"], json!(delivered.mcp_servers));
    }
}
