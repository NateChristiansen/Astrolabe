//! The ACP client against a fake adapter running as a real child process. Each test
//! names the defect it catches.
mod common;

use astrolabe::Error;
use astrolabe::acp::{
    AcpClient, CLAUDE_ADAPTER, CODEX_ADAPTER, ClientEvent, HANDSHAKE_TIMEOUT, Launch, adapter_entry,
};
use astrolabe::model::{Alert, Lifecycle, Request, RunState, SessionDisplay};
use common::{Harness, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Any wait longer than this is the hang these fixtures exist to catch.
const PATIENCE: Duration = Duration::from_secs(20);

/// Cargo builds the fake next to the test binaries, in `examples/`.
fn fake_agent() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    let name = format!("fake_agent{}", std::env::consts::EXE_SUFFIX);
    profile.join("examples").join(name)
}

fn launch_spec(flags: &[&str]) -> Launch {
    Launch {
        command: fake_agent().into(),
        args: flags.iter().map(|flag| (*flag).into()).collect(),
        env: Vec::new(),
        meta: Some(json!({ "note": "from the driver" })),
    }
}

fn start<'e>(h: &'e Harness, flags: &[&str]) -> AcpClient<'e> {
    AcpClient::launch(&h.engine, &launch_spec(flags), &h.dir.0, HANDSHAKE_TIMEOUT).unwrap()
}

/// Reads events until one matches, returning all of them.
fn until(client: &mut AcpClient, done: impl Fn(&ClientEvent) -> bool) -> Vec<ClientEvent> {
    let deadline = Instant::now() + PATIENCE;
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        if let Some(event) = client.next_event(Duration::from_millis(100)).unwrap() {
            let finished = done(&event);
            seen.push(event);
            if finished {
                return seen;
            }
        }
    }
    panic!("hung; events so far: {seen:?}");
}

fn turn_end(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::TurnEnded { .. })
}

fn exited(event: &ClientEvent) -> bool {
    matches!(event, ClientEvent::Exited { .. })
}

fn next_request(client: &mut AcpClient) -> Request {
    let is_request = |event: &ClientEvent| matches!(event, ClientEvent::Alert(Alert::NeedsYou(_)));
    match until(client, is_request).pop() {
        Some(ClientEvent::Alert(Alert::NeedsYou(request))) => request,
        other => panic!("expected a request, got {other:?}"),
    }
}

fn said(events: &[ClientEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            ClientEvent::Update(Value::Object(message)) => {
                let text = message["params"]["update"]["content"]["text"].as_str();
                text.map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

/// Catches: a broken path through the plain life of a session, a lost agent session
/// id, the prompt id going unrecorded, or a driver's `_meta` not arriving.
#[test]
fn a_full_turn_runs_through_the_engine() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    assert_eq!(client.agent_session_id(), "fake-session");
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(
        view.session.agent_session_id.as_deref(),
        Some("fake-session")
    );
    assert_eq!(view.session.pid, client.pid());
    assert_eq!(view.run.state, RunState::Queued);

    client.prompt("hello").unwrap();
    assert!(
        h.engine
            .inspect(session)
            .unwrap()
            .run
            .prompt_rpc_id
            .is_some(),
        "the prompt id must be recorded by the time the prompt is out"
    );
    let events = until(&mut client, turn_end);
    assert_eq!(said(&events), ["turn 1"]);
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Completed
    );
    let meta = std::fs::read_to_string(h.dir.0.join("meta.txt")).unwrap();
    assert!(meta.contains("from the driver"), "got {meta}");

    client.shutdown().unwrap();
    assert_eq!(
        h.engine.inspect(session).unwrap().session.lifecycle,
        Lifecycle::Closed
    );
}

/// Catches: answering with a re-typed id, dropping options the adapter offered, or
/// the run not continuing after the answer.
#[test]
fn an_approval_is_answered_and_the_turn_continues() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("approve").unwrap();
    let request = next_request(&mut client);

    assert_eq!(request.rpc_id, "\"7\"");
    assert_eq!(request.title, "git push");
    let offered: Vec<_> = request.options.iter().map(|o| &*o.option_id).collect();
    assert_eq!(offered, ["yes", "always", "no"]);
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Waiting);
    assert_eq!(view.display, SessionDisplay::NeedsYou);

    assert!(matches!(
        client.answer(&request.id, "not-offered"),
        Err(Error::UnknownOption { .. })
    ));
    assert_eq!(h.engine.open_requests().unwrap().len(), 1);

    assert!(client.answer(&request.id, "yes").unwrap());
    let events = until(&mut client, turn_end);
    assert_eq!(said(&events), ["answered yes"]);
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Completed
    );
    assert!(h.engine.open_requests().unwrap().is_empty());
    assert!(
        !client.answer(&request.id, "no").unwrap(),
        "first answer stands"
    );
    client.shutdown().unwrap();
}

