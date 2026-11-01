//! This machine's target repos: on another machine a name maps to that one's own checkout.
use crate::error::{Error, Result};
use crate::worktree::git;
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub const AGENT: &str = "agent";
const TABLE: &str = "CREATE TABLE IF NOT EXISTS machine (key TEXT PRIMARY KEY, value TEXT)";

pub struct Registry(Connection);

impl Registry {
    pub fn open(db: &Path) -> Result<Self> {
        if let Some(parent) = db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db)?;
        conn.execute(TABLE, [])?;
        Ok(Self(conn))
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let query = "SELECT value FROM machine WHERE key = ?1";
        Ok(self
            .0
            .query_row(query, [key], |row| row.get(0))
            .optional()?)
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        let query = "INSERT OR REPLACE INTO machine (key, value) VALUES (?1, ?2)";
        self.0.execute(query, [key, value])?;
        Ok(())
    }

    pub fn add(&self, name: &str, checkout: &Path) -> Result<()> {
        let inside = ["rev-parse".as_ref(), "--git-dir".as_ref()];
        git(checkout, &inside).map_err(Error::Setup)?;
        self.set(&format!("repo.{name}"), &crate::trust::native(checkout))
    }

    pub fn checkout(&self, name: &str) -> Result<PathBuf> {
        let found = self.get(&format!("repo.{name}"))?.map(PathBuf::from);
        found.ok_or_else(|| Error::Setup(format!("no repo named {name} here")))
    }

    pub fn agent_repo(&self) -> Result<Option<PathBuf>> {
        Ok(self.get(&format!("repo.{AGENT}"))?.map(PathBuf::from))
    }

    pub fn provider_on(&self, provider: &str) -> Result<bool> {
        Ok(self.get(&format!("provider.{provider}"))?.as_deref() != Some("off"))
    }

    pub fn set_provider(&self, provider: &str, on: bool) -> Result<()> {
        let value = if on { "on" } else { "off" };
        self.set(&format!("provider.{provider}"), value)
    }
}

pub fn sync(checkout: &Path) -> std::result::Result<String, String> {
    git(checkout, &["pull".as_ref(), "--ff-only".as_ref()])
}
