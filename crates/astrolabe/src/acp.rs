//! A provider-neutral ACP client: one adapter process, one session, driven through
//! the engine. A driver supplies only a `Launch`. Plain threads and channels: the
//! engine is single-threaded, so reader and writer threads talk to the owner over
//! channels, and an async runtime would add a large dependency for one pipe. The
//! owner never blocks on the agent: the handshake and every write move on as events
//! arrive, and a silent agent is the stall watch's to flag and the owner's to stop.
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::events::error_text;
use crate::host::Clock;
use crate::model::{Alert, Request, SessionId};
pub use crate::protect::{FolderCheck, Protection, ReceiptAt};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CLAUDE_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp";
pub const CODEX_ADAPTER: &str = "@agentclientprotocol/codex-acp";

const PROTOCOL_VERSION: u64 = 1;
const METHOD_NOT_FOUND: i64 = -32601;
const TICK: Duration = Duration::from_millis(50);
/// After the adapter exits, how long its last lines get to arrive when a process
/// it left behind keeps the pipe open, and when such chatter stops being read.
/// Lines already received are always delivered.
const EXIT_QUIET: Duration = Duration::from_millis(200);
const EXIT_LIMIT: Duration = Duration::from_secs(2);
/// How often to ask whether the adapter has exited with its pipe still open. End of
/// file reports an ordinary exit at once, so this slow check matters only then.
const EXIT_CHECK: Duration = Duration::from_secs(1);
/// How long an answer call waits to hear its bytes reached the pipe. Past this the
/// answer stays queued and the request resolves when they do.
const ANSWER_WAIT: Duration = Duration::from_millis(500);
const CLOSE_WAIT: Duration = Duration::from_secs(2);
/// How long `shutdown` gives an active turn to end after `session/cancel`.
const CANCEL_WAIT: Duration = Duration::from_secs(2);
/// How often an awaited receipt is looked for.
const RECEIPT_POLL: Duration = Duration::from_millis(100);
const STDERR_TAIL: usize = 4096;
/// How much of stderr a `LaunchFailed` message carries. `Exited` has all of the tail.
const STDERR_SHOWN: usize = 500;
const SHOWN_LINE: usize = 200;

/// All a provider driver supplies. Update checks must be off in `env` or `args`.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub command: OsString,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Inherited variables the adapter must not see. `env` wins over this.
    pub env_remove: Vec<OsString>,
    /// Sent as `_meta` on `session/new` and on the resume route.
    pub meta: Option<Value>,
    /// Sent as `mcpServers` on the same routes. Empty unless a driver fills it.
    pub mcp_servers: Vec<Value>,
    /// The launch fails loudly unless the session's hooks prove they run. None only for
    /// launches with no layer, such as a driver's own fixtures.
    pub protection: Option<Protection>,
    /// The launch fails loudly if the adapter reports this mode at any point.
    /// Refuses a session folder before anything starts. The seam for a fuller check.
    pub folder_check: Option<FolderCheck>,
    pub forbidden_mode: Option<String>,
}

impl Launch {
    /// Runs an adapter's script directly: a package's `.cmd` shim on Windows would
    /// add a batch file's quoting rules between us and the arguments.
    pub fn node(entry: &Path) -> Self {
        Self {
            command: "node".into(),
            args: vec![entry.into()],
            ..Self::default()
        }
    }
}

/// The one place an adapter is found: the script a pinned package names as its
/// command, under the explicit directory holding `adapters/node_modules`.
pub fn adapter_entry(adapters_dir: &Path, package: &str) -> Result<PathBuf> {
    let root = package
        .split('/')
        .fold(adapters_dir.join("node_modules"), |path, part| {
            path.join(part)
        });
    let not_found = |detail: String| Error::AdapterNotFound(format!("{package}: {detail}"));
    let manifest = std::fs::read(root.join("package.json"))
        .map_err(|error| not_found(format!("{}: {error}", root.display())))?;
    let manifest: Value = serde_json::from_slice(&manifest)
        .map_err(|error| not_found(format!("bad package.json: {error}")))?;
    let bin = manifest.get("bin");
    let script = bin
        .and_then(Value::as_str)
        .or_else(|| bin?.as_object()?.values().next()?.as_str())
        .ok_or_else(|| not_found("package.json names no command".to_owned()))?;
    let entry = script.split('/').fold(root, |path, part| path.join(part));
    if entry.is_file() {
        // Absolute: the adapter runs with the target repo as its working directory.
        Ok(std::path::absolute(entry)?)
    } else {
        Err(not_found(format!("{} is missing", entry.display())))
    }
}

