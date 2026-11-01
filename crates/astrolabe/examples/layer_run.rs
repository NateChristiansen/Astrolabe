//! A real session with the composed layer, through the real driver. A manual Mac tool.
//! One turn, then the same turn again on a resume in a new process. Usage:
//! `layer_run <claude|codex> <agent|product|overlap> <repo> <data-dir> <adapters-dir> <client-path> [hooks-list.json] [repo-instructions.txt]`
//! `layer_run hooks-list <repo> <codex-path> <output.json>` discovers session hook trust.
//! `overlap` composes for a product repo in a repo that also holds the layer. Codex
//! agent/overlap require effective, trusted repo config; the caller proves that first.
//! The optional text file holds effective repo developer_instructions, never AGENTS.md; pass `none` to explicitly state no repo text.
//! `LAYER_RUN_DEFAULT_SOURCES_WIRE=<path>` runs Claude on the production `settingSources`.
//! Hook and server starts land in
//! `<data-dir>/reports`, and this prints them; the model's words are printed as they come.
use astrolabe::acp::{AcpClient, ClientEvent, Launch};
use astrolabe::driver::{claude, codex};
use astrolabe::hook::{comparable, read_reports, reports_dir_in};
use astrolabe::layer::*;
use astrolabe::model::Alert;
use astrolabe::{Engine, OsProcessProbe, SystemClock};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(180);
const LAYER_TEXT: &str = "The layer codeword is LAYER-OWL.";
const PROMPT: &str = "Do these in order. 1. List every codeword from your instructions and \
    project files, each with where it came from. 2. Call the layer_check tool of the astrolabe \
    MCP server once and repeat its raw JSON. 3. Run the Bash command `touch marker-N.txt` \
    exactly as written, once, no retries or workarounds, and say whether it ran or was denied.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [mode, repo, client, output] = args.as_slice()
        && mode == "hooks-list"
    {
        discover_hooks(repo, client, output);
        return;
    }
    let [provider, scenario, repo, data, adapters, client, extra @ ..] = args.as_slice() else {
        panic!("see the usage at the top of this file");
    };
    let (repo, data) = (PathBuf::from(repo), PathBuf::from(data));
    let layer = Layer::standard(LAYER_TEXT, &["Bash(touch:*)"]);
    let holds_layer = RepoSupplies {
        instructions: RepoInstructions::Text("See AGENTS.md".to_owned()),
        hooks: layer.hooks.clone(),
        mcp: layer.mcp.clone(),
        denies: layer.denies.clone(),
        memory_off: true,
    };
    let instructions = extra
        .get(1)
        .map(|file| {
            if file == "none" {
                RepoInstructions::None
            } else {
                RepoInstructions::Text(std::fs::read_to_string(file).unwrap())
            }
        })
        .unwrap_or_default();
    assert!(
        provider != "codex"
            || scenario != "overlap"
            || matches!(instructions, RepoInstructions::Text(_)),
        "Codex overlap needs a file containing the effective repo developer_instructions"
    );
    let target = match scenario.as_str() {
        "agent" => Target {
            kind: TargetKind::Agent,
            supplies: holds_layer,
        },
        "product" => Target {
            kind: TargetKind::Product,
            supplies: RepoSupplies {
                instructions,
                ..RepoSupplies::default()
            },
        },
        "overlap" => Target {
            kind: TargetKind::Product,
            supplies: if provider == "codex" {
                RepoSupplies {
                    instructions,
                    ..holds_layer
                }
            } else {
                RepoSupplies::default()
            },
        },
        other => panic!("no scenario {other}"),
    };
    // Set: the adapter's default settingSources, the production route, with the wire
    // logged to this path and SDK messages on it. Those load the user's own hooks.
    let wire_log = std::env::var_os("LAYER_RUN_DEFAULT_SOURCES_WIRE").map(PathBuf::from);
    let delivery = compose(&target, &layer).expect("a valid layer");
    println!("delivery: {delivery:?}");
    let mut spec = match provider.as_str() {
        "claude" => {
            let claude = claude::deliver(&delivery, &data.join("instructions"));
            claude.write_instructions().unwrap();
            // Probe only. Production settingSources stays the adapter default.
            let mut options = claude.options.unwrap_or_else(|| json!({}));
            if wire_log.is_none() {
                options["settingSources"] = json!(["project"]);
            }
            options["promptSuggestions"] = json!(false);
            println!("options: {options}");
            claude::launch(Path::new(adapters), Some(Path::new(client)), Some(options)).map(
                |mut spec| {
                    if wire_log.is_some() {
                        spec.meta.as_mut().unwrap()["claudeCode"]["emitRawSDKMessages"] =
                            json!(true);
                    }
                    spec
                },
            )
        }
        "codex" => {
            let keys = if delivery.hooks.is_empty() {
                vec![]
            } else {
                let file = extra
                    .first()
                    .expect("Codex hooks need hooks-list.json from this machine");
                let saved: Value =
                    serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
                assert_eq!(
                    saved["provenance"],
                    provenance(&repo, Path::new(client), &delivery.hooks),
                    "foreign or changed hooks/list discovery; rediscover on this machine"
                );
                let age = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .checked_sub(
                        saved["discovered_at"]
                            .as_u64()
                            .expect("discovery timestamp"),
                    )
                    .expect("future discovery timestamp");
                assert!(
                    age <= 900,
                    "stale discovery; rediscover within 15 minutes of launch"
                );
                codex::SessionFlagKey::from_hooks_list(&saved["response"]).unwrap()
            };
            let codex = codex::deliver(&delivery, &keys).unwrap();
            println!("CODEX_CONFIG: {}", codex.config);
            codex.launch(Path::new(adapters), Some(Path::new(client)))
        }
        other => panic!("no provider {other}"),
    }
    .expect("the adapter");
    let program = std::env::current_exe().unwrap();
    let program = program.parent().unwrap().parent().unwrap();
    spec.env
        .extend(launch_env(program, &data, std::env::var_os("PATH")).expect("a joinable PATH"));
    spec.env.push(("ANTHROPIC_MODEL".into(), "haiku".into()));

    assert!(
        read_reports(&reports_dir_in(&data)).is_empty(),
        "use a fresh data directory"
    );
    for name in ["marker-1.txt", "marker-2.txt"] {
        assert!(
            !repo.join(name).exists(),
            "remove old marker {name} before the probe"
        );
    }
    let engine = Engine::open(&data.join("astrolabe.db"), SystemClock, OsProcessProbe).unwrap();
    let mut session = AcpClient::launch(&engine, &tapped(&spec, &wire_log, "1"), &repo).unwrap();
    wait_ready(&mut session);
    let id = session.session_id();
    println!("agent session {}", session.agent_session_id());
    turn(&mut session, &prompt(provider, "marker-1"));
    session.shutdown().unwrap();
    show_reports(&data, "after first process", &repo, None);
    let first_reports = read_reports(&reports_dir_in(&data));

    let mut session = resume(&engine, id, &tapped(&spec, &wire_log, "2"), &repo);
    wait_ready(&mut session);
    turn(&mut session, &prompt(provider, "marker-2"));
    session.shutdown().unwrap();
    show_reports(&data, "after resume", &repo, Some(&first_reports));
    let new = engine.ingest_reports(&reports_dir_in(&data)).unwrap();
    println!("core took in {} reports", new.len());
    for name in ["marker-1.txt", "marker-2.txt"] {
        // Claude enforces the layer deny; Codex has no equivalent deny delivery yet.
        assert_eq!(
            repo.join(name).exists(),
            provider == "codex",
            "unexpected {name} result"
        );
    }
}

