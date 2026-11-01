//! Stall-watch fixtures. Each test names the defect it catches.
mod common;

use astrolabe::model::{Alert, Card, CardAction, Lifecycle, RunState, SessionDisplay};
use astrolabe::{KEEP_WAITING_MS, STALL_AFTER_MS};
use common::{Harness, MINUTE, PID, acp};

const AGENT: &str = "agent-session";
const CHILD: &str = "child-session";

fn card(h: &Harness) -> Card {
    let mut cards = h.engine.active_cards().unwrap();
    assert_eq!(cards.len(), 1, "expected exactly one active card");
    cards.pop().unwrap()
}

/// Catches: a timer that starts at the first event, so a launch that hangs before
/// any output (a client waiting on an open stdin) is never noticed.
#[test]
fn blocked_before_first_output_raises_one_quiet_card() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();

    h.wait(STALL_AFTER_MS - 1_000);
    assert_eq!(h.stuck_alerts(), 0, "flagged before ten minutes");

    h.wait(1_000);
    assert_eq!(h.stuck_alerts(), 1);
    let card = card(&h);
    assert_eq!(card.session_id, session);
    assert_eq!(card.tool, None);
    assert_eq!(card.elapsed_ms(h.now()), STALL_AFTER_MS);
    assert_eq!(card.summary(h.now()), "quiet 10m, may be stuck");

    h.wait(3 * STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1, "the card must not repeat");
}

/// Catches: a watch that treats a long quiet tool as a failure: the card must not
/// survive the tool's success or alter the run's result.
#[test]
fn quiet_tool_that_succeeds_retires_its_card_and_the_turn_completes() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-1", "run tests"));

    h.wait(9 * MINUTE);
    h.send(session, acp::tool_done(AGENT, "call-1"));
    assert_eq!(h.stuck_alerts(), 0, "a nine-minute tool is not stuck");

    h.send(session, acp::tool_start(AGENT, "call-2", "run tests"));
    h.wait(12 * MINUTE);
    assert_eq!(h.stuck_alerts(), 1);
    assert_eq!(card(&h).tool.as_deref(), Some("run tests"));

    h.send(session, acp::tool_done(AGENT, "call-2"));
    assert!(h.engine.active_cards().unwrap().is_empty());
    h.send(session, acp::message(AGENT, "tests pass"));
    h.end_turn(session, "end_turn");

    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.run.state, RunState::Completed);
    assert_eq!(view.display, SessionDisplay::YourTurn);
    assert_eq!(h.stuck_alerts(), 1);
}

/// Catches: a plain "no event for N minutes" timer. Adapters send updates every
/// 10 to 30 seconds during a tool, so such a timer never fires on a hung tool.
#[test]
fn keepalive_only_tool_is_flagged_with_tool_and_elapsed_time() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-1", "cargo build"));

    h.run_for(STALL_AFTER_MS + 5 * MINUTE, |h, second| {
        if second % 10 == 0 {
            h.send(session, acp::tool_keepalive(AGENT, "call-1"));
            h.send(session, acp::usage(AGENT));
        }
    });

    assert_eq!(h.stuck_alerts(), 1);
    let card = card(&h);
    assert_eq!(card.tool.as_deref(), Some("cargo build"));
    assert_eq!(card.elapsed_ms(h.now()), 15 * MINUTE);
    assert_eq!(
        card.summary(h.now()),
        "cargo build running 15m, may be stuck"
    );
    assert_eq!(
        Card::ACTIONS,
        [
            CardAction::Inspect,
            CardAction::KeepWaiting,
            CardAction::Stop
        ]
    );
}

/// Catches: a tool's own output clearing its flag, so a tool looping on output
/// hides a hang. Only the tool finishing retires the card.
#[test]
fn a_tools_own_output_neither_delays_nor_retires_its_card() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "call-1", "cargo build"));

    h.run_for(STALL_AFTER_MS, |h, second| {
        if second % 30 == 0 {
            let text = format!("line {second}");
            h.send(session, acp::tool_output(AGENT, "call-1", &text));
        }
    });
    assert_eq!(h.stuck_alerts(), 1);

    h.send(session, acp::tool_output(AGENT, "call-1", "linking"));
    assert_eq!(card(&h).tool.as_deref(), Some("cargo build"));

    h.send(session, acp::tool_done(AGENT, "call-1"));
    assert!(h.engine.active_cards().unwrap().is_empty());
}

