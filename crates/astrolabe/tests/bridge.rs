//! The bridge and session host, as the app meets them: a real socket to a real process.
//! `shell_core` is the core with recordings in place of the launch contract.
mod common;

use common::TempDir;
use protocol::{BridgeChoice, BridgeEvent, BridgeProvider, BridgeRequest, BridgeState};
use protocol::{SessionRow, SessionState, TranscriptItem, TranscriptRole};
use serde_json::Value;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

const BIN: &str = env!("CARGO_BIN_EXE_astrolabe");
const ORIGIN: &str = "tauri://localhost";
const PATIENCE: Duration = Duration::from_secs(20);

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures/acp")
}

fn shell_core_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile = exe.parent().unwrap().parent().unwrap();
    profile
        .join("examples")
        .join(format!("shell_core{}", std::env::consts::EXE_SUFFIX))
}

/// A core process, killed when the test ends.
struct Core(Child);

impl Core {
    fn recorded(dir: &TempDir) -> Self {
        let mut command = Command::new(shell_core_path());
        command
            .arg(dir.db())
            .arg(fixtures())
            .arg(dir.0.join("clock"));
        Self::spawn(command, dir)
    }

    fn real(dir: &TempDir) -> Self {
        let mut command = Command::new(BIN);
        command.args(["serve", "--db"]).arg(dir.db());
        Self::spawn(command, dir)
    }

    fn spawn(mut command: Command, dir: &TempDir) -> Self {
        let _ = std::fs::remove_file(dir.0.join("bridge.txt"));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let core = Self(command.spawn().unwrap());
        let deadline = Instant::now() + PATIENCE;
        while !dir.0.join("bridge.txt").exists() {
            assert!(Instant::now() < deadline, "no bridge file");
            std::thread::sleep(Duration::from_millis(20));
        }
        core
    }

    /// As a crash would: nothing gets a chance to tidy up.
    fn kill(mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn handover(dir: &TempDir) -> (u16, String) {
    let text = std::fs::read_to_string(dir.0.join("bridge.txt")).unwrap();
    let (port, secret) = text.trim().split_once('\n').unwrap();
    (port.parse().unwrap(), secret.to_owned())
}

fn dial(port: u16, origin: Option<&str>) -> tungstenite::Result<WebSocket<TcpStream>> {
    let mut request = format!("ws://127.0.0.1:{port}").into_client_request()?;
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
    }
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let (socket, _) = tungstenite::client(request, stream).map_err(|why| match why {
        tungstenite::HandshakeError::Failure(why) => why,
        tungstenite::HandshakeError::Interrupted(_) => tungstenite::Error::ConnectionClosed,
    })?;
    Ok(socket)
}

fn send_on(socket: &mut WebSocket<TcpStream>, request: &BridgeRequest) {
    socket
        .send(Message::text(serde_json::to_string(request).unwrap()))
        .unwrap();
}

/// A window: it has passed both checks and keeps the last state it heard.
struct Window {
    socket: WebSocket<TcpStream>,
    state: Option<BridgeState>,
    items: Vec<TranscriptItem>,
    loaded: bool,
    problems: Vec<String>,
}

impl Window {
    fn open(dir: &TempDir) -> Self {
        let (port, secret) = handover(dir);
        let mut socket = dial(port, Some(ORIGIN)).unwrap();
        send_on(&mut socket, &BridgeRequest::Auth { secret });
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        Self {
            socket,
            state: None,
            items: Vec::new(),
            loaded: false,
            problems: Vec::new(),
        }
    }

    fn send(&mut self, request: BridgeRequest) {
        send_on(&mut self.socket, &request);
    }

    fn pump(&mut self) {
        match self.socket.read() {
            Ok(Message::Text(text)) => match serde_json::from_str(text.as_str()).unwrap() {
                BridgeEvent::State { state } => self.state = Some(state),
                BridgeEvent::Transcript { items, .. } => {
                    self.items = items;
                    self.loaded = true;
                }
                BridgeEvent::Item { item, .. } if self.loaded => self.items.push(item),
                BridgeEvent::Item { .. } => {}
                BridgeEvent::Problem { message } => self.problems.push(message),
            },
            Ok(_) => {}
            Err(tungstenite::Error::Io(why))
                if matches!(
                    why.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(why) => panic!("the bridge dropped: {why}"),
        }
    }

    fn until<T>(&mut self, what: &str, mut test: impl FnMut(&mut Self) -> Option<T>) -> T {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = test(self) {
                return found;
            }
            assert!(
                Instant::now() < deadline,
                "never saw {what}. State: {:?} items: {:?} problems: {:?}",
                self.state,
                self.items,
                self.problems
            );
            self.pump();
        }
    }

    fn session(&self) -> Option<&SessionRow> {
        self.state.as_ref()?.sessions.first()
    }

