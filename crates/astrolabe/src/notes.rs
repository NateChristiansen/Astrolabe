//! Writes to the notes checkout: effort files, the journal, commits and pushes. One writer per
//! machine: every mutation runs inside this process under an advisory lock in the state
//! folder, because two sessions share one git index and the audit's probe lost half its writes
//! without one. The lock dies with the process, so there is no stale timeout to evict a live
//! writer. Each write is first planned whole as an `Op` and saved as an intent file, then
//! applied idempotently, so a crash between steps leaves an intent the next lock holder
//! finishes from what's on disk, and a retry of the same command doesn't commit twice.
use chrono::Local;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a network git call may take before it counts as offline, so a save never hangs
/// on a bad connection.
const NETWORK_SECS: u64 = 5;
/// A push is batched: at a state change, at session end, at start, or this long after the last.
pub const PUSH_EVERY_MS: i64 = 10 * 60 * 1000;
const INTENTS: &str = "ops";
const SYNC: &str = "sync.json";
const LOCK: &str = "notes.lock";

/// The notes checkout and the state folder its lock and intents live in.
pub struct Notes {
    pub root: PathBuf,
    pub state: PathBuf,
}

/// Held while writing. Dropped, the lock is released; killed, the OS releases it.
pub struct Held(#[allow(dead_code)] File);

/// One journal line, kept apart so a retry can tell it's already there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLine {
    pub id: String,
    pub kind: String,
    pub effort: String,
    pub title: String,
    pub text: String,
    pub machine: String,
    pub provider: String,
    /// `HH:MM` local, and the day file `YYYY-MM-DD`, fixed when the op was planned.
    pub time: String,
    pub day: String,
}

impl JournalLine {
    pub fn rendered(&self) -> String {
        format!(
            "- {} {} {} {}: {} <!-- {} {} e:{} -->",
            self.time,
            self.kind,
            self.effort,
            self.title,
            self.text,
            self.machine,
            self.provider,
            self.id
        )
    }
}

/// A planned write, applied whole and idempotently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
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
    /// The session state file to write once the notes are committed.
    pub state: Option<(PathBuf, Value)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pull {
    Pulled,
    Offline(String),
    Diverged(String),
}

impl Notes {
    pub fn new(root: PathBuf, state: PathBuf) -> Self {
        Self { root, state }
    }

    /// Blocks until this process holds the machine's one notes writer lock.
    pub fn lock(&self) -> Result<Held, String> {
        std::fs::create_dir_all(&self.state)
            .map_err(|e| format!("{}: {e}", self.state.display()))?;
        let path = self.state.join(LOCK);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        FileExt::lock_exclusive(&file).map_err(|e| format!("notes lock: {e}"))?;
        Ok(Held(file))
    }

    pub fn git(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
        crate::worktree::git(&self.root, &args)
    }

