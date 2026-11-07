//! Coordinate note operations and their presentation effects. Storage never renders pages.
use crate::notes::{Notes, Pull};
use std::path::PathBuf;

pub fn finish(notes: &Notes, key: &str, result: &str) {
    notes.finish(key, result);
    refresh_views(notes);
}

pub fn pull(notes: &Notes) -> Pull {
    let result = notes.pull();
    refresh_views(notes);
    result
}

pub fn refresh(notes: &Notes) -> (Vec<String>, Pull) {
    let result = notes.refresh();
    if notes.take_changes()
        && let Ok(_held) = notes.lock()
    {
        crate::home_effects::redraw(notes);
    }
    result
}

pub fn sync(notes: &Notes) -> Result<String, String> {
    let result = notes.sync();
    if notes.take_changes()
        && let Ok(_held) = notes.lock()
    {
        crate::home_effects::redraw(notes);
    }
    result
}

/// A session's explicit sync does local recovery here and leaves network work to a
/// detached process, so a slow fetch or push cannot consume the pre-command hook timeout.
pub fn request_sync(notes: &Notes) -> Result<String, String> {
    let held = notes.lock()?;
    let leftovers = reconcile(notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "Unfinished writes failed again, so sync could not start: {}; check astrolabe notes status.",
            leftovers.join("; ")
        ));
    }
    let remote = notes.has_remote();
    drop(held);
    if !remote {
        Ok("Sync unavailable: no remote configured; check astrolabe notes status.".to_owned())
    } else if notes.spawn_sync() {
        Ok("Background sync started; check astrolabe notes status for the result.".to_owned())
    } else {
        Err(
            "Background sync could not start; check astrolabe notes status for the result."
                .to_owned(),
        )
    }
}

pub fn board(notes: &Notes) -> Result<PathBuf, String> {
    crate::home_effects::board(notes)
}

/// The caller holds the notes lock. Several recovered writes need only one fresh view.
fn refresh_views(notes: &Notes) {
    if notes.take_changes() {
        crate::home_effects::redraw(notes);
    }
}

pub fn reconcile(notes: &Notes) -> Vec<String> {
    let result = notes.reconcile();
    refresh_views(notes);
    result
}
