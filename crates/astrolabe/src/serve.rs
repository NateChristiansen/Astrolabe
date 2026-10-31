//! `astrolabe serve`: the long-lived core process, separate from any window.
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::host::{OsProcessProbe, SystemClock};
use crate::model::Alert;
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(15);

/// Runs until the process is stopped. Every state change is already committed,
/// so stopping needs no shutdown step; the next start reconciles.
pub fn serve(db: &Path) -> Result<()> {
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent)?;
    }
    // The OS drops this lock when the process dies, however it dies.
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(db.with_extension("lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(Error::AlreadyServing(db.to_owned())),
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }

    let engine = Engine::open(db, SystemClock, OsProcessProbe)?;
    let report = engine.reconcile()?;
    say(&format!(
        "serving {}: {} interrupted, {} cancelled, {} open requests, {} active cards",
        db.display(),
        report.interrupted.len(),
        report.cancelled.len(),
        report.open_requests,
        report.active_cards,
    ));
    loop {
        for alert in engine.tick()? {
            if let Alert::MayBeStuck(card) = alert {
                say(&format!(
                    "session {}: {}",
                    card.session_id,
                    card.summary(card.raised_ms)
                ));
            }
        }
        sleep(TICK);
    }
}

/// A closed stdout must not take the core down with it.
fn say(line: &str) {
    let _ = writeln!(io::stdout(), "{line}");
}