    /// A git call that may touch the network, killed after `NETWORK_SECS`.
    fn git_timed(&self, args: &[&str]) -> Result<String, String> {
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git could not start: {e}"))?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let out = child.wait_with_output().map_err(|e| e.to_string())?;
                    return if status.success() {
                        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
                    } else {
                        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
                    };
                }
                Ok(None) if started.elapsed() > Duration::from_secs(NETWORK_SECS) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("git {} took over {NETWORK_SECS} s", args[0]));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn has_remote(&self) -> bool {
        self.git(&["remote"]).is_ok_and(|r| !r.trim().is_empty())
    }

    /// Brings the checkout up to date when online, under the lock. A rebase conflict is
    /// aborted and recorded as diverged: local work is kept, nothing is merged unseen.
    pub fn pull(&self) -> Pull {
        if !self.has_remote() {
            return Pull::Offline("no remote".to_owned());
        }
        if let Err(why) = self.git_timed(&["fetch", "--quiet"]) {
            self.record_sync("offline", &why);
            return Pull::Offline(why);
        }
        match self.git(&["rebase", "--quiet", "@{upstream}"]) {
            Ok(_) => {
                self.clear_sync("diverged");
                self.fix_duplicate();
                Pull::Pulled
            }
            Err(why) if why.contains("no upstream") || why.contains("no tracking") => {
                Pull::Offline(why)
            }
            Err(why) => {
                let _ = self.git(&["rebase", "--abort"]);
                self.record_sync("diverged", &why);
                Pull::Diverged(why)
            }
        }
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

    /// Every `efforts/NNNN-*.md` as number and file name.
    pub fn numbers(&self) -> Vec<(u32, String)> {
        let dir = self.root.join("efforts");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.split('-').next().and_then(|n| n.parse::<u32>().ok());
            if let (Some(n), true) = (number, name.ends_with(".md")) {
                found.push((n, name));
            }
        }
        found.sort();
        found
    }

    /// Two files with one number, after a pull: the newer (by its `created` property, then
    /// name) is renumbered to the next free number. The plan for it, or None when clean.
    pub fn duplicate(&self) -> Option<(String, String, u32)> {
        let numbers = self.numbers();
        let dup = numbers.windows(2).find(|w| w[0].0 == w[1].0)?;
        let created = |name: &str| {
            std::fs::read_to_string(self.root.join("efforts").join(name))
                .ok()
                .and_then(|t| crate::effort::EffortFile::parse(&t).ok())
                .and_then(|f| f.prop("created").map(str::to_owned))
                .unwrap_or_default()
        };
        let (a, b) = (&dup[0].1, &dup[1].1);
        let newer = if created(a) > created(b) { a } else { b };
        let next = numbers.last().map_or(1, |(n, _)| n + 1);
        let new_name = format!(
            "{next:04}-{}",
            newer.splitn(2, '-').nth(1).unwrap_or("x.md")
        );
        Some((newer.clone(), new_name, next))
    }

    /// Two efforts with one number after a pull: the newer is renumbered to the next free
    /// number, its title and any session bound to it follow, and a journal line says so, so
    /// earlier lines still resolve. Offline creation on two machines is why this can happen.
    pub fn fix_duplicate(&self) -> Option<String> {
        let (old_name, new_name, next) = self.duplicate()?;
        let old_rel = format!("efforts/{old_name}");
        let new_rel = format!("efforts/{new_name}");
        let text = std::fs::read_to_string(self.root.join(&old_rel)).ok()?;
        let mut file = crate::effort::EffortFile::parse(&text).ok()?;
        let old_number = file.number().unwrap_or(0);
        file.title = format!("{next:04} {}", file.name());
        let (day, time) = local_day_time();
        let key = format!("renumber-{old_number:04}-{next:04}");
        let op = Op {
            key: key.clone(),
            files: vec![(new_rel.clone(), file.render())],
            renames: vec![(old_rel.clone(), new_rel.clone())],
            journal: vec![JournalLine {
                id: key.clone(),
                kind: "renumbered".to_owned(),
                effort: format!("{next:04}"),
                title: file.name().to_owned(),
                text: format!("was {old_number:04}, taken by another machine's effort"),
                machine: machine_name(),
                provider: "astrolabe".to_owned(),
                time,
                day,
            }],
            message: format!("{old_number:04} renumbered to {next:04}"),
            base: None,
            state: None,
        };
        if self.apply(&op).is_err() {
            return None;
        }
        // The renamed file's old path must go too, where the rename found it already moved.
        if self.root.join(&old_rel).exists() {
            let _ = std::fs::remove_file(self.root.join(&old_rel));
            let _ = self.git(&["add", "-A", "--", "efforts"]);
            let _ = self.git(&[
                "commit",
                "--quiet",
                "-m",
                &format!("{old_number:04} renumbered to {next:04}: old path"),
            ]);
        }
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
            let Ok(mut state) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if state["file"] == old_rel.as_str() {
                state["file"] = new_rel.clone().into();
                state["effort"] = next.to_string().into();
                let _ = write_text(&path, &state.to_string());
            }
        }
        Some(format!("{old_number:04} renumbered to {next:04}"))
    }

    /// Whether a commit already carries this op's key: a retried tool call was done.
    pub fn already_applied(&self, key: &str) -> bool {
        self.git(&[
            "log",
            "-1",
            "--format=%H",
            &format!("--grep=Op-Key: {key}"),
            "--fixed-strings",
        ])
        .is_ok_and(|out| !out.trim().is_empty())
    }

    /// Lock, finish leftovers, pull: what a pickup does before reading.
    pub fn refresh(&self) -> (Vec<String>, Pull) {
        match self.lock() {
            Ok(_held) => {
                let leftovers = self.reconcile();
                (leftovers, self.pull())
            }
            Err(why) => (vec![why], Pull::Offline("no lock".to_owned())),
        }
    }

    pub fn intent_path(&self, key: &str) -> PathBuf {
        self.state.join(INTENTS).join(format!("{key}.json"))
    }

    /// Plans are saved before anything is touched, so a crash leaves what to finish.
    pub fn save_intent(&self, op: &Op) -> Result<(), String> {
        let path = self.intent_path(&op.key);
        let text = serde_json::to_string(op).map_err(|e| e.to_string())?;
        write_text(&path, &text)
    }

    pub fn intents(&self) -> Vec<Op> {
        let dir = self.state.join(INTENTS);
        let mut ops = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if let Ok(bytes) = std::fs::read(entry.path())
                && let Ok(op) = serde_json::from_slice::<Op>(&bytes)
            {
                ops.push(op);
            }
        }
        ops
    }

    /// Finishes whatever an earlier run left, under the caller's lock.
    pub fn reconcile(&self) -> Vec<String> {
        self.intents()
            .iter()
            .filter_map(|op| self.apply(op).err().map(|why| format!("{}: {why}", op.key)))
            .collect()
    }

    /// Writes, journals, commits and records the session state, each step skipped when it's
    /// already done, so applying twice is applying once.
    pub fn apply(&self, op: &Op) -> Result<(), String> {
        self.save_intent(op)?;
        for (from, to) in &op.renames {
            let (from, to) = (self.root.join(from), self.root.join(to));
            if from.exists() && !to.exists() {
                std::fs::rename(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
            }
        }
        for (rel, text) in &op.files {
            let path = self.root.join(rel);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                write_text(&path, text)?;
            }
        }
        for line in &op.journal {
            let path = self.root.join("journal").join(format!("{}.md", line.day));
            let have = std::fs::read_to_string(&path).unwrap_or_default();
            if !have.contains(&format!("e:{} ", line.id)) {
                let mut text = have;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&line.rendered());
                text.push('\n');
                write_text(&path, &text)?;
            }
        }
        self.ensure_attributes()?;
        let changed = self.git(&[
            "status",
            "--porcelain",
            "--",
            "efforts",
            "journal",
            ".gitattributes",
        ])?;
        if !changed.trim().is_empty() {
            self.git(&["add", "--", "efforts", "journal", ".gitattributes"])?;
            let mut message = format!("{}\n\nOp-Key: {}", op.message, op.key);
            if let Some(base) = &op.base {
                message += &format!("\nBase: {base}");
            }
            self.git(&[
                "-c",
                "user.useConfigOnly=false",
                "commit",
                "--quiet",
                "-m",
                &message,
            ])?;
        }
        if let Some((path, state)) = &op.state {
            let text = state.to_string();
            write_text(path, &text)?;
        }
        let _ = std::fs::remove_file(self.intent_path(&op.key));
        Ok(())
    }

    /// The journal merges by union, so two machines appending the same day never conflict.
    fn ensure_attributes(&self) -> Result<(), String> {
        let path = self.root.join(".gitattributes");
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        if have
            .lines()
            .any(|l| l.trim_start().starts_with("journal/") && l.contains("merge=union"))
        {
            return Ok(());
        }
        let mut text = have;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("journal/*.md merge=union\n");
        write_text(&path, &text)
    }

    /// Pull, then push, under the lock; what happened is recorded for every session to read.
    pub fn sync(&self) -> Result<String, String> {
        let _held = self.lock()?;
        let leftovers = self.reconcile();
        if !self.has_remote() {
            return Ok("no remote: nothing to push".to_owned());
        }
        match self.pull() {
            Pull::Offline(why) => return Err(format!("offline: {why}")),
            Pull::Diverged(why) => return Err(format!("diverged: {why}")),
            Pull::Pulled => {}
        }
        let stale = self.stale_saves();
        let mut pushed = self.git_timed(&["push", "--quiet"]);
        // Another machine's push can land between our pull and push: once more, fresh.
        if pushed
            .as_ref()
            .is_err_and(|why| why.contains("fetch first") || why.contains("non-fast-forward"))
        {
            if let Pull::Diverged(why) = self.pull() {
                return Err(format!("diverged: {why}"));
            }
            pushed = self.git_timed(&["push", "--quiet"]);
        }
        match pushed {
            Ok(_) => {
                self.record_pushed();
                let mut text = "pushed".to_owned();
                if !leftovers.is_empty() {
                    text += &format!("; leftover intents failed: {}", leftovers.join("; "));
                }
                if !stale.is_empty() {
                    text += &format!("; stale saves: {}", stale.join("; "));
                }
                Ok(text)
            }
            Err(why) => {
                self.record_sync("push-failed", &why);
                Err(format!("push failed: {why}"))
            }
        }
    }

    /// After a pull: a local commit whose `Base` trailer doesn't match the file as origin had it
    /// before our commit was saved blind to another machine's save. Both versions are in git;
    /// this names the effort so a session can say so.
    fn stale_saves(&self) -> Vec<String> {
        let Ok(log) = self.git(&[
            "log",
            "@{upstream}..HEAD",
            "--format=%H%x1f%(trailers:key=Base,valueonly)",
            "--",
        ]) else {
            return Vec::new();
        };
        let mut stale = Vec::new();
        for line in log.lines() {
            let Some((sha, base)) = line.split_once('\x1f') else {
                continue;
            };
            let base = base.trim();
            if base.is_empty() {
                continue;
            }
            let Ok(files) = self.git(&["show", "--name-only", "--format=", sha]) else {
                continue;
            };
            for file in files.lines().filter(|f| f.starts_with("efforts/")) {
                let before = self
                    .git(&["show", &format!("{sha}~1:{file}")])
                    .unwrap_or_default();
                if !before.is_empty() && hash(&before) != base {
                    stale.push(file.to_owned());
                }
            }
        }
        stale.sort();
        stale.dedup();
        if !stale.is_empty() {
            self.record_sync("stale", &stale.join(", "));
        } else {
            self.clear_sync("stale");
        }
        stale
    }

    /// Runs `astrolabe notes sync` as its own process, so a hook or a save returns at once and
    /// the push survives the app closing.
    pub fn spawn_sync(&self) {
        // Tests run syncs in order themselves.
        if std::env::var_os("ASTROLABE_NO_DETACHED_SYNC").is_some() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let log = self.state.join("sync.log");
        let out = File::options().create(true).append(true).open(&log).ok();
        let err = File::options().create(true).append(true).open(&log).ok();
        let mut cmd = Command::new(exe);
        cmd.args(["notes", "sync"])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(out.map_or(Stdio::null(), Stdio::from))
            .stderr(err.map_or(Stdio::null(), Stdio::from));
        detach(&mut cmd);
        let _ = cmd.spawn();
    }

    fn sync_record(&self) -> Value {
        std::fs::read(self.state.join(SYNC))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}))
    }

    fn record_sync(&self, what: &str, why: &str) {
        let mut record = self.sync_record();
        record[what] = json!({"why": why, "at_ms": now_ms()});
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    fn clear_sync(&self, what: &str) {
        let mut record = self.sync_record();
        if record.get(what).is_some() {
            record.as_object_mut().map(|o| o.remove(what));
            let _ = write_text(&self.state.join(SYNC), &record.to_string());
        }
    }

    fn record_pushed(&self) {
        let mut record = self.sync_record();
        record["pushed_ms"] = json!(now_ms());
        for what in ["offline", "push-failed"] {
            record.as_object_mut().map(|o| o.remove(what));
        }
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    pub fn last_push_ms(&self) -> i64 {
        self.sync_record()["pushed_ms"].as_i64().unwrap_or(0)
    }

    /// Commits not on origin, so a session can say what's unpublished.
    pub fn unpublished(&self) -> usize {
        self.git(&["rev-list", "--count", "@{upstream}..HEAD"])
            .ok()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Trouble a session should hear about, in a line each; empty when all is well.
    pub fn warnings(&self) -> Vec<String> {
        let record = self.sync_record();
        let mut lines = Vec::new();
        if let Some(d) = record.get("diverged") {
            lines.push(format!(
                "Notes diverged from origin ({}); resolve in the notes checkout. Saves still land locally.",
                d["why"].as_str().unwrap_or_default().lines().next().unwrap_or_default()
            ));
        }
        if let Some(s) = record.get("stale") {
            lines.push(format!(
                "Two machines saved the same effort unseen: {}. The file holds whichever landed last; both are in git. Resave it from what's there.",
                s["why"].as_str().unwrap_or_default()
            ));
        }
        let unpublished = self.unpublished();
        if unpublished > 0 {
            let since = record["push-failed"]["why"]
                .as_str()
                .or(record["offline"]["why"].as_str());
            lines.push(match since {
                Some(why) => format!(
                    "{unpublished} notes commit(s) not yet pushed ({}).",
                    why.lines().next().unwrap_or_default()
                ),
                None => format!("{unpublished} notes commit(s) not yet pushed."),
            });
        }
        let intents = self.intents().len();
        if intents > 0 {
            lines.push(format!(
                "{intents} notes write(s) left unfinished; `astrolabe notes sync` finishes them."
            ));
        }
        lines
    }

    pub fn status_text(&self) -> String {
        let warnings = self.warnings();
        if warnings.is_empty() {
            "Notes are published: nothing pending.".to_owned()
        } else {
            warnings.join("\n")
        }
    }
}

/// What a session's product repo looks like right now, for the stamp and the nudge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    pub branch: String,
    pub head: String,
    pub unpushed: Option<u32>,
    pub dirty: bool,
    /// HEAD, the content of `git diff HEAD` and the untracked files' contents, hashed: the
    /// same tree gives the same print, and a second edit to a dirty file changes it.
    pub fingerprint: String,
}

impl RepoFacts {
    /// Facts of the repo at `cwd`, or None when it isn't a git checkout.
    pub fn read(cwd: &Path) -> Option<Self> {
        let git = |args: &[&str]| {
            let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
            crate::worktree::git(cwd, &args).ok()
        };
        let head = git(&["rev-parse", "HEAD"])?.trim().to_owned();
        let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|b| b.trim().to_owned())
            .unwrap_or_default();
        let unpushed =
            git(&["rev-list", "--count", "@{upstream}..HEAD"]).and_then(|n| n.trim().parse().ok());
        let status = git(&["status", "--porcelain", "--untracked-files=all"]).unwrap_or_default();
        let diff = git(&["diff", "HEAD"]).unwrap_or_default();
        use sha2::{Digest, Sha256};
        let mut print = Sha256::new();
        print.update(head.as_bytes());
        print.update(diff.as_bytes());
        for line in status.lines().filter(|l| l.starts_with("?? ")) {
            let file = cwd.join(&line[3..]);
            print.update(line.as_bytes());
            if let Ok(bytes) = std::fs::read(&file) {
                print.update(&bytes);
            }
        }
        let fingerprint = print
            .finalize()
            .iter()
            .take(12)
            .map(|b| format!("{b:02x}"))
            .collect();
        Some(Self {
            branch,
            head: head.chars().take(7).collect(),
            unpushed,
            dirty: !status.trim().is_empty(),
            fingerprint,
        })
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

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}