/// What the owner of the client hears. Engine state is already updated.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// The session exists; prompts can be sent. Silence before this is a stall, which
    /// the engine's watch flags from launch.
    Ready,
    /// The adapter refused or left before a session existed. `Exited` follows.
    LaunchFailed {
        message: String,
    },
    /// Raw `session/update`, for a transcript, including history replayed on resume.
    Update(Value),
    /// A request or card was raised. Answer requests with `AcpClient::answer`.
    Alert(Alert),
    TurnEnded {
        stop_reason: String,
    },
    /// The active prompt failed.
    PromptError {
        message: String,
    },
    /// An agent error that answers nothing active. No state changed; reading goes on.
    AgentError {
        rpc_id: String,
        message: String,
    },
    /// A line that was not a JSON-RPC message. Reading goes on.
    Malformed {
        line: String,
    },
    /// The agent asked for something this client does not offer, and was told so.
    UnsupportedRequest {
        method: String,
    },
    /// The pipe to the agent broke. It may still be talking, but cannot be answered.
    WriteFailed {
        message: String,
    },
    /// The last event: the process has ended and the engine knows.
    Exited {
        code: Option<i32>,
        stderr: String,
    },
}

/// What the reader and writer threads tell the owner, in one queue.
enum Input {
    Line(Vec<u8>),
    StdoutClosed,
    Written(u64),
    WriteFailed(String),
}

/// A write whose outcome matters, until the writer reports it.
enum Pending {
    Prompt(Value),
    Answer { request: String, resolution: String },
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Initializing,
    Opening,
    Ready,
}

pub struct AcpClient<'e> {
    engine: &'e Engine,
    session: SessionId,
    agent_session: String,
    child: Child,
    writes: Option<Sender<(u64, Vec<u8>)>>,
    unwritten: HashMap<u64, Pending>,
    next_write: u64,
    input: Receiver<Input>,
    stop_reading: Arc<AtomicBool>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    stderr_reader: JoinHandle<()>,
    next_id: u64,
    stage: Stage,
    open_params: Value,
    resuming: bool,
    failure: Option<String>,
    prompt_id: Option<Value>,
    /// The owner cancelled this turn: any request still arriving is answered so.
    cancelling: bool,
    awaited: Option<Value>,
    /// During a resume, history may replay; it is not live progress.
    replaying: bool,
    /// Requests this process raised: the only ones its pipe can answer.
    live: HashSet<String>,
    events: VecDeque<ClientEvent>,
    exit_checked: Instant,
    exit_seen: Option<Instant>,
    reaped: Option<ExitStatus>,
    gone: bool,
    protection: Option<Protection>,
    forbidden_mode: Option<String>,
    /// Hook reports older than this belong to an earlier process.
    started_ms: i64,
    /// Set while a receipt is awaited. Claude: before Ready. Codex: from the first prompt.
    receipt_deadline: Option<Instant>,
    receipt_checked: Option<Instant>,
    receipt_done: bool,
    /// Held for its drop: on Windows, closing it ends what the adapter started.
    _job: Option<crate::job::Job>,
}

impl<'e> AcpClient<'e> {
    /// Starts an adapter and returns at once. `Ready` or `LaunchFailed` follows.
    pub fn launch(engine: &'e Engine, launch: &Launch, cwd: &Path) -> Result<Self> {
        let mut client = Self::spawn(engine, launch, cwd, |pid| engine.launch(pid))?;
        client.begin();
        Ok(client)
    }

    /// Takes up an ended session in a new process, through whichever route the
    /// adapter says it has. `Ready` or `LaunchFailed` follows.
    pub fn resume(
        engine: &'e Engine,
        session: SessionId,
        launch: &Launch,
        cwd: &Path,
    ) -> Result<Self> {
        let agent_session = engine.inspect(session)?.session.agent_session_id;
        let agent_session = agent_session.ok_or(Error::Launch {
            session_id: session,
            message: "no agent session id was recorded".to_owned(),
        })?;
        let mut client = Self::spawn(engine, launch, cwd, |pid| {
            engine.reopen(session, pid).map(|_| session)
        })?;
        client.agent_session = agent_session;
        client.resuming = true;
        client.begin();
        Ok(client)
    }

