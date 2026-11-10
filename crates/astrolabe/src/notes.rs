//! Writes to the notes checkout: effort files, the journal, commits and pushes. One writer per
//! machine: every mutation runs inside this process under an advisory lock in the state
//! folder, because two sessions share one Git index. The OS releases the lock when the
//! process ends; no stale timeout can evict a live writer. Each write is first planned whole as an `Op` and saved as an intent file, then
//! applied idempotently, so a crash between steps leaves an intent the next lock holder
//! finishes from what's on disk, and a retry of the same command doesn't commit twice.
use chrono::{Local, NaiveDate};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Back off after a network timeout so later hooks do not pay the same delay repeatedly.
const BACKOFF_MS: i64 = 5 * 60 * 1000;
const INTENTS: &str = "ops";
/// Touched when a detached sync is asked for; its mtime throttles the Stop hook's asks.
const ASKED: &str = "sync-asked";
const SYNC: &str = "sync.json";
const LOCK: &str = "notes.lock";
/// A finished intent or a ticket older than this no longer explains a plain command.
pub const RECENT_MS: i64 = 60_000;
/// Text checks out with LF everywhere, whatever the machine's `core.autocrlf` says.
pub const EOL_RULE: &str = "* text=auto eol=lf\n";

/// The notes checkout and the state folder its lock and intents live in.
pub struct Notes {
    pub root: PathBuf,
    pub state: PathBuf,
    changed: std::sync::atomic::AtomicBool,
}

