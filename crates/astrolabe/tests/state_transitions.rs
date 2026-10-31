//! State-model fixtures. Each test names the defect it catches.
mod common;

use astrolabe::Error;
use astrolabe::events::{CoreEvent, TurnEnd, map_message};
use astrolabe::model::{Lifecycle, OutcomeState, RunState, SessionDisplay, ToolStatus};
use common::{Harness, PID, acp};
use serde_json::json;

const AGENT: &str = "agent-session";

/// Catches: an adapter's chatter or a replayed user message mapping to progress,
/// or a real update being dropped.
#[test]
fn acp_messages_map_to_core_events() {
    let event = |message| map_message(&message).map(|mapped| mapped.event);

    assert_eq!(event(acp::message(AGENT, "hi")), Some(CoreEvent::Content));
    assert_eq!(event(acp::usage(AGENT)), Some(CoreEvent::Keepalive));
    assert_eq!(
        event(json!({
            "method": "session/update",
            "params": { "update": { "sessionUpdate": "a_future_kind" } },
        })),
        Some(CoreEvent::Keepalive),
        "an unknown kind must not count as progress"
    );
    assert_eq!(
        event(json!({
            "method": "session/update",
            "params": { "update": {
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": "replay" },
            } },
        })),
        None
    );
    assert_eq!(
        event(json!({ "id": 1, "result": { "sessionId": "x" } })),
        None
    );

    let Some(CoreEvent::Tool(keepalive)) = event(acp::tool_keepalive(AGENT, "c")) else {
        panic!("tool update must map to a tool event");
    };
    assert_eq!(keepalive.status, Some(ToolStatus::Running));

    let Some(CoreEvent::RequestOpened { rpc_id, title }) =
        event(acp::request_permission(AGENT, 7, "git push"))
    else {
        panic!("permission request must open a request");
    };
    assert_eq!((rpc_id.as_str(), title.as_str()), ("7", "git push"));

    for (reason, end) in [
        ("end_turn", TurnEnd::Completed),
        ("cancelled", TurnEnd::Cancelled),
        ("max_tokens", TurnEnd::Failed),
        ("refusal", TurnEnd::Failed),
    ] {
        assert_eq!(
            event(acp::turn_end(reason)),
            Some(CoreEvent::TurnEnded(end))
        );
    }
    assert_eq!(
        event(json!({ "id": 2, "error": { "code": -32603, "message": "boom" } })),
        Some(CoreEvent::ResponseError {
            rpc_id: "2".to_owned(),
            message: "boom".to_owned(),
        })
    );
}

/// Catches: a broken path through the normal life of a run.
#[test]
fn a_run_moves_from_queued_through_waiting_to_completed() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    let state = |h: &Harness| h.engine.inspect(session).unwrap().run.state;
    assert_eq!(state(&h), RunState::Queued);
    assert_eq!(
        h.engine.inspect(session).unwrap().display,
        SessionDisplay::Working
    );

    h.send(session, acp::usage(AGENT));
    assert_eq!(state(&h), RunState::Queued, "a keepalive is not a start");

    h.send(session, acp::tool_start(AGENT, "c1", "edit"));
    assert_eq!(state(&h), RunState::Running);
    assert_eq!(h.engine.inspect(session).unwrap().running_tools.len(), 1);

    h.send(session, acp::request_permission(AGENT, 1, "edit file"));
    assert_eq!(state(&h), RunState::Waiting);

    let id = h.engine.open_requests().unwrap()[0].id.clone();
    assert!(h.engine.resolve_request(&id, "allow").unwrap());
    assert_eq!(state(&h), RunState::Running);

    h.send(session, acp::tool_done(AGENT, "c1"));
    assert!(h.engine.inspect(session).unwrap().running_tools.is_empty());
    h.send(session, acp::tool_keepalive(AGENT, "c1"));
    assert!(
        h.engine.inspect(session).unwrap().running_tools.is_empty(),
        "a late keepalive revived a finished tool"
    );

    h.send(session, acp::turn_end("end_turn"));
    assert_eq!(state(&h), RunState::Completed);

    let next = h.engine.start_turn(session).unwrap();
    let view = h.engine.inspect(session).unwrap();
    assert_eq!((view.run.id, view.run.state), (next, RunState::Queued));
}

