//! Git synchronization, retry backoff and published status.
use super::{
    ASKED, BACKOFF_MS, Notes, Op, Pull, SYNC, Unfit, content_hash, hash, md_files, now_ms,
    tidy_journal, write_text,
};
use crate::deadline::Step;
use serde_json::{Value, json};

/// The status `git rebase` exits with when it started and then stopped on something it
/// couldn't replay. One that can't start at all, as when another rebase is already under
/// way, dies with a different status and leaves no state of its own.
const REBASE_STOPPED: i32 = 1;

/// Whether the rebase in `dir`, found after this call's own rebase failed, is the one this
/// call began. Never by git's message, which is in the user's language and quotes commit
/// subjects, but by what doesn't vary: this call's rebase exited with the status of one
/// that started and stopped, and git recorded the rebase there as begun from `before`,
/// the commit this call was on, onto `onto`, the commit this call asked for. None was
/// there the moment before this call's rebase started. One the owner began by hand in
/// that same moment makes this call's rebase die without starting, which is another
/// status, so theirs is never taken for this call's, even from the same commit onto the
/// same one.
fn began_here(dir: &std::path::Path, code: Option<i32>, before: &str, onto: &str) -> bool {
    let recorded = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .map(|text| text.trim().to_owned())
            .unwrap_or_default()
    };
    let (before, onto) = (before.trim(), onto.trim());
    code == Some(REBASE_STOPPED)
        && !before.is_empty()
        && !onto.is_empty()
        && recorded("orig-head") == before
        && recorded("onto") == onto
}

