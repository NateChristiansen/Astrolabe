//! The one SQLite database. Each engine operation commits as one transaction,
//! so a process that dies at any moment leaves a state reconcile can trust.
use crate::error::{Error, Result};
use crate::model::*;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 1;
const SCHEMA: &str = "
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    pid INTEGER NOT NULL,
    lifecycle TEXT NOT NULL,
    activity_ms INTEGER NOT NULL,
    snooze_until_ms INTEGER NOT NULL DEFAULT 0,
    stop_requested_ms INTEGER
);
CREATE TABLE runs (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    state TEXT NOT NULL
);
CREATE INDEX runs_by_session ON runs(session_id);
CREATE TABLE workers (
    id TEXT PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    title TEXT NOT NULL
);
CREATE TABLE tools (
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    worker_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL,
    started_ms INTEGER NOT NULL,
    fingerprint TEXT,
    PRIMARY KEY (session_id, worker_id, tool_call_id)
);
CREATE TABLE requests (
    id TEXT PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    worker_id TEXT,
    title TEXT NOT NULL,
    opened_ms INTEGER NOT NULL,
    resolved_ms INTEGER,
    resolution TEXT
);
CREATE INDEX open_requests ON requests(session_id) WHERE resolved_ms IS NULL;
CREATE TABLE cards (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    worker_id TEXT,
    tool TEXT,
    since_ms INTEGER NOT NULL,
    raised_ms INTEGER NOT NULL,
    retired_ms INTEGER
);
CREATE UNIQUE INDEX one_active_card ON cards(session_id) WHERE retired_ms IS NULL;
CREATE TABLE outcomes (
    id INTEGER PRIMARY KEY,
    session_id INTEGER NOT NULL REFERENCES sessions(id),
    run_id INTEGER NOT NULL REFERENCES runs(id),
    summary TEXT NOT NULL,
    state TEXT NOT NULL
);
";

const REQUEST_COLUMNS: &str = "id, session_id, worker_id, title, opened_ms, resolution";
const CARD_COLUMNS: &str = "id, session_id, worker_id, tool, since_ms, raised_ms";
const OUTCOME_COLUMNS: &str = "id, session_id, run_id, summary, state";

/// The per-machine database path under the OS app-data directory.
pub fn default_db_path() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library/Application Support")
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|home| home.join(".local/share")))?
    };
    Some(base.join("Astrolabe").join("astrolabe.db"))
}

