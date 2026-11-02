//! The session host: one owner thread holds the engine and every live adapter, takes what
//! the app asks over the bridge, and tells every window what changed. Preparing a launch
//! can take many seconds, so it runs on its own thread; nothing else here waits for long.
use crate::acp::{AcpClient, ClientEvent};
use crate::bridge::Inbound;
use crate::engine::Engine;
use crate::error::{Error, Result};
use crate::host::Clock;
use crate::launch::{DEFAULT_DIALS, Machine, Prepared, Provider, Say};
use crate::model::{Lifecycle, ReconcileReport, SessionDisplay};
use crate::registry::Registry;
use crate::worktree::{self, Closed, Worktree};
use protocol::*;
use rusqlite::{Connection, params};
use serde_json::{Value, from_value, to_value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_millis(20);
const ENDED: &str = "That session has ended, so nothing can reach it.";
/// Sessions and transcripts live in the core's one database, beside the rows they extend.
const TABLES: &str = "
CREATE TABLE IF NOT EXISTS shell_sessions (id INTEGER PRIMARY KEY, repo TEXT NOT NULL,
    provider TEXT NOT NULL, session_id INTEGER, why TEXT);
CREATE TABLE IF NOT EXISTS transcript (seq INTEGER PRIMARY KEY, shell_session INTEGER NOT NULL,
    role TEXT NOT NULL, text TEXT NOT NULL, tool TEXT, status TEXT);
CREATE INDEX IF NOT EXISTS transcript_by_session ON transcript(shell_session, seq);
";

enum Done {
    Prepared(i64, Box<Result<Prepared>>),
    Said(i64, String),
}

pub type Prepare = Arc<dyn Fn(&str, Provider, &str, Say) -> Result<Prepared> + Send + Sync>;

/// The real launch contract. Tests use `serve_with` to put a recording in its place; the
/// binary has no route that skips it.
pub fn real_prepare(data_dir: PathBuf) -> Prepare {
    Arc::new(move |repo, provider, session, say| {
        let machine = Machine::at(data_dir.clone())?;
        let slug = provider.slug();
        let none = || Error::Setup(format!("no {slug} client found on this machine"));
        let client = &crate::doctor::find_client(provider).ok_or_else(none)?;
        let dials = DEFAULT_DIALS;
        let request = crate::launch::Request {
            repo,
            provider,
            session,
            dials,
            client,
        };
        crate::launch::prepare(&machine, &request, say)
    })
}

struct Live<'e> {
    client: AcpClient<'e>,
    id: i64,
    worktree: Option<Worktree>,
    /// Said with rejects: all go as the next prompt once the turn has ended.
    words: Vec<String>,
}

pub struct Host<'e> {
    engine: &'e Engine,
    registry: Registry,
    book: Connection,
    reports: PathBuf,
    prepare: Prepare,
    prepared: (Sender<Done>, Receiver<Done>),
    clients: Vec<Sender<String>>,
    live: Vec<Live<'e>>,
    last_state: String,
    interrupted: usize,
}

fn item(role: TranscriptRole, text: &str) -> TranscriptItem {
    let (seq, text, tool, status) = (0, text.to_owned(), None, None);
    TranscriptItem {
        seq,
        role,
        text,
        tool,
        status,
    }
}

fn word<T: serde::Serialize>(value: T) -> String {
    let word = to_value(value)
        .ok()
        .and_then(|word| word.as_str().map(str::to_owned));
    word.unwrap_or_default()
}

impl<'e> Host<'e> {
    pub fn new(engine: &'e Engine, db: &Path, prepare: Prepare) -> Result<Self> {
        let book = Connection::open(db)?;
        book.execute_batch(TABLES)?;
        // Its preparing thread died with the last core, so nothing will ever finish it.
        book.execute(
            "UPDATE shell_sessions SET why = ?1 WHERE session_id IS NULL AND why IS NULL",
            ["The core stopped before this session finished starting. Nothing was launched."],
        )?;
        Ok(Self {
            engine,
            registry: Registry::open(db)?,
            book,
            reports: crate::hook::reports_dir_in(db.parent().unwrap_or(Path::new("."))),
            prepare,
            prepared: channel(),
            clients: Vec::new(),
            live: Vec::new(),
            last_state: String::new(),
            interrupted: 0,
        })
    }

