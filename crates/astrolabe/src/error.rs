use crate::model::{OutcomeId, SessionId};
use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    UnsupportedSchema(i64),
    UnknownSession(SessionId),
    UnknownRequest(String),
    SessionNotOpen(SessionId),
    TurnInProgress(SessionId),
    NoActiveRun(SessionId),
    /// An agent error that answers nothing the run is waiting on. No state changed.
    AgentRpc {
        session_id: SessionId,
        rpc_id: String,
        message: String,
    },
    OutcomeNotSubmitted(OutcomeId),
    AlreadyServing(std::path::PathBuf),
    SessionStillOpen(SessionId),
    /// The adapter process has gone, or its pipe has; nothing more can be sent.
    AgentGone(SessionId),
    /// The session has no agent session yet, so nothing can be sent to it.
    NotReady(SessionId),
    /// A resume was asked for that cannot even be attempted.
    Launch {
        session_id: SessionId,
        message: String,
    },
    /// The request came from an earlier process, so no one can receive its answer.
    RequestNotLive(String),
    UnknownOption {
        request: String,
        option: String,
    },
    AdapterNotFound(String),
    /// The layer can't be delivered as written. The launch must not go ahead.
    InvalidLayer(String),
    /// The guard didn't answer its self-test. A launch would have run unguarded.
    GuardNotRunning(String),
    /// A session here would misbehave without saying so. Nothing was started.
    UnsafeSessionFolder(String),
    /// Setup can't go on, for the reason given.
    Setup(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "database error: {error}"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "database schema version {version} is not supported")
            }
            Self::UnknownSession(id) => write!(f, "unknown session {id}"),
            Self::UnknownRequest(id) => write!(f, "unknown request {id}"),
            Self::SessionNotOpen(id) => write!(f, "session {id} is not open"),
            Self::TurnInProgress(id) => write!(f, "session {id} already has an active run"),
            Self::NoActiveRun(id) => write!(f, "session {id} has no active run"),
            Self::AgentRpc {
                session_id,
                rpc_id,
                message,
            } => write!(
                f,
                "session {session_id}: agent error for {rpc_id}: {message}"
            ),
            Self::OutcomeNotSubmitted(id) => write!(f, "outcome {id} is no longer submitted"),
            Self::AlreadyServing(path) => {
                write!(f, "another core is serving {}", path.display())
            }
            Self::SessionStillOpen(id) => write!(f, "session {id} is still open"),
            Self::AgentGone(id) => write!(f, "session {id}: the agent process is gone"),
            Self::NotReady(id) => write!(f, "session {id} is not ready yet"),
            Self::Launch {
                session_id,
                message,
            } => write!(f, "session {session_id}: launch failed: {message}"),
            Self::RequestNotLive(id) => {
                write!(f, "request {id} belongs to a process that has gone")
            }
            Self::UnknownOption { request, option } => {
                write!(f, "request {request} did not offer option {option}")
            }
            Self::AdapterNotFound(detail) => write!(f, "adapter not found: {detail}"),
            Self::InvalidLayer(detail) => write!(f, "invalid layer: {detail}"),
            Self::UnsafeSessionFolder(detail) => write!(f, "unsafe session folder: {detail}"),
            Self::Setup(detail) => write!(f, "{detail}"),
            Self::GuardNotRunning(detail) => {
                write!(f, "the command guard is not running: {detail}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