    fn until_state(&mut self, want: SessionState) -> SessionRow {
        self.until(&format!("{want:?}"), |window| {
            window.session().filter(|row| row.state == want).cloned()
        })
    }

    fn start(&mut self, recording: &str, provider: BridgeProvider) -> SessionRow {
        self.send(BridgeRequest::Start {
            repo: recording.to_owned(),
            provider,
        });
        self.until("a ready session", |window| {
            window.session().filter(|row| row.can_prompt).cloned()
        })
    }

    fn agent_text(&self) -> String {
        let agent = self
            .items
            .iter()
            .filter(|item| item.role == TranscriptRole::Agent);
        agent.map(|item| item.text.as_str()).collect()
    }

    fn says(&mut self, what: &str, session: i64) {
        self.send(BridgeRequest::Open { session });
        let wanted = what.to_owned();
        self.until(what, |window| {
            window.agent_text().contains(&wanted).then_some(())
        });
    }
}

/// The nth prompt in a recording, as its client said it.
fn recorded_prompt(recording: &str, nth: usize) -> String {
    let path = fixtures().join(format!("{recording}.jsonl"));
    let text = std::fs::read_to_string(path).unwrap();
    let rows = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap());
    let prompts = rows.filter_map(|row| {
        let message = row.get("msg").or_else(|| row.get("message"))?.clone();
        (message["method"] == "session/prompt").then_some(message)
    });
    let prompts: Vec<Value> = prompts.collect();
    prompts[nth]["params"]["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn no_mismatch(dir: &TempDir) {
    let mark = dir.0.join("agents/replay.mismatch");
    assert!(!mark.exists(), "{:?}", std::fs::read_to_string(mark));
}

/// Refused means: the socket ends and not one message came first.
fn refused(socket: &mut WebSocket<TcpStream>) {
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => panic!("sent before authenticating: {text}"),
            Ok(Message::Close(_)) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

/// Catches: anything reaching a client that lacks the secret or comes from another page.
#[test]
fn the_bridge_refuses_no_secret_a_wrong_secret_and_a_wrong_origin() {
    let dir = TempDir::new();
    let mut core = Core::real(&dir);
    let (port, secret) = handover(&dir);
    assert_eq!(secret.len(), 64);

    assert!(
        dial(port, Some("https://evil.example")).is_err(),
        "wrong origin"
    );
    assert!(
        dial(port, Some("http://localhost:1420")).is_err(),
        "another local page"
    );
    assert!(dial(port, None).is_err(), "no origin");

    let mut wrong = dial(port, Some(ORIGIN)).unwrap();
    let guess = BridgeRequest::Auth {
        secret: "0".repeat(64),
    };
    send_on(&mut wrong, &guess);
    refused(&mut wrong);

    let mut skips = dial(port, Some(ORIGIN)).unwrap();
    send_on(&mut skips, &BridgeRequest::Open { session: 1 });
    refused(&mut skips);

    let mut silent = dial(port, Some(ORIGIN)).unwrap();
    refused(&mut silent);

    let mut window = Window::open(&dir);
    window.until("the first state", |window| window.state.clone());

    // The secret goes nowhere but its file.
    core.0.kill().unwrap();
    let output = core
        .0
        .stdout
        .take()
        .map(std::io::read_to_string)
        .unwrap()
        .unwrap();
    assert!(output.contains("serving"), "{output}");
    assert!(!output.contains(&secret));
}

#[cfg(unix)]
#[test]
fn the_handover_file_is_the_owners_alone_and_new_each_start() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let core = Core::real(&dir);
    let (_, first) = handover(&dir);
    let mode = std::fs::metadata(dir.0.join("bridge.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    core.kill();
    let _core = Core::real(&dir);
    assert_ne!(handover(&dir).1, first);
}

#[test]
fn the_bridge_will_not_bind_beyond_loopback() {
    let any: SocketAddr = "0.0.0.0:0".parse().unwrap();
    assert!(astrolabe::bridge::bind(any).is_err());
    let lan: SocketAddr = "192.168.1.2:0".parse().unwrap();
    assert!(astrolabe::bridge::bind(lan).is_err());
    let local: SocketAddr = "127.0.0.1:0".parse().unwrap();
    assert!(astrolabe::bridge::bind(local).is_ok());
}

/// Catches: an allow not reaching the agent, or the session not going on after it.
#[test]
fn allow_through_the_bridge_on_both_recordings() {
    for (recording, provider, answer) in [
        (
            "codex/approval-allow",
            BridgeProvider::Codex,
            "FIXTURE_DONE",
        ),
        ("claude/approve", BridgeProvider::Claude, "DONE"),
    ] {
        let dir = TempDir::new();
        let _core = Core::recorded(&dir);
        let mut window = Window::open(&dir);
        let row = window.start(recording, provider);
        let text = recorded_prompt(recording, 0);
        window.send(BridgeRequest::Prompt {
            session: row.id,
            text,
        });
        let asked = window.until_state(SessionState::NeedsYou);
        assert!(asked.requests[0].live);
        let request = asked.requests[0].id.clone();
        window.send(BridgeRequest::Answer {
            request: request.clone(),
            choice: BridgeChoice::Allow,
        });
        let done = window.until_state(SessionState::YourTurn);
        assert!(done.requests.is_empty());
        window.says(answer, row.id);

        // A second answer to one request does nothing: no problem, no write to the agent.
        window.send(BridgeRequest::Answer {
            request,
            choice: BridgeChoice::Reject,
        });
        window.send(BridgeRequest::Open { session: row.id });
        std::thread::sleep(Duration::from_millis(300));
        window.pump();
        assert!(window.problems.is_empty(), "{:?}", window.problems);
        no_mismatch(&dir);
    }
}

#[test]
fn reject_through_the_bridge_on_both_recordings() {
    for (recording, provider) in [
        ("codex/approval-reject", BridgeProvider::Codex),
        ("claude/reject", BridgeProvider::Claude),
    ] {
        let dir = TempDir::new();
        let _core = Core::recorded(&dir);
        let mut window = Window::open(&dir);
        let row = window.start(recording, provider);
        let text = recorded_prompt(recording, 0);
        window.send(BridgeRequest::Prompt {
            session: row.id,
            text,
        });
        let asked = window.until_state(SessionState::NeedsYou);
        let request = asked.requests[0].id.clone();
        window.send(BridgeRequest::Answer {
            request,
            choice: BridgeChoice::Reject,
        });
        window.until_state(SessionState::YourTurn);
        no_mismatch(&dir);
    }
}

/// Catches: the words going before the turn has ended, or never. The recording checks
/// what came, in order, and the core's own transcript says it went as the owner's.
#[test]
fn reject_and_say_sends_the_words_once_the_turn_ends() {
    let recording = "codex/approval-reject";
    let dir = TempDir::new();
    let _core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    let row = window.start(recording, BridgeProvider::Codex);
    let text = recorded_prompt(recording, 0);
    window.send(BridgeRequest::Prompt {
        session: row.id,
        text,
    });
    let asked = window.until_state(SessionState::NeedsYou);
    let words = recorded_prompt(recording, 1);
    let request = asked.requests[0].id.clone();
    let choice = BridgeChoice::RejectAndSay {
        words: words.clone(),
    };
    window.send(BridgeRequest::Answer { request, choice });
    window.says("REPLACEMENT", row.id);
    let you = window
        .items
        .iter()
        .filter(|item| item.role == TranscriptRole::You);
    assert_eq!(you.map(|item| item.text.clone()).next_back(), Some(words));
    no_mismatch(&dir);
}

/// Catches: closing the window stopping work, or a new window finding less than the old.
#[test]
fn a_session_survives_its_window_and_the_next_window_sees_all_of_it() {
    let recording = "codex/full-turn";
    let dir = TempDir::new();
    let _core = Core::recorded(&dir);
    let mut first = Window::open(&dir);
    let row = first.start(recording, BridgeProvider::Codex);
    let text = recorded_prompt(recording, 0);
    first.send(BridgeRequest::Prompt {
        session: row.id,
        text: text.clone(),
    });
    drop(first);
    let mut second = Window::open(&dir);
    second.until_state(SessionState::YourTurn);
    second.says("FIRST", row.id);
    assert_eq!(
        second.items[0].text, text,
        "the prompt is in the transcript too"
    );
    assert_eq!(second.agent_text(), "FIRST", "each chunk once");
    no_mismatch(&dir);
}

#[test]
fn a_request_open_when_the_window_closes_is_answerable_on_reopen() {
    let recording = "codex/approval-allow";
    let dir = TempDir::new();
    let _core = Core::recorded(&dir);
    let mut first = Window::open(&dir);
    let row = first.start(recording, BridgeProvider::Codex);
    let text = recorded_prompt(recording, 0);
    first.send(BridgeRequest::Prompt {
        session: row.id,
        text,
    });
    first.until_state(SessionState::NeedsYou);
    drop(first);

    let mut second = Window::open(&dir);
    let asked = second.until_state(SessionState::NeedsYou);
    assert!(asked.requests[0].live);
    let request = asked.requests[0].id.clone();
    second.send(BridgeRequest::Answer {
        request,
        choice: BridgeChoice::Allow,
    });
    second.says("FIXTURE_DONE", row.id);
    no_mismatch(&dir);
}

/// Catches: a request whose process died looking answerable, or vanishing unexplained.
#[test]
fn a_request_open_at_core_restart_shows_what_reconcile_decided() {
    let recording = "codex/approval-allow";
    let dir = TempDir::new();
    let core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    let row = window.start(recording, BridgeProvider::Codex);
    let text = recorded_prompt(recording, 0);
    window.send(BridgeRequest::Prompt {
        session: row.id,
        text,
    });
    let asked = window.until_state(SessionState::NeedsYou);
    let request = asked.requests[0].id.clone();
    drop(window);
    core.kill();

    let _core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    let shown = window.until("the restart's decision", |window| {
        let row = window.session()?;
        let done = row.requests.first().is_some_and(|request| {
            !request.live
                && request
                    .decided
                    .as_ref()
                    .is_some_and(|d| d.contains("interrupted"))
        });
        let said = window.state.as_ref()?.restart.clone()?;
        (done && said.contains("1 session(s) marked interrupted") && said.contains("1 request(s)"))
            .then(|| row.clone())
    });
    // The card still says "needs you" until the owner has seen it: nothing was answered.
    assert_eq!(shown.requests.len(), 1);

    window.send(BridgeRequest::Answer {
        request: request.clone(),
        choice: BridgeChoice::Allow,
    });
    window.until("the refusal", |window| window.problems.first().cloned());
    window.send(BridgeRequest::Dismiss { request });
    window.until_state(SessionState::Interrupted);
}

/// Catches: refusals vanishing, or being retold by the app in other words.
#[test]
fn launch_failures_reach_the_client_in_the_cores_own_words() {
    let dir = TempDir::new();
    let _core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    let start = BridgeRequest::Start {
        repo: "refused-before-launch".to_owned(),
        provider: BridgeProvider::Codex,
    };
    window.send(start);
    let failed = window.until_state(SessionState::CouldNotStart);
    assert_eq!(
        failed.why.as_deref(),
        Some("the receipt never came, in these words")
    );

    window.send(BridgeRequest::Start {
        repo: "claude/launch_refused".to_owned(),
        provider: BridgeProvider::Claude,
    });
    let refused = window.until("the adapter's refusal", |window| {
        let row = window.session().filter(|row| row.id != failed.id)?;
        row.why.is_some().then(|| row.clone())
    });
    assert!(refused.why.unwrap().contains("native binary not found"));
}

/// The real binary and the real launch contract, as far as it gets with no such repo.
#[test]
fn the_real_contracts_refusal_reaches_the_client() {
    let dir = TempDir::new();
    let _core = Core::real(&dir);
    let mut window = Window::open(&dir);
    let start = BridgeRequest::Start {
        repo: "nope".to_owned(),
        provider: BridgeProvider::Claude,
    };
    window.send(start);
    let failed = window.until_state(SessionState::CouldNotStart);
    let why = failed.why.unwrap();
    assert!(
        why.contains("no repo named nope") || why.contains("client found"),
        "{why}"
    );
}

/// Catches: a stall card not reaching the window, and a restart raising it again.
#[test]
fn the_stall_card_reaches_the_client_and_a_restart_does_not_repeat_it() {
    let dir = TempDir::new();
    let core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    window.send(BridgeRequest::Start {
        repo: "silent".to_owned(),
        provider: BridgeProvider::Codex,
    });
    window.until("the session", |window| {
        window.session().filter(|row| row.can_stop).cloned()
    });
    std::fs::write(dir.0.join("clock"), (11 * 60_000).to_string()).unwrap();
    let card = window.until("the card", |window| {
        let row = window.session()?;
        row.cards.first().cloned()
    });
    assert!(card.summary.contains("may be stuck"), "{}", card.summary);

    drop(window);
    core.kill();
    let _core = Core::recorded(&dir);
    let mut window = Window::open(&dir);
    let again = window.until("the state", |window| window.session().cloned());
    assert!(again.cards.len() <= 1, "{:?}", again.cards);
    if let Some(shown) = again.cards.first() {
        assert_eq!(shown.id, card.id, "the same card, not a new one");
    }
    // The agent stays a moment after its pipe closes; the card lasts as long as it does,
    // and no other is ever raised.
    let shown = window.until("the interruption", |window| {
        let shown = window.session()?;
        assert!(
            shown.cards.iter().all(|seen| seen.id == card.id),
            "{:?}",
            shown.cards
        );
        (shown.state == SessionState::Interrupted).then(|| shown.clone())
    });
    assert!(shown.cards.is_empty());
}

/// Catches: a bare `serve` ignoring the data folder and opening the owner's real one.
#[test]
fn a_bare_serve_uses_the_data_folder_it_is_given() {
    let dir = TempDir::new();
    let mut command = Command::new(BIN);
    command
        .arg("serve")
        .env(astrolabe::layer::DATA_DIR_VAR, &dir.0);
    let _core = Core::spawn(command, &dir);
    assert!(dir.db().exists());
}