/// Catches: a child's approval attributed only to the child, leaving the parent
/// shown as working, or flagged as stuck while it waits on the owner.
#[test]
fn child_approval_needs_the_owner_at_once_and_is_never_a_stall() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::tool_start(AGENT, "spawn-1", "child: review"));
    h.engine.register_worker(session, CHILD, "review").unwrap();
    h.wait(2 * MINUTE);

    h.send(session, acp::request_permission(CHILD, 7, "git push"));

    assert_eq!(h.needs_you_alerts(), 1, "must alert at once, with no tick");
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.display, SessionDisplay::NeedsYou);
    assert_eq!(view.run.state, RunState::Waiting);
    let request = &view.open_requests[0];
    assert_eq!(
        request.worker_id.as_deref(),
        Some(CHILD),
        "which child asked"
    );
    let request_id = request.id.clone();

    h.wait(3 * STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 0, "waiting on the owner is not a stall");

    // Time on the owner must not count against the tool once answered.
    assert!(h.engine.resolve_request(&request_id, "allow").unwrap());
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.display, SessionDisplay::Working);
    assert_eq!(view.run.state, RunState::Running);
    assert_eq!(h.stuck_alerts(), 0);
    h.wait(STALL_AFTER_MS - 1_000);
    assert_eq!(h.stuck_alerts(), 0);
    h.wait(1_000);
    assert_eq!(h.stuck_alerts(), 1);
    assert_eq!(card(&h).elapsed_ms(h.now()), STALL_AFTER_MS);
}

/// Catches: a child's progress not counting toward its parent, so a working child
/// reads as a stuck parent.
#[test]
fn child_content_counts_as_activity_for_the_parent() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.engine.register_worker(session, CHILD, "review").unwrap();
    h.run_for(25 * MINUTE, |h, second| {
        if second % 300 == 0 {
            h.send(session, acp::message(CHILD, "still reading"));
        }
    });
    assert_eq!(h.stuck_alerts(), 0);
}

/// Catches: a restart that re-raises what the owner has already seen, or loses it.
#[test]
fn restart_does_not_duplicate_cards_or_alert_again() {
    let mut h = Harness::new();
    let carded = h.engine.launch(PID).unwrap();
    let asking = h.engine.launch(PID + 1).unwrap();
    h.send(asking, acp::request_permission(AGENT, 1, "git push"));
    h.wait(STALL_AFTER_MS);
    assert_eq!((h.stuck_alerts(), h.needs_you_alerts()), (1, 1));
    let before = card(&h);

    // The asking agent's process died with the core; the carded one survived.
    h.processes.kill(PID + 1);
    let report = h.restart();

    assert_eq!(report.interrupted, vec![asking]);
    assert_eq!((report.open_requests, report.active_cards), (1, 1));

    h.wait(3 * STALL_AFTER_MS);
    h.restart();
    h.wait(STALL_AFTER_MS);
    assert_eq!(
        (h.stuck_alerts(), h.needs_you_alerts()),
        (1, 1),
        "alerted again"
    );
    assert_eq!(card(&h), before, "the same card, not a new one");
    assert_eq!(card(&h).session_id, carded);

    let asking = h.engine.inspect(asking).unwrap();
    assert_eq!(
        asking.open_requests.len(),
        1,
        "a restart must not resolve it"
    );
    assert_eq!(asking.display, SessionDisplay::NeedsYou);
    assert_eq!(asking.session.lifecycle, Lifecycle::Interrupted);
}

/// Catches: a card left on a session that no longer runs, shown as "may be stuck"
/// forever and raising a second card if the session is ever reopened.
#[test]
fn restart_retires_the_card_of_a_gone_process() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);

    h.processes.kill(PID);
    let report = h.restart();

    assert_eq!(report.active_cards, 0);
    h.wait(3 * STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);
    assert_eq!(
        h.engine.inspect(session).unwrap().display,
        SessionDisplay::Interrupted
    );
}

