//! The Claude driver, and recordings of the real adapter played back through the real
//! client. No provider or network. `replay_agent` fails the run if the client says
//! anything the real session did not hear. Re-record with `examples/record_claude`,
//! then scrub as `protocol/fixtures/acp/claude/README.md` says.
mod common;

use astrolabe::acp::{AcpClient, ClientEvent, Launch};
use astrolabe::driver::claude;
use astrolabe::model::{Alert, Lifecycle, Request, RunState, ToolStatus};
use astrolabe::{Error, STALL_AFTER_MS};
use common::{Harness, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PATIENCE: Duration = Duration::from_secs(20);

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures/acp/claude")
}

/// The options the recordings were made with.
fn recorded_options() -> Value {
    json!({ "promptSuggestions": false, "settingSources": [] })
}

/// An adapters directory with just enough of the real package's shape.
fn adapters(dir: &TempDir) -> PathBuf {
    let package = dir
        .0
        .join("node_modules/@agentclientprotocol/claude-agent-acp");
    std::fs::create_dir_all(package.join("dist")).unwrap();
    let manifest = json!({ "bin": { "claude-agent-acp": "dist/index.js" } });
    std::fs::write(package.join("package.json"), manifest.to_string()).unwrap();
    std::fs::write(package.join("dist/index.js"), "").unwrap();
    dir.0.clone()
}

/// The real driver's launch, pointed at a recording instead of the adapter. Its
/// `_meta` still goes out, and the replay checks it against what the adapter got.
fn replaying(h: &Harness, recording: &str) -> Launch {
    let mut spec = claude::launch(&adapters(&h.dir), None, Some(recorded_options())).unwrap();
    let profile = std::env::current_exe().unwrap();
    let profile = profile.parent().unwrap().parent().unwrap();
    let name = format!("replay_agent{}", std::env::consts::EXE_SUFFIX);
    spec.command = profile.join("examples").join(name).into();
    spec.args = vec![fixtures().join(recording).into()];
    spec
}

fn until(client: &mut AcpClient, done: impl Fn(&ClientEvent) -> bool) -> Vec<ClientEvent> {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = client.next_event(Duration::from_millis(100)).unwrap() {
            let finished = done(&event);
            let early_exit = matches!(event, ClientEvent::Exited { .. }) && !finished;
            seen.push(event);
            assert!(!early_exit, "the replay ended early: {seen:?}");
            if finished {
                return seen;
            }
        }
    }
    panic!("hung; events so far: {seen:?}");
}

fn start<'e>(h: &'e Harness, recording: &str) -> AcpClient<'e> {
    let spec = replaying(h, recording);
    let mut client = AcpClient::launch(&h.engine, &spec, &h.dir.0).unwrap();
    until(&mut client, |event| matches!(event, ClientEvent::Ready));
    client
}

/// Closes the session, then holds the replay to the whole recording: its exit code
/// goes unread here, so a client that stopped short or said more leaves a file.
fn finish(client: AcpClient, h: &Harness) {
    client.shutdown().unwrap();
    replay_was_clean(h);
}

fn replay_was_clean(h: &Harness) {
    let mismatch = std::fs::read_to_string(h.dir.0.join("replay.mismatch"));
    assert!(mismatch.is_err(), "replay: {mismatch:?}");
}

/// A recorded turn must be one the client understood: nothing refused or unreadable.
fn turn(client: &mut AcpClient) -> Vec<ClientEvent> {
    let events = until(client, |event| {
        matches!(event, ClientEvent::TurnEnded { .. })
    });
    for event in &events {
        assert!(
            !matches!(
                event,
                ClientEvent::Malformed { .. }
                    | ClientEvent::UnsupportedRequest { .. }
                    | ClientEvent::AgentError { .. }
                    | ClientEvent::PromptError { .. }
            ),
            "{event:?}"
        );
    }
    events
}

fn stop_reason(events: &[ClientEvent]) -> &str {
    match events.last() {
        Some(ClientEvent::TurnEnded { stop_reason }) => stop_reason,
        other => panic!("no turn end: {other:?}"),
    }
}

fn reply(events: &[ClientEvent]) -> String {
    let chunk = |event: &ClientEvent| match event {
        ClientEvent::Update(message)
            if message["params"]["update"]["sessionUpdate"] == "agent_message_chunk" =>
        {
            message["params"]["update"]["content"]["text"]
                .as_str()
                .map(str::to_owned)
        }
        _ => None,
    };
    events.iter().filter_map(chunk).collect()
}

fn next_request(client: &mut AcpClient) -> Request {
    let is_request = |event: &ClientEvent| matches!(event, ClientEvent::Alert(Alert::NeedsYou(_)));
    match until(client, is_request).pop() {
        Some(ClientEvent::Alert(Alert::NeedsYou(request))) => request,
        other => panic!("expected a request, got {other:?}"),
    }
}

