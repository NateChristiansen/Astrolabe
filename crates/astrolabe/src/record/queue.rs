//! A save in a hook that finds another writer holding the notes lock waits its turn instead
//! of waiting for the lock. It plans its write without the lock, keeps it as a queued intent,
//! and waits: whoever holds the lock commits it (`Notes::reconcile`), or the save takes the
//! lock itself once it's free. A queued save costs the lock its few git calls, not a pull and
//! a reconcile of its own: each save held the lock about 3 s on Windows, so of 24 sessions
//! saving in the same minute there, 3 were refused on every try (2026-11-18).
//!
//! It isn't pulled first, and its reply says so. Its note is checked again when it's committed,
//! and one that changed while it waited is left out. A note that reads as changed when it's
//! planned may be a sync's rebase under way, so that save waits for the lock and is planned
//! again under it, refused there if it's really stale. Another machine's saves reach it as they reach any local
//! commit, through the sync's merge.
use super::{
    Attempt, KEPT, PLAN_UNDER_LOCK, Plan, Reply, SaveRequest, attempt, bound_label,
    canonical_membership, now_reads, plan, saved_reply, told, unfinished_pieces,
};
use crate::activate::{read_state, state_path};
use crate::effort::{EffortCommand, EffortFile};
use crate::notes::{self, Notes, Op, Pull, Waiting};
use crate::session::SessionState;
use std::path::Path;
use std::time::Duration;

/// Whether this save waits its turn when the lock is held: a save of the session's own
/// effort, in a hook. A save by number, an initiative page's (its reply reads every member)
/// and a done save (its lessons and issue) wait for the lock as before.
pub(super) fn queues(req: &SaveRequest) -> bool {
    crate::deadline::in_hook()
        && req.target.is_none()
        && !req.page_binding
        && req.status.as_deref() != Some("done")
}

/// The key of this session's queued save of this same command, not yet committed or left
/// out: a retry of a save whose hook ran out of time while it waited.
pub(super) fn pending(
    notes: &Notes,
    cmd: &EffortCommand,
    session: &str,
    cwd: &Path,
) -> Option<String> {
    let bound = bound_label(&notes.state, session);
    let (digest, cwd) = (cmd.digest(), crate::hook::comparable(cwd));
    let ops = notes.intents().into_iter();
    ops.filter(|op| op.queued && !op.committed && op.result.is_none())
        .find(|op| {
            op.digest.as_deref() == Some(digest.as_str())
                && op.cwd.as_deref() == Some(cwd.as_str())
                && op.session.as_deref() == Some(session)
                && op.bound == bound
        })
        .map(|op| op.key)
}

/// The save, waiting its turn. None when its note read as changed since it was delivered:
/// read without the lock, that may be a sync's rebase under way, so the save is planned
/// again under the lock instead of refused.
#[allow(clippy::too_many_arguments)]
pub(super) fn save(
    notes: &Notes,
    cmd: &EffortCommand,
    req: &SaveRequest,
    state: &SessionState,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    waiting: Waiting,
) -> Result<Option<String>, String> {
    let record = &req.record;
    let key = match attempt(notes, cmd, key, session, cwd) {
        Attempt::Answered(reply) => return Ok(Some(reply)),
        Attempt::Key(key) => key,
    };
    if notes.already_applied(&key) {
        return Ok(Some(format!(
            "Already saved: this same tool call was done before (a retry), so nothing new was written to {record}."
        )));
    }
    let mut notes_lines = Vec::new();
    let (op, file) = match pending(notes, cmd, session, cwd) {
        // A retry of a save still queued waits for that one: planned again, it would land
        // twice, or be left out against its own first attempt.
        Some(first) => {
            let op = (notes.intents().into_iter())
                .find(|op| op.key == first)
                .ok_or("its queued write went missing; run the same command again")?;
            let text = op.files.first().map(|(_, text)| text.as_str());
            let file = EffortFile::parse(text.unwrap_or_default())?;
            told().earlier = Some(op.key.clone());
            (op, file)
        }
        None => {
            let initiative = (req.initiative)
                .map(|name| canonical_membership(&notes.root, name))
                .transpose()?;
            let planned = plan(
                notes,
                false,
                cmd,
                req,
                state,
                &key,
                session,
                provider,
                cwd,
                initiative.as_deref(),
                &mut notes_lines,
            );
            let Plan { mut op, file, .. } = match planned {
                Err(why) if why == PLAN_UNDER_LOCK => return Ok(None),
                planned => planned?,
            };
            op.queued = true;
            notes.save_intent(&op)?;
            (op, file)
        }
    };
    let started = std::time::Instant::now();
    let turn = wait_turn(notes, &op.key);
    drop(waiting);
    match turn {
        Turn::Saved => {
            crate::nudge::clear_shell(session);
            let pulled = Pull::Offline(match notes.has_remote() {
                true => "queued behind other saves on this machine, so not pulled first".to_owned(),
                false => "no remote".to_owned(),
            });
            let head = saved_reply(&Reply {
                record,
                status: file.prop("status").unwrap_or("?"),
                body_replaced: cmd.body.is_some(),
                journal_written: !op.journal.is_empty(),
                pulled: &pulled,
                sync_started: notes.spawn_sync(),
                unchanged: None,
                notes_lines: &notes_lines,
                issue: None,
                unfinished: &unfinished_pieces(&file),
            });
            notes.finish(&op.key, &head);
            Ok(Some(head))
        }
        Turn::LeftOut => Err(left_out(notes, cmd, session, cwd, record)),
        Turn::Gone => Err(
            "this save's queued write is gone and no commit has it, so nothing was written. Run the same command again"
                .to_owned(),
        ),
        Turn::Waiting => {
            let subject = record[..1].to_uppercase() + &record[1..];
            Err(format!(
                "{subject} not saved yet: other saves on this machine held the notes for the {} s this hook could wait, so this save is queued behind them. It's committed only if the effort still reads as this save found it. {KEPT}: run the same command again to see whether it landed, changing nothing in it.",
                crate::deadline::seconds(started.elapsed())
            ))
        }
    }
}