/// Catches: a stall watch that ever acts on its own. Hours of silence must leave
/// the session running with no stop on record.
#[test]
fn the_watch_never_stops_anything() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(6 * 60 * MINUTE);
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.session.lifecycle, Lifecycle::Open);
    assert_eq!(view.session.stop_requested_ms, None);
    assert!(view.run.state.is_active());
    assert_eq!(h.stuck_alerts(), 1);
}

/// Catches: keep waiting either not holding off for 30 minutes, or silencing the
/// watch for good.
#[test]
fn keep_waiting_holds_off_for_thirty_minutes_then_raises_one_card() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(STALL_AFTER_MS);
    let first = card(&h);

    assert!(h.engine.keep_waiting(first.id).unwrap());
    assert!(h.engine.active_cards().unwrap().is_empty());
    assert!(!h.engine.keep_waiting(first.id).unwrap(), "already retired");

    h.wait(KEEP_WAITING_MS - 1_000);
    assert_eq!(h.stuck_alerts(), 1);
    h.wait(1_000);
    assert_eq!(h.stuck_alerts(), 2);
    let second = card(&h);
    assert_eq!(second.session_id, session);
    assert_eq!(second.elapsed_ms(h.now()), STALL_AFTER_MS + KEEP_WAITING_MS);
}

/// Catches: new activity closing a real request along with the card.
#[test]
fn new_activity_retires_the_card_but_never_a_request() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);

    h.send(session, acp::request_permission(AGENT, 1, "git push"));
    assert!(h.engine.active_cards().unwrap().is_empty());

    h.send(session, acp::message(AGENT, "while I wait"));
    h.end_turn(session, "end_turn");
    let view = h.engine.inspect(session).unwrap();
    assert_eq!(view.open_requests.len(), 1);
    assert_eq!(view.display, SessionDisplay::NeedsYou);

    // A redelivered request must not alert twice.
    h.send(session, acp::request_permission(AGENT, 1, "git push"));
    let alerts = h.alerts.iter().filter(|a| matches!(a, Alert::NeedsYou(_)));
    assert_eq!(alerts.count(), 1);
}

/// Catches: a session whose turn has ended being watched as if it should be busy.
/// Idle is the owner's turn, not a stall.
#[test]
fn an_idle_session_is_not_watched() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.send(session, acp::message(AGENT, "done for now"));
    h.end_turn(session, "end_turn");
    h.wait(3 * STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 0);
}

/// Catches: a message after the handshake moving a run nobody started to `Running`,
/// which is not a turn and must not read as one going quiet. A prompt ends this.
#[test]
fn a_ready_session_stays_the_owners_turn_until_a_prompt() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.engine.session_ready(session).unwrap();
    h.send(session, acp::message(AGENT, "hello from the adapter"));
    h.wait(3 * STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 0);

    h.engine
        .prompt_sent(session, &serde_json::json!(5))
        .unwrap();
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);
}

/// Catches: a card raised by a slow handshake outliving the handshake.
#[test]
fn becoming_ready_retires_a_handshakes_card() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    h.wait(STALL_AFTER_MS);
    assert_eq!(h.stuck_alerts(), 1);
    h.engine.session_ready(session).unwrap();
    assert!(h.engine.active_cards().unwrap().is_empty());
}

/// Catches: a database from before `ready` refusing to open.
#[test]
fn a_previous_schema_opens_and_keeps_its_sessions() {
    let mut h = Harness::new();
    let session = h.engine.launch(PID).unwrap();
    let conn = rusqlite::Connection::open(h.dir.db()).unwrap();
    conn.execute_batch(
        "DROP TABLE hook_reports; ALTER TABLE sessions DROP COLUMN ready;
             PRAGMA user_version = 2;",
    )
    .unwrap();
    drop(conn);
    h.restart();
    let view = h.engine.inspect(session).unwrap();
    assert!(!view.session.ready);
    h.engine.session_ready(session).unwrap();
    assert!(h.engine.inspect(session).unwrap().session.ready);
}

/// Catches: a database from before hook reports refusing to open.
#[test]
fn a_database_without_reports_opens_and_takes_them() {
    let mut h = Harness::new();
    let conn = rusqlite::Connection::open(h.dir.db()).unwrap();
    conn.execute_batch("DROP TABLE hook_reports; PRAGMA user_version = 3;")
        .unwrap();
    drop(conn);
    h.restart();
    assert_eq!(h.engine.session_reports("s1").unwrap(), vec![]);
}