/// Every status the recording gave one tool, in order.
fn statuses(events: &[ClientEvent]) -> Vec<String> {
    let status = |event: &ClientEvent| match event {
        ClientEvent::Update(message) => message["params"]["update"]["status"]
            .as_str()
            .map(str::to_owned),
        _ => None,
    };
    events.iter().filter_map(status).collect()
}

#[test]
fn the_driver_runs_the_pinned_adapter_with_updates_off() {
    let dir = TempDir::new();
    let spec = claude::launch(&adapters(&dir), None, None).unwrap();
    assert_eq!(spec.command, "node");
    let entry = Path::new(&spec.args[0]);
    assert!(entry.is_absolute() && entry.ends_with("dist/index.js"));
    let off = spec
        .env
        .iter()
        .find(|(name, _)| name == "DISABLE_AUTOUPDATER");
    assert_eq!(off.map(|(_, value)| value.to_str().unwrap()), Some("1"));
    assert_eq!(spec.meta, None, "nothing to deliver means no _meta at all");
}

#[test]
fn a_given_client_path_is_the_one_the_adapter_runs() {
    let dir = TempDir::new();
    let client = dir.0.join("claude");
    let spec = claude::launch(&adapters(&dir), Some(&client), None).unwrap();
    let set = spec
        .env
        .iter()
        .find(|(name, _)| name == "CLAUDE_CODE_EXECUTABLE");
    assert_eq!(set.map(|(_, value)| Path::new(value)), Some(&*client));
    let spec = claude::launch(&adapters(&dir), None, None).unwrap();
    assert!(
        !spec
            .env
            .iter()
            .any(|(name, _)| name == "CLAUDE_CODE_EXECUTABLE")
    );
}

/// The replay is only evidence if it notices. Each client here goes wrong a
/// different way, and each must leave the mark `finish` looks for.
#[test]
fn the_replay_catches_a_client_that_differs() {
    // Says more than the recording.
    let h = Harness::new();
    let mut client = start(&h, "full_turn.jsonl");
    client.prompt("Reply with just OK.").unwrap();
    turn(&mut client);
    client.prompt("Reply with just OK.").unwrap();
    until(&mut client, |event| {
        matches!(event, ClientEvent::Exited { .. })
    });
    assert!(h.dir.0.join("replay.mismatch").exists(), "extra prompt");

    // Stops short of it.
    let h = Harness::new();
    let client = start(&h, "full_turn.jsonl");
    client.shutdown().unwrap();
    assert!(h.dir.0.join("replay.mismatch").exists(), "no prompt");

    // Says something else.
    let h = Harness::new();
    let mut client = start(&h, "full_turn.jsonl");
    client.prompt("Something else.").unwrap();
    until(&mut client, |event| {
        matches!(event, ClientEvent::Exited { .. })
    });
    assert!(h.dir.0.join("replay.mismatch").exists(), "other prompt");
}

#[test]
fn delivery_options_travel_under_claude_code_options() {
    let dir = TempDir::new();
    let options = json!({ "settingSources": ["project"] });
    let spec = claude::launch(&adapters(&dir), None, Some(options.clone())).unwrap();
    assert_eq!(
        spec.meta,
        Some(json!({ "claudeCode": { "options": options } }))
    );
}

#[test]
fn a_missing_adapter_is_named_not_a_failed_launch_later() {
    let dir = TempDir::new();
    let error = claude::launch(&dir.0, None, None).unwrap_err();
    assert!(matches!(error, Error::AdapterNotFound(_)), "{error:?}");
}

#[test]
fn a_full_turn() {
    let h = Harness::new();
    let mut client = start(&h, "full_turn.jsonl");
    let session = client.session_id();
    assert_eq!(client.agent_session_id(), "id-1");
    client.prompt("Reply with just OK.").unwrap();
    let events = turn(&mut client);
    assert_eq!(stop_reason(&events), "end_turn");
    assert_eq!(reply(&events), "OK");
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Completed
    );
    finish(client, &h);
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.session.lifecycle, Lifecycle::Closed);
}

const APPROVE: &str =
    "Use the Bash tool to run exactly: touch approved.txt\nThen reply with just DONE.";

#[test]
fn approval_allowed_and_the_turn_continues() {
    let h = Harness::new();
    let mut client = start(&h, "approve.jsonl");
    client.prompt(APPROVE).unwrap();
    let request = next_request(&mut client);
    assert_eq!(request.title, "touch approved.txt");
    assert_eq!(
        request.rpc_id, "0",
        "the adapter numbers its requests from zero"
    );
    assert!(client.answer(&request.id, "allow-once").unwrap());
    let events = turn(&mut client);
    assert_eq!(statuses(&events), ["completed"]);
    assert_eq!(reply(&events), "DONE");
    let view = h.engine.inspect(client.session_id()).unwrap();
    assert_eq!(view.run.state, RunState::Completed);
    assert!(view.open_requests.is_empty() && view.running_tools.is_empty());
    finish(client, &h);
}

