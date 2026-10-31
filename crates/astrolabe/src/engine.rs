//! Session, run, worker, request and outcome transitions, plus restart reconcile.
use crate::error::{Error, Result};
use crate::events::{AgentMessage, CoreEvent, ToolEvent, TurnEnd, map_message};
use crate::host::{Clock, ProcessProbe};
use crate::model::*;
use crate::store::Store;
use serde_json::Value;
use std::path::Path;

pub struct Engine {
    pub(crate) store: Store,
    pub(crate) clock: Box<dyn Clock>,
    probe: Box<dyn ProcessProbe>,
}

impl Engine {
    pub fn open(
        path: &Path,
        clock: impl Clock + 'static,
        probe: impl ProcessProbe + 'static,
    ) -> Result<Self> {
        Ok(Self {
            store: Store::open(path)?,
            clock: Box::new(clock),
            probe: Box::new(probe),
        })
    }

    /// Starts the stall timer: a launch that never speaks must still be noticed.
    pub fn launch(&self, pid: u32) -> Result<SessionId> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let id = self.store.insert_session(pid, now)?;
            self.store.insert_run(id, RunState::Queued)?;
            Ok(id)
        })
    }

    /// Begins the next turn of an open session whose last run has ended.
    pub fn start_turn(&self, session_id: SessionId) -> Result<RunId> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            if self.store.session(session_id)?.lifecycle != Lifecycle::Open {
                return Err(Error::SessionNotOpen(session_id));
            }
            if self.store.latest_run(session_id)?.state.is_active() {
                return Err(Error::TurnInProgress(session_id));
            }
            self.store.record_activity(session_id, now)?;
            self.store.insert_run(session_id, RunState::Queued)
        })
    }

    /// A worker is a child session under its parent: it has no state of its own,
    /// and its activity and requests count toward the parent.
    pub fn register_worker(
        &self,
        session_id: SessionId,
        agent_session_id: &str,
        title: &str,
    ) -> Result<()> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            self.store.session(session_id)?;
            self.store
                .insert_worker(agent_session_id, session_id, title)?;
            self.touch(session_id, now)
        })
    }

    /// Applies one raw ACP-shaped message from the agent process of this session.
    pub fn ingest(&self, session_id: SessionId, message: &Value) -> Result<Vec<Alert>> {
        match map_message(message) {
            Some(message) => self.apply(session_id, &message),
            None => Ok(Vec::new()),
        }
    }

    pub fn apply(&self, session_id: SessionId, message: &AgentMessage) -> Result<Vec<Alert>> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let session = self.store.session(session_id)?;
            let worker = match &message.agent_session_id {
                Some(id) if self.store.is_worker(id, session_id)? => Some(id.as_str()),
                _ => None,
            };
            // A request is never dropped, even from a session thought to be gone.
            if let CoreEvent::RequestOpened { rpc_id, title } = &message.event {
                return self.open_request(session_id, worker, rpc_id, title, now);
            }
            if session.lifecycle != Lifecycle::Open {
                return Ok(Vec::new());
            }
            match &message.event {
                CoreEvent::Content => self.touch(session_id, now)?,
                CoreEvent::Tool(event) => self.apply_tool(session_id, worker, event, now)?,
                CoreEvent::TurnEnded(end) => self.end_turn(session_id, *end, now)?,
                CoreEvent::Keepalive | CoreEvent::RequestOpened { .. } => {}
            }
            Ok(Vec::new())
        })
    }

    fn apply_tool(
        &self,
        session_id: SessionId,
        worker: Option<&str>,
        event: &ToolEvent,
        now: i64,
    ) -> Result<()> {
        // After the turn ends, a late tool must not linger into the next run's card.
        if !self.store.latest_run(session_id)?.state.is_active() {
            return Ok(());
        }
        let Some(mut tool) = self.store.tool(session_id, worker, &event.tool_call_id)? else {
            self.store.upsert_tool(
                session_id,
                &Tool {
                    tool_call_id: event.tool_call_id.clone(),
                    worker_id: worker.map(str::to_owned),
                    title: event.title.clone().unwrap_or(event.tool_call_id.clone()),
                    status: event.status.unwrap_or(ToolStatus::Pending),
                    started_ms: now,
                    fingerprint: event.fingerprint.clone(),
                },
            )?;
            return self.touch(session_id, now);
        };
        // A late keepalive must not bring a finished tool back to life.
        if tool.status == ToolStatus::Finished {
            return Ok(());
        }
        let mut progressed = false;
        if let Some(status) = event.status.filter(|status| *status != tool.status)
            && status != ToolStatus::Pending
        {
            if tool.status == ToolStatus::Pending {
                tool.started_ms = now;
            }
            tool.status = status;
            progressed = true;
        }
        if event.fingerprint.is_some() && event.fingerprint != tool.fingerprint {
            tool.fingerprint.clone_from(&event.fingerprint);
            progressed = true;
        }
        if let Some(title) = &event.title {
            tool.title.clone_from(title);
        }
        self.store.upsert_tool(session_id, &tool)?;
        if progressed {
            self.touch(session_id, now)?;
        }
        Ok(())
    }

    fn open_request(
        &self,
        session_id: SessionId,
        worker: Option<&str>,
        rpc_id: &str,
        title: &str,
        now: i64,
    ) -> Result<Vec<Alert>> {
        // JSON-RPC ids restart with each process, so the id is scoped to the run.
        let run = self.store.latest_run(session_id)?;
        let request = Request {
            id: format!("{}:{rpc_id}", run.id),
            session_id,
            worker_id: worker.map(str::to_owned),
            title: title.to_owned(),
            opened_ms: now,
            resolution: None,
        };
        if !self.store.insert_request(&request)? {
            return Ok(Vec::new());
        }
        self.touch(session_id, now)?;
        if run.state.is_active() {
            self.store.set_run_state(run.id, RunState::Waiting)?;
        }
        Ok(vec![Alert::NeedsYou(request)])
    }

    fn end_turn(&self, session_id: SessionId, end: TurnEnd, now: i64) -> Result<()> {
        let run = self.store.latest_run(session_id)?;
        if !run.state.is_active() {
            return Ok(());
        }
        let state = match end {
            TurnEnd::Completed => RunState::Completed,
            TurnEnd::Failed => RunState::Failed,
            TurnEnd::Cancelled => RunState::Cancelled,
        };
        self.store.set_run_state(run.id, state)?;
        self.store.finish_tools(session_id)?;
        self.store.set_stop_requested(session_id, None)?;
        self.store.record_activity(session_id, now)?;
        self.store.retire_card(session_id, now)
    }

    /// Real activity restarts the stall timer and retires the card. Requests are
    /// untouched: only `resolve_request` closes one.
    pub(crate) fn touch(&self, session_id: SessionId, now: i64) -> Result<()> {
        self.store.record_activity(session_id, now)?;
        self.store.retire_card(session_id, now)?;
        let run = self.store.latest_run(session_id)?;
        if run.state == RunState::Queued {
            self.store.set_run_state(run.id, RunState::Running)?;
        }
        Ok(())
    }

    /// Resolves by id. Returns false when already resolved: the first answer stands.
    pub fn resolve_request(&self, id: &str, resolution: &str) -> Result<bool> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let (request, resolved) = self
                .store
                .request(id)?
                .ok_or_else(|| Error::UnknownRequest(id.to_owned()))?;
            if resolved {
                return Ok(false);
            }
            self.store.resolve_request(id, resolution, now)?;
            let session_id = request.session_id;
            if self.store.open_requests(Some(session_id))?.is_empty() {
                let run = self.store.latest_run(session_id)?;
                if run.state == RunState::Waiting {
                    self.store.set_run_state(run.id, RunState::Running)?;
                    self.store.restart_tool_clocks(session_id, now)?;
                    self.touch(session_id, now)?;
                }
            }
            Ok(true)
        })
    }

    /// Records the owner's stop. The run is not cancelled until there is evidence
    /// the agent stopped; the driver delivers the stop itself (a later step).
    pub fn request_stop(&self, session_id: SessionId) -> Result<()> {
        let now = self.clock.now_ms();
        self.store.session(session_id)?;
        self.store.set_stop_requested(session_id, Some(now))
    }

    /// The agent process exited. After a requested stop this is stop evidence;
    /// otherwise an unfinished run failed. Open requests stay open either way.
    pub fn process_exited(&self, session_id: SessionId) -> Result<()> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let session = self.store.session(session_id)?;
            if session.lifecycle == Lifecycle::Closed {
                return Ok(());
            }
            let state = if session.stop_requested_ms.is_some() {
                RunState::Cancelled
            } else {
                RunState::Failed
            };
            self.close_out(&session, state, Lifecycle::Closed, now)
        })
    }

    fn close_out(
        &self,
        session: &Session,
        run_state: RunState,
        lifecycle: Lifecycle,
        now: i64,
    ) -> Result<()> {
        let run = self.store.latest_run(session.id)?;
        if run.state.is_active() {
            self.store.set_run_state(run.id, run_state)?;
        }
        self.store.finish_tools(session.id)?;
        self.store.retire_card(session.id, now)?;
        self.store.set_lifecycle(session.id, lifecycle)
    }

    /// Runs once at start. Sessions whose process is gone become interrupted, or
    /// cancelled when a stop was already asked for. Nothing here alerts: open
    /// requests stay open and live sessions keep their timers and cards.
    pub fn reconcile(&self) -> Result<ReconcileReport> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let mut report = ReconcileReport::default();
            for session in self.store.open_sessions()? {
                if self.probe.is_alive(session.pid) {
                    continue;
                }
                if session.stop_requested_ms.is_some() {
                    self.close_out(&session, RunState::Cancelled, Lifecycle::Closed, now)?;
                    report.cancelled.push(session.id);
                } else {
                    self.close_out(&session, RunState::Interrupted, Lifecycle::Interrupted, now)?;
                    report.interrupted.push(session.id);
                }
            }
            report.open_requests = self.store.open_requests(None)?.len();
            report.active_cards = self.store.active_cards()?.len();
            Ok(report)
        })
    }

    /// Submits an outcome, replacing any earlier one still awaiting a check.
    pub fn record_outcome(&self, session_id: SessionId, summary: &str) -> Result<OutcomeId> {
        self.store.transaction(|| {
            let run = self.store.latest_run(session_id)?;
            self.store.supersede_submitted(session_id)?;
            self.store.insert_outcome(session_id, run.id, summary)
        })
    }

    pub fn accept_outcome(&self, id: OutcomeId) -> Result<()> {
        if self.store.accept_outcome(id)? {
            Ok(())
        } else {
            Err(Error::OutcomeNotSubmitted(id))
        }
    }

    pub fn outcomes(&self, session_id: SessionId) -> Result<Vec<Outcome>> {
        self.store.outcomes(session_id)
    }

    pub fn open_requests(&self) -> Result<Vec<Request>> {
        self.store.open_requests(None)
    }

    pub fn active_cards(&self) -> Result<Vec<Card>> {
        self.store.active_cards()
    }

    /// The card's inspect action. Reads only.
    pub fn inspect(&self, session_id: SessionId) -> Result<SessionView> {
        let session = self.store.session(session_id)?;
        let run = self.store.latest_run(session_id)?;
        let open_requests = self.store.open_requests(Some(session_id))?;
        let display = display(&session, &run, !open_requests.is_empty());
        Ok(SessionView {
            display,
            run,
            running_tools: self.store.running_tools(session_id)?,
            open_requests,
            card: self.store.active_card(session_id)?,
            session,
        })
    }
}

fn display(session: &Session, run: &Run, has_open_request: bool) -> SessionDisplay {
    if has_open_request {
        return SessionDisplay::NeedsYou;
    }
    match (session.lifecycle, run.state) {
        (Lifecycle::Interrupted, _) => SessionDisplay::Interrupted,
        (Lifecycle::Closed, RunState::Failed) => SessionDisplay::Failed,
        (Lifecycle::Closed, _) => SessionDisplay::Ended,
        (Lifecycle::Open, RunState::Failed) => SessionDisplay::Failed,
        (Lifecycle::Open, state) if state.is_active() => SessionDisplay::Working,
        // The agent went idle. That is the owner's turn, never "done".
        (Lifecycle::Open, _) => SessionDisplay::YourTurn,
    }
}
