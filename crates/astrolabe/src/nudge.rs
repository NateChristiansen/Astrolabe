//! Detection, so the save instruction isn't advisory: the audit found agents asked once wrote
//! well and skipped the save; "update memory" prompts stay advisory unless a missing write is
//! detected. A mark is set when the session edits a file or commits; at turn end, the repo's
//! fingerprint is compared with the last save's. Work moved without a save gets one bounded
//! message, the same on both clients, never a requirement to save before finishing.
use crate::activate::{read_state, state_dir, state_path};
use crate::notes::{self, Notes, RepoFacts};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Tools whose success means a file changed. Codex's `apply_patch` matches `Edit` or `Write`.
pub const EDIT_TOOLS: [&str; 5] = ["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch"];

/// An answer this long in a session with no effort is more likely findings than chat.
const LONG_ANSWER: usize = 600;

/// Where an unbound session's mark goes.
fn mark_path(dir: &Path, session: &str) -> PathBuf {
    dir.join("marks")
        .join(state_path(dir, session).file_name().unwrap_or_default())
}

/// Records that this session did work: in its state when bound, else in a mark file.
pub fn mark(session: &str) {
    let Some(dir) = state_dir() else { return };
    match read_state(&dir, session) {
        Ok(Some(mut state)) => {
            if !state.mark.is_true() {
                state.mark.set(true);
                let _ = notes::write_text(&state_path(&dir, session), &state.to_string());
            }
        }
        _ => {
            let _ = notes::write_text(
                &mark_path(&dir, session),
                &json!({"at_ms": notes::now_ms()}).to_string(),
            );
        }
    }
}

/// Whether a shell command commits: the one shell edit the guard can see cheaply.
pub fn commits(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    words.windows(2).any(|w| w[0] == "git" && w[1] == "commit")
}

/// The Stop hook's answer, or None when nothing moved since the last save. `stop_hook_active`
/// means this turn was already continued once: then nothing, so it can't loop.
pub fn at_stop(input: &Value, session: &str) -> Option<Value> {
    if input["stop_hook_active"] == true {
        return None;
    }
    let dir = state_dir()?;
    push_due();
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    let codex = crate::activate::provider(input) == "codex";
    let text = match read_state(&dir, session) {
        Ok(Some(state)) if state.worker.is_true() => return None,
        Ok(Some(mut state)) => {
            let facts = RepoFacts::read(&cwd);
            let print = facts.as_ref().map(|f| f.fingerprint.as_str());
            let baseline = state.baseline.as_deref().map(str::to_owned);
            let acknowledged = state.acknowledged.as_deref().map(str::to_owned);
            let moved = print.is_some_and(|p| {
                baseline.as_deref() != Some(p) && acknowledged.as_deref() != Some(p)
            });
            let marked = state.mark.is_true();
            let effort = state.effort.as_deref().unwrap_or_default().to_owned();
            if !moved && !marked {
                // Created and never saved, and a long answer just went by: the findings
                // are probably in that answer and not in the notes. Once per session.
                let unsaved = state.saves.as_ref().copied() == Some(0);
                let long = input["last_assistant_message"]
                    .as_str()
                    .is_some_and(|m| m.len() > LONG_ANSWER);
                let asked = mark_path(&dir, session).with_extension("asked");
                if !(unsaved && long) || asked.exists() {
                    return None;
                }
                let _ = notes::write_text(&asked, "");
                let text = format!(
                    "Astrolabe: effort {effort} was created in this session and never saved, and this turn ended with a long answer. If that answer holds what the owner asked for, put it in the notes: replace Current state with the findings and run `astrolabe effort save` (with --status done if the work is done). If there's nothing to keep yet, say so in a few words and finish. This is the plugin's hook, not the owner: not a request."
                );
                return Some(if codex {
                    json!({"decision": "block", "reason": text})
                } else {
                    json!({"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": text}})
                });
            }
            let what = match (&facts, moved) {
                (Some(f), true) => format!(
                    "the repo changed since the last save of effort {effort} (now {} at {}{})",
                    f.branch,
                    f.head,
                    if f.dirty { ", tree dirty" } else { "" }
                ),
                _ => format!(
                    "files were edited or commits made since the last save of effort {effort}"
                ),
            };
            // Said once per state: the print is acknowledged and the mark cleared.
            state.acknowledge(print);
            let _ = notes::write_text(&state_path(&dir, session), &state.to_string());
            format!(
                "Astrolabe: {what}. If that's work worth remembering, replace Current state and run `astrolabe effort save` now (a closed window saves nothing); if nothing changed worth saving, say so and finish. This is the plugin's hook, not the owner: not a request, and no reason to start an effort."
            )
        }
        Ok(None) => {
            let path = mark_path(&dir, session);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
                "Astrolabe: files were edited or commits made in this session with no effort recorded. If this is work the owner asked for (a change or a plan), run `astrolabe effort new` with its title and goal, then save; if another agent dispatched you, run `astrolabe activate --effort <n> --worker` under its effort; if it was only a look, say so and finish. This is the plugin's hook, not the owner: not a request.".to_owned()
            } else {
                // A long answer with nothing bound and nothing edited gets no ask: a question
                // answered now is not an effort, and the one ask that stood here turned a code
                // review into a journal line (2026-11-04). The session text says when a
                // finding earns a line; the model decides.
                return None;
            }
        }
        Err(_) => return None,
    };
    Some(if codex {
        // Codex's Stop can only continue by a block, whose reason becomes a prompt; the text
        // says what it is, so the model doesn't take it for the owner.
        json!({"decision": "block", "reason": text})
    } else {
        json!({"hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": text}})
    })
}

/// Session end: the session's state says it closed, and a push starts for anything unpublished.
pub fn at_end(session: &str) {
    let Some(dir) = state_dir() else { return };
    if let Ok(Some(mut state)) = read_state(&dir, session) {
        state.closed_ms.set(notes::now_ms());
        let _ = notes::write_text(&state_path(&dir, session), &state.to_string());
    }
    let _ = std::fs::remove_file(mark_path(&dir, session));
    push_pending();
}

/// A detached push once the batching window has passed with commits origin lacks. Every turn
/// end checks, because a session that saves and then sits open never reaches SessionEnd.
pub fn push_due() {
    let Some(dir) = state_dir() else { return };
    let Ok(root) = crate::activate::notes_root() else {
        return;
    };
    let notes = Notes::new(root, dir);
    let now = notes::now_ms();
    // One request at a time: a sync asked for in the last minute is still running or
    // waiting on the lock, and a timed-out one has set its own backoff. An ask stamped in
    // the future (a clock set back) counts as old.
    if now - notes.last_push_ms() > notes::PUSH_EVERY_MS
        && !(0..=notes::RECENT_MS).contains(&(now - notes.last_request_ms()))
        && notes.backing_off().is_none()
        && notes.unpublished() > 0
    {
        notes.spawn_sync();
    }
}

/// A detached push when notes have commits origin lacks, or writes left unfinished.
pub fn push_pending() {
    let Some(dir) = state_dir() else { return };
    let Ok(root) = crate::activate::notes_root() else {
        return;
    };
    let notes = Notes::new(root, dir);
    if notes.unpublished() > 0 || !notes.intents().is_empty() {
        notes.spawn_sync();
    }
}