#[test]
fn approval_rejected_fails_the_tool_but_not_the_turn() {
    let h = Harness::new();
    let mut client = start(&h, "reject.jsonl");
    let prompt = "Use the Bash tool to run exactly: touch rejected.txt\nIf it is refused, reply with just REFUSED.";
    client.prompt(prompt).unwrap();
    let request = next_request(&mut client);
    let reject = request.reject_option().unwrap().to_owned();
    assert!(client.answer(&request.id, &reject).unwrap());
    let events = turn(&mut client);
    assert_eq!(statuses(&events), ["failed"]);
    assert_eq!(stop_reason(&events), "end_turn");
    assert_eq!(reply(&events), "REFUSED");
    finish(client, &h);
}

#[test]
fn two_turns_in_one_process() {
    let h = Harness::new();
    let mut client = start(&h, "two_turns.jsonl");
    client.prompt("Reply with just ONE.").unwrap();
    assert_eq!(stop_reason(&turn(&mut client)), "end_turn");
    client.prompt("Reply with just TWO.").unwrap();
    assert_eq!(stop_reason(&turn(&mut client)), "end_turn");
    finish(client, &h);
}

#[test]
fn resume_in_a_new_process_keeps_context_and_replays_nothing() {
    let h = Harness::new();
    let mut client = start(&h, "resume.jsonl");
    let session = client.session_id();
    let agent_session = client.agent_session_id().to_owned();
    client
        .prompt("The word is OWL. Reply with just OK.")
        .unwrap();
    assert_eq!(stop_reason(&turn(&mut client)), "end_turn");
    finish(client, &h);

    let spec = replaying(&h, "resume.2.jsonl");
    let mut client = AcpClient::resume(&h.engine, session, &spec, &h.dir.0).unwrap();
    let opened = until(&mut client, |event| matches!(event, ClientEvent::Ready));
    assert_eq!(reply(&opened), "", "session/resume sends no history");
    assert_eq!(client.agent_session_id(), agent_session);
    client
        .prompt("What was the word? Reply with just the word.")
        .unwrap();
    assert_eq!(reply(&turn(&mut client)), "OWL");
    finish(client, &h);
}

#[test]
fn cancel_during_a_tool_ends_on_the_agents_word() {
    let h = Harness::new();
    let mut client = start(&h, "cancel.jsonl");
    let prompt = "Use the Bash tool to run exactly: python3 -c \"import time; time.sleep(47)\"\nThen reply DONE.";
    client.prompt(prompt).unwrap();
    let request = next_request(&mut client);
    assert!(client.answer(&request.id, "allow-once").unwrap());
    client.cancel().unwrap();
    let session = client.session_id();
    let events = turn(&mut client);
    assert_eq!(stop_reason(&events), "cancelled");
    assert_eq!(statuses(&events), ["failed"]);
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Cancelled);
    assert!(view.running_tools.is_empty());
    finish(client, &h);
}

/// A tool stays `pending` after its approval, for 30 s here, while it really runs.
/// Short tools go straight from `pending` to `completed`. So `pending` must count as
/// running, and does.
#[test]
fn a_running_tool_reads_pending_until_the_first_keepalive() {
    let h = Harness::new();
    let mut client = start(&h, "long_tool.jsonl");
    let session = client.session_id();
    client
        .prompt("Use the Bash tool to run exactly: python3 -c \"import time; time.sleep(35)\"\nThen reply DONE.")
        .unwrap();
    let request = next_request(&mut client);
    assert!(client.answer(&request.id, "allow-once").unwrap());
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(
        view.running_tools.len(),
        1,
        "counted while it reads pending"
    );
    assert_eq!(view.running_tools[0].status, ToolStatus::Pending);
    let events = turn(&mut client);
    assert_eq!(statuses(&events), ["in_progress", "completed"]);
    finish(client, &h);
}

/// Without the `subagents` capability, which the client does not declare, a child's
/// tool speaks under the parent's session id. Only `parentToolUseId` tells them apart.
#[test]
fn a_child_tool_arrives_under_the_parents_session_id() {
    let h = Harness::new();
    let mut client = start(&h, "child.jsonl");
    let parent = client.agent_session_id().to_owned();
    client
        .prompt("Use the Agent tool to start one subagent. Tell it to use the Bash tool to run exactly: touch child.txt\nThen reply with just DONE.")
        .unwrap();
    let request = next_request(&mut client);
    assert_eq!(request.worker_id, None, "seen as the parent's own");
    assert!(client.answer(&request.id, "allow-once").unwrap());
    let events = turn(&mut client);
    assert_eq!(stop_reason(&events), "end_turn");
    for event in &events {
        if let ClientEvent::Update(message) = event {
            assert_eq!(message["params"]["sessionId"], parent.as_str());
        }
    }
    finish(client, &h);
    let recording = std::fs::read_to_string(fixtures().join("child.jsonl")).unwrap();
    assert!(recording.contains("parentToolUseId"));
}

