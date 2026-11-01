//! A provider-neutral ACP client: one adapter process, one session, driven through
//! the engine. A driver supplies only a `Launch`. Plain threads and channels: the
//! engine is single-threaded, so one reader thread hands lines to the owner, and an
//! async runtime would add a large dependency for one pipe.
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::model::{Alert, Request, SessionId};
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const CLAUDE_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp";
pub const CODEX_ADAPTER: &str = "@agentclientprotocol/codex-acp";
/// A starting value. Real adapters are measured in part B.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);

const PROTOCOL_VERSION: u64 = 1;
const METHOD_NOT_FOUND: i64 = -32601;
const TICK: Duration = Duration::from_millis(50);
/// After the adapter exits, how long its last lines get to arrive when a process
/// it left behind keeps the pipe open, and how long such chatter can delay the end.
const EXIT_QUIET: Duration = Duration::from_millis(200);
const EXIT_LIMIT: Duration = Duration::from_secs(5);
const CLOSE_WAIT: Duration = Duration::from_secs(2);
const STDERR_TAIL: usize = 4096;
const SHOWN_LINE: usize = 200;

/// All a provider driver supplies. Update checks must be off in `env` or `args`.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub command: OsString,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    /// Sent as `_meta` on `session/new` and on the resume route.
    pub meta: Option<Value>,
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
        Ok(entry)
    } else {
        Err(not_found(format!("{} is missing", entry.display())))
    }
}

/// What the owner of the client hears. Engine state is already updated.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// Raw `session/update`, for a transcript.
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

pub struct AcpClient<'e> {
    engine: &'e Engine,
    session: SessionId,
    agent_session: String,
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Vec<u8>>,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    stderr_reader: JoinHandle<()>,
    next_id: u64,
    prompt_id: Option<Value>,
    awaited: Option<Value>,
    response: Option<Value>,
    /// During a resume, history may replay; it is not live progress.
    replaying: bool,
    /// Requests this process raised: the only ones its pipe can answer.
    live: HashSet<String>,
    events: VecDeque<ClientEvent>,
    exit_seen: Option<Instant>,
    gone: bool,
}

impl<'e> AcpClient<'e> {
    /// Starts an adapter and a new session. Any failure leaves no process behind
    /// and, once the session exists, marks it failed.
    pub fn launch(
        engine: &'e Engine,
        launch: &Launch,
        cwd: &Path,
        handshake_timeout: Duration,
    ) -> Result<Self> {
        let client = Self::spawn(engine, launch, cwd, |pid| engine.launch(pid))?;
        client.handshake(launch, cwd, None, handshake_timeout)
    }

    /// Takes up an ended session in a new process, through whichever route the
    /// adapter says it has.
    pub fn resume(
        engine: &'e Engine,
        session: SessionId,
        launch: &Launch,
        cwd: &Path,
        handshake_timeout: Duration,
    ) -> Result<Self> {
        let agent_session = engine.inspect(session)?.session.agent_session_id;
        let agent_session = agent_session.ok_or(Error::Launch {
            session_id: session,
            message: "no agent session id was recorded".to_owned(),
        })?;
        let client = Self::spawn(engine, launch, cwd, |pid| {
            engine.reopen(session, pid).map(|_| session)
        })?;
        client.handshake(launch, cwd, Some(agent_session), handshake_timeout)
    }

