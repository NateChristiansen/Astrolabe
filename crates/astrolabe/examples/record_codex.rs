//! Manual Mac recorder. Run only outside the builder sandbox, in a disposable
//! clone under /private/tmp. Output is PRIVATE, unsanitized JSONL; do not commit it.
//! Each wire record contains direction, process id, elapsed_ms, and the original
//! line (including its newline). Resume appends a second process to the same log.
//! Usage: cargo run --offline --locked -p astrolabe --example record_codex --
//!        <full-turn|approval-allow|approval-reject|two-turns|resume|cancel-tool|idle-launch>
//!        <cwd> <output.jsonl> < /dev/null
use astrolabe::acp::{AcpClient, ClientEvent};
use astrolabe::driver::codex;
use astrolabe::model::Alert;
use astrolabe::{Engine, OsProcessProbe, SystemClock};
use serde_json::{Value, json};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SCENARIOS: &[&str] = &[
    "full-turn",
    "approval-allow",
    "approval-reject",
    "two-turns",
    "resume",
    "cancel-tool",
    "idle-launch",
];

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--proxy") {
        if args.len() != 4 {
            return Err("invalid internal proxy arguments".into());
        }
        return proxy(Path::new(&args[1]), &args[2], &args[3]);
    }
    if args.len() != 3 {
        return Err(format!(
            "usage: record_codex <{}> <cwd> <output.jsonl>",
            SCENARIOS.join("|")
        )
        .into());
    }
    let scenario = args[0].to_str().ok_or("invalid scenario")?;
    if !SCENARIOS.contains(&scenario) {
        return Err("unknown scenario".into());
    }
    let cwd = fs::canonicalize(&args[1])?;
    if !cwd.starts_with("/private/tmp") || !cwd.join(".git").is_dir() {
        return Err("cwd must be a disposable clone under /private/tmp (not a worktree)".into());
    }
    let output = std::path::absolute(&args[2])?;
    let mut options = OpenOptions::new();
    options.append(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut log = options.open(&output)?;
    let before = snapshots()?;
    write_record(
        &mut log,
        json!({"kind": "before", "config": before, "scenario": scenario,
        "unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis(),
        "node": version("node")?, "codex": version("codex")?,
        "adapter": "@agentclientprotocol/codex-acp 2.5.0"}),
    )?;
    let result = record(scenario, &cwd, &output, &mut log);
    // Check even after a failed scenario. Never restore or edit user config.
    let after = snapshots()?;
    let unchanged = before == after;
    write_record(
        &mut log,
        json!({"kind": "after", "config": after, "unchanged": unchanged,
        "result": result.as_ref().map(|_| "ok").map_err(ToString::to_string)}),
    )?;
    if !unchanged {
        return Err("user configuration changed; stop, inspect the private recording".into());
    }
    result
}

