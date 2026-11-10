//! Clear an owner wait when the bound conversation records a new human reply. Silent,
//! bounded and conservative: provider transcripts are not a stable API.
use crate::activate::{self, read_state, state_path};
use crate::effort::EffortFile;
use crate::notes::{self, Op};
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path};

const WINDOW: u64 = 64 * 1024;

/// Best effort only. Never wait for another writer or network, and never add model context.
pub(crate) fn check(input: &Value, session: &str) -> Option<()> {
    let (_, notes) = crate::record::notes_here().ok()?;
    let _held = notes.try_lock()?;
    let mut state = read_state(&notes.state, session).ok()??;
    if state.worker.is_true()
        || activate::block_reason(&notes.state, &state, session).is_some()
        || state.provider.as_deref()? != activate::provider(input)
    {
        return None;
    }
    let after = *state.delivered_ms.as_ref()?;
    let path = state.file.as_deref()?;
    if !path.starts_with("efforts/")
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let original = std::fs::read_to_string(notes.root.join(path)).ok()?;
    let base = notes::content_hash(&original);
    if state.delivered.as_deref()? != base || notes.has_stale(path) {
        return None;
    }
    let mut effort = EffortFile::parse(&original).ok()?;
    if effort.number()? != state.effort.as_deref()?.parse::<u32>().ok()?
        || effort.prop("session")? != session
        || !can_clear_wait(&effort)
    {
        return None;
    }
    let now = notes::now_ms();
    if !has_reply(
        Path::new(input["transcript_path"].as_str()?),
        state.provider.as_deref()?,
        session,
        after,
        now,
    )
    .unwrap_or(false)
    {
        return None;
    }
    // Ordinary saves recover pending writes. A hook must not race one or pay its recovery cost.
    if notes.intents().iter().any(|op| op.result.is_none()) || !notes.broken_intents().is_empty() {
        return None;
    }
    effort.set("status", "active");
    effort.set("waits_for", "");
    effort.set("updated", &notes::local_stamp());
    let text = effort.render();
    let path = path.to_owned();
    state.delivered.set(notes::content_hash(&text));
    state.delivered_ms.set(now);
    // This is not a progress save: preserve prose, journal and save bookkeeping.
    let op = Op {
        reply_after_ms: Some(after),
        key: notes::hash(&format!("reply:{session}:{base}:{after}")),
        files: vec![(path, text)],
        message: format!("{:04}: owner replied", effort.number()?),
        base: Some(base),
        states: vec![(state_path(&notes.state, session), state.to_value())],
        lesson_close: None,
        renames: Vec::new(),
        journal: Vec::new(),
        committed: false,
        digest: None,
        result: None,
        cwd: None,
        session: None,
        rebaseline: None,
    };
    notes.apply(&op).ok()?;
    crate::application::finish(&notes, &op.key, "Owner wait cleared");
    notes.forget(&op.key);
    notes.spawn_sync();
    Some(())
}

/// Review is a deliberate handoff, so conversation replies never advance it.
fn can_clear_wait(effort: &EffortFile) -> bool {
    matches!(effort.prop("status"), Some("active" | "blocked"))
        && effort
            .prop("waits_for")
            .is_some_and(|w| w.starts_with("owner:"))
}

fn has_reply(path: &Path, provider: &str, session: &str, after: i64, now: i64) -> Option<bool> {
    if after <= 0 || after >= now {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let len = metadata.len();
    // Codex records session identity in the header, not on each message. Bound that read too.
    if provider == "codex" {
        let mut head = Vec::new();
        (&mut file).take(WINDOW).read_to_end(&mut head).ok()?;
        let end = head.iter().position(|b| *b == b'\n')?;
        let record: Value = serde_json::from_slice(&head[..end]).ok()?;
        if record["type"] != "session_meta" || record["payload"]["id"] != session {
            return None;
        }
    }
    let offset = len.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut tail = Vec::new();
    file.take(WINDOW).read_to_end(&mut tail).ok()?;
    let start = if offset == 0 {
        0
    } else {
        tail.iter().position(|b| *b == b'\n')? + 1
    };
    let end = tail.iter().rposition(|b| *b == b'\n')?;
    if start > end {
        return Some(false);
    }
    Some(tail[start..end].split(|b| *b == b'\n').any(|line| {
        serde_json::from_slice::<Value>(line)
            .ok()
            .and_then(|record| human_time(&record, provider, session))
            .is_some_and(|at| at > after && at <= now)
    }))
}

fn human_time(record: &Value, provider: &str, session: &str) -> Option<i64> {
    match provider {
        "claude" => {
            if record["sessionId"] != session || record["isSidechain"] == true {
                return None;
            }
            let message = match record["type"].as_str()? {
                "user" => record,
                "attachment" if record["attachment"]["type"] == "queued_command" => {
                    &record["attachment"]
                }
                _ => return None,
            };
            if message["origin"]["kind"] != "human" {
                return None;
            }
            chrono::DateTime::parse_from_rfc3339(message["timestamp"].as_str()?)
                .ok()
                .map(|t| t.timestamp_millis())
        }
        "codex" => {
            let message = &record["payload"];
            if record["type"] != "response_item"
                || message["type"] != "message"
                || message["role"] != "user"
            {
                return None;
            }
            let meta = &message["internal_chat_message_metadata_passthrough"];
            if !meta["content_item_metadata"]
                .as_array()?
                .iter()
                .any(|item| item["provenance"]["type"] == "user")
            {
                return None;
            }
            let seconds = meta["create_time"].as_f64()?;
            (seconds.is_finite() && seconds > 0.0 && seconds < i64::MAX as f64 / 1000.0)
                .then_some((seconds * 1000.0) as i64)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