pub(crate) struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => conn.execute_batch(&format!(
                "BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
            ))?,
            SCHEMA_VERSION => {}
            other => return Err(Error::UnsupportedSchema(other)),
        }
        Ok(Self { conn })
    }

    pub fn transaction<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        let transaction = self.conn.unchecked_transaction()?;
        let value = work()?;
        transaction.commit()?;
        Ok(value)
    }

    pub fn insert_session(&self, pid: u32, now: i64) -> Result<SessionId> {
        self.conn.execute(
            "INSERT INTO sessions (pid, lifecycle, activity_ms) VALUES (?1, ?2, ?3)",
            params![pid, Lifecycle::Open, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn session(&self, id: SessionId) -> Result<Session> {
        self.conn
            .query_row(
                "SELECT id, pid, lifecycle, activity_ms, snooze_until_ms, stop_requested_ms
                 FROM sessions WHERE id = ?1",
                [id],
                session_row,
            )
            .optional()?
            .ok_or(Error::UnknownSession(id))
    }

    pub fn open_sessions(&self) -> Result<Vec<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, pid, lifecycle, activity_ms, snooze_until_ms, stop_requested_ms
             FROM sessions WHERE lifecycle = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([Lifecycle::Open], session_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_lifecycle(&self, id: SessionId, lifecycle: Lifecycle) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET lifecycle = ?2 WHERE id = ?1",
            params![id, lifecycle],
        )?;
        Ok(())
    }

    pub fn set_snooze(&self, id: SessionId, until_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET snooze_until_ms = ?2 WHERE id = ?1",
            params![id, until_ms],
        )?;
        Ok(())
    }

    pub fn set_stop_requested(&self, id: SessionId, at_ms: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET stop_requested_ms = ?2 WHERE id = ?1",
            params![id, at_ms],
        )?;
        Ok(())
    }

    /// Records real activity: restarts the stall timer and clears any snooze.
    pub fn record_activity(&self, id: SessionId, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET activity_ms = ?2, snooze_until_ms = 0 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    pub fn insert_run(&self, session_id: SessionId, state: RunState) -> Result<RunId> {
        self.conn.execute(
            "INSERT INTO runs (session_id, state) VALUES (?1, ?2)",
            params![session_id, state],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn latest_run(&self, session_id: SessionId) -> Result<Run> {
        Ok(self.conn.query_row(
            "SELECT id, session_id, state FROM runs WHERE session_id = ?1
             ORDER BY id DESC LIMIT 1",
            [session_id],
            |row| {
                Ok(Run {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    state: row.get(2)?,
                })
            },
        )?)
    }

    pub fn set_run_state(&self, id: RunId, state: RunState) -> Result<()> {
        self.conn.execute(
            "UPDATE runs SET state = ?2 WHERE id = ?1",
            params![id, state],
        )?;
        Ok(())
    }

    pub fn insert_worker(&self, id: &str, session_id: SessionId, title: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO workers (id, session_id, title) VALUES (?1, ?2, ?3)",
            params![id, session_id, title],
        )?;
        Ok(())
    }

    pub fn is_worker(&self, id: &str, session_id: SessionId) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM workers WHERE id = ?1 AND session_id = ?2",
                params![id, session_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn tool(
        &self,
        session_id: SessionId,
        worker_id: Option<&str>,
        tool_call_id: &str,
    ) -> Result<Option<Tool>> {
        Ok(self
            .conn
            .query_row(
                "SELECT tool_call_id, worker_id, title, status, started_ms, fingerprint
                 FROM tools WHERE session_id = ?1 AND worker_id = ?2 AND tool_call_id = ?3",
                params![session_id, worker_id.unwrap_or(""), tool_call_id],
                tool_row,
            )
            .optional()?)
    }

    pub fn upsert_tool(&self, session_id: SessionId, tool: &Tool) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tools
             (session_id, worker_id, tool_call_id, title, status, started_ms, fingerprint)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id,
                tool.worker_id.as_deref().unwrap_or(""),
                tool.tool_call_id,
                tool.title,
                tool.status,
                tool.started_ms,
                tool.fingerprint,
            ],
        )?;
        Ok(())
    }

    /// Oldest first, so the longest-running tool leads a card.
    pub fn running_tools(&self, session_id: SessionId) -> Result<Vec<Tool>> {
        let mut statement = self.conn.prepare(
            "SELECT tool_call_id, worker_id, title, status, started_ms, fingerprint
             FROM tools WHERE session_id = ?1 AND status != ?2 ORDER BY started_ms, rowid",
        )?;
        let rows = statement.query_map(params![session_id, ToolStatus::Finished], tool_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn finish_tools(&self, session_id: SessionId) -> Result<()> {
        self.conn.execute(
            "UPDATE tools SET status = ?2 WHERE session_id = ?1",
            params![session_id, ToolStatus::Finished],
        )?;
        Ok(())
    }

    /// Time spent waiting on the owner is not time the tool ran.
    pub fn restart_tool_clocks(&self, session_id: SessionId, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tools SET started_ms = ?3 WHERE session_id = ?1 AND status != ?2",
            params![session_id, ToolStatus::Finished, now],
        )?;
        Ok(())
    }

    /// Returns none when the id already exists, so a redelivery cannot alert twice.
    pub fn insert_request(&self, request: &Request) -> Result<bool> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO requests (id, session_id, worker_id, title, opened_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request.id,
                request.session_id,
                request.worker_id,
                request.title,
                request.opened_ms,
            ],
        )?;
        Ok(inserted == 1)
    }

    pub fn request(&self, id: &str) -> Result<Option<(Request, bool)>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {REQUEST_COLUMNS}, resolved_ms IS NOT NULL FROM requests WHERE id = ?1"
                ),
                [id],
                |row| Ok((request_row(row)?, row.get(6)?)),
            )
            .optional()?)
    }

    pub fn resolve_request(&self, id: &str, resolution: &str, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE requests SET resolved_ms = ?3, resolution = ?2 WHERE id = ?1",
            params![id, resolution, now],
        )?;
        Ok(())
    }

    pub fn open_requests(&self, session_id: Option<SessionId>) -> Result<Vec<Request>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {REQUEST_COLUMNS} FROM requests
             WHERE resolved_ms IS NULL AND (?1 IS NULL OR session_id = ?1)
             ORDER BY opened_ms, id"
        ))?;
        let rows = statement.query_map([session_id], request_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Returns none when the session already has an active card.
    pub fn insert_card(
        &self,
        session_id: SessionId,
        tool: Option<&Tool>,
        since_ms: i64,
        now: i64,
    ) -> Result<Option<CardId>> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO cards (session_id, worker_id, tool, since_ms, raised_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id,
                tool.and_then(|tool| tool.worker_id.as_deref()),
                tool.map(|tool| tool.title.as_str()),
                since_ms,
                now,
            ],
        )?;
        Ok((inserted == 1).then(|| self.conn.last_insert_rowid()))
    }

    pub fn active_card(&self, session_id: SessionId) -> Result<Option<Card>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {CARD_COLUMNS} FROM cards
                     WHERE session_id = ?1 AND retired_ms IS NULL"
                ),
                [session_id],
                card_row,
            )
            .optional()?)
    }

    pub fn card_if_active(&self, id: CardId) -> Result<Option<Card>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {CARD_COLUMNS} FROM cards WHERE id = ?1 AND retired_ms IS NULL"),
                [id],
                card_row,
            )
            .optional()?)
    }

    pub fn active_cards(&self) -> Result<Vec<Card>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {CARD_COLUMNS} FROM cards WHERE retired_ms IS NULL ORDER BY id"
        ))?;
        let rows = statement.query_map([], card_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn retire_card(&self, session_id: SessionId, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET retired_ms = ?2 WHERE session_id = ?1 AND retired_ms IS NULL",
            params![session_id, now],
        )?;
        Ok(())
    }

    pub fn insert_outcome(
        &self,
        session_id: SessionId,
        run_id: RunId,
        summary: &str,
    ) -> Result<OutcomeId> {
        self.conn.execute(
            "INSERT INTO outcomes (session_id, run_id, summary, state)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, run_id, summary, OutcomeState::Submitted],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn supersede_submitted(&self, session_id: SessionId) -> Result<()> {
        self.conn.execute(
            "UPDATE outcomes SET state = ?2 WHERE session_id = ?1 AND state = ?3",
            params![
                session_id,
                OutcomeState::Superseded,
                OutcomeState::Submitted
            ],
        )?;
        Ok(())
    }

    /// Returns false unless the outcome was still submitted.
    pub fn accept_outcome(&self, id: OutcomeId) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE outcomes SET state = ?2 WHERE id = ?1 AND state = ?3",
            params![id, OutcomeState::Accepted, OutcomeState::Submitted],
        )?;
        Ok(changed == 1)
    }

    pub fn outcomes(&self, session_id: SessionId) -> Result<Vec<Outcome>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {OUTCOME_COLUMNS} FROM outcomes WHERE session_id = ?1 ORDER BY id"
        ))?;
        let rows = statement.query_map([session_id], |row| {
            Ok(Outcome {
                id: row.get(0)?,
                session_id: row.get(1)?,
                run_id: row.get(2)?,
                summary: row.get(3)?,
                state: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn session_row(row: &Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        pid: row.get(1)?,
        lifecycle: row.get(2)?,
        activity_ms: row.get(3)?,
        snooze_until_ms: row.get(4)?,
        stop_requested_ms: row.get(5)?,
    })
}

fn tool_row(row: &Row<'_>) -> rusqlite::Result<Tool> {
    let worker_id: String = row.get(1)?;
    Ok(Tool {
        tool_call_id: row.get(0)?,
        worker_id: (!worker_id.is_empty()).then_some(worker_id),
        title: row.get(2)?,
        status: row.get(3)?,
        started_ms: row.get(4)?,
        fingerprint: row.get(5)?,
    })
}

fn request_row(row: &Row<'_>) -> rusqlite::Result<Request> {
    Ok(Request {
        id: row.get(0)?,
        session_id: row.get(1)?,
        worker_id: row.get(2)?,
        title: row.get(3)?,
        opened_ms: row.get(4)?,
        resolution: row.get(5)?,
    })
}

fn card_row(row: &Row<'_>) -> rusqlite::Result<Card> {
    Ok(Card {
        id: row.get(0)?,
        session_id: row.get(1)?,
        worker_id: row.get(2)?,
        tool: row.get(3)?,
        since_ms: row.get(4)?,
        raised_ms: row.get(5)?,
    })
}