/// Held while writing. Dropped, the lock is released; killed, the OS releases it.
pub struct Held(#[allow(dead_code)] File);

/// A planned write, applied whole and idempotently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    /// Two ordered lesson commits, recovered through the same intent as a save.
    #[serde(default)]
    pub lesson_close: Option<LessonClose>,
    /// A reply only advances the matching delivery checkpoint, preserving intervening
    /// session edits. Persist the expected time so recovery follows the same rule.
    #[serde(default)]
    pub reply_after_ms: Option<i64>,
    /// Random, minted by the hook; reused for a retry of the same tool call.
    pub key: String,
    /// Relative path and full text, replaced whole.
    pub files: Vec<(String, String)>,
    /// Old path to remove, new path to write: a renumber.
    pub renames: Vec<(String, String)>,
    pub journal: Vec<JournalLine>,
    pub message: String,
    /// The effort file's hash as delivered to the session, for the stale-save check at sync.
    pub base: Option<String>,
    /// Session state files to write once the notes are committed.
    #[serde(default)]
    pub states: Vec<(PathBuf, Value)>,
    /// Set once the notes commit landed, so a rerun writes no file again: a newer save may
    /// have landed since.
    #[serde(default)]
    pub committed: bool,
    /// The command this op answers, so the plain command can find it.
    #[serde(default)]
    pub digest: Option<String>,
    /// What the hook showed, kept until the ticket exists, then the intent goes.
    #[serde(default)]
    pub result: Option<String>,
    /// The folder the command ran in, with the digest.
    #[serde(default)]
    pub cwd: Option<String>,
    /// The notes checkout itself, when the command ran there: the states' nudge baseline is
    /// taken again as they're written, after the commit, since that commit is in the folder
    /// the fingerprint reads. Kept in the intent so a replay does the same.
    #[serde(default)]
    pub rebaseline: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonClose {
    pub path: String,
    pub original: String,
    pub disposed: String,
}

/// Why the notes lock wasn't taken.
enum Locked {
    /// Another process held it for all the time this hook could wait.
    Busy(std::time::Duration),
    /// The lock file couldn't be opened or locked at all.
    Failed(String),
}

/// A state of the notes checkout that a pull or a write must leave alone.
pub(crate) enum Unfit {
    MidRebase,
    Detached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pull {
    Pulled,
    Offline(String),
    Diverged(String),
}

mod facts;
mod journal;
mod process;
pub(crate) use process::keep_stdio_here;
mod storage;
mod sync;
pub use facts::{Looked, RepoFacts};
pub use journal::{JournalEntry, JournalLine, parse_journal, tidy_journal};

impl Notes {
    pub fn new(root: PathBuf, state: PathBuf) -> Self {
        Self {
            root,
            state,
            changed: Default::default(),
        }
    }

    /// Notes or synchronization status changed. The caller decides when to refresh views.
    pub(crate) fn take_changes(&self) -> bool {
        self.changed
            .swap(false, std::sync::atomic::Ordering::Relaxed)
    }

    fn changed(&self) {
        self.changed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Waits for the machine's one notes writer lock: as long as it takes outside a hook,
    /// and inside one for the share of its time a hook can wait (see `deadline`), after
    /// which the write is refused with nothing touched.
    pub fn lock(&self) -> Result<Held, String> {
        self.take_lock().map_err(|locked| match locked {
            Locked::Busy(waited) => format!(
                "the notes are busy: another session's write or a sync held them for the {} s this hook could wait, so nothing was written. Run the same command again",
                crate::deadline::seconds(waited)
            ),
            Locked::Failed(why) => why,
        })
    }

    fn take_lock(&self) -> Result<Held, Locked> {
        std::fs::create_dir_all(&self.state)
            .map_err(|e| Locked::Failed(format!("{}: {e}", self.state.display())))?;
        let path = self.state.join(LOCK);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| Locked::Failed(format!("{}: {e}", path.display())))?;
        match crate::deadline::lock(&file) {
            Ok(crate::deadline::Wait::Held) => Ok(Held(file)),
            Ok(crate::deadline::Wait::Busy(waited)) => Err(Locked::Busy(waited)),
            Err(e) => Err(Locked::Failed(format!("notes lock: {e}"))),
        }
    }

    /// The writer lock if it's free now; None while a writer holds it, so a background
    /// caller skips a turn instead of queueing behind a save.
    pub fn try_lock(&self) -> Option<Held> {
        std::fs::create_dir_all(&self.state).ok()?;
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.state.join(LOCK))
            .ok()?;
        FileExt::try_lock_exclusive(&file)
            .ok()?
            .then_some(Held(file))
    }

    /// The folder git keeps for a rebase under way in the checkout, if there is one; an
    /// error when that couldn't be found out.
    pub(crate) fn rebase_dir(&self) -> Result<Option<PathBuf>, String> {
        let dot = self.root.join(".git");
        let dir = match dot.is_dir() {
            true => dot,
            // A linked worktree keeps its own elsewhere.
            false => PathBuf::from(self.git(&["rev-parse", "--absolute-git-dir"])?.trim()),
        };
        Ok(["rebase-merge", "rebase-apply"]
            .map(|name| dir.join(name))
            .into_iter()
            .find(|folder| folder.exists()))
    }

    /// A git call in the notes checkout, as part of a pull or a write. A hook never ends
    /// one: whether there was time for its step was decided before the step began.
    pub fn git(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
        crate::worktree::git_as(crate::deadline::Git::Notes, &self.root, &args)
    }

    /// Why the checkout can't take a pull or a write as it stands, if it can't: it's
    /// partway through a rebase, or off its branch. Either is the owner's to put right,
    /// so nothing is fetched, rebased or written over it. An error when that couldn't be
    /// found out.
    pub(crate) fn unfit(&self) -> Result<Option<Unfit>, String> {
        if self.rebase_dir()?.is_some() {
            return Ok(Some(Unfit::MidRebase));
        }
        match self.git(&["symbolic-ref", "--quiet", "HEAD"]) {
            Ok(_) => Ok(None),
            // Git says a detached HEAD by failing with nothing to say.
            Err(why) if why.is_empty() => Ok(Some(Unfit::Detached)),
            Err(why) => Err(why),
        }
    }

    /// What a write must not land on, checked before anything is planned or touched,
    /// and the only place a write is refused for time: past here it runs to its end. A
    /// hook with too little time left. A checkout mid-rebase or off its branch: a commit
    /// there is dropped when the rebase ends or the branch is checked out again. Files
    /// the index holds unmerged: a save can't stage those. This program only refuses: a
    /// rebase it finds is the owner's to end.
    pub(crate) fn refuse_unmerged(&self) -> Result<(), String> {
        if let Some(why) = crate::deadline::no_time_to_write() {
            return Err(why);
        }
        // A check that couldn't be run refuses too: nothing is written on a guess.
        let unchecked = |why: String| {
            format!(
                "the notes checkout couldn't be checked before writing ({why}). Nothing was written"
            )
        };
        match self.unfit() {
            Ok(None) => {}
            Ok(Some(Unfit::MidRebase)) => {
                return Err("the notes checkout is in the middle of a rebase; finish it there, or end it with `git rebase --abort`, first. Nothing was written".to_owned());
            }
            Ok(Some(Unfit::Detached)) => {
                return Err("the notes checkout isn't on a branch (its HEAD is detached); check its branch out there first. Nothing was written".to_owned());
            }
            Err(why) => return Err(unchecked(why)),
        }
        let unmerged = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .map_err(unchecked)?;
        if unmerged.trim().is_empty() {
            crate::deadline::writing();
            return Ok(());
        }
        Err(format!(
            "the notes checkout has unmerged files ({}); resolve them there first. Nothing was written",
            unmerged.trim().replace('\n', ", ")
        ))
    }

    /// The highest effort number on disk plus one.
    pub fn next_number(&self) -> u32 {
        self.numbers()
            .into_iter()
            .map(|(n, _)| n)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Every effort file as number and path from the checkout root, wherever it sits under
    /// `efforts/`: the month shelf of its creation, or the root for one from before shelves.
    pub fn numbers(&self) -> Vec<(u32, String)> {
        effort_files(&self.root)
    }

    /// Two files with one number, after a pull: the newer (by its `created` property, then
    /// name) is renumbered to the next free number. The plan for it, or None when clean.
    pub fn duplicate(&self) -> Option<(String, String, u32)> {
        let numbers = self.numbers();
        let dup = numbers.windows(2).find(|w| w[0].0 == w[1].0)?;
        let created = |rel: &str| {
            std::fs::read_to_string(self.root.join(rel))
                .ok()
                .and_then(|t| crate::effort::EffortFile::parse(&t).ok())
                .and_then(|f| f.prop("created").map(str::to_owned))
                .unwrap_or_default()
        };
        let (a, b) = (&dup[0].1, &dup[1].1);
        let newer = if created(a) > created(b) { a } else { b };
        let next = numbers.last().map_or(1, |(n, _)| n + 1);
        let (dir, name) = newer.rsplit_once('/').unwrap_or(("efforts", newer));
        let new_rel = format!(
            "{dir}/{next:04}-{}",
            name.split_once('-').map_or("x.md", |x| x.1)
        );
        Some((newer.clone(), new_rel, next))
    }

    /// Two efforts with one number after a pull: the newer is renumbered to the next free
    /// number, its title and any session bound to it follow, and a journal line says so, so
    /// earlier lines still resolve. Offline creation on two machines is why this can happen.
    pub fn fix_duplicate(&self) -> Option<String> {
        let (old_rel, new_rel, next) = self.duplicate()?;
        let text = std::fs::read_to_string(self.root.join(&old_rel)).ok()?;
        let mut file = crate::effort::EffortFile::parse(&text).ok()?;
        // A note whose header never closes is all header: it parses with no title or
        // sections, and writing it back from that would drop them. It moves as it is, its
        // number taken from its file name; its title keeps the old number and it gets no
        // decision line, since either would mean rewriting it.
        let unclosed = crate::effort::header_bounds(&text).is_some_and(|at| !at.closed());
        let old_number = if unclosed {
            let name = old_rel.rsplit('/').next().unwrap_or(&old_rel);
            name.split('-').next().and_then(|n| n.parse().ok())
        } else {
            file.number()
        }
        .unwrap_or(0);
        let rendered = if unclosed {
            text
        } else {
            file.title = format!("{next:04} {}", file.name());
            let (day, _) = local_day_time();
            file.add_decision(&format!(
                "{day}: renumbered from {old_number:04}; that number was taken by another machine's effort first"
            ));
            file.render()
        };
        let key = format!("renumber-{old_number:04}-{next:04}");
        let mut files = vec![(new_rel.clone(), rendered)];
        // The renumbered effort's own journal lines follow it: the ones written by the
        // machine that saved it, which the tail names. The other machine's lines for that
        // number belong to the effort that keeps it.
        let writer = file
            .prop("saved_by")
            .and_then(|s| s.split(" · ").next())
            .unwrap_or_default()
            .to_owned();
        let (from, to) = (
            format!("<!-- {old_number:04} {writer} "),
            format!("<!-- {next:04} {writer} "),
        );
        for rel in md_files(&self.root, "journal") {
            let Ok(text) = std::fs::read_to_string(self.root.join(&rel)) else {
                continue;
            };
            if text.contains(&from) {
                files.push((rel, text.replace(&from, &to)));
            }
        }
        let mut op = Op {
            reply_after_ms: None,
            lesson_close: None,
            key: key.clone(),
            files,
            renames: vec![(old_rel.clone(), new_rel.clone())],
            journal: Vec::new(),
            message: format!("{old_number:04} renumbered to {next:04}"),
            base: None,
            states: Vec::new(),
            committed: false,
            digest: None,
            result: None,
            cwd: None,
            rebaseline: None,
        };
        // Every session bound to the old path follows, inside the same recoverable op.
        for entry in std::fs::read_dir(&self.state)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(mut state) = serde_json::from_slice::<crate::session::SessionState>(&bytes)
            else {
                continue;
            };
            if state.file.as_deref() == Some(old_rel.as_str()) {
                state.file.set(new_rel.clone());
                state.effort.set(next.to_string());
                if let crate::session::Field::Known(crate::session::Binding::Effort {
                    key, ..
                }) = &mut state.binding
                {
                    *key = next.to_string();
                }
                op.states.push((path, state.to_value()));
            }
        }
        if self.apply(&op).is_err() {
            return None;
        }
        self.forget(&key);
        Some(format!("{old_number:04} renumbered to {next:04}"))
    }

    /// Lock, finish leftovers, pull: what a pickup does before reading.
    pub fn refresh(&self) -> (Vec<String>, Pull) {
        match self.take_lock() {
            Ok(_held) => {
                let leftovers = self.reconcile();
                if !leftovers.is_empty() {
                    // Nothing moves under an unfinished write.
                    return (leftovers, Pull::Offline("unfinished writes".to_owned()));
                }
                (leftovers, self.pull())
            }
            // Another session writing isn't a write of this one's that failed: the notes
            // just weren't refreshed this time.
            Err(Locked::Busy(_)) => (
                Vec::new(),
                Pull::Offline("another session held the notes".to_owned()),
            ),
            Err(Locked::Failed(why)) => (vec![why], Pull::Offline("no lock".to_owned())),
        }
    }
}

pub fn hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text.as_bytes())
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A note's text hashed with its line endings as LF, for every delivered, base and on-disk
/// comparison: git on Windows checks text out with CRLF, which changes the bytes, not the note.
pub fn content_hash(text: &str) -> String {
    hash(&text.replace("\r\n", "\n"))
}