    fn spawn(
        engine: &'e Engine,
        launch: &Launch,
        cwd: &Path,
        register: impl FnOnce(u32) -> Result<SessionId>,
    ) -> Result<Self> {
        let mut command = Command::new(&launch.command);
        for name in &launch.env_remove {
            command.env_remove(name);
        }
        command
            .args(&launch.args)
            .envs(launch.env.iter().map(|(name, value)| (name, value)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(check) = launch.folder_check {
            check(cwd).map_err(Error::UnsafeSessionFolder)?;
        }
        if let Some(protection) = &launch.protection {
            protection
                .check_guard(launch, cwd)
                .map_err(Error::GuardNotRunning)?;
        }
        if launch.protection.is_some() {
            command.env(crate::layer::WORKTREE_VAR, cwd);
        }
        detach_from_terminal(&mut command);
        let started_ms = crate::host::SystemClock.now_ms();
        let mut child = command.spawn()?;
        let job = crate::job::Job::assign(&child);
        let session = match register(child.id()) {
            Ok(session) => session,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let (sender, input) = mpsc::channel();
        let (writes, queue) = mpsc::channel();
        let stop_reading = Arc::new(AtomicBool::new(false));
        // These threads end at end of file or a failed write. A process the adapter
        // left behind can hold the pipes open, so nothing joins them.
        spawn_writer(stdin, queue, sender.clone());
        spawn_reader(stdout, sender, stop_reading.clone());
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let writer = tail.clone();
        let stderr_reader = thread::spawn(move || keep_tail(stderr, &writer));
        let mut open_params = json!({
            "cwd": cwd.to_string_lossy(),
            "mcpServers": launch.mcp_servers,
        });
        if let Some(meta) = &launch.meta {
            open_params["_meta"] = meta.clone();
        }
        Ok(Self {
            engine,
            session,
            agent_session: String::new(),
            child,
            writes: Some(writes),
            unwritten: HashMap::new(),
            next_write: 1,
            input,
            stop_reading,
            stderr: tail,
            stderr_reader,
            next_id: 1,
            stage: Stage::Initializing,
            open_params,
            resuming: false,
            failure: None,
            prompt_id: None,
            cancelling: false,
            awaited: None,
            replaying: false,
            live: HashSet::new(),
            events: VecDeque::new(),
            exit_checked: Instant::now(),
            exit_seen: None,
            reaped: None,
            gone: false,
            protection: launch.protection.clone(),
            forbidden_mode: launch.forbidden_mode.clone(),
            started_ms,
            receipt_deadline: None,
            receipt_checked: None,
            receipt_done: false,
            _job: job,
        })
    }

    fn begin(&mut self) {
        self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false,
                    // So a child speaks under an id of its own. Without it Codex hides a
                    // child's tools altogether and Claude reports them as the parent's.
                    "subagents": {},
                },
                "clientInfo": { "name": "astrolabe", "version": env!("CARGO_PKG_VERSION") },
            }),
        );
    }

    /// A handshake request. A pipe already gone shows up as `WriteFailed` input.
    fn request(&mut self, method: &str, params: Value) {
        let id = self.take_id();
        self.awaited = Some(id.clone());
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let _ = self.write(&message, None);
    }

    /// The adapter answered the handshake request. An adapter that refuses is closed:
    /// that is its answer, not silence.
    fn advance(&mut self, response: &Value) -> Result<()> {
        if let Some(error) = response.get("error") {
            return self.fail_launch(error_text(error));
        }
        let result = response.get("result").unwrap_or(&Value::Null);
        if self.stage == Stage::Initializing {
            let version = result.get("protocolVersion").and_then(Value::as_u64);
            if version != Some(PROTOCOL_VERSION) {
                return self.fail_launch(format!("adapter speaks protocol {version:?}"));
            }
            let mut params = self.open_params.take();
            let route = if self.resuming {
                let Some(route) = resume_route(result) else {
                    return self.fail_launch("adapter offers no way to resume".to_owned());
                };
                params["sessionId"] = json!(self.agent_session);
                self.replaying = true;
                route
            } else {
                "session/new"
            };
            self.stage = Stage::Opening;
            self.request(route, params);
            return Ok(());
        }
        if !self.resuming {
            let Some(id) = result.get("sessionId").and_then(Value::as_str) else {
                return self.fail_launch("no sessionId from session/new".to_owned());
            };
            self.agent_session = id.to_owned();
            self.engine.set_agent_session(self.session, id)?;
        }
        if let Some(message) = self.forbidden_mode_in(result.pointer("/modes/currentModeId")) {
            return self.fail_launch(message);
        }
        if self
            .protection
            .as_ref()
            .is_some_and(|p| p.at == ReceiptAt::BeforeReady)
        {
            self.start_receipt_wait();
            return Ok(());
        }
        self.become_ready()
    }

    fn become_ready(&mut self) -> Result<()> {
        self.replaying = false;
        self.engine.session_ready(self.session)?;
        self.stage = Stage::Ready;
        self.events.push_back(ClientEvent::Ready);
        Ok(())
    }

    fn forbidden_mode_in(&self, mode: Option<&Value>) -> Option<String> {
        let forbidden = self.forbidden_mode.as_deref()?;
        match mode.and_then(Value::as_str) {
            Some(mode) if mode == forbidden => Some(format!(
                "the session landed in mode \"{mode}\", where approvals would be answered without reaching you; no safe mode is set for this platform or it was ignored"
            )),
            Some(_) => None,
            None => Some(format!(
                "the adapter did not report a mode, so a landing in \"{forbidden}\" can't be ruled out"
            )),
        }
    }

    fn start_receipt_wait(&mut self) {
        if let Some(protection) = &self.protection
            && !self.receipt_done
            && self.receipt_deadline.is_none()
        {
            self.receipt_deadline = Some(Instant::now() + protection.wait);
        }
    }

    /// At most a few times a second, and only while a receipt is awaited.
    fn check_receipt(&mut self) -> Result<()> {
        let (Some(deadline), Some(protection)) = (self.receipt_deadline, &self.protection) else {
            return Ok(());
        };
        if self
            .receipt_checked
            .is_some_and(|at| at.elapsed() < RECEIPT_POLL)
        {
            return Ok(());
        }
        self.receipt_checked = Some(Instant::now());
        if protection.receipt_arrived(&self.agent_session, self.started_ms) {
            self.receipt_deadline = None;
            self.receipt_done = true;
            if self.stage != Stage::Ready {
                return self.become_ready();
            }
            return Ok(());
        }
        if Instant::now() < deadline {
            return Ok(());
        }
        self.receipt_deadline = None;
        let message = protection.missing_message();
        if self.stage == Stage::Ready && self.prompt_id.is_some() {
            // Not `cancel`: this is a failed launch, not the owner stopping a run.
            let _ = self.write(
                &json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": self.agent_session },
                }),
                None,
            );
        }
        self.fail_launch(message)
    }

    fn fail_launch(&mut self, message: String) -> Result<()> {
        self.failure = Some(message);
        self.finish()
    }

    pub fn session_id(&self) -> SessionId {
        self.session
    }

    pub fn agent_session_id(&self) -> &str {
        &self.agent_session
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Queues a prompt, on the launch's waiting run or as a new turn. A turn still
    /// going is refused: say-what-instead is a reject, then this once the turn ends.
    pub fn prompt(&mut self, text: &str) -> Result<()> {
        if self.gone {
            return Err(Error::AgentGone(self.session));
        }
        if self.stage != Stage::Ready {
            return Err(Error::NotReady(self.session));
        }
        let run = self.engine.inspect(self.session)?.run;
        if !run.state.is_active() || run.prompt_rpc_id.is_some() {
            self.engine.start_turn(self.session)?;
        }
        let id = self.take_id();
        self.engine.prompt_sent(self.session, &id)?;
        self.prompt_id = Some(id.clone());
        if self
            .protection
            .as_ref()
            .is_some_and(|p| p.at == ReceiptAt::AfterFirstPrompt)
        {
            self.start_receipt_wait();
        }
        let sent = self.write(
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/prompt",
                "params": {
                    "sessionId": self.agent_session,
                    "prompt": [{ "type": "text", "text": text }],
                },
            }),
            Some(Pending::Prompt(id.clone())),
        );
        if let Err(error) = sent {
            self.fail_prompt(&format!("prompt not delivered: {error}"))?;
            return Err(error);
        }
        Ok(())
    }

    /// Delivers the option the owner chose; the request resolves once the bytes reach
    /// the pipe. Choosing is the caller's job. Returns false when it was already
    /// resolved or answered. An agent not reading leaves the answer queued.
    pub fn answer(&mut self, request_id: &str, option_id: &str) -> Result<bool> {
        let Some((request, resolved)) = self.engine.request(request_id)? else {
            return Err(Error::UnknownRequest(request_id.to_owned()));
        };
        if resolved || self.answering(request_id) {
            return Ok(false);
        }
        if !request.options.iter().any(|o| o.option_id == option_id) {
            return Err(Error::UnknownOption {
                request: request_id.to_owned(),
                option: option_id.to_owned(),
            });
        }
        let outcome = json!({ "outcome": "selected", "optionId": option_id });
        self.reply(&request, outcome, option_id)?;
        self.settle()?;
        if self.gone || self.writes.is_none() {
            let (_, resolved) = self.engine.request(request_id)?.unwrap_or((request, false));
            if !resolved {
                return Err(Error::AgentGone(self.session));
            }
        }
        Ok(true)
    }

    /// Asks the agent to stop. The run ends only on the agent's own evidence. Open
    /// requests, and any that arrive before the turn ends, are answered as cancelled,
    /// as the protocol requires.
    pub fn cancel(&mut self) -> Result<()> {
        if self.stage != Stage::Ready {
            return Err(Error::NotReady(self.session));
        }
        self.engine.request_stop(self.session)?;
        self.write(
            &json!({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": { "sessionId": self.agent_session },
            }),
            None,
        )?;
        self.cancelling = self.prompt_id.is_some();
        for id in self.live.clone() {
            if let Some((request, false)) = self.engine.request(&id)?
                && !self.answering(&id)
            {
                self.reply(&request, json!({ "outcome": "cancelled" }), "cancelled")?;
            }
        }
        self.settle()
    }

    /// Waits up to `wait` for the next event. None means nothing arrived, or the
    /// process has ended and `Exited` was already delivered.
    pub fn next_event(&mut self, wait: Duration) -> Result<Option<ClientEvent>> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if self.gone {
                return Ok(None);
            }
            self.pump(left)?;
            if self.events.is_empty() && left.is_zero() {
                return Ok(None);
            }
        }
    }

    /// Stops an active turn, closes the pipe, gives the adapter a moment, then ends
    /// it. Only an explicit call does this: nothing here reacts to a stall. The cancel
    /// comes first because a tool can leave the adapter's process group, where only
    /// the adapter can stop it. A crash gets no such chance.
    pub fn shutdown(mut self) -> Result<()> {
        let deadline = Instant::now() + CANCEL_WAIT;
        if self.prompt_id.is_some() && !self.cancelling && !self.gone {
            match self.cancel() {
                Ok(()) | Err(Error::AgentGone(_)) => {}
                Err(error) => return Err(error),
            }
        }
        while self.prompt_id.is_some() && self.writes.is_some() && !self.gone {
            if Instant::now() >= deadline {
                break;
            }
            self.pump(TICK)?;
        }
        if !self.gone {
            self.finish()?;
        }
        Ok(())
    }

    fn take_id(&mut self) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        json!(id)
    }

    fn answering(&self, request_id: &str) -> bool {
        self.unwritten.values().any(
            |pending| matches!(pending, Pending::Answer { request, .. } if request == request_id),
        )
    }

    /// Queues an answer. Never reads, so it is safe while handling a line.
    fn reply(&mut self, request: &Request, outcome: Value, resolution: &str) -> Result<()> {
        if !self.live.contains(&request.id) {
            return Err(Error::RequestNotLive(request.id.clone()));
        }
        let raw_id: Value = serde_json::from_str(&request.rpc_id)
            .map_err(|error| protocol_error(format!("stored request id: {error}")))?;
        self.write(
            &json!({
                "jsonrpc": "2.0",
                "id": raw_id,
                "result": { "outcome": outcome },
            }),
            Some(Pending::Answer {
                request: request.id.clone(),
                resolution: resolution.to_owned(),
            }),
        )
    }

    /// Reads for a short while so queued answers can resolve before the call returns.
    fn settle(&mut self) -> Result<()> {
        let deadline = Instant::now() + ANSWER_WAIT;
        while !self.gone
            && Instant::now() < deadline
            && self
                .unwritten
                .values()
                .any(|pending| matches!(pending, Pending::Answer { .. }))
        {
            self.pump(TICK)?;
        }
        Ok(())
    }

    /// Hands a message to the writer thread, so a full pipe can never block the owner.
    fn write(&mut self, message: &Value, pending: Option<Pending>) -> Result<()> {
        let Some(writes) = &self.writes else {
            return Err(Error::AgentGone(self.session));
        };
        let tag = self.next_write;
        self.next_write += 1;
        let mut line = message.to_string();
        line.push('\n');
        writes
            .send((tag, line.into_bytes()))
            .map_err(|_| Error::AgentGone(self.session))?;
        if let Some(pending) = pending {
            self.unwritten.insert(tag, pending);
        }
        Ok(())
    }

    fn written(&mut self, tag: u64) -> Result<()> {
        if let Some(Pending::Answer {
            request,
            resolution,
        }) = self.unwritten.remove(&tag)
        {
            self.live.remove(&request);
            self.engine.resolve_request(&request, &resolution)?;
        }
        Ok(())
    }

    /// Nothing more can reach the agent. Answers not delivered leave their requests
    /// open; a prompt not delivered fails its run, since no answer can come.
    fn write_failed(&mut self, message: String) -> Result<()> {
        self.writes = None;
        let lost_prompt = self.unwritten.drain().any(
            |(_, pending)| matches!(pending, Pending::Prompt(id) if Some(&id) == self.prompt_id.as_ref()),
        );
        self.events.push_back(ClientEvent::WriteFailed {
            message: message.clone(),
        });
        if self.stage != Stage::Ready {
            return self.fail_launch(format!("write failed: {message}"));
        }
        if lost_prompt {
            self.fail_prompt(&format!("prompt not delivered: {message}"))?;
        }
        Ok(())
    }

    fn pump(&mut self, wait: Duration) -> Result<()> {
        if self.gone {
            return Ok(());
        }
        self.check_receipt()?;
        if self.gone {
            return Ok(());
        }
        if self.exit_seen.is_none() && self.exit_checked.elapsed() >= EXIT_CHECK {
            self.exit_checked = Instant::now();
            if self.has_exited()? {
                self.exit_seen = Some(Instant::now());
            }
        }
        let since_exit = self.exit_seen.map(|seen| seen.elapsed());
        if since_exit.is_some_and(|since| since >= EXIT_LIMIT) {
            // What is already queued still gets delivered; only further reads stop.
            self.stop_reading.store(true, Ordering::Relaxed);
        }
        match self.input.recv_timeout(wait.min(TICK)) {
            Ok(Input::Line(line)) => self.handle_line(&line),
            Ok(Input::Written(tag)) => self.written(tag),
            Ok(Input::WriteFailed(message)) => self.write_failed(message),
            Ok(Input::StdoutClosed) | Err(RecvTimeoutError::Disconnected) => self.finish(),
            Err(RecvTimeoutError::Timeout) => {
                // Exited, yet something it started still holds the pipe open.
                if since_exit.is_some_and(|since| since >= EXIT_QUIET) {
                    return self.finish();
                }
                Ok(())
            }
        }
    }

    fn handle_line(&mut self, line: &[u8]) -> Result<()> {
        let text = String::from_utf8_lossy(line);
        if text.trim().is_empty() {
            return Ok(());
        }
        let message = serde_json::from_slice::<Value>(line)
            .ok()
            .filter(Value::is_object);
        let Some(message) = message else {
            self.malformed(&text);
            return Ok(());
        };
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        match (message.get("method").and_then(Value::as_str), id) {
            (Some("session/request_permission"), Some(_)) => self.ingest(&message),
            (Some(method), Some(id)) => {
                let method = method.to_owned();
                self.refuse(id, &method);
                Ok(())
            }
            (Some("session/update"), None) => {
                let update = message.pointer("/params/update");
                if update
                    .and_then(|update| update.get("sessionUpdate"))
                    .is_some_and(|kind| kind == "current_mode_update")
                    && let Some(mode) = update.and_then(|update| update.get("currentModeId"))
                    && let Some(failure) = self.forbidden_mode_in(Some(mode))
                {
                    return self.fail_launch(failure);
                }
                if !self.replaying {
                    self.ingest(&message)?;
                }
                self.events.push_back(ClientEvent::Update(message));
                Ok(())
            }
            (Some(_), None) => Ok(()),
            (None, id) if id.is_some() || message.get("error").is_some() => {
                self.handle_response(message, id)
            }
            (None, _) => {
                self.malformed(&text);
                Ok(())
            }
        }
    }

    fn handle_response(&mut self, message: Value, id: Option<Value>) -> Result<()> {
        if id.is_some() && id == self.awaited {
            self.awaited = None;
            return self.advance(&message);
        }
        let is_prompt = id.is_some() && id == self.prompt_id;
        let stop_reason = message
            .pointer("/result/stopReason")
            .and_then(Value::as_str);
        if is_prompt && stop_reason.is_none() && message.get("error").is_none() {
            // Left alone, this run would look active forever.
            return self.fail_prompt("prompt answered without a stop reason");
        }
        self.ingest(&message)?;
        if is_prompt {
            self.prompt_id = None;
            self.cancelling = false;
            self.events.push_back(match stop_reason {
                Some(reason) => ClientEvent::TurnEnded {
                    stop_reason: reason.to_owned(),
                },
                None => ClientEvent::PromptError {
                    message: error_text(&message["error"]),
                },
            });
        }
        Ok(())
    }

    /// Feeds the engine and turns its answer into events. `AgentRpc` is news, not a
    /// failure of this loop, so it never stops the reading.
    fn ingest(&mut self, message: &Value) -> Result<()> {
        match self.engine.ingest(self.session, message) {
            Ok(alerts) => {
                for alert in alerts {
                    if let Alert::NeedsYou(request) = &alert {
                        self.live.insert(request.id.clone());
                        if self.cancelling {
                            let cancelled = json!({ "outcome": "cancelled" });
                            let replied = self.reply(request, cancelled, "cancelled");
                            if replied.is_ok() {
                                continue;
                            }
                        }
                    }
                    self.events.push_back(ClientEvent::Alert(alert));
                }
                Ok(())
            }
            Err(Error::AgentRpc {
                rpc_id, message, ..
            }) => {
                let event = ClientEvent::AgentError { rpc_id, message };
                self.events.push_back(event);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn fail_prompt(&mut self, message: &str) -> Result<()> {
        let Some(id) = self.prompt_id.take() else {
            return Ok(());
        };
        self.cancelling = false;
        let failure = json!({ "id": id, "error": { "message": message } });
        self.engine.ingest(self.session, &failure)?;
        self.events.push_back(ClientEvent::PromptError {
            message: message.to_owned(),
        });
        Ok(())
    }

    fn refuse(&mut self, id: Value, method: &str) {
        let refused = self.write(
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": METHOD_NOT_FOUND,
                    "message": format!("{method} is not supported"),
                },
            }),
            None,
        );
        if let Err(error) = refused {
            self.events.push_back(ClientEvent::WriteFailed {
                message: error.to_string(),
            });
        }
        self.events.push_back(ClientEvent::UnsupportedRequest {
            method: method.to_owned(),
        });
    }

    fn malformed(&mut self, text: &str) {
        let line = text.trim_end().chars().take(SHOWN_LINE).collect();
        self.events.push_back(ClientEvent::Malformed { line });
    }

    /// Waits briefly for the last of it: the pipe can outlast the process.
    fn stderr_text(&self) -> String {
        let deadline = Instant::now() + EXIT_QUIET;
        while !self.stderr_reader.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let tail = self.stderr.lock().map(|tail| tail.clone());
        let tail: Vec<u8> = tail.unwrap_or_default().into();
        String::from_utf8_lossy(&tail).trim().to_owned()
    }

    /// The process is over or cannot be heard any more, or the owner or the adapter's
    /// own refusal ended it. Closing stdin lets a healthy adapter leave by itself.
    /// Silence never leads here: a live adapter that is merely quiet stays up.
    fn finish(&mut self) -> Result<()> {
        self.writes = None;
        let status = self.wait_or_kill()?;
        self.reconcile_writes()?;
        self.unwritten.clear();
        self.gone = true;
        self.live.clear();
        self.engine.process_exited(self.session)?;
        let stderr = self.stderr_text();
        if self.stage != Stage::Ready || self.failure.is_some() {
            let reason = self.failure.take();
            let mut message =
                reason.unwrap_or_else(|| "process ended before the session was ready".to_owned());
            let useful = useful_stderr(&stderr);
            if !useful.is_empty() {
                message.push_str("; stderr: ");
                message.push_str(&useful);
            }
            self.events.push_back(ClientEvent::LaunchFailed { message });
        }
        self.events.push_back(ClientEvent::Exited {
            code: status.code(),
            stderr,
        });
        Ok(())
    }

    /// An answer the agent acted on before exiting may have its report still on the
    /// way. Without this it would stay open forever although it was delivered.
    fn reconcile_writes(&mut self) -> Result<()> {
        let deadline = Instant::now() + EXIT_QUIET;
        while !self.unwritten.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.input.recv_timeout(left) {
                Ok(Input::Written(tag)) => self.written(tag)?,
                Ok(Input::Line(_) | Input::StdoutClosed) => {}
                Ok(Input::WriteFailed(_)) | Err(_) => break,
            }
        }
        Ok(())
    }

    /// Never reaps on Unix: a zombie leader keeps its pid, so the group number cannot
    /// pass to anyone else before `wait_or_kill` signals it. On Windows `Child` holds
    /// a handle, which keeps the pid just the same.
    fn has_exited(&mut self) -> Result<bool> {
        if self.reaped.is_some() {
            return Ok(true);
        }
        if !cfg!(windows) {
            let output = Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", &self.child.id().to_string()])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            if let Ok(output) = output {
                let stat = String::from_utf8_lossy(&output.stdout);
                let stat = stat.trim();
                return Ok(stat.is_empty() || stat.starts_with('Z'));
            }
        }
        // With no way to look without reaping, the group goes unsignalled if it exited.
        self.reaped = self.child.try_wait()?;
        Ok(self.reaped.is_some())
    }

    /// What is left in the adapter's group goes with it. A tool in a group of its own
    /// does not; see `shutdown`. Once the leader is reaped its pid can pass to
    /// anyone, so `reaped` is set first and no later cleanup signals. Windows can only
    /// walk the tree from a live root, which is what is left by the signal below.
    fn wait_or_kill(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + CLOSE_WAIT;
        let mut exited = self.has_exited()?;
        while !exited && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
            exited = self.has_exited()?;
        }
        if let Some(status) = self.reaped {
            return Ok(status);
        }
        kill_tree(self.child.id());
        let _ = self.child.kill();
        let status = self.child.wait()?;
        self.reaped = Some(status);
        Ok(status)
    }
}