/// Records the known false card (step 2 part A): a real launch left without a prompt
/// is quiet, and its queued run reads as active.
#[test]
fn an_idle_launch_raises_the_quiet_card() {
    let h = Harness::new();
    let spec = replaying(&h, "idle.jsonl");
    let session;
    {
        let mut client = AcpClient::launch(&h.engine, &spec, &h.dir.0).unwrap();
        until(&mut client, |event| matches!(event, ClientEvent::Ready));
        session = client.session_id();
        h.advance(STALL_AFTER_MS + 1_000);
        let alerts = h.engine.tick().unwrap();
        assert!(
            matches!(&alerts[..], [Alert::MayBeStuck(card)] if card.tool_call_id.is_none()),
            "{alerts:?}"
        );
        finish(client, &h);
    }
    assert_eq!(
        h.engine.inspect(session).unwrap().session.lifecycle,
        Lifecycle::Closed
    );
}

#[test]
fn an_adapter_killed_after_launch_is_an_exit_not_a_hang() {
    let h = Harness::new();
    let mut client = start(&h, "crash.jsonl");
    let session = client.session_id();
    let events = until(&mut client, |event| {
        matches!(event, ClientEvent::Exited { .. })
    });
    let Some(ClientEvent::Exited { code, .. }) = events.last() else {
        panic!("{events:?}");
    };
    assert_ne!(*code, Some(0));
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Failed);
    assert_eq!(view.session.lifecycle, Lifecycle::Closed);
    finish(client, &h);
}

/// The adapter's real refusal when it cannot find Claude Code: an error on
/// `session/new`, then a clean exit once the client closes its pipe.
#[test]
fn a_refused_session_is_a_failed_launch() {
    let h = Harness::new();
    let spec = replaying(&h, "launch_refused.jsonl");
    let mut client = AcpClient::launch(&h.engine, &spec, &h.dir.0).unwrap();
    let events = until(&mut client, |event| {
        matches!(event, ClientEvent::Exited { .. })
    });
    assert!(
        matches!(&events[0], ClientEvent::LaunchFailed { message } if message.starts_with("Internal error")),
        "{events:?}"
    );
    assert!(matches!(
        events[1],
        ClientEvent::Exited { code: Some(0), .. }
    ));
    replay_was_clean(&h);
}

/// The repo is public. A re-recording that skipped the scrub fails here. The tool
/// rule is a rule, not a list of words: what a tool reports back is where the client's
/// internal text and ids turned up.
#[test]
fn recordings_name_no_machine_or_account() {
    let forbidden = [
        "/Users/",
        "\\Users\\",
        "/home/",
        "\"email\"",
        "organization",
        "rateLimit",
        "agentId",
        "runId",
        "outputFile",
        "output_file",
        "toolResponse",
        "rawOutput",
    ];
    let mut checked = 0;
    for file in std::fs::read_dir(fixtures()).unwrap() {
        let path = file.unwrap().path();
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        checked += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        for marker in forbidden {
            assert!(!text.contains(marker), "{marker} in {}", path.display());
        }
        let outside_fixture = text.replace("/private/tmp/astrolabe-fixture", "");
        assert!(!outside_fixture.contains("/tmp/"), "{}", path.display());
        assert!(!long_hex_run(&text), "raw id in {}", path.display());
        for line in text.lines() {
            let entry: Value = serde_json::from_str(line).unwrap();
            let update = &entry["msg"]["params"]["update"];
            let is_tool = update["sessionUpdate"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("tool_call"));
            assert!(!is_tool || update.get("content").is_none(), "{line}");
            let commands = update.get("availableCommands").and_then(Value::as_array);
            assert!(commands.is_none_or(Vec::is_empty), "{line}");
        }
    }
    assert!(checked > 0);
}

/// Ids this scrub does not know the shape of still look like this. A letter is
/// required, since long decimals in reported costs are not ids.
fn long_hex_run(text: &str) -> bool {
    let mut run = 0;
    let mut letter = false;
    text.chars().any(|c| {
        if !c.is_ascii_hexdigit() {
            run = 0;
            letter = false;
            return false;
        }
        run += 1;
        letter |= c.is_ascii_alphabetic();
        run >= 12 && letter
    })
}