/// Catches: treating an idle agent as finished work. A completed turn is the
/// owner's turn, and only an explicit record creates an outcome.
#[test]
fn idle_never_means_done() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::message(AGENT, "all finished!"));
    h.send(session, acp::turn_end("end_turn"));

    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.display, SessionDisplay::YourTurn);
    assert_eq!(view.session.lifecycle, Lifecycle::Open);
    assert!(h.engine.outcomes(session).unwrap().is_empty());
}

/// Catches: marking a run cancelled because a stop was asked for, when nothing
/// shows that it stopped.
#[test]
fn cancelled_needs_actual_stop_evidence() {
    let mut h = Harness::new();

    // No evidence: asking changes nothing about the run.
    let asked = h.engine.launch(PID).unwrap();
    h.send(asked, acp::message(AGENT, "working"));
    h.engine.request_stop(asked).unwrap();
    assert_eq!(
        h.engine.inspect(asked).unwrap().run.state,
        RunState::Running
    );

    // Evidence: the process exited after the stop.
    h.engine.process_exited(asked).unwrap();
    let view = h.engine.inspect(asked).unwrap();
    assert_eq!(view.run.state, RunState::Cancelled);
    assert_eq!(view.display, SessionDisplay::Ended);

    // Evidence: the agent reported a cancelled turn.
    let reported = h.engine.launch(PID).unwrap();
    h.send(reported, acp::turn_end("cancelled"));
    assert_eq!(
        h.engine.inspect(reported).unwrap().run.state,
        RunState::Cancelled
    );

    // A process dying unasked is a failure, not a cancel.
    let crashed = h.engine.launch(PID).unwrap();
    h.engine.process_exited(crashed).unwrap();
    let view = h.engine.inspect(crashed).unwrap();
    assert_eq!(view.run.state, RunState::Failed);
    assert_eq!(view.display, SessionDisplay::Failed);

    // A process gone after a stop was asked for counts, across a restart too.
    let stopped = h.engine.launch(PID + 1).unwrap();
    h.engine.request_stop(stopped).unwrap();
    h.processes.kill(PID + 1);
    let report = h.restart();
    assert_eq!(report.cancelled, vec![stopped]);
    assert_eq!(
        h.engine.inspect(stopped).unwrap().run.state,
        RunState::Cancelled
    );
}

/// Catches: a finished run coming back to life on a late message.
#[test]
fn terminal_runs_stay_terminal() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::turn_end("max_tokens"));
    let run = h.engine.inspect(session).unwrap();
    assert_eq!(run.run.state, RunState::Failed);
    assert_eq!(run.display, SessionDisplay::Failed);

    h.send(session, acp::message(AGENT, "late"));
    h.send(session, acp::turn_end("end_turn"));
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Failed
    );

    h.send(session, acp::tool_start(AGENT, "late", "late tool"));
    let id = h.engine.start_turn(session).unwrap();
    assert!(
        h.engine.inspect(session).unwrap().running_tools.is_empty(),
        "a late tool leaked into the next run"
    );
    assert!(matches!(
        h.engine.start_turn(session),
        Err(Error::TurnInProgress(s)) if s == session
    ));
    assert_eq!(h.engine.inspect(session).unwrap().run.id, id);
}

/// Catches: a request closing any way but by its id, or a second answer
/// overwriting the first.
#[test]
fn requests_resolve_by_id_and_the_first_answer_stands() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::request_permission(AGENT, 1, "first"));
    h.send(session, acp::request_permission(AGENT, 2, "second"));
    assert_eq!(h.needs_you_alerts(), 2);

    let open = h.engine.open_requests().unwrap();
    let (first, second) = (open[0].id.clone(), open[1].id.clone());
    assert!(matches!(
        h.engine.resolve_request("no-such-id", "x"),
        Err(Error::UnknownRequest(_))
    ));

    assert!(h.engine.resolve_request(&first, "allow").unwrap());
    assert!(!h.engine.resolve_request(&first, "deny").unwrap());
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(
        view.run.state,
        RunState::Waiting,
        "one request is still open"
    );
    assert_eq!(view.open_requests[0].id, second);

    assert!(h.engine.resolve_request(&second, "allow").unwrap());
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Running
    );
}

