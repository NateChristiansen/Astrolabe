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

/// The board server's pull while a page is open: what a pickup does (leftovers finished,
/// then a pull), but skipped while a writer holds the lock, and with the tree left alone
/// unless origin has something new. None when skipped.
pub fn background_pull(notes: &Notes) -> Option<Pull> {
    let _held = notes.try_lock()?;
    let leftovers = notes.reconcile();
    let result = if leftovers.is_empty() {
        notes.pull_if_behind()
    } else {
        // Nothing moves under an unfinished write.
        Pull::Offline("unfinished writes".to_owned())
    };
    refresh_views(notes);
    Some(result)
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
