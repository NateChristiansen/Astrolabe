//! The real-login run of P8: fresh, resumed, compacted and child, through `launch::prepare`.
//! Skipped unless `ASTROLABE_P8` names the registered product repo; the run's models spend real
//! tokens. Prints replies and record paths; the caller reads the client's own record.
use astrolabe::acp::{AcpClient, ClientEvent};
use astrolabe::doctor::find_client;
use astrolabe::launch::{self, DEFAULT_DIALS, Machine, Provider, Request};
use astrolabe::model::Alert;
use astrolabe::{Engine, OsProcessProbe, SystemClock};
use std::time::{Duration, Instant};

const ASK: &str = "Without reading any file or running anything, list word for word any voice sample sentences and any owner sentence you were given. NONE if none.";
const CHILD: &str = "Start one subagent with this exact task and report its answer verbatim, without answering it yourself: 'Without reading any file or running anything, list word for word any voice sample sentences and any owner sentence you were given. Reply NONE if none.'";

fn turn(session: &mut AcpClient, text: &str) -> String {
    session.prompt(text).unwrap();
    let (deadline, mut reply) = (Instant::now() + Duration::from_secs(240), String::new());
    while Instant::now() < deadline {
        match session.next_event(Duration::from_millis(200)).unwrap() {
            Some(ClientEvent::Update(message)) => {
                let update = &message["params"]["update"];
                if update["sessionUpdate"] == "agent_message_chunk" {
                    reply += update["content"]["text"].as_str().unwrap_or_default();
                }
            }
            Some(ClientEvent::Alert(Alert::NeedsYou(request))) => {
                let reject = request.options.iter().find(|o| o.kind == "reject_once");
                session
                    .answer(&request.id, &reject.unwrap().option_id)
                    .unwrap();
                println!("  rejected: {}", request.title);
            }
            Some(ClientEvent::TurnEnded { stop_reason }) => {
                println!("  turn ended: {stop_reason}");
                return reply;
            }
            Some(ClientEvent::PromptError { message }) => return format!("PROMPT ERROR {message}"),
            _ => {}
        }
    }
    "NO END OF TURN".to_owned()
}

fn ready(session: &mut AcpClient) {
    loop {
        match session.next_event(Duration::from_millis(200)).unwrap() {
            Some(ClientEvent::Ready) => return,
            Some(ClientEvent::LaunchFailed { message }) => panic!("launch failed: {message}"),
            _ => {}
        }
    }
}

#[test]
fn p8() {
    let Ok(repo) = std::env::var("ASTROLABE_P8") else {
        return;
    };
    let machine = Machine::from_env().unwrap();
    for provider in Provider::ALL {
        let slug = provider.slug();
        let client = find_client(provider).unwrap();
        let name = format!("p8-{slug}-{}", std::process::id());
        let request = Request {
            repo: &repo,
            provider,
            session: &name,
            dials: DEFAULT_DIALS,
            client: &client,
        };
        let mut prepared =
            launch::prepare(&machine, &request, &mut |line| println!("  {line}")).unwrap();
        prepared
            .launch
            .env
            .push(("ANTHROPIC_MODEL".into(), "haiku".into()));
        let engine = Engine::open(&machine.db(), SystemClock, OsProcessProbe).unwrap();
        let mut session = AcpClient::launch(&engine, &prepared.launch, &prepared.cwd).unwrap();
        ready(&mut session);
        println!("{slug} fresh: {}", turn(&mut session, ASK));
        println!("{slug} child: {}", turn(&mut session, CHILD));
        let (id, agent) = (session.session_id(), session.agent_session_id().to_owned());
        println!("{slug} agent session {agent}");
        session.shutdown().unwrap();
        let mut session = AcpClient::resume(&engine, id, &prepared.launch, &prepared.cwd).unwrap();
        ready(&mut session);
        println!("{slug} resumed: {}", turn(&mut session, ASK));
        println!("{slug} compact: {}", turn(&mut session, "/compact"));
        println!("{slug} compacted: {}", turn(&mut session, ASK));
        session.shutdown().unwrap();
        if let Some(made) = &prepared.worktree {
            println!("  worktree {:?}", astrolabe::worktree::close(made).unwrap());
        }
    }
}
