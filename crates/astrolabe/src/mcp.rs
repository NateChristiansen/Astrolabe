//! `astrolabe mcp`: a stdio tool server with one tool, `layer_check`. Phase 2 adds the
//! rest. Each start leaves a report, so a double load can be counted.
use crate::hook::{Report, comparable, read_reports, reports_dir, write_report};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub const START_EVENT: &str = "mcp-start";
const NOT_FOUND: i64 = -32601;
const PARSE_ERROR: i64 = -32700;
/// All these need is what this server does. The newest comes first, as the answer to
/// a version not listed, which the client may then refuse.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
/// Enough to see a session's own reports. The rest stay in the folder.
const SHOWN_REPORTS: usize = 50;

pub fn serve() -> io::Result<()> {
    let dir = reports_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(dir) = &dir
        && let Err(error) = write_report(dir, START_EVENT, None, &cwd)
    {
        // Not `eprintln!`, which panics if stderr is closed.
        let _ = writeln!(io::stderr(), "astrolabe mcp: {error}");
    }
    let mut out = io::stdout();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = handle(&line, dir.as_deref(), &cwd) {
            writeln!(out, "{answer}")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn handle(line: &str, dir: Option<&Path>, cwd: &Path) -> Option<Value> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(failure(&Value::Null, PARSE_ERROR, "not JSON"));
    };
    // No id: a notification, which gets no answer.
    let id = message.get("id")?;
    let params = &message["params"];
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": PROTOCOLS
                .into_iter()
                .find(|version| params["protocolVersion"] == *version)
                .unwrap_or(PROTOCOLS[0]),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "astrolabe", "version": env!("CARGO_PKG_VERSION") },
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [{
            "name": "layer_check",
            "description": "Report what of the Astrolabe layer reached this session: \
                this server, and the hooks that fired in this folder.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true },
        }]}),
        "tools/call" if params["name"] == "layer_check" => {
            let text = layer_check(dir, cwd).to_string();
            json!({ "content": [{ "type": "text", "text": text }] })
        }
        method => return Some(failure(id, NOT_FOUND, &format!("no method {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Reports what is known, not a verdict. Only a model can show its instructions arrived,
/// and only the launch knows what to expect.
fn layer_check(dir: Option<&Path>, cwd: &Path) -> Value {
    let cwd = comparable(cwd);
    let reports: Vec<Report> = dir
        .map(read_reports)
        .unwrap_or_default()
        .into_iter()
        .filter(|report| report.cwd == cwd)
        .collect();
    let mcp_starts = reports.iter().filter(|r| r.event == START_EVENT).count();
    let shown = reports.iter().rev().take(SHOWN_REPORTS).rev();
    json!({
        "server": "astrolabe",
        "version": env!("CARGO_PKG_VERSION"),
        "cwd": cwd,
        "mcp_starts": mcp_starts,
        "reports": shown.map(|report| json!({
            "event": report.event,
            "session_id": report.session_id,
            "at_ms": report.at_ms,
        })).collect::<Vec<_>>(),
    })
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