impl Notes {
    /// A git call that may touch the network, stopped when its time is up (see `deadline`).
    fn git_timed(&self, args: &[&str]) -> Result<String, String> {
        let args: Vec<&std::ffi::OsStr> = args.iter().map(AsRef::as_ref).collect();
        crate::worktree::git_as(crate::deadline::Git::Network, &self.root, &args)
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
        let started = std::time::Instant::now();
        let pulled = match self.fetch() {
            Ok(()) => self.rebase(),
            Err(offline) => offline,
        };
        // For a write refused after it, which says where the hook's time went.
        crate::deadline::pulled(started.elapsed());
        pulled
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
        // A checkout partway through a rebase, or off its branch, is the owner's to put
        // right: nothing is fetched or rebased over it, and the write after is refused.
        match self.unfit() {
            Ok(None) => {}
            Ok(Some(Unfit::MidRebase)) => {
                return Err(Pull::Offline(
                    "a rebase is already under way in the notes checkout".to_owned(),
                ));
            }
            Ok(Some(Unfit::Detached)) => {
                return Err(Pull::Offline(
                    "the notes checkout isn't on a branch".to_owned(),
                ));
            }
            Err(why) => return Err(Pull::Offline(why)),
        }
        // The one step a hook can do without: late in its time it works from the local copy.
        if crate::deadline::short_of(Step::Pull).is_some() {
            return Err(Pull::Offline(
                "this hook had no time left for a pull".to_owned(),
            ));
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
        if crate::deadline::short_of(Step::Rebase).is_some() {
            return Pull::Offline(
                "fetched, but this hook had no time left to rebase onto it".to_owned(),
            );
        }
        // Looked at again, right before starting: the fetch took seconds, and a rebase the
        // owner began by hand meanwhile is theirs. Starting over it would fail, and
        // cleaning up after that failure would end it.
        match self.rebase_dir() {
            Ok(None) => {}
            Ok(Some(_)) => {
                return Pull::Offline(
                    "a rebase is already under way in the notes checkout".to_owned(),
                );
            }
            Err(why) => return Pull::Offline(why),
        }
        // The backstop waits for a rebase and what follows it, after it answers.
        let _under_way = crate::deadline::under_way();
        let before = self.git(&["rev-parse", "HEAD"]).unwrap_or_default();
        let onto = self.git(&["rev-parse", "@{upstream}"]).unwrap_or_default();
        let stashes_before = self.stashes();
        // Never ended by a hook, which decided above whether there was time to start it.
        let args = ["rebase", "--quiet", "--autostash", "@{upstream}"].map(AsRef::as_ref);
        let rebased = crate::worktree::git_or(crate::deadline::Git::Notes, &self.root, &args);
        let code = rebased.as_ref().err().and_then(|failed| failed.code);
        match rebased.map_err(|failed| failed.why) {
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
                // Only this call's own rebase is put back: one it started, which then
                // failed. Never one that was already there, which is why it failed.
                if let Ok(Some(dir)) = self.rebase_dir()
                    && began_here(&dir, code, &before, &onto)
                {
                    let _ = self.git(&["rebase", "--abort"]);
                }
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
            for file in files
                .lines()
                .filter(|f| f.starts_with("efforts/") || f.starts_with("initiatives/"))
            {
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

    /// A status-only reply transition cannot resolve an unseen concurrent save.
    pub(crate) fn has_stale(&self, rel: &str) -> bool {
        self.sync_record()["stale"]["files"]
            .as_array()
            .is_some_and(|files| files.iter().any(|file| file.as_str() == Some(rel)))
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
            reply_after_ms: None,
            lesson_close: None,
            lesson_closes: Vec::new(),
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
            session: None,
            bound: None,
            at_ms: None,
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
                "Two machines saved the same record unseen: {}. The file holds whichever landed last; both are in git. Resave it from what's there.",
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
                "A journal tidy after a merge failed ({}); the day file may show two lines for one record until the next save.",
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

#[cfg(test)]
mod tests {
    use super::{REBASE_STOPPED, began_here};

    /// A rebase folder as git leaves one, with what it recorded.
    fn rebase_folder(name: &str, from: &str, onto: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("astrolabe-rebase-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, text) in [("orig-head", from), ("onto", onto)] {
            std::fs::write(dir.join(file), format!("{text}\n")).unwrap();
        }
        dir
    }

    /// What git holds while it works is looked for where this checkout keeps it: a linked
    /// worktree's `.git` is a file, and its index lock and rebase folder are elsewhere.
    #[test]
    fn git_in_the_way_is_seen_in_a_checkout_and_in_a_linked_worktree() {
        use std::path::{Path, PathBuf};
        let dir = std::env::temp_dir().join(format!("astrolabe-in-the-way-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (main, linked) = (dir.join("main"), dir.join("linked"));
        std::fs::create_dir_all(&main).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                .args(args)
                .current_dir(cwd)
                .output()
                .unwrap();
            assert!(out.status.success(), "{args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        git(&main, &["init", "--quiet"]);
        git(&main, &["commit", "--quiet", "--allow-empty", "-m", "seed"]);
        git(&main, &["worktree", "add", "--quiet", "../linked"]);
        assert!(linked.join(".git").is_file());
        for root in [&main, &linked] {
            let notes = crate::notes::Notes::new(root.clone(), dir.join("state"));
            let kept = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"]));
            assert!(!notes.git_in_the_way(), "{root:?}");
            let lock = kept.join("index.lock");
            std::fs::write(&lock, "").unwrap();
            assert!(notes.git_in_the_way(), "the index lock in {root:?}");
            std::fs::remove_file(lock).unwrap();
            for name in ["rebase-merge", "rebase-apply"] {
                std::fs::create_dir(kept.join(name)).unwrap();
                assert!(notes.git_in_the_way(), "{name} in {root:?}");
                std::fs::remove_dir(kept.join(name)).unwrap();
            }
            assert!(!notes.git_in_the_way(), "{root:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_rebase_this_call_began_is_its_to_abort() {
        let (here, upstream) = ("1111111111111111111111111111111111111111", "2222222222");
        let is = |name: &str, code: Option<i32>, from: &str, onto: &str| {
            let dir = rebase_folder(name, from, onto);
            let ours = began_here(&dir, code, &format!("{here}\n"), &format!("{upstream}\n"));
            let _ = std::fs::remove_dir_all(dir);
            ours
        };
        let stopped = Some(REBASE_STOPPED);
        // It started from where this call was, onto what it asked for, and stopped.
        assert!(is("ours", stopped, here, upstream));
        // The owner's by hand, begun in the same moment from the same commit onto the same
        // one: this call's rebase died on finding it, which is another status. What git
        // said as it died, in whatever language, plays no part.
        assert!(!is("theirs", Some(128), here, upstream));
        // Ended by something else, or not run to an end.
        assert!(!is("signal", None, here, upstream));
        // Begun somewhere else, or onto something else.
        assert!(!is("from", stopped, "3333333", upstream));
        assert!(!is("onto", stopped, here, "4444444"));
        // Where this call was, or what it asked for, isn't known: nothing is claimed.
        let dir = rebase_folder("unknown", "", "");
        assert!(!began_here(&dir, stopped, "", ""));
        assert!(!began_here(&dir, stopped, here, ""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
