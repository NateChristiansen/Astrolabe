//! `astrolabe serve`: the long-lived core process, separate from any window.
use crate::error::{Error, Result};
use crate::host::{Clock, OsProcessProbe, SystemClock};
use crate::shell::{Host, Prepare, real_prepare};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(15);

/// Runs until the process is stopped. Every state change is already committed,
/// so stopping needs no shutdown step; the next start reconciles.
pub fn serve(db: &Path) -> Result<()> {
    let data_dir = db.parent().unwrap_or(Path::new(".")).to_path_buf();
    serve_with(db, SystemClock, TICK, real_prepare(data_dir))
}

/// `serve`, with the launch, clock and tick given. The binary only ever calls `serve`.
pub fn serve_with(
    db: &Path,
    clock: impl Clock + 'static,
    tick: Duration,
    prepare: Prepare,
) -> Result<()> {
    let data_dir = db.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(data_dir)?;
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

    let engine = crate::Engine::open(db, clock, OsProcessProbe)?;
    let report = engine.reconcile()?;
    say(&format!(
        "serving {}: {} interrupted, {} cancelled, {} open requests, {} active cards",
        db.display(),
        report.interrupted.len(),
        report.cancelled.len(),
        report.open_requests,
        report.active_cards,
    ));
    let mut host = Host::new(&engine, db, prepare)?;
    host.reconciled(&report);
    let (inbound, requests) = mpsc::channel();
    crate::bridge::start(data_dir, inbound)?;
    host.run(&requests, tick)
}

/// A closed stdout must not take the core down with it.
fn say(line: &str) {
    let _ = writeln!(io::stdout(), "{line}");
}