/// Catches: reject needing a cancel, or no way to say what to do instead.
#[test]
fn a_rejection_is_followed_by_a_new_prompt() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    client.prompt("approve").unwrap();
    let request = next_request(&mut client);

    assert!(matches!(
        client.prompt("do this instead"),
        Err(Error::TurnInProgress(_))
    ));
    let reject = request.reject_option().unwrap();
    assert_eq!(reject, "no");
    client.answer(&request.id, reject).unwrap();
    let events = until(&mut client, turn_end);
    assert_eq!(said(&events), ["answered no"]);

    client.prompt("do this instead").unwrap();
    let events = until(&mut client, turn_end);
    assert_eq!(said(&events), ["turn 2"]);
    client.shutdown().unwrap();
}

/// Catches: the second turn's run not being created or correlated.
#[test]
fn two_turns_run_in_one_process() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("one").unwrap();
    let first = until(&mut client, turn_end);
    let first_run = h.engine.inspect(session).unwrap().run;
    client.prompt("two").unwrap();
    let second = until(&mut client, turn_end);
    let second_run = h.engine.inspect(session).unwrap().run;

    assert_eq!(said(&first), ["turn 1"]);
    assert_eq!(said(&second), ["turn 2"]);
    assert_ne!(first_run.id, second_run.id);
    assert_ne!(first_run.prompt_rpc_id, second_run.prompt_rpc_id);
    assert_eq!(second_run.state, RunState::Completed);
    client.shutdown().unwrap();
}

fn resume_in_new_process(flags: &[&str]) {
    let h = Harness::new();
    let mut first = start(&h, flags);
    let session = first.session_id();
    first.prompt("one").unwrap();
    until(&mut first, turn_end);
    let old_pid = first.pid();
    first.shutdown().unwrap();

    let mut second = AcpClient::resume(
        &h.engine,
        session,
        &launch_spec(flags),
        &h.dir.0,
        HANDSHAKE_TIMEOUT,
    )
    .unwrap();
    let view = h.engine.inspect(session).unwrap();
    assert_ne!(view.session.pid, old_pid);
    assert_eq!(view.session.lifecycle, Lifecycle::Open);
    assert!(
        view.running_tools.is_empty(),
        "replayed history must not look like live work: {:?}",
        view.running_tools
    );
    second.prompt("two").unwrap();
    let events = until(&mut second, turn_end);
    assert_eq!(said(&events), ["turn 2"], "the session must carry on");
    second.shutdown().unwrap();
}

/// Catches: resume starting a new session, or replay counting as progress.
#[test]
fn a_session_resumes_in_a_new_process() {
    resume_in_new_process(&[]);
}

/// Catches: the client knowing only one resume route.
#[test]
fn a_session_resumes_through_load_when_that_is_the_route() {
    resume_in_new_process(&["--load"]);
}

