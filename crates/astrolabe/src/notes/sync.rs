//! Git synchronization, retry backoff and published status.
use super::{
    ASKED, BACKOFF_MS, NETWORK_SECS, Notes, Op, Pull, SYNC, content_hash, hash, md_files, now_ms,
    tidy_journal, write_text,
};
use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

impl Notes {
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

    pub(crate) fn has_remote(&self) -> bool {
        self.git(&["remote"]).is_ok_and(|r| !r.trim().is_empty())
    }

    /// Brings the checkout up to date when online, under the lock. A rebase conflict is
    /// aborted and recorded as diverged: local work is kept, nothing is merged unseen. Hand
    /// edits in the tree (the owner's seat is often mid-edit) are stashed around the rebase;
    /// one that won't reapply stays in the stash, named by id until it's gone, and the tree
    /// is set back to what was pulled, so no save commits a conflict marker.
    pub fn pull(&self) -> Pull {
        match self.fetch() {
            Ok(()) => self.rebase(),
            Err(offline) => offline,
        }
    }

    /// A pull that leaves the tree alone unless origin has commits this checkout lacks, for
    /// the board server's background pulls: hand edits aren't stashed and reapplied every
    /// minute when nothing came in.
    pub fn pull_if_behind(&self) -> Pull {
        if let Err(offline) = self.fetch() {
            return offline;
        }
        match self.git(&["rev-list", "--count", "HEAD..@{upstream}"]) {
            Ok(n) if n.trim() == "0" => Pull::Pulled,
            Ok(_) => self.rebase(),
            Err(why) => Pull::Offline(why),
        }
    }

    /// The fetch half of a pull, bounded and backed off after a timeout.
    fn fetch(&self) -> Result<(), Pull> {
        if !self.has_remote() {
            return Err(Pull::Offline("no remote".to_owned()));
        }
        if let Some(left) = self.backing_off() {
            return Err(Pull::Offline(format!(
                "a network call timed out lately; no fetch for another {} s",
                left / 1000
            )));
        }
        if let Err(why) = self.git_timed(&["fetch", "--quiet"]) {
            self.record_sync("offline", &why);
            if why.contains("took over") {
                self.record_backoff();
            }
            self.changed();
            return Err(Pull::Offline(why));
        }
        Ok(())
    }

    /// The rebase half of a pull, onto what the last fetch brought.
    fn rebase(&self) -> Pull {
        let before = self.git(&["rev-parse", "HEAD"]).unwrap_or_default();
        let stashes_before = self.stashes();
        match self.git(&["rebase", "--quiet", "--autostash", "@{upstream}"]) {
            Ok(_) => {
                self.clear_sync("diverged");
                if let Err(why) = self.keep_stash(&stashes_before) {
                    self.record_sync("diverged", &why);
                    self.changed();
                    return Pull::Diverged(why);
                }
                self.fix_duplicate();
                // Every pull, not only a sync's: a pickup's pull merges day files too. A
                // failure is recorded, not hidden: the next status and session text say so.
                match self.tidy_after_merge() {
                    Ok(()) => self.clear_sync("tidy-failed"),
                    Err(why) => self.record_sync("tidy-failed", &why),
                }
                if self.git(&["rev-parse", "HEAD"]).unwrap_or_default() != before {
                    self.changed();
                }
                Pull::Pulled
            }
            Err(why) if why.contains("no upstream") || why.contains("no tracking") => {
                Pull::Offline(why)
            }
            Err(why) => {
                let _ = self.git(&["rebase", "--abort"]);
                self.record_sync("diverged", &why);
                self.changed();
                Pull::Diverged(why)
            }
        }
    }