fn version(command: &str) -> Result<String> {
    let output = Command::new(command)
        .arg("--version")
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(format!("{command} --version failed").into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn sha256(bytes: &[u8]) -> Result<String> {
    let mut child = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("missing hash stdin")?
        .write_all(bytes)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err("shasum failed".into());
    }
    Ok(String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .ok_or("missing hash")?
        .to_owned())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn snapshots() -> Result<Value> {
    let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is missing")?);
    // Also check the effective Codex home if the caller explicitly selected one.
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or(home.join(".codex"));
    let mut result = json!({});
    for (name, path) in [
        ("codex", home.join(".codex/config.toml")),
        ("effective_codex", codex_home.join("config.toml")),
        ("claude_settings", home.join(".claude/settings.json")),
    ] {
        result[name] = match read_optional(&path)? {
            Some(bytes) => json!(sha256(&bytes)?),
            None => Value::Null,
        };
    }
    let trust = match read_optional(&home.join(".claude.json"))? {
        None => Value::Null,
        Some(bytes) => {
            let config: Value = serde_json::from_slice(&bytes)?;
            let mut trust = serde_json::Map::new();
            if let Some(projects) = config["projects"].as_object() {
                for (path, project) in projects {
                    if let Some(value) = project.get("hasTrustDialogAccepted") {
                        trust.insert(path.clone(), value.clone());
                    }
                }
            }
            json!(trust)
        }
    };
    result["claude_trust"] = json!(sha256(trust.to_string().as_bytes())?);
    Ok(result)
}

fn record(scenario: &str, cwd: &Path, output: &Path, log: &mut File) -> Result<()> {
    let fixture = cwd.join(format!("{scenario}-fixture.txt"));
    if scenario.starts_with("approval-") && fixture.exists() {
        return Err("approval fixture already exists; use a fresh clone".into());
    }
    let adapters = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters");
    let mut launch = codex::launch(&adapters, &json!({}))?;
    // The pinned adapter supports this env preset. Force host-reviewed approvals
    // for the approval probes, rather than the adapter's auto-review default.
    launch
        .env
        .push(("INITIAL_AGENT_MODE".into(), "read-only".into()));
    let entry = launch.args.first().ok_or("missing adapter entry")?.clone();
    launch.args = vec!["--proxy".into(), output.into(), launch.command, entry];
    launch.command = std::env::current_exe()?.into();
    let engine = Engine::open(Path::new(":memory:"), SystemClock, OsProcessProbe)?;
    let client = AcpClient::launch(&engine, &launch, cwd)?;
    let session = client.session_id();
    phase(client, scenario, false, &engine, log)?;
    if scenario == "resume" {
        let client = AcpClient::resume(&engine, session, &launch, cwd)?;
        phase(client, scenario, true, &engine, log)?;
    }
    if scenario == "approval-allow" && fs::read_to_string(&fixture)? != "fixture" {
        return Err("approved command did not produce the expected fixture".into());
    }
    if scenario == "approval-reject" && fixture.exists() {
        return Err("rejected command created the fixture".into());
    }
    Ok(())
}

fn phase(
    mut client: AcpClient<'_>,
    scenario: &str,
    resumed: bool,
    engine: &Engine,
    log: &mut File,
) -> Result<()> {
    let result = run_phase(&mut client, scenario, resumed, engine, log);
    // Explicit cleanup on success AND failure; AcpClient has no Drop cleanup.
    let shutdown = client.shutdown();
    result?;
    shutdown?;
    Ok(())
}

fn event(client: &mut AcpClient<'_>, deadline: Instant, log: &mut File) -> Result<ClientEvent> {
    while Instant::now() < deadline {
        if let Some(event) = client.next_event(Duration::from_millis(100))? {
            write_record(log, json!({"kind": "event", "event": format!("{event:?}")}))?;
            match event {
                ClientEvent::LaunchFailed { ref message }
                | ClientEvent::PromptError { ref message }
                | ClientEvent::WriteFailed { ref message }
                | ClientEvent::AgentError { ref message, .. } => {
                    return Err(message.clone().into());
                }
                ClientEvent::Exited { .. } => return Err("adapter exited unexpectedly".into()),
                _ => return Ok(event),
            }
        }
    }
    Err("scenario deadline exceeded".into())
}

fn run_phase(
    client: &mut AcpClient<'_>,
    scenario: &str,
    resumed: bool,
    engine: &Engine,
    log: &mut File,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !matches!(event(client, deadline, log)?, ClientEvent::Ready) {}
    if scenario == "idle-launch" {
        let until = Instant::now() + Duration::from_secs(605);
        let mut cards = 0;
        while Instant::now() < until {
            if let Some(event) = client.next_event(Duration::from_millis(100))? {
                write_record(
                    log,
                    json!({"kind": "idle_event", "event": format!("{event:?}")}),
                )?;
                if matches!(
                    event,
                    ClientEvent::Exited { .. } | ClientEvent::LaunchFailed { .. }
                ) {
                    return Err("adapter exited during idle observation".into());
                }
            }
            for alert in engine.tick()? {
                cards += usize::from(matches!(alert, Alert::MayBeStuck(_)));
                write_record(
                    log,
                    json!({"kind": "idle_alert", "alert": format!("{alert:?}")}),
                )?;
            }
        }
        write_record(log, json!({"kind": "idle_result", "stuck_cards": cards}))?;
        return Ok(());
    }
    let approval_prompt = format!(
        "Run exactly this shell command to create a disposable fixture: python3 -c \"from pathlib import Path; Path('{scenario}-fixture.txt').write_text('fixture')\". Request approval if needed. After it succeeds reply only FIXTURE_DONE. Do not use other tools or commands."
    );
    let prompt = match scenario {
        "approval-allow" | "approval-reject" => &approval_prompt,
        "cancel-tool" => {
            "Run python3 -c \"import time; time.sleep(30)\" in the foreground, then reply only DONE. Do not background it or use other tools."
        }
        "resume" if resumed => {
            "What was the fixture codeword from the previous turn? Reply with just the codeword. Do not use tools."
        }
        "resume" => "The fixture codeword is ORBIT. Reply only ACK. Do not use tools.",
        _ => "Reply only FIRST. Do not use tools.",
    };
    client.prompt(prompt)?;
    let mut turns = 0;
    let mut permissions = 0;
    let mut cancelled = false;
    let mut text = String::new();
    loop {
        let next = event(client, deadline, log)?;
        match next {
            ClientEvent::Alert(Alert::NeedsYou(request)) => {
                if turns > 0
                    || !matches!(
                        scenario,
                        "approval-allow" | "approval-reject" | "cancel-tool"
                    )
                {
                    return Err("unexpected approval request".into());
                }
                permissions += 1;
                let kind = if scenario == "approval-reject" {
                    "reject_once"
                } else {
                    "allow_once"
                };
                let option = request
                    .options
                    .iter()
                    .find(|option| option.kind == kind)
                    .ok_or("expected permission option absent")?;
                if !client.answer(&request.id, &option.option_id)? {
                    return Err("permission was already resolved".into());
                }
            }
            ClientEvent::Update(message) => {
                let update = &message["params"]["update"];
                if update["sessionUpdate"] == "agent_message_chunk"
                    && let Some(chunk) = update["content"]["text"].as_str()
                {
                    text.push_str(chunk);
                }
                if scenario == "cancel-tool"
                    && !cancelled
                    && update["status"] == "in_progress"
                    && update["toolCallId"].is_string()
                {
                    client.cancel()?;
                    cancelled = true;
                }
            }
            ClientEvent::TurnEnded { stop_reason } => {
                turns += 1;
                if scenario.starts_with("approval-") && permissions == 0 {
                    return Err("turn ended without the required approval request".into());
                }
                if scenario == "cancel-tool" && (!cancelled || stop_reason != "cancelled") {
                    return Err("missing tool cancellation evidence".into());
                }
                let expected = match (scenario, turns, resumed) {
                    ("full-turn" | "two-turns", 1, _) => Some("FIRST"),
                    ("two-turns", 2, _) => Some("SECOND"),
                    ("resume", _, false) => Some("ACK"),
                    ("resume", _, true) => Some("ORBIT"),
                    ("approval-allow", _, _) => Some("FIXTURE_DONE"),
                    ("approval-reject", 2, _) => Some("REPLACEMENT"),
                    _ => None,
                };
                if expected.is_some_and(|expected| text.trim() != expected) {
                    return Err(
                        format!("unexpected response: {text:?}; expected {expected:?}").into(),
                    );
                }
                text.clear();
                if turns == 1 && matches!(scenario, "two-turns" | "approval-reject") {
                    let text = if scenario == "approval-reject" {
                        "Instead, reply only REPLACEMENT. Do not run commands or use tools."
                    } else {
                        "Reply only SECOND. Do not use tools."
                    };
                    client.prompt(text)?;
                } else {
                    if scenario != "cancel-tool" && stop_reason != "end_turn" {
                        return Err(format!("unexpected final stop reason: {stop_reason}").into());
                    }
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

// A transparent byte-line proxy keeps wire capture out of the shared client.
// Its adapter and tools inherit the shared client's process group on Unix.
fn proxy(output: &Path, command: &std::ffi::OsStr, entry: &std::ffi::OsStr) -> Result<()> {
    let log = Arc::new(Mutex::new(OpenOptions::new().append(true).open(output)?));
    let start = Instant::now();
    let mut child = Command::new(command)
        .arg(entry)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take().ok_or("missing adapter stdin")?;
    let stdout = child.stdout.take().ok_or("missing adapter stdout")?;
    let stderr = child.stderr.take().ok_or("missing adapter stderr")?;
    let sent = log.clone();
    thread::spawn(move || {
        if let Err(error) = forward(io::stdin(), stdin, "to_agent", sent, start) {
            eprintln!("recorder stdin: {error}");
        }
    });
    let errors = log.clone();
    let stderr_thread =
        thread::spawn(move || forward(stderr, io::stderr(), "stderr", errors, start));
    forward(stdout, io::stdout(), "from_agent", log.clone(), start)?;
    let status = child.wait()?;
    stderr_thread
        .join()
        .map_err(|_| "stderr thread panicked")??;
    write_record(
        &mut *log.lock().map_err(|_| "log poisoned")?,
        json!({"kind": "exit", "code": status.code(), "process": std::process::id()}),
    )?;
    std::process::exit(status.code().unwrap_or(1));
}

fn forward(
    source: impl Read,
    mut target: impl Write,
    direction: &str,
    log: Arc<Mutex<File>>,
    start: Instant,
) -> io::Result<()> {
    let mut reader = BufReader::new(source);
    loop {
        let mut bytes = Vec::new();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            return Ok(());
        }
        let mut log = log.lock().map_err(|_| io::Error::other("log poisoned"))?;
        // bytes preserves even non-UTF8 stderr; line is convenient for JSON replay.
        write_record(
            &mut log,
            json!({"kind": "wire", "direction": direction,
            "process": std::process::id(), "elapsed_ms": start.elapsed().as_millis(),
            "line": String::from_utf8_lossy(&bytes), "bytes": bytes}),
        )?;
        log.flush()?;
        drop(log);
        target.write_all(&bytes)?;
        target.flush()?;
    }
}

// O_APPEND plus a single serialized write keeps parent and proxy records whole.
fn write_record(log: &mut File, value: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    log.write_all(&bytes)
}
