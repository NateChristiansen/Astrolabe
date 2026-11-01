mod common;

use astrolabe::driver::codex;
use common::TempDir;
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

fn adapters(dir: &TempDir, bin: Value) -> std::path::PathBuf {
    let root = dir.0.join("adapters with spaces");
    let package = root.join("node_modules/@agentclientprotocol/codex-acp");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        json!({"bin": bin}).to_string(),
    )
    .unwrap();
    fs::write(package.join("dist/index.js"), "// fixture, never executed").unwrap();
    root
}

fn config(launch: &astrolabe::acp::Launch) -> Value {
    let value = &launch
        .env
        .iter()
        .find(|(k, _)| k == "CODEX_CONFIG")
        .unwrap()
        .1;
    serde_json::from_str(value.to_str().unwrap()).unwrap()
}

#[test]
fn launches_node_directly_with_absolute_pinned_entry_and_cheap_defaults() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!({"codex-acp": "dist/index.js"}));
    let launch = codex::launch(&root, &json!({})).unwrap();
    assert_eq!(launch.command, OsStr::new("node"));
    assert_eq!(
        launch.args,
        [root.join("node_modules/@agentclientprotocol/codex-acp/dist/index.js")]
    );
    assert!(std::path::Path::new(&launch.args[0]).is_absolute());
    assert_eq!(
        config(&launch),
        json!({
            "model": "gpt-6.1-terra", "model_reasoning_effort": "low",
            "check_for_update_on_startup": false
        })
    );
    assert_eq!(launch.env.len(), 1);
    assert!(launch.meta.is_none());
}

#[test]
fn config_is_json_and_cannot_enable_update_checks() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!("dist/index.js"));
    let input = json!({"model": "test-model", "model_reasoning_effort": "medium",
        "check_for_update_on_startup": true, "test": {"text": "quotes \" and \\ and\nλ"}});
    let launch = codex::launch(&root, &input).unwrap();
    let mut expected = input.clone();
    expected["check_for_update_on_startup"] = json!(false);
    assert_eq!(config(&launch), expected);
    assert_eq!(input["check_for_update_on_startup"], true);
}

#[test]
fn retains_existing_login_home_on_all_platforms() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!("dist/index.js"));
    let launch = codex::launch(&root, &json!({})).unwrap();
    // No isolated home, auth copy, login invocation, or platform-specific shim.
    // In particular Windows gets no extra home component in its socket paths.
    assert!(
        launch
            .env
            .iter()
            .all(|(key, _)| key != "CODEX_HOME" && key != "HOME" && key != "USERPROFILE")
    );
    assert_eq!(launch.args.len(), 1);
    assert!(!dir.0.join("auth.json").exists());
}

#[test]
fn missing_adapter_is_reported_without_launching_anything() {
    let dir = TempDir::new();
    assert!(matches!(
        codex::launch(&dir.0, &json!({})),
        Err(astrolabe::Error::AdapterNotFound(_))
    ));
}

#[test]
fn rejects_non_object_config_instead_of_silently_discarding_it() {
    let dir = TempDir::new();
    for config in [json!(null), json!([]), json!("bad"), json!(false)] {
        assert!(
            matches!(codex::launch(&dir.0, &config), Err(astrolabe::Error::Io(error))
            if error.kind() == std::io::ErrorKind::InvalidInput)
        );
    }
}

fn example(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

fn proxy_run(flag: &str) -> (std::process::Output, Vec<Value>) {
    let dir = TempDir::new();
    let path = dir.0.join("wire.jsonl");
    fs::write(&path, "{\"kind\":\"parent\"}\n").unwrap();
    let mut child = Command::new(example("record_codex"))
        .args([
            std::ffi::OsStr::new("--proxy"),
            path.as_os_str(),
            example("fake_agent").as_os_str(),
            std::ffi::OsStr::new(flag),
        ])
        .current_dir(&dir.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n";
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let records: Vec<Value> = fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let sent = records
        .iter()
        .find(|r| r["direction"] == "to_agent")
        .unwrap();
    assert_eq!(sent["line"], std::str::from_utf8(input).unwrap());
    assert_eq!(sent["bytes"], json!(input.as_slice()));
    assert_eq!(records[0]["kind"], "parent");
    (output, records)
}

#[test]
fn recorder_proxy_preserves_raw_json_rpc_in_both_directions() {
    let (output, records) = proxy_run("--no-resume");
    assert!(output.status.success());
    let received = records
        .iter()
        .find(|r| r["direction"] == "from_agent")
        .unwrap();
    assert_eq!(
        received["line"],
        std::str::from_utf8(&output.stdout).unwrap()
    );
    assert_eq!(received["bytes"], json!(output.stdout));
    let message: Value = serde_json::from_str(received["line"].as_str().unwrap()).unwrap();
    assert_eq!(message["id"], 1);
    assert_eq!(message["result"]["protocolVersion"], 1);
    assert_eq!(records.last().unwrap()["code"], 0);
}

#[test]
fn recorder_proxy_preserves_stderr_and_nonzero_adapter_exit() {
    let (output, records) = proxy_run("--stderr-then-exit");
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"adapter cannot start\n");
    let stderr = records.iter().find(|r| r["direction"] == "stderr").unwrap();
    assert_eq!(stderr["bytes"], json!(output.stderr));
    assert_eq!(records.last().unwrap()["code"], 5);
}