    fn spawn(
        engine: &'e Engine,
        launch: &Launch,
        cwd: &Path,
        register: impl FnOnce(u32) -> Result<SessionId>,
    ) -> Result<Self> {
        let mut command = Command::new(&launch.command);
        command
            .args(&launch.args)
            .envs(launch.env.iter().map(|(name, value)| (name, value)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        detach_from_terminal(&mut command);
        let mut child = command.spawn()?;
        let session = match register(child.id()) {
            Ok(session) => session,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let (sender, lines) = mpsc::channel();
        // Both threads end at end of file. A process the adapter left behind can
        // hold the pipes open, so nothing joins them.
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(n) if n > 0 && sender.send(line).is_ok() => {}
                    _ => return,
                }
            }
        });
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let writer = tail.clone();
        let stderr_reader = thread::spawn(move || keep_tail(stderr, &writer));
        Ok(Self {
            engine,
            session,
            agent_session: String::new(),
            child,
            stdin,
            lines,
            stderr: tail,
            stderr_reader,
            next_id: 1,
            prompt_id: None,
            awaited: None,
            response: None,
            replaying: false,
            live: HashSet::new(),
            events: VecDeque::new(),
            exit_seen: None,
            gone: false,
        })
    }

    fn handshake(
        mut self,
        launch: &Launch,
        cwd: &Path,
        resume: Option<String>,
        timeout: Duration,
    ) -> Result<Self> {
        match self.exchange(launch, cwd, resume, Instant::now() + timeout) {
            Ok(()) => Ok(self),
            Err(error) => {
                // No turn has begun, so closing here cannot cost any work.
                let session_id = self.session;
                if !self.gone {
                    self.finish()?;
                }
                let stderr = self.stderr_text();
                let mut message = error.to_string();
                if !stderr.is_empty() {
                    message.push_str("; stderr: ");
                    message.push_str(&stderr);
                }
                Err(Error::Launch {
                    session_id,
                    message,
                })
            }
        }
    }

    fn exchange(
        &mut self,
        launch: &Launch,
        cwd: &Path,
        resume: Option<String>,
        deadline: Instant,
    ) -> Result<()> {
        let initialized = self.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": { "readTextFile": false, "writeTextFile": false },
                    "terminal": false,
                },
                "clientInfo": { "name": "astrolabe", "version": env!("CARGO_PKG_VERSION") },
            }),
            deadline,
        )?;
        let version = initialized.get("protocolVersion").and_then(Value::as_u64);
        if version != Some(PROTOCOL_VERSION) {
            return Err(protocol_error(format!(
                "adapter speaks protocol {version:?}"
            )));
        }
        let mut params = json!({ "cwd": cwd.to_string_lossy(), "mcpServers": [] });
        if let Some(meta) = &launch.meta {
            params["_meta"] = meta.clone();
        }
        match resume {
            None => {
                let created = self.request("session/new", params, deadline)?;
                let id = created.get("sessionId").and_then(Value::as_str);
                let id = id.ok_or_else(|| protocol_error("no sessionId from session/new"))?;
                self.agent_session = id.to_owned();
                self.engine.set_agent_session(self.session, id)?;
            }
            Some(id) => {
                let route = resume_route(&initialized)
                    .ok_or_else(|| protocol_error("adapter offers no way to resume"))?;
                params["sessionId"] = json!(id);
                self.agent_session = id;
                self.replaying = true;
                let resumed = self.request(route, params, deadline);
                self.replaying = false;
                resumed?;
            }
        }
        Ok(())
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

    /// Sends a prompt, on the launch's waiting run or as a new turn. A turn still
    /// going is refused: say-what-instead is a reject, then this once the turn ends.
    pub fn prompt(&mut self, text: &str) -> Result<()> {
        if self.gone {
            return Err(Error::AgentGone(self.session));
        }
        let run = self.engine.inspect(self.session)?.run;
        if !run.state.is_active() || run.prompt_rpc_id.is_some() {
            self.engine.start_turn(self.session)?;
        }
        let id = self.take_id();
        self.engine.prompt_sent(self.session, &id)?;
        self.prompt_id = Some(id.clone());
        let sent = self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": {
                "sessionId": self.agent_session,
                "prompt": [{ "type": "text", "text": text }],
            },
        }));
        if let Err(error) = sent {
            self.fail_prompt(&id, &format!("prompt not delivered: {error}"))?;
            return Err(error);
        }
        Ok(())
    }

    /// Delivers the option the owner chose, then resolves the request. Choosing is
    /// the caller's job. Returns false when it was already resolved.
    pub fn answer(&mut self, request_id: &str, option_id: &str) -> Result<bool> {
        let Some((request, resolved)) = self.engine.request(request_id)? else {
            return Err(Error::UnknownRequest(request_id.to_owned()));
        };
        if resolved {
            return Ok(false);
        }
        if !request.options.iter().any(|o| o.option_id == option_id) {
            return Err(Error::UnknownOption {
                request: request_id.to_owned(),
                option: option_id.to_owned(),
            });
        }
        let outcome = json!({ "outcome": "selected", "optionId": option_id });
        self.reply(&request, outcome, option_id)
    }

    /// Asks the agent to stop. The run ends only on the agent's own evidence. Open
    /// requests are answered as cancelled, as the protocol requires.
    pub fn cancel(&mut self) -> Result<()> {
        self.engine.request_stop(self.session)?;
        self.write(&json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": self.agent_session },
        }))?;
        for id in self.live.clone() {
            if let Some((request, false)) = self.engine.request(&id)? {
                self.reply(&request, json!({ "outcome": "cancelled" }), "cancelled")?;
            }
        }
        Ok(())
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

    /// Closes the pipe, gives the adapter a moment, then ends it. Only an explicit
    /// call does this: nothing here reacts to a stall.
    pub fn shutdown(mut self) -> Result<()> {
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

    fn request(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value> {
        let id = self.take_id();
        self.awaited = Some(id.clone());
        self.response = None;
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            if let Some(response) = self.response.take() {
                if let Some(error) = response.get("error") {
                    return Err(Error::AgentRpc {
                        session_id: self.session,
                        rpc_id: id.to_string(),
                        message: error_text(error),
                    });
                }
                return Ok(response.get("result").cloned().unwrap_or(Value::Null));
            }
            if self.gone {
                return Err(Error::AgentGone(self.session));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(protocol_error(format!("no answer to {method} in time")));
            }
            self.pump(left)?;
        }
    }

    fn reply(&mut self, request: &Request, outcome: Value, resolution: &str) -> Result<bool> {
        if !self.live.contains(&request.id) {
            return Err(Error::RequestNotLive(request.id.clone()));
        }
        let raw_id: Value = serde_json::from_str(&request.rpc_id)
            .map_err(|error| protocol_error(format!("stored request id: {error}")))?;
        // Delivered first: a failed write must leave the request open, not lost.
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": raw_id,
            "result": { "outcome": outcome },
        }))?;
        self.live.remove(&request.id);
        self.engine.resolve_request(&request.id, resolution)
    }

    fn write(&mut self, message: &Value) -> Result<()> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(Error::AgentGone(self.session));
        };
        let mut line = message.to_string();
        line.push('\n');
        let written = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush());
        if let Err(error) = written {
            self.stdin = None;
            return Err(error.into());
        }
        Ok(())
    }

    fn pump(&mut self, wait: Duration) -> Result<()> {
        if self.exit_seen.is_none() && self.child.try_wait()?.is_some() {
            self.exit_seen = Some(Instant::now());
        }
        let since_exit = self.exit_seen.map(|seen| seen.elapsed());
        if since_exit.is_some_and(|since| since >= EXIT_LIMIT) {
            return self.finish();
        }
        match self.lines.recv_timeout(wait.min(TICK)) {
            Ok(line) => self.handle_line(&line),
            Err(RecvTimeoutError::Disconnected) => self.finish(),
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
            (Some("session/update"), None) if !self.replaying => {
                self.ingest(&message)?;
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
            self.response = Some(message);
            return Ok(());
        }
        let is_prompt = id.is_some() && id == self.prompt_id;
        let stop_reason = message
            .pointer("/result/stopReason")
            .and_then(Value::as_str);
        if is_prompt && stop_reason.is_none() && message.get("error").is_none() {
            // Left alone, this run would look active forever.
            let id = id.unwrap_or_default();
            self.prompt_id = None;
            return self.fail_prompt(&id, "prompt answered without a stop reason");
        }
        self.ingest(&message)?;
        if is_prompt {
            self.prompt_id = None;
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

    fn fail_prompt(&mut self, id: &Value, message: &str) -> Result<()> {
        let failure = json!({ "id": id, "error": { "message": message } });
        self.engine.ingest(self.session, &failure)?;
        self.prompt_id = None;
        self.events.push_back(ClientEvent::PromptError {
            message: message.to_owned(),
        });
        Ok(())
    }

    fn refuse(&mut self, id: Value, method: &str) {
        let refused = self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": METHOD_NOT_FOUND, "message": format!("{method} is not supported") },
        }));
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

    /// The process is over or cannot be heard any more. Closing stdin lets a healthy
    /// adapter leave by itself; one still there has lost its only channel to us.
    fn finish(&mut self) -> Result<()> {
        self.stdin = None;
        let status = self.wait_or_kill()?;
        self.gone = true;
        self.live.clear();
        self.engine.process_exited(self.session)?;
        self.events.push_back(ClientEvent::Exited {
            code: status.code(),
            stderr: self.stderr_text(),
        });
        Ok(())
    }

    fn wait_or_kill(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + CLOSE_WAIT;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        Ok(self.child.wait()?)
    }
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

fn error_text(error: &Value) -> String {
    let message = error.get("message").and_then(Value::as_str);
    message.unwrap_or("unknown error").to_owned()
}

fn protocol_error(message: impl Into<String>) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}