/// The short name of this machine, as the stamp shows it.
pub fn machine_name() -> String {
    let from_env = std::env::var("ASTROLABE_MACHINE").ok();
    let from_host = || {
        Command::new("hostname")
            .stdin(Stdio::null())
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let name = from_env
        .or_else(from_host)
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "unknown".to_owned());
    let short = name
        .trim()
        .split('.')
        .next()
        .unwrap_or("unknown")
        .to_ascii_lowercase();
    if short.is_empty() {
        "unknown".to_owned()
    } else {
        short
    }
}

/// Local day and time, in the owner's zone: a UTC date once fell on the next day.
/// Where an effort created on `day` (`2026-11-05`) is filed: `efforts/2026/11-November`.
/// The shelf is the month it started, which never changes, so the file never moves and
/// every link to it holds. A day that doesn't parse files at the root.
pub fn effort_dir(day: &str) -> String {
    match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        Ok(d) => format!("efforts/{}", d.format("%Y/%m-%B")),
        Err(_) => "efforts".to_owned(),
    }
}

/// The journal's day file: `journal/2026/11-November/05-Thursday.md`.
pub fn journal_file(day: &str) -> String {
    match NaiveDate::parse_from_str(day, "%Y-%m-%d") {
        Ok(d) => format!("journal/{}.md", d.format("%Y/%m-%B/%d-%A")),
        Err(_) => format!("journal/{day}.md"),
    }
}

