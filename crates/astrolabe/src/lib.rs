//! The Astrolabe core: live state for agent sessions, kept in one local database.
pub mod acp;
pub mod driver;
mod engine;
mod error;
pub mod events;
pub mod guard;
pub mod hook;
mod host;
mod job;
pub mod layer;
pub mod mcp;
pub mod model;
pub mod protect;
mod serve;
mod stall;
mod store;
pub mod worktree;

pub use engine::Engine;
pub use error::{Error, Result};
pub use host::{Clock, OsProcessProbe, ProcessProbe, SystemClock};
pub use serve::serve;
pub use stall::{KEEP_WAITING_MS, STALL_AFTER_MS};
pub use store::default_db_path;