/// Catches: an adapter with no resume route looking like a working session, or a
/// session with no agent id being resumable.
#[test]
fn a_resume_with_no_route_fails_visibly() {
    let h = Harness::new();
    let first = start(&h, &["--no-resume"]);
    let session = first.session_id();
    first.shutdown().unwrap();
    let spec = launch_spec(&["--no-resume"]);
    let result = AcpClient::resume(&h.engine, session, &spec, &h.dir.0, HANDSHAKE_TIMEOUT);
    assert!(matches!(result, Err(Error::Launch { .. })));
    let lifecycle = h.engine.inspect(session).unwrap().session.lifecycle;
    assert_eq!(lifecycle, Lifecycle::Closed);
}

/// Catches: an answer to a dead process going down a new process's pipe, where its
/// id could match a different request.
#[test]
fn a_request_from_an_earlier_process_cannot_be_answered() {
    let h = Harness::new();
    let mut first = start(&h, &[]);
    let session = first.session_id();
    first.prompt("approve").unwrap();
    let request = next_request(&mut first);
    first.shutdown().unwrap();

    let mut second = AcpClient::resume(
        &h.engine,
        session,
        &launch_spec(&[]),
        &h.dir.0,
        HANDSHAKE_TIMEOUT,
    )
    .unwrap();
    assert!(matches!(
        second.answer(&request.id, "yes"),
        Err(Error::RequestNotLive(_))
    ));
    assert_eq!(h.engine.open_requests().unwrap().len(), 1, "never dropped");
    second.shutdown().unwrap();
}

/// Catches: a crash mid-turn hanging, the run staying active, or losing the reason.
#[test]
fn an_adapter_crash_mid_turn_fails_the_run() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("crash").unwrap();
    let events = until(&mut client, exited);
    let Some(ClientEvent::Exited { code, stderr }) = events.last() else {
        panic!("no exit in {events:?}");
    };
    assert_eq!(*code, Some(3));
    assert!(stderr.contains("adapter crashed"), "got {stderr}");

    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Failed);
    assert_eq!(view.session.lifecycle, Lifecycle::Closed);
    assert_eq!(view.display, SessionDisplay::Failed);
    assert!(view.running_tools.is_empty());

    assert!(matches!(client.prompt("x"), Err(Error::AgentGone(_))));
    assert_eq!(client.next_event(Duration::ZERO).unwrap(), None);
}

/// Catches: waiting on end of file when a process left behind holds stdout open.
#[test]
fn an_exit_is_seen_while_a_left_behind_process_holds_the_pipe() {
    let h = Harness::new();
    let spec = launch_spec(&["--hold-stdout"]);
    let started = Instant::now();
    let result = AcpClient::launch(&h.engine, &spec, &h.dir.0, PATIENCE);
    assert!(matches!(result, Err(Error::Launch { .. })));
    assert!(started.elapsed() < Duration::from_secs(15));
}

/// Catches: a prompt to a process that has gone hanging the run.
#[test]
fn a_prompt_to_a_gone_process_is_never_a_hang() {
    let h = Harness::new();
    let mut client = start(&h, &["--exit-after-new"]);
    let session = client.session_id();
    // Delivery may fail outright or the write may land just before the exit.
    let sent = client.prompt("hello");
    assert!(
        sent.is_ok() || matches!(sent, Err(Error::Io(_))),
        "{sent:?}"
    );
    until(&mut client, exited);
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Failed);
    assert_eq!(view.session.lifecycle, Lifecycle::Closed);
}

/// Catches: a malformed line stopping the read loop, or CRLF being treated as noise.
#[test]
fn malformed_lines_are_reported_and_reading_goes_on() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    client.prompt("malformed").unwrap();
    let events = until(&mut client, turn_end);
    let malformed = events
        .iter()
        .filter(|event| matches!(event, ClientEvent::Malformed { .. }))
        .count();
    assert_eq!(malformed, 4, "{events:?}");
    assert_eq!(said(&events), ["after noise"]);
    client.shutdown().unwrap();
}

/// Catches: silence to a request the client does not support, which would hang it.
#[test]
fn an_unsupported_agent_request_gets_a_proper_error() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    client.prompt("unsupported").unwrap();
    let events = until(&mut client, turn_end);
    assert!(events.contains(&ClientEvent::UnsupportedRequest {
        method: "fs/read_text_file".to_owned()
    }));
    assert_eq!(said(&events), ["code -32601"]);
    client.shutdown().unwrap();
}