fn spawn_reader(stdout: ChildStdout, input: Sender<Input>, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let read = reader.read_until(b'\n', &mut line);
            if !matches!(read, Ok(1..)) || stop.load(Ordering::Relaxed) {
                let _ = input.send(Input::StdoutClosed);
                return;
            }
            if input.send(Input::Line(line)).is_err() {
                return;
            }
        }
    });
}

fn spawn_writer(mut stdin: ChildStdin, queue: Receiver<(u64, Vec<u8>)>, input: Sender<Input>) {
    thread::spawn(move || {
        for (tag, bytes) in queue {
            let written = stdin.write_all(&bytes).and_then(|()| stdin.flush());
            let report = match written {
                Ok(()) => Input::Written(tag),
                Err(error) => Input::WriteFailed(error.to_string()),
            };
            let failed = matches!(report, Input::WriteFailed(_));
            if input.send(report).is_err() || failed {
                return;
            }
        }
    });
}

/// Asks the OS tool, as `OsProcessProbe` does, so this needs no native bindings. The
/// adapter leads its own process group on Unix. A process that left the group, or
/// any tree under an already-dead root on Windows, is out of reach.
fn kill_tree(pid: u32) {
    let mut command = if cfg!(windows) {
        let mut taskkill = Command::new("taskkill");
        taskkill.args(["/T", "/F", "/PID", &pid.to_string()]);
        taskkill
    } else {
        let mut kill = Command::new("/bin/kill");
        kill.args(["-KILL", "--", &format!("-{pid}")]);
        kill
    };
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The adapter's stdin is the JSON-RPC pipe; nothing it starts should find a
/// terminal to wait on.
fn detach_from_terminal(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn resume_route(initialized: &Value) -> Option<&'static str> {
    let capabilities = initialized.get("agentCapabilities")?;
    if capabilities
        .pointer("/sessionCapabilities/resume")
        .is_some()
    {
        return Some("session/resume");
    }
    let load = capabilities.get("loadSession").and_then(Value::as_bool);
    load.unwrap_or(false).then_some("session/load")
}

fn keep_tail(mut source: impl Read, tail: &Mutex<VecDeque<u8>>) {
    let mut chunk = [0; 1024];
    while let Ok(n @ 1..) = source.read(&mut chunk) {
        let Ok(mut tail) = tail.lock() else { return };
        tail.extend(&chunk[..n]);
        let extra = tail.len().saturating_sub(STDERR_TAIL);
        tail.drain(..extra);
    }
}

/// Stderr means nothing by itself: a healthy adapter may write on every launch, as
/// the Claude adapter does with its phase timings. Those go, and they would otherwise
/// push out a reason written before them; of the rest, the end is the part most likely
/// to say why. Only that exact shape goes: other lines can carry timings and a cause.
fn useful_stderr(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !is_phase_timing(line))
        .collect();
    let useful = lines.join("\n");
    let skip = useful.chars().count().saturating_sub(STDERR_SHOWN);
    useful.chars().skip(skip).collect()
}

fn is_phase_timing(line: &str) -> bool {
    line.starts_with("[session/create] sessionId=")
        && line.contains(" phase=")
        && line.contains(" durationMs=")
}

fn protocol_error(message: impl Into<String>) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}
