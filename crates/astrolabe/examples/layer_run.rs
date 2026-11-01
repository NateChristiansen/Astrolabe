//! A real session with the composed layer, through the real driver. A manual Mac tool.
//! One turn, then the same turn again on a resume in a new process. Usage:
//! `layer_run <agent|product|overlap> <repo> <data-dir> <adapters-dir> <client-path>`
//! `overlap` composes for a product repo in a repo that also holds the layer. The Codex
//! delivery adds its own provider argument here. Hook and server starts land in
//! `<data-dir>/reports`, and this prints them; the model's words are printed as they come.
use astrolabe::acp::{AcpClient, ClientEvent, Launch};
use astrolabe::driver::claude;
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
    let [scenario, repo, data, adapters, client] = args.as_slice() else {
        panic!("see the usage at the top of this file");
    };
    let (repo, data) = (PathBuf::from(repo), PathBuf::from(data));
    let layer = Layer::standard(LAYER_TEXT, &["Bash(touch:*)"]);
    let holds_layer = RepoSupplies {
        instructions: Some("See AGENTS.md".to_owned()),
        hooks: layer.hooks.clone(),
        mcp_names: vec![MCP_NAME.to_owned()],
        denies: layer.denies.clone(),
        memory_off: true,
    };
    let target = match scenario.as_str() {
        "agent" => Target {
            kind: TargetKind::Agent,
            supplies: holds_layer,
        },
        "product" | "overlap" => Target {
            kind: TargetKind::Product,
            supplies: RepoSupplies::default(),
        },
        other => panic!("no scenario {other}"),
    };
    let delivery = compose(&target, &layer);
    println!("delivery: {delivery:?}");
    let claude = claude::deliver(&delivery, &data.join("instructions"));
    claude.write_instructions().unwrap();

    // `project` only: the user's own hooks and plugins stay out of a probe.
    let mut options = claude.options.clone().unwrap_or_else(|| json!({}));
    options["settingSources"] = json!(["project"]);
    options["promptSuggestions"] = json!(false);
    println!("options: {options}");
    let mut spec = claude::launch(Path::new(adapters), Some(Path::new(client)), Some(options))
        .expect("the adapter");
    let program = std::env::current_exe().unwrap();
    let program = program.parent().unwrap().parent().unwrap();
    spec.env
        .extend(launch_env(program, &data, std::env::var_os("PATH")).expect("a joinable PATH"));
    spec.env.push(("ANTHROPIC_MODEL".into(), "haiku".into()));

    let engine = Engine::open(&data.join("astrolabe.db"), SystemClock, OsProcessProbe).unwrap();
    let mut session = AcpClient::launch(&engine, &spec, &repo).unwrap();
    wait_ready(&mut session);
    let id = session.session_id();
    println!("agent session {}", session.agent_session_id());
    turn(&mut session, &PROMPT.replace("marker-N", "marker-1"));
    session.shutdown().unwrap();
    show_reports(&data, "after first process");

    let mut session = resume(&engine, id, &spec, &repo);
    wait_ready(&mut session);
    turn(&mut session, &PROMPT.replace("marker-N", "marker-2"));
    session.shutdown().unwrap();
    show_reports(&data, "after resume");
    let new = engine.ingest_reports(&reports_dir_in(&data)).unwrap();
    println!("core took in {} reports", new.len());
    for name in ["marker-1.txt", "marker-2.txt"] {
        println!("{name} exists: {}", repo.join(name).exists());
    }
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
