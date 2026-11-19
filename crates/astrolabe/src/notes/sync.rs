//! Git synchronization, retry backoff and published status.
use super::{
    ASKED, BACKOFF_MS, Notes, Op, Pull, SYNC, Unfit, hash, md_files, now_ms, tidy_journal,
    write_text,
};
use crate::deadline::Step;
use serde_json::{Value, json};

/// The status `git rebase` exits with when it started and then stopped on something it
/// couldn't replay. One that can't start at all, as when another rebase is already under
/// way, dies with a different status and leaves no state of its own.
const REBASE_STOPPED: i32 = 1;

/// The sync record's entry for a rebase this program started and couldn't abort: the
/// commits it started from and onto.
const OWN_REBASE: &str = "own-rebase";

/// The file this program leaves in git's rebase folder for a rebase it started and is
/// aborting. Git removes the folder whole when the rebase ends, mark and all.
const OWN_MARK: &str = "astrolabe-own";

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
        // The rebase itself leaves the tree alone when nothing came in.
        match self.fetch() {
            Ok(()) => self.rebase(),
            Err(offline) => offline,
        }
    }

    /// The fetch half of a pull, bounded and backed off after a timeout.
    fn fetch(&self) -> Result<(), Pull> {
        // A rebase this program started and couldn't abort is its own, offline or not.
        self.finish_own_rebase();
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
        // Nothing came in: no rebase, so hand edits aren't stashed and reapplied for nothing.
        // Every pickup pulls, and each round trip was a chance for the reapply to fail.
        let before = self.git(&["rev-parse", "HEAD"]).unwrap_or_default();
        if self
            .git(&["rev-list", "--count", "HEAD..@{upstream}"])
            .is_ok_and(|n| n.trim() == "0")
        {
            return self.settle(&before, &self.stashes());
        }
        // Another program's git holding the index stops the rebase before it starts, and
        // is no divergence: the next pull goes through.
        if self.index_held() {
            return Pull::Offline(super::INDEX_HELD.to_owned());
        }
        // The backstop waits for a rebase and what follows it, after it answers.
        let _under_way = crate::deadline::under_way();
        let onto = self.git(&["rev-parse", "@{upstream}"]).unwrap_or_default();
        if let Err(why) = self.clear_the_way() {
            self.record_sync("diverged", &why);
            self.changed();
            return Pull::Diverged(why);
        }
        let stashes_before = self.stashes();
        // Records merge through the program's driver (see `merge`), so two machines' saves of
        // one record don't conflict on their stamp lines. Its config rides on this command
        // alone: a pull by hand merges as git always has.
        let mut args: Vec<String> = Vec::new();
        if let Some(driver) = super::merge::driver_command()
            && self.route_records().is_ok()
        {
            args.extend([
                "-c".to_owned(),
                "merge.astrolabe.name=astrolabe records".to_owned(),
                "-c".to_owned(),
                format!("merge.astrolabe.driver={driver}"),
            ]);
        }
        // `--merge` holds even where `rebase.backend` is set to apply, which skips drivers.
        args.extend(
            ["rebase", "--merge", "--quiet", "--autostash", "@{upstream}"].map(str::to_owned),
        );
        let args: Vec<&std::ffi::OsStr> = args.iter().map(AsRef::as_ref).collect();
        // Never ended by a hook, which decided above whether there was time to start it.
        let rebased = crate::worktree::git_or(crate::deadline::Git::Notes, &self.root, &args);
        let code = rebased.as_ref().err().and_then(|failed| failed.code);
        match rebased.map_err(|failed| failed.why) {
            Ok(_) => self.settle(&before, &stashes_before),
            Err(why) if why.contains("no upstream") || why.contains("no tracking") => {
                Pull::Offline(why)
            }
            Err(why) => {
                // Only this call's own rebase is put back: one it started, which then
                // failed. Never one that was already there, which is why it failed.
                let mut why = why;
                let dir = self.rebase_dir();
                if matches!(dir, Ok(None)) && self.index_held() {
                    return Pull::Offline(super::INDEX_HELD.to_owned());
                }
                if let Ok(Some(dir)) = dir
                    && began_here(&dir, code, &before, &onto)
                {
                    // Marked as this program's first, inside git's own folder for it, so a
                    // rebase left standing is finished off by the next pull instead of
                    // holding every write back, and one the owner starts later never is.
                    let started = format!("{} {}", before.trim(), onto.trim());
                    let _ = std::fs::write(dir.join(OWN_MARK), &started);
                    self.record_sync(OWN_REBASE, &started);
                    if let Err(left) = self.abort_own_rebase() {
                        why = format!("{why}\n{left}");
                    }
                }
                self.record_sync("diverged", &why);
                self.changed();
                Pull::Diverged(why)
            }
        }
    }

    /// Untracked files where the incoming commits add one. A rebase won't write over them and
    /// `--autostash` doesn't keep them, so one would stop every pull here: a hand-off file a
    /// save committed on the other machine can sit untracked here too, copied across or
    /// written again. One git would store exactly as the incoming file goes, since the pull
    /// puts that content back, line endings as the notes check them out. Any other is named and nothing is pulled, because only the owner
    /// can say which copy to keep.
    fn clear_the_way(&self) -> Result<(), String> {
        let added = self.git(&[
            "diff",
            "--name-only",
            "-z",
            "--relative",
            "--no-renames",
            "--diff-filter=A",
            "HEAD",
            "@{upstream}",
            "--",
        ])?;
        let here: Vec<String> = (added.split('\0'))
            .filter(|p| !p.is_empty() && self.root.join(p).is_file())
            .map(|p| format!(":(literal){p}"))
            .collect();
        if here.is_empty() {
            return Ok(());
        }
        // Ignored files git writes over anyway, and tracked ones the autostash keeps.
        let mut untracked = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
        untracked.extend(here.iter().map(String::as_str));
        let untracked = self.git(&untracked)?;
        let untracked: Vec<&str> = untracked.split('\0').filter(|p| !p.is_empty()).collect();
        if untracked.is_empty() {
            return Ok(());
        }
        // Hashed as git would store it, through the path's attributes: a copy written with
        // CRLF on Windows is the same text as one committed with LF, and checks out as it.
        let mut local = vec!["hash-object", "--"];
        local.extend(&untracked);
        let local = self.git(&local)?;
        let wanted: Vec<String> = untracked
            .iter()
            .map(|p| format!("@{{upstream}}:./{p}"))
            .collect();
        let mut incoming = vec!["rev-parse"];
        incoming.extend(wanted.iter().map(String::as_str));
        let incoming = self.git(&incoming)?;
        let mut differ = Vec::new();
        for ((path, mine), theirs) in untracked.iter().zip(local.lines()).zip(incoming.lines()) {
            if mine.trim() == theirs.trim() {
                std::fs::remove_file(self.root.join(path)).map_err(|e| format!("{path}: {e}"))?;
            } else {
                differ.push(*path);
            }
        }
        if differ.is_empty() {
            return Ok(());
        }
        let mut named: Vec<String> = differ.iter().take(5).map(|p| p.to_string()).collect();
        if differ.len() > 5 {
            named.push(format!("{} more", differ.len() - 5));
        }
        Err(format!(
            "nothing was pulled, because the other machine committed files that sit here untracked with other content: {}. Move or rename them in the notes checkout, then run `astrolabe notes sync`",
            named.join(", ")
        ))
    }

    /// What follows a rebase that went through, or one there was nothing to do for.
    fn settle(&self, before: &str, stashes_before: &[String]) -> Pull {
        self.clear_sync("diverged");
        if let Err(why) = self.keep_stash(stashes_before) {
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

    /// Aborts the rebase this program started, trying again while something else holds the
    /// index for a moment (an agent's `git status`, say): an abort that failed once left the
    /// Mac partway through a rebase, refusing every write, until the owner aborted it. Only
    /// a rebase folder holding this program's own mark is touched: one the owner starts
    /// after it is a new folder without one.
    fn abort_own_rebase(&self) -> Result<(), String> {
        let mut last = String::new();
        for attempt in 0..5 {
            let Some(dir) = self.own_rebase_dir() else {
                self.clear_sync(OWN_REBASE);
                return Ok(());
            };
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(200 * attempt));
            }
            // Looked at again right before: the owner may have aborted and begun their own.
            if !dir.join(OWN_MARK).exists() {
                continue;
            }
            if let Err(why) = self.git(&["rebase", "--abort"]) {
                last = why;
            }
        }
        if self.own_rebase_dir().is_none() {
            self.clear_sync(OWN_REBASE);
            return Ok(());
        }
        Err(format!(
            "The rebase it started couldn't be aborted ({}); the next pull tries again.",
            last.lines().next().unwrap_or("no reason given")
        ))
    }

    /// The rebase folder, when the rebase in it is one this program started and marked.
    fn own_rebase_dir(&self) -> Option<std::path::PathBuf> {
        let dir = self.rebase_dir().ok()??;
        dir.join(OWN_MARK).exists().then_some(dir)
    }

    /// A rebase this program started and couldn't abort, still standing with its mark.
    /// Finished off before a pull; any other rebase is the owner's and is left alone.
    fn finish_own_rebase(&self) {
        if self.sync_record().get(OWN_REBASE).is_none() {
            return;
        }
        if self.own_rebase_dir().is_some() {
            let _ = self.abort_own_rebase();
        } else if !matches!(self.rebase_dir(), Ok(Some(_))) {
            self.clear_sync(OWN_REBASE);
        }
    }

    /// Sends records to the merge driver through the checkout's own attributes file, which
    /// git reads but never tracks, so the first pull after an update uses it too.
    fn route_records(&self) -> Result<(), String> {
        let rel = self.git(&["rev-parse", "--git-path", "info/attributes"])?;
        let path = self.root.join(rel.trim());
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        let missing: Vec<&str> = super::merge::ATTRIBUTES
            .into_iter()
            .filter(|line| !have.lines().any(|l| l.trim() == *line))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let mut text = have;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        for line in missing {
            text.push_str(line);
            text.push('\n');
        }
        write_text(&path, &text)
    }

    /// The stash entries, one `<id> <subject>` line each.
    fn stashes(&self) -> Vec<String> {
        self.git(&["stash", "list", "--format=%H %gs"])
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// After a rebase: a stash git created for the hand edits and couldn't reapply is put
    /// back when nothing clashed, since git also leaves one when a file or the index was held
    /// for a moment, and exits 0 either way. One that clashed (git leaves the conflict in the
    /// tree and the edits in the stash), or that still won't go back, is recorded by id with
    /// its files, one line per stash still there. Only a clash resets the tree to the pulled
    /// state, so no save commits a marker; a reset that fails is an error for that reason.
    fn keep_stash(&self, before: &[String]) -> Result<(), String> {
        let unmerged = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .unwrap_or_default();
        let new = self
            .stashes()
            .into_iter()
            .find(|l| !before.contains(l) && l.ends_with(" autostash"))
            .map(|l| l.split(' ').next().unwrap_or_default().to_owned());
        let new = match new {
            Some(id) if unmerged.trim().is_empty() && self.reapply(&id) => None,
            other => other,
        };
        let now = self.stashes();
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
        if let Some(id) = &new {
            let files = self
                .git(&["stash", "show", "--name-only", "--format=", id])
                .unwrap_or_default();
            let files: Vec<&str> = files.lines().filter(|l| !l.trim().is_empty()).collect();
            kept.push(format!("{id} {}", files.join(", ")));
        }
        // Recorded before the reset, so a reset that fails still leaves the stash named.
        if kept.is_empty() {
            self.clear_sync("stash");
        } else {
            self.record_sync(
                "stash",
                &kept.join(
                    "
",
                ),
            );
        }
        // Looked at again: a retried reapply can clash too, and leave its markers.
        let clashed = self
            .git(&["diff", "--name-only", "--diff-filter=U"])
            .is_ok_and(|u| !u.trim().is_empty());
        if new.is_some() && clashed {
            self.git(&["reset", "--quiet", "--hard"]).map_err(|why| {
                format!("hand edits that wouldn't reapply are in a stash, and the tree couldn't be reset after them: {why}")
            })?;
        }
        Ok(())
    }

    /// Puts back the stash with commit `id` and drops it, trying again while something holds
    /// a file or the index for a moment. True once the stash is gone and its edits are in.
    fn reapply(&self, id: &str) -> bool {
        // Something wrote a stashed file during the pull: git won't put the stash over that
        // write however often it's asked, and the write is what the tree keeps.
        let stashed = self
            .git(&["stash", "show", "--name-only", "--format=", id])
            .unwrap_or_default();
        let changed = self.git(&["diff", "--name-only"]).unwrap_or_default();
        if changed.lines().any(|f| stashed.lines().any(|s| s == f)) {
            return false;
        }
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(200 * attempt));
            }
            // A stash is dropped by its place in the list, which can shift between tries.
            let Some(place) = self
                .git(&["stash", "list", "--format=%gd %H"])
                .unwrap_or_default()
                .lines()
                .find_map(|l| l.strip_suffix(id).map(|p| p.trim().to_owned()))
            else {
                return false;
            };
            if self.git(&["stash", "pop", "--quiet", &place]).is_ok() {
                return true;
            }
            // A pop that clashed leaves markers and keeps the stash: no retry undoes that.
            let clashed = self
                .git(&["diff", "--name-only", "--diff-filter=U"])
                .is_ok_and(|u| !u.trim().is_empty());
            if clashed {
                return false;
            }
        }
        false
    }

    /// Pull, then push, under the lock; what happened is recorded for every session to read.
    pub fn sync(&self) -> Result<String, String> {
        let Some(_held) = self.lock_for_sync()? else {
            return Ok("another sync is waiting for the notes, and it pushes these too".to_owned());
        };
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
        let mut pushed = self.git_timed(&["push", "--quiet"]);
        // Another machine's push can land between our pull and push: once more, fresh. The
        // remote's "cannot lock ref" is that race seen from its side, mid-update.
        let raced = |why: &String| {
            ["fetch first", "non-fast-forward", "cannot lock ref"]
                .iter()
                .any(|said| why.contains(said))
        };
        if pushed.as_ref().is_err_and(raced) {
            if let Pull::Diverged(why) = self.pull() {
                return Err(format!("diverged: {why}"));
            }
            pushed = self.git_timed(&["push", "--quiet"]);
        }
        match pushed {
            Ok(_) => {
                self.record_pushed();
                self.changed();
                Ok("pushed".to_owned())
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

    /// Records holding what a merge kept from the earlier of two saves (see `merge`), or an
    /// older build's `unseen` mark. Read from the records, so every machine says the same,
    /// and a save of the record on either machine clears it.
    pub(crate) fn unseen(&self) -> Vec<String> {
        let mark = format!("^{}:", super::merge::UNSEEN);
        let section = format!("^## {}", super::merge::UNMERGED);
        self.git(&[
            "grep",
            "-l",
            "-e",
            &mark,
            "-e",
            &section,
            "--",
            "efforts",
            "initiatives",
        ])
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
    }

    /// A status-only reply transition cannot resolve what a merge left unmerged.
    pub(crate) fn has_stale(&self, rel: &str) -> bool {
        std::fs::read_to_string(self.root.join(rel)).is_ok_and(|text| {
            crate::effort::frontmatter(&text)
                .iter()
                .any(|(k, v)| k == super::merge::UNSEEN && !v.is_empty())
                || crate::effort::EffortFile::parse(&text).is_ok_and(|f| f.unmerged().is_some())
        })
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
            handoffs: None,
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
        for what in ["offline", "push-failed", "backoff_until_ms", "stale"] {
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
        let unseen = self.unseen();
        if !unseen.is_empty() {
            lines.push(format!(
                "Two machines saved the same record unseen: {}. The merge kept the later save, and what it didn't keep is in the record's Unmerged state section. Read it and save the record with a Current state that folds in what still holds, which clears this.",
                unseen.join(", ")
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
    use crate::notes::Notes;

    fn run(dir: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Git leaves an autostash behind when a file or the index was held for a moment, with
    /// nothing clashing: it goes back into the tree, and nothing is recorded or reset.
    #[test]
    fn an_autostash_left_without_a_clash_is_put_back() {
        let dir = std::env::temp_dir().join(format!("astrolabe-stash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("notes");
        std::fs::create_dir_all(&root).unwrap();
        run(&root, &["init", "-q"]);
        std::fs::write(
            root.join("note.md"),
            "one
",
        )
        .unwrap();
        run(&root, &["add", "."]);
        run(&root, &["commit", "-qm", "one"]);
        std::fs::write(
            root.join("note.md"),
            "one
by hand
",
        )
        .unwrap();
        run(&root, &["stash", "push", "-q", "-m", "autostash"]);
        let notes = Notes::new(root.clone(), dir.join("state"));
        notes.keep_stash(&[]).unwrap();
        // Line endings follow the machine's git config.
        assert!(
            std::fs::read_to_string(root.join("note.md"))
                .unwrap()
                .contains("by hand")
        );
        assert!(run(&root, &["stash", "list"]).trim().is_empty());
        assert!(notes.sync_record().get("stash").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