/// Catches: a prompt error leaving the run active, or killing the session.
#[test]
fn a_prompt_error_fails_the_run_and_the_next_turn_works() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("error").unwrap();
    let events = until(&mut client, |e| {
        matches!(e, ClientEvent::PromptError { .. })
    });
    assert_eq!(
        events.last(),
        Some(&ClientEvent::PromptError {
            message: "prompt refused".to_owned()
        })
    );
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Failed
    );
    client.prompt("again").unwrap();
    assert_eq!(said(&until(&mut client, turn_end)), ["turn 2"]);
    client.shutdown().unwrap();
}

/// Catches: a prompt answer with no stop reason leaving the run active forever.
#[test]
fn a_prompt_answer_without_a_stop_reason_fails_the_run() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    client.prompt("no-stop-reason").unwrap();
    until(&mut client, |e| {
        matches!(e, ClientEvent::PromptError { .. })
    });
    let run = h.engine.inspect(client.session_id()).unwrap().run;
    assert_eq!(run.state, RunState::Failed);
    client.shutdown().unwrap();
}

/// Catches: the read loop stopping on `Error::AgentRpc`, or a stop reason for some
/// other id ending the turn.
#[test]
fn unrelated_answers_leave_the_turn_running() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    client.prompt("unrelated-error").unwrap();
    let events = until(&mut client, turn_end);
    assert!(events.contains(&ClientEvent::AgentError {
        rpc_id: "999".to_owned(),
        message: "unrelated".to_owned(),
    }));
    assert_eq!(
        said(&events),
        ["still going"],
        "the turn ended early: {events:?}"
    );
    client.shutdown().unwrap();
}

/// Catches: a child's tool or approval counted as the parent's, or the parent's run
/// ended by the child.
#[test]
fn a_child_session_update_is_attributed_to_its_parent() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("child").unwrap();
    let request = next_request(&mut client);
    assert_eq!(request.session_id, session);
    assert_eq!(request.worker_id.as_deref(), Some("child-1"));
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.running_tools[0].worker_id.as_deref(), Some("child-1"));
    assert_eq!(view.run.state, RunState::Waiting);

    client.answer(&request.id, "yes").unwrap();
    until(&mut client, turn_end);
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Completed
    );
    client.shutdown().unwrap();
}

/// Catches: cancel leaving an open approval unanswered, which would deadlock the
/// adapter, or the run being marked cancelled before the agent said so.
#[test]
fn cancel_answers_open_requests_and_waits_for_the_agent() {
    let h = Harness::new();
    let mut client = start(&h, &[]);
    let session = client.session_id();
    client.prompt("cancel").unwrap();
    next_request(&mut client);

    client.cancel().unwrap();
    assert!(h.engine.open_requests().unwrap().is_empty());
    assert!(
        h.engine.inspect(session).unwrap().run.state.is_active(),
        "cancelled only on evidence"
    );
    until(&mut client, turn_end);
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Cancelled
    );
    client.shutdown().unwrap();
}

fn launch_failure(flags: &[&str], timeout: Duration) -> (String, Harness) {
    let h = Harness::new();
    let result = AcpClient::launch(&h.engine, &launch_spec(flags), &h.dir.0, timeout);
    let Err(Error::Launch {
        session_id,
        message,
    }) = result
    else {
        panic!("expected a launch failure for {flags:?}");
    };
    let view = h.engine.inspect(session_id).unwrap();
    assert_eq!(view.run.state, RunState::Failed, "{flags:?}");
    assert_eq!(view.session.lifecycle, Lifecycle::Closed, "{flags:?}");
    (message, h)
}

/// Catches: an error before any prompt being lost, or leaving a session "working".
#[test]
fn errors_before_any_prompt_fail_the_launch_visibly() {
    let (message, _h) = launch_failure(&["--fail-initialize"], PATIENCE);
    assert!(message.contains("cannot initialize"), "{message}");
    let (message, _h) = launch_failure(&["--fail-new"], PATIENCE);
    assert!(message.contains("cannot create a session"), "{message}");
    let (message, _h) = launch_failure(&["--stderr-then-exit"], PATIENCE);
    assert!(message.contains("adapter cannot start"), "{message}");
}