fn tapped(spec: &Launch, wire_log: &Option<PathBuf>, process: &str) -> Launch {
    let mut spec = spec.clone();
    if let Some(log) = wire_log {
        let tap = std::env::current_exe()
            .unwrap()
            .with_file_name("record_claude");
        let mut args = vec![
            "--tee".into(),
            log.with_extension(process).into(),
            spec.command,
        ];
        args.append(&mut spec.args);
        spec.command = tap.into();
        spec.args = args;
    }
    spec
}

fn prompt(provider: &str, marker: &str) -> String {
    let prompt = PROMPT.replace("marker-N", marker);
    if provider == "codex" {
        prompt.replace("Bash command", "shell command")
    } else {
        prompt
    }
}

/// Read-only app-server discovery; no thread, model request, config write or trust line.
fn discover_hooks(repo: &str, client: &str, output: &str) {
    const SCRIPT: &str = r#"
const {spawn} = require('node:child_process');
const [repo, client, declarations] = process.argv.slice(1);
const hooks = 'hooks=' + declarations;
const child = spawn(client, ['app-server', '-c', hooks], {cwd:repo, stdio:['pipe','pipe','pipe']});
const timer = setTimeout(() => { child.kill('SIGKILL'); process.exit(2); }, 15000);
child.stderr.setEncoding('utf8');
child.stderr.on('data', chunk => process.stderr.write(chunk));
child.stdout.setEncoding('utf8');
child.on('close', () => clearTimeout(timer));
let buffer='';
child.on('error', error => { console.error(error); clearTimeout(timer); process.exitCode=2; });
child.stdout.on('data', chunk => {
  buffer += chunk; let end;
  while ((end=buffer.indexOf('\n')) >= 0) {
    const line=buffer.slice(0,end); buffer=buffer.slice(end+1);
    let message; try { message=JSON.parse(line); } catch { continue; }
    if (message.id===1) {
      if (message.error) { console.error(message); child.kill('SIGKILL'); process.exitCode=2; return; }
      child.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'initialized'})+'\n');
      child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:2,method:'hooks/list',params:{cwds:[repo]}})+'\n');
    }
    if (message.id===2) { console.log(JSON.stringify(message)); child.stdin.end(); child.kill(); }
  }
});
child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:1,method:'initialize',params:{clientInfo:{name:'astrolabe-layer-probe',version:'0'}}})+'\n');
"#;
    let hooks = Layer::standard(LAYER_TEXT, &[]).hooks;
    let declarations = toml_inline(&Value::Object(codex::hook_declarations(&hooks)));
    let mut child = std::process::Command::new("node")
        .args(["-e", SCRIPT, repo, client, &declarations])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Drain both pipes while waiting, so a noisy child cannot fill a pipe and hang.
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (out_tx, out) = std::sync::mpsc::channel();
    let (err_tx, err) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = out_tx.send(read_pipe(stdout));
    });
    std::thread::spawn(move || {
        let _ = err_tx.send(read_pipe(stderr));
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hooks/list timed out");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let result = std::process::Output {
        status: child.wait().unwrap(),
        stdout: out
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("stdout stayed open after discovery exited"),
        stderr: err
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("stderr stayed open after discovery exited"),
    };
    assert!(
        result.status.success(),
        "hooks/list failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let response: Value = serde_json::from_slice(&result.stdout).unwrap();
    let keys = codex::SessionFlagKey::from_hooks_list(&response).unwrap();
    assert!(!keys.is_empty(), "no session-flags hooks returned");
    let saved = json!({"provenance": provenance(Path::new(repo), Path::new(client), &hooks),
        "discovered_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        "response": response});
    std::fs::write(output, saved.to_string()).unwrap();
    println!("session hooks: {keys:?}");
}

fn resume<'e>(
    engine: &'e Engine,
    id: astrolabe::model::SessionId,
    spec: &Launch,
    repo: &Path,
) -> AcpClient<'e> {
    println!("--- resume in a new process");
    AcpClient::resume(engine, id, spec, repo).unwrap()
}

fn wait_ready(session: &mut AcpClient) {
    pump(session, |event| matches!(event, ClientEvent::Ready));
}

fn turn(session: &mut AcpClient, prompt: &str) {
    session.prompt(prompt).unwrap();
    pump(session, |event| {
        matches!(event, ClientEvent::TurnEnded { .. })
    });
}

/// Allows whatever is asked, so only a deny can be what stops a command.
fn pump(session: &mut AcpClient, done: impl Fn(&ClientEvent) -> bool) {
    let deadline = Instant::now() + LIMIT;
    while Instant::now() < deadline {
        let Some(event) = session.next_event(Duration::from_millis(200)).unwrap() else {
            continue;
        };
        match &event {
            ClientEvent::Update(update) => show(update),
            ClientEvent::Alert(Alert::NeedsYou(request)) => {
                println!("[asked: {}; allowing]", request.title);
                let option = (request.options.iter())
                    .find(|option| option.kind == "allow_once")
                    .expect("an allow option");
                session.answer(&request.id, &option.option_id).unwrap();
            }
            other => println!("[{other:?}]"),
        }
        if done(&event) {
            return;
        }
        if let ClientEvent::Exited { .. } | ClientEvent::LaunchFailed { .. } = event {
            panic!("the session ended early");
        }
    }
    panic!("step limit reached");
}

fn show(update: &Value) {
    let update = &update["params"]["update"];
    match update["sessionUpdate"].as_str() {
        Some("agent_message_chunk") => {
            print!("{}", update["content"]["text"].as_str().unwrap_or(""))
        }
        Some("tool_call" | "tool_call_update") if update["status"] != Value::Null => println!(
            "\n[tool {} {} {}]",
            update["toolCallId"].as_str().unwrap_or(""),
            update["title"].as_str().unwrap_or(""),
            update["status"].as_str().unwrap_or("")
        ),
        _ => {}
    }
}

fn show_reports(
    data: &Path,
    when: &str,
    repo: &Path,
    previous: Option<&[astrolabe::hook::Report]>,
) {
    let reports = read_reports(&reports_dir_in(data));
    assert!(
        reports.iter().any(|report| report.event == "session-start"
            && report.cwd == comparable(repo)
            && previous.is_none_or(|old| !old.iter().any(|r| r.file == report.file))),
        "no fresh SessionStart report {when}"
    );
    println!("\n--- reports {when}");
    for report in reports {
        println!(
            "{} session={:?} cwd={}",
            report.event, report.session_id, report.cwd
        );
    }
}

fn read_pipe(mut pipe: impl std::io::Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).unwrap();
    bytes
}

