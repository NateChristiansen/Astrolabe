//! A real session with the composed layer, through the real driver. A manual Mac tool.
//! One turn, then the same turn again on a resume in a new process. Usage:
//! `layer_run <claude|codex> <agent|product|overlap> <repo> <data-dir> <adapters-dir> <client-path> [hooks-list.json] [repo-instructions.txt]`
//! `layer_run hooks-list <repo> <codex-path> <output.json>` discovers session hook trust.
//! `overlap` composes for a product repo in a repo that also holds the layer. Codex
//! agent/overlap require effective, trusted repo config; the caller proves that first.
//! The optional text file holds effective repo developer_instructions, never AGENTS.md.
//! `LAYER_RUN_DEFAULT_SOURCES_WIRE=<path>` runs Claude on the production `settingSources`.
//! Hook and server starts land in
//! `<data-dir>/reports`, and this prints them; the model's words are printed as they come.
use astrolabe::acp::{AcpClient, ClientEvent, Launch};
use astrolabe::driver::{claude, codex};
use astrolabe::hook::{read_reports, reports_dir_in};
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
        instructions: Some("See AGENTS.md".to_owned()),
        hooks: layer.hooks.clone(),
        mcp: layer.mcp.clone(),
        denies: layer.denies.clone(),
        memory_off: true,
    };
    let instructions = extra
        .get(1)
        .map(|file| std::fs::read_to_string(file).unwrap());
    assert!(
        provider != "codex" || scenario != "overlap" || instructions.is_some(),
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
                let response =
                    serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
                codex::SessionFlagKey::from_hooks_list(&response).unwrap()
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

    let engine = Engine::open(&data.join("astrolabe.db"), SystemClock, OsProcessProbe).unwrap();
    let mut session = AcpClient::launch(&engine, &tapped(&spec, &wire_log, "1"), &repo).unwrap();
    wait_ready(&mut session);
    let id = session.session_id();
    println!("agent session {}", session.agent_session_id());
    turn(&mut session, &prompt(provider, "marker-1"));
    session.shutdown().unwrap();
    show_reports(&data, "after first process");

    let mut session = resume(&engine, id, &tapped(&spec, &wire_log, "2"), &repo);
    wait_ready(&mut session);
    turn(&mut session, &prompt(provider, "marker-2"));
    session.shutdown().unwrap();
    show_reports(&data, "after resume");
    let new = engine.ingest_reports(&reports_dir_in(&data)).unwrap();
    println!("core took in {} reports", new.len());
    for name in ["marker-1.txt", "marker-2.txt"] {
        println!("{name} exists: {}", repo.join(name).exists());
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
const [repo, client] = process.argv.slice(1);
const hooks = 'hooks={SessionStart=[{hooks=[{type="command",command="astrolabe hook session-start"}]}]}';
const child = spawn(client, ['app-server', '-c', hooks], {cwd:repo, stdio:['pipe','pipe','inherit']});
const timer = setTimeout(() => { child.kill(); process.exitCode=2; }, 15000);
let buffer='';
child.on('error', error => { console.error(error); clearTimeout(timer); process.exitCode=2; });
child.stdout.on('data', chunk => {
  buffer += chunk; let end;
  while ((end=buffer.indexOf('\n')) >= 0) {
    const line=buffer.slice(0,end); buffer=buffer.slice(end+1);
    let message; try { message=JSON.parse(line); } catch { continue; }
    if (message.id===1) {
      if (message.error) { console.error(message); child.kill(); process.exitCode=2; clearTimeout(timer); return; }
      child.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'initialized'})+'\n');
      child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:2,method:'hooks/list',params:{cwds:[repo]}})+'\n');
    }
    if (message.id===2) { console.log(JSON.stringify(message)); clearTimeout(timer); child.stdin.end(); child.kill(); }
  }
});
child.stdin.write(JSON.stringify({jsonrpc:'2.0',id:1,method:'initialize',params:{clientInfo:{name:'astrolabe-layer-probe',version:'0'}}})+'\n');
"#;
    let result = std::process::Command::new("node")
        .args(["-e", SCRIPT, repo, client])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "hooks/list failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let response: Value = serde_json::from_slice(&result.stdout).unwrap();
    let keys = codex::SessionFlagKey::from_hooks_list(&response).unwrap();
    assert!(!keys.is_empty(), "no session-flags hooks returned");
    std::fs::write(output, result.stdout).unwrap();
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

fn show_reports(data: &Path, when: &str) {
    println!("\n--- reports {when}");
    for report in read_reports(&reports_dir_in(data)) {
        println!(
            "{} session={:?} cwd={}",
            report.event, report.session_id, report.cwd
        );
    }
}