/// Catches: an adapter that never answers the handshake hanging the launch.
#[test]
fn a_silent_adapter_fails_the_launch_in_time() {
    let (message, _h) = launch_failure(&["--hang-initialize"], Duration::from_millis(300));
    assert!(message.contains("initialize"), "{message}");
}

/// Catches: a stray error during startup, with no prompt to answer, ending the read.
#[test]
fn a_stray_error_before_any_prompt_does_not_stop_the_launch() {
    let h = Harness::new();
    let mut client = start(&h, &["--stray-error"]);
    let event = client.next_event(PATIENCE).unwrap();
    assert_eq!(
        event,
        Some(ClientEvent::AgentError {
            rpc_id: "null".to_owned(),
            message: "stray".to_owned(),
        })
    );
    let run = h.engine.inspect(client.session_id()).unwrap().run;
    assert_eq!(run.state, RunState::Queued);
    client.shutdown().unwrap();
}

/// Catches: a missing program being anything but an immediate error with no session.
#[test]
fn a_missing_program_is_an_error_and_creates_no_session() {
    let h = Harness::new();
    let spec = Launch {
        command: h.dir.0.join("no-such-adapter").into(),
        ..Launch::default()
    };
    let result = AcpClient::launch(&h.engine, &spec, &h.dir.0, PATIENCE);
    assert!(matches!(result, Err(Error::Io(_))));
    assert!(h.engine.inspect(1).is_err());
}

fn write_package(root: &Path, package: &str, manifest: &Value, script: Option<&str>) {
    let dir = root.join("node_modules").join(package);
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    std::fs::write(dir.join("package.json"), manifest.to_string()).unwrap();
    if let Some(script) = script {
        std::fs::write(dir.join(script), "").unwrap();
    }
}

/// Catches: looking up an adapter any way but through the explicit directory, or a
/// missing install surfacing later as a confusing launch failure.
#[test]
fn an_adapter_resolves_from_its_explicit_directory() {
    let dir = TempDir::new();
    let missing = adapter_entry(&dir.0, CODEX_ADAPTER);
    assert!(matches!(missing, Err(Error::AdapterNotFound(_))));

    let manifest = json!({ "bin": { "codex-acp": "dist/index.js" } });
    write_package(&dir.0, CODEX_ADAPTER, &manifest, Some("dist/index.js"));
    let entry = adapter_entry(&dir.0, CODEX_ADAPTER).unwrap();
    assert!(entry.is_file());
    assert_eq!(
        Launch::node(&entry).args,
        [std::ffi::OsString::from(entry.as_os_str())]
    );

    let manifest = json!({ "bin": "dist/index.js" });
    write_package(&dir.0, CLAUDE_ADAPTER, &manifest, None);
    let no_script = adapter_entry(&dir.0, CLAUDE_ADAPTER);
    assert!(matches!(no_script, Err(Error::AdapterNotFound(_))));
}

/// Catches: an adapter pin drifting from the manifest, the lockfile or the docs.
#[test]
fn adapter_pins_agree_everywhere() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |path: &str| std::fs::read_to_string(repo.join(path)).unwrap();
    let manifest: Value = serde_json::from_str(&read("adapters/package.json")).unwrap();
    let dependencies = manifest["dependencies"].as_object().unwrap();
    assert_eq!(dependencies.len(), 2);
    let lockfile = read("adapters/pnpm-lock.yaml");
    let versions = read("docs/versions.md");
    for (package, version) in [(CLAUDE_ADAPTER, "0.99.1"), (CODEX_ADAPTER, "2.5.0")] {
        assert_eq!(dependencies[package], version);
        assert!(lockfile.contains(&format!("'{package}@{version}':")));
        assert!(versions.contains(&format!("| {package} | {version} |")));
    }
}