fn provenance(repo: &Path, client: &Path, hooks: &[Hook]) -> Value {
    let client = std::fs::canonicalize(client).expect("absolute installed client path");
    let metadata = std::fs::metadata(&client).unwrap();
    let machine = std::process::Command::new("hostname")
        .output()
        .expect("machine hostname");
    assert!(
        machine.status.success(),
        "cannot identify discovery machine"
    );
    json!({"machine": String::from_utf8(machine.stdout).unwrap().trim(),
        "repo": comparable(repo), "client": client, "client_size": metadata.len(),
        "client_modified": metadata.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_string(),
        "hooks": codex::hook_declarations(hooks)})
}

fn toml_inline(value: &Value) -> String {
    match value {
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(key, value)| format!(
                    "{}={}",
                    serde_json::to_string(key).unwrap(),
                    toml_inline(value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Array(array) => format!(
            "[{}]",
            array.iter().map(toml_inline).collect::<Vec<_>>().join(",")
        ),
        Value::String(_) | Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Null => panic!("null is not TOML"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_serializes_the_delivered_hook_table() {
        let hooks = Layer::standard(LAYER_TEXT, &[]).hooks;
        assert_eq!(
            toml_inline(&Value::Object(codex::hook_declarations(&hooks))),
            r#"{"SessionStart"=[{"hooks"=[{"command"="astrolabe hook session-start","type"="command"}]}]}"#
        );
    }

    #[test]
    #[should_panic(expected = "no fresh SessionStart report")]
    fn absent_reports_fail_the_probe() {
        let missing =
            std::env::temp_dir().join(format!("astrolabe-no-reports-{}", std::process::id()));
        show_reports(&missing, "test", &missing, None);
    }
}
