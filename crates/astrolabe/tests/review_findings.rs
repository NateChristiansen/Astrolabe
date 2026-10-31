//! Regressions from review. Each test names the defect it catches.
mod common;

use astrolabe::events::{AgentMessage, CoreEvent, TurnEnd};
use astrolabe::model::RunState;
use astrolabe::{Error, KEEP_WAITING_MS, STALL_AFTER_MS};
use common::{Harness, MINUTE, PID, acp};
use serde_json::json;

const AGENT: &str = "agent-session";
const CHILD: &str = "child-session";

/// Catches: other activity hiding a hung tool, because the tool's timer was
/// based on the session's last activity rather than on the tool's start.
#[test]
fn a_hung_tool_flags_despite_other_activity() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-a", "hung tool"));

    h.run_for(16 * MINUTE, |h, second| {
        if second % 300 == 0 {
            h.send(session, acp::message(AGENT, "still talking"));
        }
    });

    assert_eq!(h.stuck_alerts(), 1);
    let cards = h.engine.active_cards().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool.as_deref(), Some("hung tool"));
    assert_eq!(cards[0].elapsed_ms(h.now()), 16 * MINUTE);
}

/// Catches: an unrelated JSON-RPC error failing the active run.
#[test]
fn an_unrelated_error_response_leaves_the_run_alone() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::message(AGENT, "working"));

    let unrelated = json!({ "id": 99, "error": { "code": -32603, "message": "boom" } });
    h.engine.prompt_sent(session, &json!(2)).unwrap();
    let result = h.engine.ingest(session, &unrelated);

    assert!(
        matches!(&result, Err(Error::AgentRpc { rpc_id, message, .. })
            if rpc_id == "99" && message == "boom"),
        "the error must reach the caller: {result:?}"
    );
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Running
    );
}

/// Catches: correlation that loses real failures, or trusts an id from an earlier
/// run: only an error for the active prompt's id ends the turn.
#[test]
fn only_an_error_for_the_active_prompt_fails_the_run() {
    let h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    let error = json!({ "id": "prompt-1", "error": { "code": 1, "message": "boom" } });
    let state = |h: &Harness| h.engine.inspect(session).unwrap().run.state;

    // No prompt has been sent, so nothing can answer one.
    assert!(h.engine.ingest(session, &error).is_err());
    assert_eq!(state(&h), RunState::Queued);

    h.engine.prompt_sent(session, &json!("prompt-1")).unwrap();
    h.engine.ingest(session, &error).unwrap();
    assert_eq!(state(&h), RunState::Failed);

    // The next run has no prompt yet; the old id must not fail it.
    h.engine.start_turn(session).unwrap();
    assert!(h.engine.ingest(session, &error).is_err());
    assert_eq!(state(&h), RunState::Queued);

    // A number and its string form are different JSON-RPC ids.
    h.engine.prompt_sent(session, &json!(7)).unwrap();
    let string_id = json!({ "id": "7", "error": { "message": "boom" } });
    assert!(h.engine.ingest(session, &string_id).is_err());
    assert_eq!(state(&h), RunState::Queued);
    h.engine
        .ingest(session, &json!({ "id": 7, "error": { "message": "boom" } }))
        .unwrap();
    assert_eq!(state(&h), RunState::Failed);

    assert!(matches!(
        h.engine.prompt_sent(session, &json!(8)),
        Err(Error::NoActiveRun(_))
    ));
}

/// Catches: each stuck tool not getting its own card, or one tool finishing or
/// being snoozed taking the other's card with it.
#[test]
fn each_stuck_tool_has_its_own_card_and_snooze() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-a", "tool a"));
    h.wait(MINUTE);
    h.send(session, acp::tool_start(AGENT, "call-b", "tool b"));
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 2);
    let cards = h.engine.active_cards().unwrap();
    let (a, b) = (cards[0].clone(), cards[1].clone());
    assert_eq!(a.tool.as_deref(), Some("tool a"));
    assert_eq!(b.tool.as_deref(), Some("tool b"));

    assert!(h.engine.keep_waiting(a.id).unwrap());
    h.send(session, acp::tool_done(AGENT, "call-b"));
    assert!(h.engine.active_cards().unwrap().is_empty());

    h.wait(KEEP_WAITING_MS - 1_000);
    assert_eq!(h.stuck_alerts(), 2, "snoozed tool came back early");
    h.wait(1_000);
    assert_eq!(h.stuck_alerts(), 3);
    let again = h.engine.active_cards().unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].tool.as_deref(), Some("tool a"));
    assert_eq!(
        again[0].elapsed_ms(h.now()),
        MINUTE + STALL_AFTER_MS + KEEP_WAITING_MS,
        "elapsed runs from the tool's start"
    );
}

