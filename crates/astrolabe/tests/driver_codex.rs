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
    let launch = codex::launch(&root, None, &json!({})).unwrap();
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
fn config_is_json_and_requests_update_checks_disabled() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!("dist/index.js"));
    let input = json!({"model": "test-model", "model_reasoning_effort": "medium",
        "check_for_update_on_startup": true, "test": {"text": "quotes \" and \\ and\nλ"}});
    let launch = codex::launch(&root, None, &input).unwrap();
    let mut expected = input.clone();
    expected["check_for_update_on_startup"] = json!(false);
    assert_eq!(config(&launch), expected);
    assert_eq!(input["check_for_update_on_startup"], true);
}

#[test]
fn launch_does_not_override_login_home() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!("dist/index.js"));
    let launch = codex::launch(&root, None, &json!({})).unwrap();
    // No isolated home, auth copy, login invocation, or platform-specific shim.
    // This checks launch construction, not Windows socket creation or length.
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
        codex::launch(&dir.0, None, &json!({})),
        Err(astrolabe::Error::AdapterNotFound(_))
    ));
}

#[test]
fn rejects_non_object_config_instead_of_silently_discarding_it() {
    let dir = TempDir::new();
    for config in [json!(null), json!([]), json!("bad"), json!(false)] {
        assert!(
            matches!(codex::launch(&dir.0, None, &config), Err(astrolabe::Error::Io(error))
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

fn fixture_path(scenario: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol/fixtures/acp/codex")
        .join(format!("{scenario}.jsonl"))
}

fn replay_spec(scenario: &str, process: u64) -> astrolabe::acp::Launch {
    let path = fixture_path(scenario);
    astrolabe::acp::Launch {
        command: example("fake_agent").into(),
        args: vec!["--replay".into(), path.into(), process.to_string().into()],
        ..Default::default()
    }
}

fn replay_event(client: &mut astrolabe::acp::AcpClient<'_>) -> astrolabe::acp::ClientEvent {
    use astrolabe::acp::ClientEvent;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Some(event) = client
            .next_event(std::time::Duration::from_millis(50))
            .unwrap()
        {
            assert!(
                matches!(
                    event,
                    ClientEvent::Ready
                        | ClientEvent::Update(_)
                        | ClientEvent::Alert(_)
                        | ClientEvent::TurnEnded { .. }
                        | ClientEvent::Exited { .. }
                ),
                "unexpected event: {event:?}"
            );
            return event;
        }
    }
    panic!("replay timed out");
}

fn recorded_scenario(scenario: &str) {
    use astrolabe::acp::{AcpClient, ClientEvent};
    use astrolabe::model::{Alert, Lifecycle, RunState};
    let h = common::Harness::new();
    let mut client = AcpClient::launch(&h.engine, &replay_spec(scenario, 0), &h.dir.0).unwrap();
    let session = client.session_id();
    let processes = if scenario == "resume" { 2 } else { 1 };
    for process in 0..processes {
        if process > 0 {
            client = AcpClient::resume(
                &h.engine,
                session,
                &replay_spec(scenario, process),
                &h.dir.0,
            )
            .unwrap();
        }
        // The real resume advertised both routes, chose session/resume, and sent
        // no history updates before its response. The fake checks the route.
        assert_eq!(replay_event(&mut client), ClientEvent::Ready);
        assert_eq!(client.agent_session_id(), "session-1");
        assert_eq!(
            h.engine.inspect(session).unwrap().run.state,
            RunState::Queued
        );
        let turns = if matches!(scenario, "two-turns" | "approval-reject") {
            2
        } else {
            1
        };
        for turn in 1..=turns {
            client.prompt(&format!("fixture prompt {turn}")).unwrap();
            let mut text = String::new();
            let mut approvals = 0;
            let mut tools = 0;
            let mut cancelled = false;
            loop {
                match replay_event(&mut client) {
                    ClientEvent::Alert(Alert::NeedsYou(request)) => {
                        approvals += 1;
                        assert_eq!(request.rpc_id, "0", "preserve numeric zero request id");
                        assert_eq!(request.options.len(), 3);
                        let kind = if scenario == "approval-reject" {
                            "reject_once"
                        } else {
                            "allow_once"
                        };
                        let option = request.options.iter().find(|o| o.kind == kind).unwrap();
                        assert!(client.answer(&request.id, &option.option_id).unwrap());
                        assert!(h.engine.request(&request.id).unwrap().unwrap().1);
                    }
                    ClientEvent::Update(message) => {
                        let update = &message["params"]["update"];
                        if update["sessionUpdate"] == "agent_message_chunk" {
                            text.push_str(update["content"]["text"].as_str().unwrap());
                        }
                        if update["sessionUpdate"] == "tool_call" {
                            tools += 1;
                            assert_eq!(update["status"], "in_progress");
                            assert_eq!(h.engine.inspect(session).unwrap().running_tools.len(), 1);
                            if scenario == "cancel-tool" {
                                client.cancel().unwrap();
                                cancelled = true;
                                // Stop is intent; the trace's stopReason is the evidence.
                                assert_ne!(
                                    h.engine.inspect(session).unwrap().run.state,
                                    RunState::Cancelled
                                );
                            }
                        }
                    }
                    ClientEvent::TurnEnded { stop_reason } => {
                        let stopped = scenario == "cancel-tool"
                            || (scenario == "approval-reject" && turn == 1);
                        assert_eq!(stop_reason, if stopped { "cancelled" } else { "end_turn" });
                        let view = h.engine.inspect(session).unwrap();
                        assert_eq!(
                            view.run.state,
                            if stopped {
                                RunState::Cancelled
                            } else {
                                RunState::Completed
                            }
                        );
                        assert!(view.running_tools.is_empty());
                        break;
                    }
                    event => panic!("unexpected turn event: {event:?}"),
                }
            }
            let expected = match (scenario, process, turn) {
                ("full-turn" | "two-turns", _, 1) => "FIRST",
                ("two-turns", _, 2) => "SECOND",
                ("approval-allow", _, _) => "FIXTURE_DONE",
                ("approval-reject", _, 2) => "REPLACEMENT",
                ("resume", 0, _) => "ACK",
                ("resume", 1, _) => "ORBIT",
                _ => "",
            };
            assert_eq!(text, expected);
            let approval = scenario.starts_with("approval-") && turn == 1;
            assert_eq!(approvals, usize::from(approval));
            assert_eq!(tools, usize::from(approval || scenario == "cancel-tool"));
            assert_eq!(cancelled, scenario == "cancel-tool");
        }
        let exit = replay_event(&mut client);
        let expected_stderr = if scenario == "cancel-tool" {
            ""
        } else {
            "AppServerUnavailableError: The Codex app-server exited with code 0. The agent is shutting down."
        };
        let ClientEvent::Exited { code, stderr } = exit else {
            panic!("expected replay exit, got {exit:?}");
        };
        assert_eq!(stderr, expected_stderr);
        let trace = fs::read_to_string(fixture_path(scenario)).unwrap();
        let recorded_exit = trace
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|row| row["process"] == process && row.get("exit_code").is_some());
        if let Some(row) = recorded_exit {
            assert_eq!(code, Some(row["exit_code"].as_i64().unwrap() as i32));
        } // Otherwise code is harness behavior, not recorded evidence.
        assert_eq!(
            h.engine.inspect(session).unwrap().session.lifecycle,
            Lifecycle::Closed
        );
        // Drop is deferred because resume replaces the client on the next iteration.
    }
    client.shutdown().unwrap();
}

#[test]
fn recorded_full_turn() {
    recorded_scenario("full-turn");
}
#[test]
fn recorded_allow_continues() {
    recorded_scenario("approval-allow");
}
#[test]
fn recorded_reject_then_replacement_continues_same_session() {
    recorded_scenario("approval-reject");
}
#[test]
fn recorded_two_turns() {
    recorded_scenario("two-turns");
}
#[test]
fn recorded_resume_in_new_process_without_history_replay() {
    recorded_scenario("resume");
}
#[test]
fn recorded_cancel_during_tool() {
    recorded_scenario("cancel-tool");
}

#[test]
fn explicit_client_path_is_passed_without_shell_quoting() {
    let dir = TempDir::new();
    let root = adapters(&dir, json!("dist/index.js"));
    let path = dir.0.join("client with spaces/codex.exe");
    let launch = codex::launch(&root, Some(&path), &json!({})).unwrap();
    assert_eq!(
        launch
            .env
            .iter()
            .find(|(k, _)| k == "CODEX_PATH")
            .unwrap()
            .1,
        path.as_os_str()
    );
}

#[test]
fn replay_rejects_trailing_client_writes_with_or_without_recorded_exit() {
    for exit_row in ["", "{\"process\":0,\"exit_code\":0}\n"] {
        let dir = TempDir::new();
        let trace = dir.0.join("trace.jsonl");
        fs::write(&trace, exit_row).unwrap();
        let mut child = Command::new(example("fake_agent"))
            .args([OsStr::new("--replay"), trace.as_os_str(), OsStr::new("0")])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"{}\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("extra client write")
        );
    }
}