enum Turn {
    Saved,
    /// Its note changed while it waited, so it wasn't committed.
    LeftOut,
    /// Its intent is gone with no commit carrying it.
    Gone,
    /// Neither committed nor left out in the time this hook could wait.
    Waiting,
}

/// Waits for the queued save with this key to be committed or left out, taking the lock to
/// commit it whenever the lock is free, until the hook has no time left for a write.
fn wait_turn(notes: &Notes, key: &str) -> Turn {
    loop {
        let op = std::fs::read(notes.intent_path(key)).ok();
        match op.and_then(|bytes| serde_json::from_slice::<Op>(&bytes).ok()) {
            // Its committer records a result once the session's state is written too.
            Some(op) if op.committed && op.result.is_some() => return Turn::Saved,
            Some(op) if !op.committed && op.result.is_some() => return Turn::LeftOut,
            Some(_) => {}
            None if notes.already_applied(key) => return Turn::Saved,
            None => return Turn::Gone,
        }
        if crate::deadline::short_of(crate::deadline::Step::Write).is_some() {
            return Turn::Waiting;
        }
        if let Some(_held) = notes.try_lock() {
            crate::application::reconcile_first(notes, Some(key));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The reply for a save left out because its note changed while it waited: the note as it
/// is now, delivered anew so the next save from it goes through, as for any stale save.
fn left_out(notes: &Notes, cmd: &EffortCommand, session: &str, cwd: &Path, record: &str) -> String {
    let why = format!(
        "{record} changed while this save waited its turn behind other saves on this machine, so this save was left out and nothing was overwritten."
    );
    // Read under the lock, or with no rebase under way: mid-rebase the note reads as origin's
    // older copy, and delivering that would refuse the session's next save for nothing.
    let held = notes.try_lock();
    if held.is_none() && notes.rebase_dir().ok().flatten().is_some() {
        return format!("{why} Save again: that save's reply shows the effort as it is now.");
    }
    let dir = &notes.state;
    let now = read_state(dir, session)
        .ok()
        .flatten()
        .and_then(|mut state| {
            let req = SaveRequest::parse(cmd, &state, cwd).ok()?;
            let text = std::fs::read_to_string(notes.root.join(&req.rel)).ok()?;
            let file = EffortFile::parse(&text).ok()?;
            state.delivered.set(notes::content_hash(&text));
            state.delivered_ms.set(notes::now_ms());
            let _ = notes::write_text(&state_path(dir, session), &state.to_string());
            Some(now_reads(&file))
        });
    match now {
        Some(now) => format!("{why} {now}"),
        None => format!(
            "{why} Load it again with `astrolabe activate --effort <n>` and save from what it says."
        ),
    }
}