/// Catches: the tool rule swallowing the quiet rule. A quiet card still retires
/// on new activity, and quiet is judged only when no tool is running.
#[test]
fn the_quiet_card_retires_on_activity_and_ignores_running_tools() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.engine.active_cards().unwrap()[0].tool, None);

    h.send(session, acp::message(AGENT, "back"));
    assert!(h.engine.active_cards().unwrap().is_empty());

    h.send(session, acp::tool_start(AGENT, "call-a", "tool a"));
    h.wait(STALL_AFTER_MS);
    let cards = h.engine.active_cards().unwrap();
    assert_eq!(cards.len(), 1, "a running tool must not also read as quiet");
    assert_eq!(cards[0].tool.as_deref(), Some("tool a"));
}

/// Catches: an owner's wait counting against a tool, or a stale card surviving
/// the restart of that tool's clock, or a card's retirement closing a request.
#[test]
fn a_resolved_request_restarts_a_flagged_tools_clock() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-a", "tool a"));
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);

    h.send(session, acp::request_permission(AGENT, 1, "git push"));
    h.wait(30 * MINUTE);
    assert_eq!(
        h.stuck_alerts(),
        1,
        "no new card while waiting on the owner"
    );
    assert_eq!(h.engine.open_requests().unwrap().len(), 1);

    let id = h.engine.open_requests().unwrap()[0].id.clone();
    h.engine.resolve_request(&id, "allow").unwrap();
    assert!(h.engine.active_cards().unwrap().is_empty());

    h.wait(STALL_AFTER_MS - 1_000);
    assert_eq!(h.stuck_alerts(), 1);
    h.wait(1_000);
    assert_eq!(h.stuck_alerts(), 2);
    let cards = h.engine.active_cards().unwrap();
    assert_eq!(cards[0].elapsed_ms(h.now()), STALL_AFTER_MS);
}

/// Catches: duplicate tool cards after a restart under the per-tool rule.
#[test]
fn restart_keeps_one_card_per_tool_and_does_not_alert_again() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-a", "tool a"));
    h.send(session, acp::tool_start(AGENT, "call-b", "tool b"));
    h.wait(STALL_AFTER_MS);
    let before = h.engine.active_cards().unwrap();
    assert_eq!(before.len(), 2);

    h.restart();
    h.wait(STALL_AFTER_MS);

    assert_eq!(h.stuck_alerts(), 2);
    assert_eq!(h.engine.active_cards().unwrap(), before);
    assert_eq!(h.engine.inspect(session).unwrap().cards, before);
}

/// Catches: a worker's turn end ending its parent's run and finishing the
/// parent's tools.
#[test]
fn a_worker_turn_end_touches_only_that_worker() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.engine.register_worker(session, CHILD, "review").unwrap();
    h.send(session, acp::tool_start(AGENT, "parent-tool", "spawn"));
    h.send(session, acp::tool_start(CHILD, "child-tool", "read"));

    let turn_end = AgentMessage {
        agent_session_id: Some(CHILD.to_owned()),
        event: CoreEvent::TurnEnded(TurnEnd::Completed),
    };
    h.engine.apply(session, &turn_end).unwrap();

    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Running);
    let running: Vec<_> = view
        .running_tools
        .iter()
        .map(|tool| tool.tool_call_id.as_str())
        .collect();
    assert_eq!(running, ["parent-tool"]);
}

/// Catches: a worker's turn end retiring the parent's tool card, or leaving the
/// worker's own card behind.
#[test]
fn a_worker_turn_end_retires_only_that_workers_cards() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.engine.register_worker(session, CHILD, "review").unwrap();
    h.send(session, acp::tool_start(AGENT, "shared-id", "spawn"));
    h.send(session, acp::tool_start(CHILD, "shared-id", "read"));
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 2);

    let turn_end = AgentMessage {
        agent_session_id: Some(CHILD.to_owned()),
        event: CoreEvent::TurnEnded(TurnEnd::Failed),
    };
    h.engine.apply(session, &turn_end).unwrap();

    let cards = h.engine.active_cards().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].tool.as_deref(), Some("spawn"));
    assert_eq!(cards[0].worker_id, None);
    assert_eq!(
        h.engine.inspect(session).unwrap().run.state,
        RunState::Running
    );
}