    pub fn reconciled(&mut self, report: &ReconcileReport) {
        self.interrupted += report.interrupted.len();
    }

    pub fn run(&mut self, inbound: &Receiver<Inbound>, tick: Duration) -> Result<()> {
        let mut ticked: Option<Instant> = None;
        loop {
            match inbound.recv_timeout(WAIT) {
                Ok(Inbound::Connected(client)) => {
                    self.last_state.clear();
                    self.clients.push(client);
                }
                Ok(Inbound::Request(request)) => {
                    if let Err(why) = self.handle(request) {
                        self.broadcast(&BridgeEvent::Problem {
                            message: why.to_string(),
                        });
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
            while let Ok(done) = self.prepared.1.try_recv() {
                match done {
                    Done::Prepared(id, prepared) => self.launch(id, *prepared)?,
                    Done::Said(id, line) => self.add(id, item(TranscriptRole::Note, &line))?,
                }
            }
            self.read_agents()?;
            if ticked.is_none_or(|at| at.elapsed() >= tick) {
                ticked = Some(Instant::now());
                self.engine.ingest_reports(&self.reports)?;
                self.engine.tick()?;
                let report = self.engine.reconcile()?;
                self.reconciled(&report);
            }
            let state = self.state()?;
            let text = serde_json::to_string(&state).unwrap_or_default();
            if text != self.last_state {
                self.broadcast(&BridgeEvent::State { state });
                self.last_state = text;
            }
        }
    }

    fn broadcast(&mut self, event: &BridgeEvent) {
        let text = serde_json::to_string(event).unwrap_or_default();
        self.clients
            .retain(|client| client.send(text.clone()).is_ok());
    }

    fn live_where(&self, test: impl Fn(&Live) -> bool) -> Result<usize> {
        let found = self.live.iter().position(test);
        found.ok_or_else(|| Error::Setup(ENDED.to_owned()))
    }

    fn handle(&mut self, request: BridgeRequest) -> Result<()> {
        match request {
            BridgeRequest::Auth { .. } => {}
            BridgeRequest::Start { repo, provider } => {
                let insert = "INSERT INTO shell_sessions (repo, provider) VALUES (?1, ?2)";
                self.book
                    .execute(insert, [repo.as_str(), &word(provider)])?;
                let id = self.book.last_insert_rowid();
                let name = format!("chat-{}-{id}", crate::host::SystemClock.now_ms());
                let codex = provider == BridgeProvider::Codex;
                let provider = if codex {
                    Provider::Codex
                } else {
                    Provider::Claude
                };
                let (prepare, done) = (self.prepare.clone(), self.prepared.0.clone());
                std::thread::spawn(move || {
                    let said = done.clone();
                    let mut say = |line| drop(said.send(Done::Said(id, line)));
                    let prepared = prepare(&repo, provider, &name, &mut say);
                    let _ = done.send(Done::Prepared(id, Box::new(prepared)));
                });
            }
            BridgeRequest::Open { session } => {
                let items = self.items(session)?;
                self.broadcast(&BridgeEvent::Transcript { session, items });
            }
            BridgeRequest::Prompt { session, text } => {
                let index = self.live_where(|live| live.id == session)?;
                self.send_prompt(index, &text)?;
            }
            BridgeRequest::Answer { request, choice } => self.answer(&request, choice)?,
            BridgeRequest::Dismiss { request } => self
                .engine
                .resolve_request(&request, "dismissed")
                .map(drop)?,
            BridgeRequest::KeepWaiting { card } => self.engine.keep_waiting(card).map(drop)?,
            BridgeRequest::Stop { session } => {
                let index = self.live_where(|live| live.id == session)?;
                let live = self.live.remove(index);
                self.engine.request_stop(live.client.session_id())?;
                let own = live.client.agent_session_id().to_owned();
                for event in live.client.stop()? {
                    if let ClientEvent::Update(update) = event
                        && let Some(new) = item_from(&update, &own)
                    {
                        self.add(session, new)?;
                    }
                }
                self.close_worktree(session, live.worktree.as_ref())?;
                self.add(
                    session,
                    item(TranscriptRole::Note, "You stopped this session."),
                )?;
            }
        }
        Ok(())
    }

    fn launch(&mut self, id: i64, prepared: Result<Prepared>) -> Result<()> {
        let launched = prepared.and_then(|ready| {
            let client = AcpClient::launch(self.engine, &ready.launch, &ready.cwd);
            if let (Err(_), Some(made)) = (&client, &ready.worktree) {
                let _ = worktree::close_if_nothing_lost(made);
            }
            Ok((client?, ready.worktree))
        });
        match launched {
            Ok((client, worktree)) => {
                let query = "UPDATE shell_sessions SET session_id = ?2 WHERE id = ?1";
                self.book.execute(query, params![id, client.session_id()])?;
                self.live.push(Live {
                    client,
                    id,
                    worktree,
                    words: Vec::new(),
                });
                Ok(())
            }
            Err(why) => self.fail(id, &why.to_string()),
        }
    }

    fn fail(&mut self, id: i64, why: &str) -> Result<()> {
        let query = "UPDATE shell_sessions SET why = ?2 WHERE id = ?1";
        self.book.execute(query, params![id, why])?;
        self.add(
            id,
            item(TranscriptRole::Note, &format!("Could not start: {why}")),
        )
    }

    fn send_prompt(&mut self, index: usize, text: &str) -> Result<()> {
        self.live[index].client.prompt(text)?;
        self.add(self.live[index].id, item(TranscriptRole::You, text))
    }

    /// Only allow-once or reject: an owner's click never widens into an "always".
    fn answer(&mut self, request_id: &str, choice: BridgeChoice) -> Result<()> {
        let Some((request, resolved)) = self.engine.request(request_id)? else {
            return Err(Error::UnknownRequest(request_id.to_owned()));
        };
        if resolved {
            return Ok(());
        }
        let index = self.live_where(|live| live.client.session_id() == request.session_id)?;
        let option = match choice {
            BridgeChoice::Allow => {
                let once = request.options.iter().find(|o| o.kind == "allow_once");
                once.map(|option| option.option_id.as_str())
            }
            _ => {
                let once = request.options.iter().find(|o| o.kind == "reject_once");
                once.map(|option| option.option_id.as_str())
            }
        };
        let none = || Error::Setup("the agent did not offer that answer".to_owned());
        let answered = self.live[index]
            .client
            .answer(request_id, option.ok_or_else(none)?)?;
        if let (true, BridgeChoice::RejectAndSay { words }) = (answered, choice) {
            self.live[index].words.push(words);
        }
        Ok(())
    }

    fn read_agents(&mut self) -> Result<()> {
        let mut index = 0;
        while index < self.live.len() {
            let Some(event) = self.live[index].client.next_event(Duration::ZERO)? else {
                index += 1;
                continue;
            };
            let gone = matches!(event, ClientEvent::Exited { .. });
            let id = self.live[index].id;
            // One session's trouble is that session's: every other one stays up.
            if let Err(why) = self.on_event(index, event) {
                self.add(
                    id,
                    item(TranscriptRole::Note, &format!("Trouble here: {why}")),
                )?;
            }
            if gone {
                let live = self.live.remove(index);
                self.close_worktree(id, live.worktree.as_ref())?;
            }
        }
        Ok(())
    }

    fn on_event(&mut self, index: usize, event: ClientEvent) -> Result<()> {
        let id = self.live[index].id;
        let (note, words_lost) = match event {
            ClientEvent::Update(update) => {
                let own = self.live[index].client.agent_session_id();
                return item_from(&update, own).map_or(Ok(()), |new| self.add(id, new));
            }
            ClientEvent::TurnEnded { .. } => {
                let words = std::mem::take(&mut self.live[index].words).join("\n\n");
                if words.is_empty() {
                    return Ok(());
                }
                return match self.send_prompt(index, &words) {
                    Ok(()) => Ok(()),
                    Err(why) => {
                        let note = format!("Could not send: {why}. Not sent: {words}");
                        self.add(id, item(TranscriptRole::Note, &note))
                    }
                };
            }
            ClientEvent::LaunchFailed { message } => return self.fail(id, &message),
            ClientEvent::PromptError { message } => (format!("The turn failed: {message}."), true),
            ClientEvent::Exited { .. } => ("The agent process ended.".to_owned(), true),
            ClientEvent::AgentError { message, .. } | ClientEvent::WriteFailed { message } => {
                (format!("The agent reported: {message}"), false)
            }
            _ => return Ok(()),
        };
        let mut lost = String::new();
        if words_lost && !self.live[index].words.is_empty() {
            let words = std::mem::take(&mut self.live[index].words).join("\n\n");
            lost = format!(" Not sent: {words}");
        }
        self.add(id, item(TranscriptRole::Note, &format!("{note}{lost}")))
    }

    /// Closes it only when that loses nothing. Otherwise the transcript says where and why.
    fn close_worktree(&mut self, id: i64, made: Option<&Worktree>) -> Result<()> {
        let Some(made) = made else { return Ok(()) };
        let why = match worktree::close_if_nothing_lost(made) {
            Ok(Closed::KeptDirty(why)) => why,
            Ok(Closed::Removed { .. }) => return Ok(()),
            Err(why) => why.to_string(),
        };
        let note = format!("Kept the worktree at {}: {why}", made.path.display());
        self.add(id, item(TranscriptRole::Note, &note))
    }

    fn add(&mut self, session: i64, mut item: TranscriptItem) -> Result<()> {
        self.book.execute(
            "INSERT INTO transcript (shell_session, role, text, tool, status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session, word(item.role), item.text, item.tool, item.status],
        )?;
        item.seq = self.book.last_insert_rowid();
        self.broadcast(&BridgeEvent::Item { session, item });
        Ok(())
    }

    fn items(&self, session: i64) -> Result<Vec<TranscriptItem>> {
        let mut query = self.book.prepare(
            "SELECT seq, role, text, tool, status FROM transcript
             WHERE shell_session = ?1 ORDER BY seq",
        )?;
        let rows = query.query_map([session], |row| {
            Ok(TranscriptItem {
                seq: row.get(0)?,
                role: from_value(row.get::<_, String>(1)?.into()).unwrap_or(TranscriptRole::Note),
                text: row.get(2)?,
                tool: row.get(3)?,
                status: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn state(&self) -> Result<BridgeState> {
        let slugs = Provider::ALL.map(Provider::slug);
        let on = slugs
            .iter()
            .filter(|slug| self.registry.provider_on(slug).unwrap_or(false));
        let restart = (self.interrupted > 0).then(|| {
            let gone = self.interrupted;
            let open = self.engine.open_requests().map_or(0, |open| open.len());
            format!(
                "Since the core restarted: {gone} session(s) marked interrupted because their process had gone, {open} request(s) still open."
            )
        });
        Ok(BridgeState {
            repos: self.registry.repos()?,
            providers: on
                .filter_map(|slug| from_value(Value::from(*slug)).ok())
                .collect(),
            sessions: self.rows()?,
            plain: self.plain()?,
            restart,
        })
    }

    fn rows(&self) -> Result<Vec<SessionRow>> {
        let mut query = self.book.prepare(
            "SELECT id, repo, provider, why, session_id, (SELECT text FROM transcript t
                 WHERE t.shell_session = s.id AND t.role = 'you' ORDER BY seq LIMIT 1)
             FROM shell_sessions s ORDER BY id DESC LIMIT 50",
        )?;
        let mapped = query.query_map([], |row| {
            let provider = from_value(row.get::<_, String>(2)?.into());
            let shown = SessionRow {
                id: row.get(0)?,
                repo: row.get(1)?,
                provider: provider.unwrap_or(BridgeProvider::Claude),
                title: row.get(5)?,
                state: SessionState::Starting,
                why: row.get(3)?,
                can_prompt: false,
                can_stop: false,
                requests: Vec::new(),
                cards: Vec::new(),
            };
            Ok((shown, row.get::<_, Option<i64>>(4)?))
        })?;
        let mut rows = Vec::new();
        for found in mapped {
            let (mut row, session) = found?;
            let failed = row.why.is_some();
            if let Some(session) = session {
                self.fill(&mut row, session)?;
            }
            if failed {
                row.state = SessionState::CouldNotStart;
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn fill(&self, row: &mut SessionRow, session: i64) -> Result<()> {
        let view = self.engine.inspect(session)?;
        let live = self.live.iter().find(|live| live.id == row.id);
        row.state = match view.display {
            SessionDisplay::Working if !view.session.ready => SessionState::Starting,
            SessionDisplay::Working => SessionState::Working,
            SessionDisplay::NeedsYou => SessionState::NeedsYou,
            SessionDisplay::YourTurn => SessionState::YourTurn,
            SessionDisplay::Ended => SessionState::Ended,
            SessionDisplay::Failed => SessionState::Failed,
            SessionDisplay::Interrupted => SessionState::Interrupted,
        };
        row.can_stop = live.is_some();
        row.can_prompt = live.is_some() && view.display == SessionDisplay::YourTurn;
        let closed = "This session's process ended. No answer can reach it.";
        let lost = "The core restarted and this session's process had gone, so it is marked interrupted. No answer can reach it.";
        let still = "This session is still running from before the core restarted, and the core has no line to it. No answer can reach it.";
        let decided = match (view.session.lifecycle, live) {
            (Lifecycle::Open, Some(_)) => None,
            (Lifecycle::Closed, _) => Some(closed),
            (Lifecycle::Interrupted, _) => Some(lost),
            (Lifecycle::Open, None) => {
                // A recorded pid can pass to another program, so this core stops nothing.
                row.state = SessionState::StillRunning;
                row.why = Some(format!(
                    "Still running from before the core restarted. This app can't talk to it or stop it. To end it, check that process {} is its node adapter, then end that process. The row updates when it has gone.",
                    view.session.pid
                ));
                Some(still)
            }
        };
        let detail = |rpc_id: &str| {
            let asked = live?.client.requested_input(rpc_id)?;
            let command = asked.get("command").and_then(Value::as_str);
            Some(command.map_or_else(|| asked.to_string(), str::to_owned))
        };
        for request in &view.open_requests {
            row.requests.push(ApprovalRow {
                id: request.id.clone(),
                asking: request.title.clone(),
                detail: detail(&request.rpc_id),
                live: decided.is_none(),
                decided: decided.map(str::to_owned),
            });
        }
        let now = self.engine.clock.now_ms();
        for card in &view.cards {
            row.cards.push(StallRow {
                id: card.id,
                summary: card.summary(now),
            });
        }
        Ok(())
    }

    fn plain(&self) -> Result<Vec<PlainRow>> {
        let mut query = self.book.prepare(
            "SELECT cwd, MAX(at_ms) FROM hook_reports
             WHERE event = 'session-start' AND session_id IS NOT NULL AND session_id != ?1
             AND session_id NOT IN
                 (SELECT agent_session_id FROM sessions WHERE agent_session_id IS NOT NULL)
             GROUP BY session_id ORDER BY 2 DESC LIMIT 20",
        )?;
        let rows = query.query_map([crate::hook::SELF_TEST_SESSION], |row| {
            Ok(PlainRow {
                folder: row.get(0)?,
                at_ms: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

/// Message text and one line per tool. A child's text and thoughts stay out of a plain chat.
fn item_from(message: &Value, own: &str) -> Option<TranscriptItem> {
    let params = message.get("params")?;
    let speaker = params.get("sessionId").and_then(Value::as_str)?;
    let update = params.get("update")?;
    match update.get("sessionUpdate").and_then(Value::as_str)? {
        "agent_message_chunk" if speaker == own => {
            let text = update.pointer("/content/text").and_then(Value::as_str)?;
            Some(item(TranscriptRole::Agent, text))
        }
        "tool_call" | "tool_call_update" => {
            let field = |name| update.get(name).and_then(Value::as_str).map(str::to_owned);
            let (title, status) = (field("title"), field("status"));
            let mut line = item(TranscriptRole::Tool, &title.unwrap_or_default());
            line.tool = Some(format!("{speaker}/{}", field("toolCallId")?));
            line.status = status;
            Some(line)
        }
        _ => None,
    }
}