/// Every `.md` under `top` in the checkout, as paths from the root with `/` between parts,
/// sorted. Folders starting with a dot are skipped, and the walk stops four levels down,
/// because the shelves are two deep and anything deeper isn't the plugin's.
pub fn md_files(root: &Path, top: &str) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, depth: u8, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let here = format!("{rel}/{name}");
            if path.is_dir() {
                if depth < 4 && !name.starts_with('.') {
                    walk(&path, &here, depth + 1, out);
                }
            } else if name.ends_with(".md") {
                out.push(here);
            }
        }
    }
    let mut out = Vec::new();
    walk(&root.join(top), top, 0, &mut out);
    out.sort();
    out
}

/// Every effort file as number and path from the root, in number order.
pub fn effort_files(root: &Path) -> Vec<(u32, String)> {
    let mut found: Vec<(u32, String)> = md_files(root, "efforts")
        .into_iter()
        .filter_map(|rel| {
            let name = rel.rsplit('/').next()?;
            let n = name.split('-').next()?.parse::<u32>().ok()?;
            Some((n, rel))
        })
        .collect();
    found.sort();
    found
}

pub fn local_day_time() -> (String, String) {
    let now = Local::now();
    (
        now.format("%Y-%m-%d").to_string(),
        now.format("%H:%M").to_string(),
    )
}

pub fn local_stamp() -> String {
    Local::now().format("%Y-%m-%dT%H:%M%:z").to_string()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Whole or not at all: written aside, then renamed.
pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(fail)?;
    let aside = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&aside, text).map_err(fail)?;
    std::fs::rename(&aside, path).map_err(fail)
}