/// Catches: a request lost when its process exits, or unanswerable afterward.
/// Late answers must work.
#[test]
fn a_request_outlives_its_process_and_can_still_be_answered() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::request_permission(AGENT, 1, "git push"));
    h.engine.process_exited(session).unwrap();

    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.display, SessionDisplay::NeedsYou);
    let id = view.open_requests[0].id.clone();

    assert!(h.engine.resolve_request(&id, "allow").unwrap());
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(
        view.run.state,
        RunState::Failed,
        "an answer must not revive it"
    );
    assert_eq!(view.display, SessionDisplay::Failed);
}

/// Catches: rpc ids restarting with a new process colliding with an old request,
/// which would swallow a real approval.
#[test]
fn the_same_rpc_id_in_a_later_run_is_a_new_request() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::request_permission(AGENT, 1, "first"));
    let first = h.engine.open_requests().unwrap()[0].id.clone();
    h.engine.resolve_request(&first, "allow").unwrap();
    h.send(session, acp::turn_end("end_turn"));
    h.engine.start_turn(session).unwrap();

    h.send(session, acp::request_permission(AGENT, 1, "second"));

    assert_eq!(h.needs_you_alerts(), 2);
    assert_eq!(h.engine.open_requests().unwrap()[0].title, "second");
}

/// Catches: an outcome skipping or reversing its states.
#[test]
fn outcomes_go_submitted_then_accepted_or_superseded() {
    let h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    let state = |index: usize| h.engine.outcomes(session).unwrap()[index].state;

    let first = h.engine.record_outcome(session, "draft").unwrap();
    assert_eq!(state(0), OutcomeState::Submitted);

    let second = h.engine.record_outcome(session, "better draft").unwrap();
    assert_eq!(state(0), OutcomeState::Superseded);
    assert_eq!(state(1), OutcomeState::Submitted);
    assert!(matches!(
        h.engine.accept_outcome(first),
        Err(Error::OutcomeNotSubmitted(_))
    ));

    h.engine.accept_outcome(second).unwrap();
    assert_eq!(state(1), OutcomeState::Accepted);
    assert!(h.engine.accept_outcome(second).is_err());

    h.engine.record_outcome(session, "follow-up").unwrap();
    assert_eq!(
        state(1),
        OutcomeState::Accepted,
        "an accepted one stays accepted"
    );
}

/// Catches: a restart losing state, or running reconcile's effects twice.
#[test]
fn state_survives_a_restart_and_reconcile_is_idempotent() {
    let mut h = Harness::new();
    let live = h.engine.launch(PID).unwrap();
    let idle_gone = h.engine.launch(PID + 1).unwrap();
    let active_gone = h.engine.launch(PID + 2).unwrap();
    let ended = h.engine.launch(PID + 3).unwrap();
    h.send(live, acp::tool_start(AGENT, "c1", "build"));
    h.send(idle_gone, acp::turn_end("end_turn"));
    h.engine.process_exited(ended).unwrap();
    h.processes.kill(PID + 1);
    h.processes.kill(PID + 2);
    h.processes.kill(PID + 3);

    let report = h.restart();
    assert_eq!(report.interrupted, vec![idle_gone, active_gone]);

    let live_view = h.engine.inspect(live).unwrap();
    assert_eq!(live_view.run.state, RunState::Running);
    assert_eq!(live_view.running_tools[0].title, "build");
    assert_eq!(
        h.engine.inspect(idle_gone).unwrap().run.state,
        RunState::Completed,
        "a finished run must keep its result"
    );
    let active = h.engine.inspect(active_gone).unwrap();
    assert_eq!(
        active.run.state,
        RunState::Interrupted,
        "not cancelled, not failed"
    );
    assert_eq!(active.display, SessionDisplay::Interrupted);
    assert_eq!(
        h.engine.inspect(ended).unwrap().session.lifecycle,
        Lifecycle::Closed
    );

    assert!(h.restart().interrupted.is_empty(), "already reconciled");
    assert!(matches!(
        h.engine.start_turn(active_gone),
        Err(Error::SessionNotOpen(_))
    ));
}