    /// The stash entries, one `<id> <subject>` line each.
    fn stashes(&self) -> Vec<String> {
        self.git(&["stash", "list", "--format=%H %gs"])
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// After a rebase: a stash git created for the hand edits and couldn't reapply (it leaves
    /// the conflict in the tree and the edits in the stash) is recorded by id, one line per
    /// stash still there, and the tree reset to the pulled state. A reset that fails is an
    /// error, since the next save would stage the markers.
    fn keep_stash(&self, before: &[String]) -> Result<(), String> {
        let unmerged = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .unwrap_or_default();
        let now = self.stashes();
        let new = now
            .iter()
            .find(|l| !before.contains(l) && l.ends_with(" autostash"));
        let mut kept: Vec<String> = self.sync_record()["stash"]["why"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .filter(|l| {
                let id = l.split(' ').next().unwrap_or_default();
                !id.is_empty() && now.iter().any(|s| s.starts_with(id))
            })
            .map(str::to_owned)
            .collect();
        if let Some(line) = new {
            let id = line.split(' ').next().unwrap_or_default();
            let files: Vec<&str> = unmerged.lines().collect();
            kept.push(format!("{id} {}", files.join(", ")));
        }
        // Recorded before the reset, so a reset that fails still leaves the stash named.
        if kept.is_empty() {
            self.clear_sync("stash");
        } else {
            self.record_sync("stash", &kept.join("\n"));
        }
        if new.is_some() {
            self.git(&["reset", "--quiet", "--hard"]).map_err(|why| {
                format!("hand edits that wouldn't reapply are in a stash, and the tree couldn't be reset after them: {why}")
            })?;
        }
        Ok(())
    }

    /// Pull, then push, under the lock; what happened is recorded for every session to read.
    pub fn sync(&self) -> Result<String, String> {
        let _held = self.lock()?;
        let leftovers = self.reconcile();
        if !leftovers.is_empty() {
            return Err(format!(
                "unfinished writes failed again, so nothing was pulled or pushed: {}",
                leftovers.join("; ")
            ));
        }
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
                self.changed();
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
                if why.contains("took over") {
                    self.record_backoff();
                }
                self.changed();
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
                if !before.is_empty() && content_hash(&before) != base {
                    stale.push(file.to_owned());
                }
            }
        }
        stale.sort();
        stale.dedup();
        if !stale.is_empty() {
            // Kept, per file, until a later save of that file lands.
            let mut record = self.sync_record();
            let mut files: Vec<String> = record["stale"]["files"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            files.extend(stale.iter().cloned());
            files.sort();
            files.dedup();
            record["stale"] = json!({"files": files, "at_ms": now_ms()});
            let _ = write_text(&self.state.join(SYNC), &record.to_string());
        }
        stale
    }

    pub(super) fn clear_stale(&self, rel: &str) {
        let mut record = self.sync_record();
        let Some(files) = record["stale"]["files"].as_array() else {
            return;
        };
        let kept: Vec<Value> = files
            .iter()
            .filter(|f| f.as_str() != Some(rel))
            .cloned()
            .collect();
        if kept.len() == files.len() {
            return;
        }
        if kept.is_empty() {
            record.as_object_mut().map(|o| o.remove("stale"));
        } else {
            record["stale"]["files"] = kept.into();
        }
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    fn sync_record(&self) -> Value {
        std::fs::read(self.state.join(SYNC))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}))
    }

    /// After a pull: day files a union merge left with two lines for one effort are tidied
    /// and committed, so the file the owner reads has one.
    fn tidy_after_merge(&self) -> Result<(), String> {
        let mut files = Vec::new();
        for rel in md_files(&self.root, "journal") {
            let Ok(text) = std::fs::read_to_string(self.root.join(&rel)) else {
                continue;
            };
            let tidied = tidy_journal(&text);
            if tidied != text {
                files.push((rel, tidied));
            }
        }
        if files.is_empty() {
            return Ok(());
        }
        let key = format!(
            "tidy-{}",
            hash(&files.iter().map(|(_, t)| t.as_str()).collect::<String>())
        );
        let op = Op {
            lesson_close: None,
            key: key.clone(),
            files,
            renames: Vec::new(),
            journal: Vec::new(),
            message: "journal tidied after a merge".to_owned(),
            base: None,
            states: Vec::new(),
            committed: false,
            digest: None,
            result: None,
            cwd: None,
            rebaseline: None,
        };
        self.apply(&op)?;
        self.forget(&key);
        Ok(())
    }

    /// Milliseconds left of a backoff after a timed-out network call, if any.
    pub fn backing_off(&self) -> Option<i64> {
        let until = self.sync_record()["backoff_until_ms"].as_i64()?;
        let left = until - now_ms();
        (left > 0).then_some(left)
    }

    fn record_backoff(&self) {
        let mut record = self.sync_record();
        record["backoff_until_ms"] = (now_ms() + BACKOFF_MS).into();
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    pub(crate) fn record_sync(&self, what: &str, why: &str) {
        let mut record = self.sync_record();
        record[what] = json!({"why": why, "at_ms": now_ms()});
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    pub(crate) fn clear_sync(&self, what: &str) {
        let mut record = self.sync_record();
        if record.get(what).is_some() {
            record.as_object_mut().map(|o| o.remove(what));
            let _ = write_text(&self.state.join(SYNC), &record.to_string());
        }
    }

    fn record_pushed(&self) {
        let mut record = self.sync_record();
        record["pushed_ms"] = json!(now_ms());
        for what in ["offline", "push-failed", "backoff_until_ms"] {
            record.as_object_mut().map(|o| o.remove(what));
        }
        let _ = write_text(&self.state.join(SYNC), &record.to_string());
    }

    /// When a detached sync was last asked for, as epoch milliseconds; 0 when never.
    pub fn last_request_ms(&self) -> i64 {
        std::fs::metadata(self.state.join(ASKED))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64)
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
        if let Some(files) = record["stale"]["files"]
            .as_array()
            .filter(|f| !f.is_empty())
        {
            let names: Vec<&str> = files.iter().filter_map(Value::as_str).collect();
            lines.push(format!(
                "Two machines saved the same effort unseen: {}. The file holds whichever landed last; both are in git. Resave it from what's there.",
                names.join(", ")
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
        for entry in record["stash"]["why"].as_str().unwrap_or_default().lines() {
            let (id, files) = entry.split_once(' ').unwrap_or((entry, ""));
            let short: String = id.chars().take(7).collect();
            lines.push(format!(
                "Hand edits in the notes checkout ({files}) didn't reapply after a pull and sit in stash {short}; `git stash apply {short}` there brings them back, and `git stash list` shows which entry to drop once they're in.",
            ));
        }
        if let Some(t) = record.get("tidy-failed") {
            lines.push(format!(
                "A journal tidy after a merge failed ({}); the day file may show two lines for one effort until the next save.",
                t["why"].as_str().unwrap_or_default().lines().next().unwrap_or_default()
            ));
        }
        if let Some(b) = record.get("board-failed") {
            lines.push(format!(
                "The board (home.html) could not be written ({}); it's redrawn at the next save.",
                b["why"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
            ));
        }
        let intents = self
            .intents()
            .iter()
            .filter(|op| op.result.is_none())
            .count();
        if intents > 0 {
            lines.push(format!(
                "{intents} notes write(s) left unfinished; `astrolabe notes sync` finishes them."
            ));
        }
        let broken = self.broken_intents();
        if !broken.is_empty() {
            lines.push(format!(
                "{} unfinished notes write(s) can't be read, so no new notes write lands until they're repaired or moved aside: {}",
                broken.len(),
                broken.join("; ")
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
