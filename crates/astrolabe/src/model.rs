//! Live-state vocabulary. Rows store these as text so the database stays readable.
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.as_str()))
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                match value.as_str()? {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(FromSqlError::InvalidType),
                }
            }
        }
    };
}

text_enum!(
    /// Interrupted means the process vanished without stop evidence: a restart
    /// cannot say whether the work finished, failed or was stopped.
    Lifecycle { Open = "open", Interrupted = "interrupted", Closed = "closed" }
);

text_enum!(
    /// Cancelled is reachable only from actual stop evidence.
    RunState {
        Queued = "queued",
        Running = "running",
        Waiting = "waiting",
        Completed = "completed",
        Failed = "failed",
        Cancelled = "cancelled",
        Interrupted = "interrupted",
    }
);

impl RunState {
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Waiting)
    }
}

text_enum!(
    ToolStatus { Pending = "pending", Running = "running", Finished = "finished" }
);

text_enum!(
    /// A turn ending is not a submitted outcome; only an explicit record creates one.
    OutcomeState { Submitted = "submitted", Accepted = "accepted", Superseded = "superseded" }
);

/// What the owner sees for a session. Derived, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDisplay {
    Working,
    NeedsYou,
    YourTurn,
    Ended,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAction {
    Inspect,
    KeepWaiting,
    Stop,
}

pub type SessionId = i64;
pub type RunId = i64;
pub type CardId = i64;
pub type OutcomeId = i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub pid: u32,
    pub lifecycle: Lifecycle,
    pub activity_ms: i64,
    pub snooze_until_ms: i64,
    pub stop_requested_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: RunId,
    pub session_id: SessionId,
    pub state: RunState,
    /// The JSON-RPC id of this run's prompt, once sent.
    pub prompt_rpc_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    pub tool_call_id: String,
    pub worker_id: Option<String>,
    pub title: String,
    pub status: ToolStatus,
    pub started_ms: i64,
    pub snooze_until_ms: i64,
}

/// An open request stays open until it is resolved by id; nothing else closes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub session_id: SessionId,
    /// The child session that asked, when a child asked.
    pub worker_id: Option<String>,
    pub title: String,
    pub opened_ms: i64,
    pub resolution: Option<String>,
}

/// One "may be stuck" card: one per stuck tool, or one for a quiet session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub id: CardId,
    pub session_id: SessionId,
    pub worker_id: Option<String>,
    /// The stuck tool's call id and title, or none when the session is quiet.
    pub tool_call_id: Option<String>,
    pub tool: Option<String>,
    pub since_ms: i64,
    pub raised_ms: i64,
}

impl Card {
    pub const ACTIONS: [CardAction; 3] = [
        CardAction::Inspect,
        CardAction::KeepWaiting,
        CardAction::Stop,
    ];

    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.since_ms).max(0)
    }

    pub fn summary(&self, now_ms: i64) -> String {
        let minutes = self.elapsed_ms(now_ms) / 60_000;
        match &self.tool {
            Some(tool) => format!("{tool} running {minutes}m, may be stuck"),
            None => format!("quiet {minutes}m, may be stuck"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub id: OutcomeId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub summary: String,
    pub state: OutcomeState,
}

/// Something newly worth telling the owner. Alerts are produced only at the
/// moment a request or card is created, so restarts cannot repeat them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    NeedsYou(Request),
    MayBeStuck(Card),
}

/// The inspect view: everything the core knows about one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    pub session: Session,
    pub display: SessionDisplay,
    pub run: Run,
    pub running_tools: Vec<Tool>,
    pub open_requests: Vec<Request>,
    pub cards: Vec<Card>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub interrupted: Vec<SessionId>,
    pub cancelled: Vec<SessionId>,
    pub open_requests: usize,
    pub active_cards: usize,
}
