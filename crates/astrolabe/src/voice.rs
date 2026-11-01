//! Owner and voice reach every session by one route: the session-start hook reads them from
//! this machine's agent repo. No copy, so no drift. `delivered.jsonl` is where doctor looks.
use crate::registry::Registry;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

const LOG: &str = "delivered.jsonl";
/// Both clients took 9,850 bytes whole and silently cut 10,875, so it's refused here.
const LIMIT: usize = 9800;
const HEADER: &str = "Owner and voice files from the agent repo. Samples in them teach style only and are never a source of facts.";
pub const NO_AGENT: &str =
    "No agent repo on this machine, so no owner or voice file was delivered.";

pub fn read(agent: &Path) -> Result<String, String> {
    let mut text = HEADER.to_owned();
    for file in ["knowledge/owner.md", "knowledge/voice.md"] {
        let body = std::fs::read_to_string(agent.join(file));
        let body = body.map_err(|e| format!("{file} in {}: {e}", agent.display()))?;
        text += &format!("\n\n--- {file}\n\n{}", body.trim());
    }
    match text.len() {
        size if size > LIMIT => Err(format!("owner and voice: {size} bytes, limit {LIMIT}")),
        _ => Ok(text),
    }
}

pub fn context(data_dir: &Path) -> String {
    let read = match Registry::open(&data_dir.join("astrolabe.db")).map(|r| r.agent_repo()) {
        Ok(Ok(None)) => return NO_AGENT.to_owned(),
        Ok(Ok(Some(agent))) => read(&agent),
        Ok(Err(why)) | Err(why) => Err(why.to_string()),
    };
    read.unwrap_or_else(|why| format!("Owner and voice NOT delivered: {why}. Tell the owner."))
}

pub fn hook_output(text: &str) -> String {
    let inner = json!({"hookEventName": "SessionStart", "additionalContext": text});
    json!({ "hookSpecificOutput": inner }).to_string()
}

fn json_lines(text: &str) -> impl DoubleEndedIterator<Item = Value> + '_ {
    text.lines().filter_map(|l| serde_json::from_str(l).ok())
}

pub fn record(data_dir: &Path, at_ms: i64, cwd: &Path, input: &Value, text: &str) {
    let cwd = crate::hook::comparable(cwd);
    let (source, transcript) = (&input["source"], &input["transcript_path"]);
    let line = json!({"at_ms": at_ms, "cwd": cwd, "source": source, "transcript": transcript, "bytes": text.len()});
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join(LOG));
    let _ = log.and_then(|mut log| writeln!(log, "{line}"));
}

/// The latest delivery here that names a client's record. A child's may not.
pub fn last_delivery(data_dir: &Path, cwd: &Path, since_ms: i64) -> Option<PathBuf> {
    let log = std::fs::read_to_string(data_dir.join(LOG)).ok()?;
    let cwd = crate::hook::comparable(cwd);
    json_lines(&log)
        .rev()
        .filter(|l| l["cwd"] == cwd.as_str() && l["at_ms"].as_i64() >= Some(since_ms))
        .find_map(|l| Some(PathBuf::from(l["transcript"].as_str()?)))
}

/// In what the client sent, not its copy of the hook's stdout, which stays whole past a cut.
pub fn in_transcript(transcript: &str, text: &str) -> bool {
    let wanted = Value::from(text).to_string();
    let sent = |v: &Value| v["type"] == "api-request-blob" || v["type"] == "response_item";
    json_lines(transcript)
        .filter(sent)
        .any(|v| v.to_string().contains(wanted.trim_matches('"')))
}

/// Codex: missing means enabled. Claude's record names its memory folder only when it's on.
pub fn memory_on(codex: bool, record: &Path, transcript: &str) -> bool {
    if codex {
        return !transcript.contains(r#""memory_mode":"disabled""#);
    }
    let folder = record.with_file_name("memory").display().to_string();
    transcript.contains(&folder.replace('\\', "\\\\"))
}
