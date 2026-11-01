//! A fake ACP agent, clock and process table. Each test binary uses part of this.
#![allow(dead_code)]
use astrolabe::model::{Alert, SessionId};
use astrolabe::{Clock, Engine, ProcessProbe};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

pub const MINUTE: i64 = 60_000;
pub const PID: u32 = 4242;

#[derive(Clone, Default)]
pub struct FakeClock(Rc<Cell<i64>>);

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}

#[derive(Clone, Default)]
pub struct FakeProcesses(Rc<RefCell<HashSet<u32>>>);

impl FakeProcesses {
    pub fn kill(&self, pid: u32) {
        self.0.borrow_mut().insert(pid);
    }
}

impl ProcessProbe for FakeProcesses {
    fn is_alive(&self, pid: u32) -> bool {
        !self.0.borrow().contains(&pid)
    }
}

/// A unique database directory, removed when the test ends.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "astrolabe-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn db(&self) -> PathBuf {
        self.0.join("astrolabe.db")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An engine on a fake clock, with every alert it has ever produced.
pub struct Harness {
    pub dir: TempDir,
    pub clock: FakeClock,
    pub processes: FakeProcesses,
    pub engine: Engine,
    pub alerts: Vec<Alert>,
}

impl Harness {
    pub fn new() -> Self {
        let dir = TempDir::new();
        let clock = FakeClock::default();
        clock.0.set(1_000 * MINUTE);
        let processes = FakeProcesses::default();
        let engine = Engine::open(&dir.db(), clock.clone(), processes.clone()).unwrap();
        Self {
            dir,
            clock,
            processes,
            engine,
            alerts: Vec::new(),
        }
    }

    pub fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Replaces the engine as a restarted core would, then reconciles.
    pub fn restart(&mut self) -> astrolabe::model::ReconcileReport {
        self.engine =
            Engine::open(&self.dir.db(), self.clock.clone(), self.processes.clone()).unwrap();
        self.engine.reconcile().unwrap()
    }

    pub fn send(&mut self, session: SessionId, message: Value) {
        let alerts = self.engine.ingest(session, &message).unwrap();
        self.alerts.extend(alerts);
    }

    /// A prompt goes out and is answered with this stop reason. Sessions that are
    /// closed refuse the prompt, which is what those fixtures test.
    pub fn end_turn(&mut self, session: SessionId, reason: &str) {
        let answer = acp::turn_end(reason);
        let _ = self.engine.prompt_sent(session, &answer["id"]);
        self.send(session, answer);
    }

    /// Moves the clock with no ticks.
    pub fn advance(&self, ms: i64) {
        self.clock.0.set(self.now() + ms);
    }

    /// Moves time forward a second at a time, as the watch would see it, calling
    /// `each_second` first so a keepalive stream can run alongside.
    pub fn run_for(&mut self, ms: i64, mut each_second: impl FnMut(&mut Self, i64)) {
        let end = self.now() + ms;
        let mut elapsed = 0;
        while self.now() < end {
            self.clock.0.set(self.now() + 1_000);
            elapsed += 1;
            each_second(self, elapsed);
            let alerts = self.engine.tick().unwrap();
            self.alerts.extend(alerts);
        }
    }

    pub fn wait(&mut self, ms: i64) {
        self.run_for(ms, |_, _| {});
    }

    pub fn stuck_alerts(&self) -> usize {
        self.alerts
            .iter()
            .filter(|alert| matches!(alert, Alert::MayBeStuck(_)))
            .count()
    }

    pub fn needs_you_alerts(&self) -> usize {
        self.alerts
            .iter()
            .filter(|alert| matches!(alert, Alert::NeedsYou(_)))
            .count()
    }
}

/// ACP-shaped messages, as an agent adapter would send them.
pub mod acp {
    use super::*;

    fn update(session: &str, update: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": session, "update": update },
        })
    }

    pub fn message(session: &str, text: &str) -> Value {
        update(
            session,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text },
            }),
        )
    }

    pub fn tool_start(session: &str, id: &str, title: &str) -> Value {
        update(
            session,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": title,
                "status": "in_progress",
            }),
        )
    }

    /// The periodic update adapters send while a tool runs: no new content.
    pub fn tool_keepalive(session: &str, id: &str) -> Value {
        update(
            session,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": "in_progress",
            }),
        )
    }

    pub fn tool_output(session: &str, id: &str, text: &str) -> Value {
        update(
            session,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "content": [{ "type": "content", "content": { "type": "text", "text": text } }],
            }),
        )
    }

    pub fn tool_done(session: &str, id: &str) -> Value {
        update(
            session,
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": "completed",
            }),
        )
    }

    pub fn usage(session: &str) -> Value {
        update(
            session,
            json!({ "sessionUpdate": "usage_update", "used": 1200, "size": 200000 }),
        )
    }

    pub fn request_permission(session: &str, rpc_id: u64, title: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": rpc_id,
            "method": "session/request_permission",
            "params": {
                "sessionId": session,
                "toolCall": { "toolCallId": "call-permission", "title": title },
                "options": [
                    { "optionId": "yes", "name": "Allow", "kind": "allow_once" },
                    { "optionId": "no", "name": "Reject", "kind": "reject_once" },
                ],
            },
        })
    }

    pub fn turn_end(reason: &str) -> Value {
        json!({ "jsonrpc": "2.0", "id": 2, "result": { "stopReason": reason } })
    }
}
