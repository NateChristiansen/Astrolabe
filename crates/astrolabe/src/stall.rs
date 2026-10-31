//! Core-observed stall watch. It flags and never stops anything.
use crate::engine::Engine;
use crate::error::Result;
use crate::model::*;

/// A starting value, to be tuned from recorded warnings.
pub const STALL_AFTER_MS: i64 = 10 * 60_000;
pub const KEEP_WAITING_MS: i64 = 30 * 60_000;

impl Engine {
    /// Raises a card for each session with an active run, no open request and
    /// no real activity for `STALL_AFTER_MS`. The timer's base is the session's
    /// last activity, which starts at launch and which keepalives do not move.
    pub fn tick(&self) -> Result<Vec<Alert>> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let mut alerts = Vec::new();
            for session in self.store.open_sessions()? {
                if let Some(card) = self.check(&session, now)? {
                    alerts.push(Alert::MayBeStuck(card));
                }
            }
            Ok(alerts)
        })
    }

    fn check(&self, session: &Session, now: i64) -> Result<Option<Card>> {
        let quiet_for = now - session.activity_ms;
        if quiet_for < STALL_AFTER_MS
            || now < session.snooze_until_ms
            || !self.store.latest_run(session.id)?.state.is_active()
            || !self.store.open_requests(Some(session.id))?.is_empty()
        {
            return Ok(None);
        }
        let tools = self.store.running_tools(session.id)?;
        let tool = tools.first();
        let since_ms = tool.map_or(session.activity_ms, |tool| tool.started_ms);
        match self.store.insert_card(session.id, tool, since_ms, now)? {
            Some(id) => self.store.card_if_active(id),
            None => Ok(None),
        }
    }

    /// The card's keep-waiting action: hides the card and holds off another for
    /// 30 minutes. Returns false when the card has already retired.
    pub fn keep_waiting(&self, card_id: CardId) -> Result<bool> {
        let now = self.clock.now_ms();
        self.store.transaction(|| {
            let Some(card) = self.store.card_if_active(card_id)? else {
                return Ok(false);
            };
            self.store.retire_card(card.session_id, now)?;
            self.store
                .set_snooze(card.session_id, now + KEEP_WAITING_MS)?;
            Ok(true)
        })
    }
}
