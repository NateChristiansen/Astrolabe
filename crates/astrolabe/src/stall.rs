//! Core-observed stall watch. It flags and never stops anything.
use crate::engine::Engine;
use crate::error::Result;
use crate::model::*;

/// A starting value, to be tuned from recorded warnings.
pub const STALL_AFTER_MS: i64 = 10 * 60_000;
pub const KEEP_WAITING_MS: i64 = 30 * 60_000;

impl Engine {
    /// Raises what has become due in each session with an active run and no open
    /// request. A running tool flags on its own elapsed time, whatever else is
    /// happening. Only with no tool running does the quiet rule apply, based on
    /// the last activity, which starts at launch and which keepalives do not move.
    pub fn tick(&self) -> Result<Vec<Alert>> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let mut alerts = Vec::new();
            for session in self.store.open_sessions()? {
                for card in self.check(&session, now)? {
                    alerts.push(Alert::MayBeStuck(card));
                }
            }
            Ok(alerts)
        })
    }

    fn check(&self, session: &Session, now: i64) -> Result<Vec<Card>> {
        let mut raised = Vec::new();
        if !self.store.latest_run(session.id)?.state.is_active()
            || !self.store.open_requests(Some(session.id))?.is_empty()
        {
            return Ok(raised);
        }
        let tools = self.store.running_tools(session.id)?;
        let mut raise = |tool: Option<&Tool>, since_ms: i64, snooze_until_ms: i64| -> Result<()> {
            if now - since_ms >= STALL_AFTER_MS
                && now >= snooze_until_ms
                && let Some(id) = self.store.insert_card(session.id, tool, since_ms, now)?
            {
                raised.extend(self.store.card_if_active(id)?);
            }
            Ok(())
        };
        if tools.is_empty() {
            raise(None, session.activity_ms, session.snooze_until_ms)?;
        }
        for tool in &tools {
            raise(Some(tool), tool.started_ms, tool.snooze_until_ms)?;
        }
        Ok(raised)
    }

    /// The card's keep-waiting action: hides this card and holds off another for
    /// the same tool or quiet spell for 30 minutes. Returns false when the card
    /// has already retired.
    pub fn keep_waiting(&self, card_id: CardId) -> Result<bool> {
        let now = self.clock.now_ms();
        let until = now + KEEP_WAITING_MS;
        self.store.transaction(|| {
            let Some(card) = self.store.card_if_active(card_id)? else {
                return Ok(false);
            };
            self.store.retire_card(card.id, now)?;
            match &card.tool_call_id {
                Some(id) => {
                    let worker = card.worker_id.as_deref();
                    if let Some(mut tool) = self.store.tool(card.session_id, worker, id)? {
                        tool.snooze_until_ms = until;
                        self.store.upsert_tool(card.session_id, &tool)?;
                    }
                }
                None => self.store.set_snooze(card.session_id, until)?,
            }
            Ok(true)
        })
    }
}
