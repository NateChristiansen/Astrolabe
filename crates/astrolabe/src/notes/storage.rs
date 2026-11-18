//! Recoverable note writes and their commit scope. Callers hold the notes lock.
use super::handoffs::{self, Handoffs, Left};
use super::{
    EOL_RULE, INTENTS, Notes, Op, RECENT_MS, RECOVERED, RETRY_MS, journal_file, now_ms,
    tidy_journal, write_text,
};
use std::path::PathBuf;

/// Where a write's path lists for git wait while git reads them, beside the intents.
const PATHS: &str = "paths";
/// Leaves out a write that died between being written aside and renamed into place.
const NOT_TEMP: &str = ":(exclude)*.tmp";

/// What a notes write commits, whoever changed it.
const COMMITTED: [&str; 6] = [
    "efforts",
    "journal",
    "initiatives",
    "lessons",
    "drafts",
    ".gitattributes",
];

/// `git status --porcelain -z` entries as status letters and a path from the notes folder.
/// Each entry is two status letters, a space and a path, and a rename or copy is followed by
/// the path it came from, given here with no letters. Git names paths from the top of its
/// repo, so a notes folder inside a bigger repo has `prefix` (as `rev-parse --show-prefix`
/// says it, ending in `/`) taken off first, and a path outside it is dropped.
fn entries<'a>(status: &'a str, prefix: &str) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    let mut raw = status.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = raw.next() {
        let (letters, path) = (entry.get(..2).unwrap_or(""), entry.get(3..).unwrap_or(""));
        out.extend(path.strip_prefix(prefix).map(|path| (letters, path)));
        if letters.contains(['R', 'C']) {
            out.extend(
                raw.next()
                    .and_then(|from| from.strip_prefix(prefix))
                    .map(|p| ("", p)),
            );
        }
    }
    out
}

/// The committed paths that status entries fall under, in `COMMITTED` order: a renamed
/// file's old path counts too.
fn changed_under(entries: &[(&str, &str)]) -> Vec<&'static str> {
    let under = |root: &str, path: &str| {
        path == root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    (COMMITTED.into_iter())
        .filter(|root| entries.iter().any(|(_, path)| under(root, path)))
        .collect()
}

/// The changed hand-off files a save owns that go in its commit, and those it leaves and
/// why. A deletion goes in when the file was committed before; a rename's old path, or a
/// file someone added to the index and then removed, is left alone.
fn handoffs_changed(
    root: &std::path::Path,
    entries: &[(&str, &str)],
    scope: &Handoffs,
) -> (Vec<String>, Vec<(String, Left)>) {
    let (mut take, mut left) = (Vec::new(), Vec::new());
    for (letters, path) in entries {
        if letters.is_empty() || !scope.owns(path) {
            continue;
        }
        let deleted = letters.contains('D');
        if deleted && letters.starts_with('A') {
            continue;
        }
        match handoffs::fits(root, path, deleted) {
            Ok(()) => take.push((*path).to_owned()),
            Err(Some(why)) => left.push(((*path).to_owned(), why)),
            Err(None) => {}
        }
    }
    (take, left)
}

/// A path for git to take as written, so a name with `*` or `[` in it isn't a pattern.
fn literal(path: &str) -> String {
    format!(":(literal){path}")
}

impl Notes {
    /// Save's intent/reconcile machinery owns both commits. Commit trailers are phase
    /// markers too, covering a crash after git commits but before the intent is updated.
    fn apply_lesson(&self, op: &Op, lesson: &super::LessonClose) -> Result<(), String> {
        self.close_lesson(lesson, &format!("disposition-{}", op.key), &op.key, false)
    }

    /// The lessons a done save closes, after its own commit. Each has its own two keys, never
    /// starting with the save's: a key is found by a fixed-string search of `Op-Key: <key>`.
    fn apply_closes(&self, op: &Op) -> Result<(), String> {
        for (i, lesson) in op.lesson_closes.iter().enumerate() {
            let first = format!("lesson-disposition-{i}-{}", op.key);
            let second = format!("lesson-remove-{i}-{}", op.key);
            self.close_lesson(lesson, &first, &second, true)?;
        }
        Ok(())
    }

    fn close_lesson(
        &self,
        lesson: &super::LessonClose,
        first_key: &str,
        key: &str,
        // A done save's close: its save already landed, so a lesson edited since it was
        // planned stays open rather than holding every later write back.
        lenient: bool,
    ) -> Result<(), String> {
        if self.already_applied(key) {
            return Ok(());
        }
        let path = crate::record::lesson::safe_path(self, &lesson.path)?;
        let literal = format!(":(literal){}", lesson.path);
        let first = self.applied_commit(first_key);
        let have = std::fs::read_to_string(&path).ok();
        if first.is_none() {
            // Gone with no disposition committed: removed by hand or by another close, so
            // there's nothing left to close, and failing here would fail every later write.
            let Some(have) = have else {
                return Ok(());
            };
            if have != lesson.original && have != lesson.disposed {
                if lenient {
                    return Ok(());
                }
                return Err(
                    "the lesson changed before its disposition could be committed".to_owned(),
                );
            }
            super::write_text(&path, &lesson.disposed)?;
            let message = format!(
                "Record disposition for lesson {}\n\nOp-Key: {first_key}",
                lesson.path
            );
            let committed = self.git(&["add", "--", &literal]).and_then(|_| {
                self.git(&[
                    "-c",
                    "user.useConfigOnly=false",
                    "commit",
                    "--quiet",
                    "--only",
                    "-m",
                    &message,
                    "--",
                    &literal,
                ])
            });
            if let Err(why) = committed {
                // The lesson as it was, unstaged: a failure leaves no disposition line behind.
                let _ = self.git(&["reset", "--quiet", "--", &literal]);
                let _ = super::write_text(&path, &have);
                return Err(why);
            }
        } else if have.as_deref().is_some_and(|text| text != lesson.disposed) {
            return Err("the lesson changed after its disposition was committed".to_owned());
        }
        // An absent file is the crash window after removal, before its commit.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let message = format!("Remove closed lesson {}\n\nOp-Key: {key}", lesson.path);
        self.git(&[
            "-c",
            "user.useConfigOnly=false",
            "commit",
            "--quiet",
            "--only",
            "-m",
            &message,
            "--",
            &literal,
        ])?;
        Ok(())
    }

    /// Whether a commit already carries this op's key: a retried tool call was done.
    pub fn already_applied(&self, key: &str) -> bool {
        self.applied_commit(key).is_some()
    }

    /// The commit that carries this op's key, if any.
    pub fn applied_commit(&self, key: &str) -> Option<String> {
        self.git(&[
            "log",
            "-1",
            "--format=%H",
            &format!("--grep=Op-Key: {key}"),
            "--fixed-strings",
        ])
        .ok()
        .map(|out| out.trim().to_owned())
        .filter(|sha| !sha.is_empty())
    }

    /// The effort file a commit added or changed, by number.
    pub fn effort_in_commit(&self, sha: &str) -> Option<u32> {
        let files = self.git(&["show", "--name-only", "--format=", sha]).ok()?;
        files
            .lines()
            .filter(|f| f.starts_with("efforts/"))
            .find_map(|f| f.rsplit('/').next()?.split('-').next()?.parse().ok())
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
        self.scan_intents().0
    }

    /// Intents that are there but can't be read or parsed, each as its path and why. They're
    /// never deleted or guessed at: an unfinished write may be in one, so it waits for repair.
    pub fn broken_intents(&self) -> Vec<String> {
        self.scan_intents().1
    }

    fn scan_intents(&self) -> (Vec<Op>, Vec<String>) {
        let dir = self.state.join(INTENTS);
        let (mut ops, mut broken) = (Vec::new(), Vec::new());
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (ops, broken),
            Err(e) => {
                broken.push(format!("{}: {e}", dir.display()));
                return (ops, broken);
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(e) => {
                    broken.push(format!("{}: {e}", dir.display()));
                    continue;
                }
            };
            // A `.tmp` beside it is a write that never got renamed into place: not an intent.
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let read = std::fs::read(&path).map_err(|e| e.to_string());
            match read.and_then(|b| serde_json::from_slice::<Op>(&b).map_err(|e| e.to_string())) {
                Ok(op) => ops.push(op),
                Err(why) => broken.push(format!("{}: {why}", path.display())),
            }
        }
        (ops, broken)
    }

    /// Finishes whatever an earlier run left, under the caller's lock. A finished op whose
    /// ticket never got written is dropped once it's old; nothing is applied twice. An intent
    /// that can't be read fails here every time, so nothing new lands over it until it's
    /// repaired.
    pub fn reconcile(&self) -> Vec<String> {
        let mut failed: Vec<String> = (self.broken_intents().into_iter())
            .map(|b| format!("{b} (kept for repair)"))
            .collect();
        for op in self.intents() {
            // Finished and its session told: nothing waits on it.
            if op.result.is_some() && self.is_answered(&op.key) {
                self.forget(&op.key);
                continue;
            }
            if op.result.is_some() {
                let age = std::fs::metadata(self.intent_path(&op.key))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok());
                // One whose session was never answered waits for that session's retry.
                let awaited =
                    self.untold(&op) && op.at_ms.is_some_and(|at| now_ms() - at <= RETRY_MS);
                if !awaited && age.is_some_and(|a| a.as_millis() as i64 > RECENT_MS) {
                    let _ = std::fs::remove_file(self.intent_path(&op.key));
                }
                continue;
            }
            // A renumber planned but not begun, its file still where it was: the note may
            // have been saved since, at that path, and replaying the plan would write the
            // text from before that save over it. The plan goes, and the renumber is made
            // again from the note as it is now.
            let begun = |(from, to): &(String, String)| {
                !self.root.join(from).exists() || self.root.join(to).exists()
            };
            if !op.committed && !op.renames.is_empty() && !op.renames.iter().any(begun) {
                let _ = std::fs::remove_file(self.intent_path(&op.key));
                self.fix_duplicate();
                continue;
            }
            // A crash after the commit but before the intent recorded it leaves the commit as
            // the only proof: its key says this write landed, so its files aren't written
            // again over whatever was saved since.
            let mut op = op;
            if !op.committed && op.lesson_close.is_none() && self.already_applied(&op.key) {
                op.committed = true;
            }
            match self.apply(&op) {
                Ok(()) => self.finish(&op.key, RECOVERED),
                Err(why) => failed.push(format!("{}: {why}", op.key)),
            }
        }
        // A mark whose intent is gone marks nothing.
        let marks = std::fs::read_dir(self.state.join(INTENTS));
        for mark in marks.into_iter().flatten().flatten().map(|m| m.path()) {
            if mark
                .extension()
                .is_some_and(|x| x == "answered" || x == "unanswered")
                && !mark.with_extension("json").exists()
            {
                let _ = std::fs::remove_file(mark);
            }
        }
        failed
    }

    /// Records what the hook showed for this op; the caller removes the intent once the
    /// session has its answer.
    pub fn finish(&self, key: &str, result: &str) {
        let path = self.intent_path(key);
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(mut op) = serde_json::from_slice::<Op>(&bytes)
        {
            op.result = Some(result.to_owned());
            let _ = write_text(&path, &serde_json::to_string(&op).unwrap_or_default());
            // Marked unanswered if this hook's own answer never goes out.
            crate::deadline::finished(self.unanswered_mark(key));
        }
        self.changed();
    }

    pub fn forget(&self, key: &str) {
        let _ = std::fs::remove_file(self.intent_path(key));
        let _ = std::fs::remove_file(self.answered_mark(key));
        let _ = std::fs::remove_file(self.unanswered_mark(key));
    }

    /// The mark beside a finished intent that says its session was not told: the backstop
    /// answered in its hook's place, or the hook failed. Made by `deadline`.
    fn unanswered_mark(&self, key: &str) -> PathBuf {
        self.state.join(INTENTS).join(format!("{key}.unanswered"))
    }

    /// Whether there is evidence this finished write's session was never told of it: a
    /// later run finished it, so no reply was ever made, or its hook's answer was the
    /// backstop's. A finished write with neither is taken as answered or about to be: its
    /// hook may be printing this moment, and the same command beside it is its own request.
    fn untold(&self, op: &Op) -> bool {
        op.result.as_deref() == Some(RECOVERED) || self.unanswered_mark(&op.key).exists()
    }

    /// The mark beside an intent that says its session was answered. Not `.json`, so no
    /// scan of the intents reads it.
    fn answered_mark(&self, key: &str) -> PathBuf {
        self.state.join(INTENTS).join(format!("{key}.answered"))
    }

    /// Records that the session was told about this write, at once and without the notes
    /// lock: one empty file, created whole or not at all. The hook does this as it answers,
    /// when a sync the write started is often holding the lock already; waiting for the
    /// lock there left the intent looking unanswered whenever the wait gave out. The intent
    /// itself is removed by whoever next holds the lock.
    pub fn mark_answered(&self, key: &str) {
        if self.intent_path(key).exists() {
            let mut mark = std::fs::File::options();
            let _ = (mark.write(true).create_new(true)).open(self.answered_mark(key));
        }
    }

    /// The bare command just printed the recorded result for this command in this folder,
    /// so whoever ran it has been told: every finished write it could have been showing is
    /// marked answered. Which session ran it isn't known here; marking errs toward a later
    /// identical command writing, never toward it being swallowed.
    pub fn mark_shown(&self, digest: &str, cwd: &str) {
        for op in self.intents() {
            if op.result.is_some()
                && op.digest.as_deref() == Some(digest)
                && op.cwd.as_deref() == Some(cwd)
            {
                self.mark_answered(&op.key);
            }
        }
    }

    pub fn is_answered(&self, key: &str) -> bool {
        self.answered_mark(key).exists()
    }

    /// A recent op for this command in this folder: finished (its result) or still going.
    pub fn intent_for(&self, digest: &str, cwd: &str) -> Option<Op> {
        self.intents().into_iter().find(|op| {
            let fresh = std::fs::metadata(self.intent_path(&op.key))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_millis() as i64 <= RECENT_MS);
            fresh && op.digest.as_deref() == Some(digest) && op.cwd.as_deref() == Some(cwd)
        })
    }

    /// The write a retry of this command repeats: one this session attempted with this exact
    /// command, in this folder, for the record it is on now, within `RETRY_MS`, that is
    /// finished, that its session is known not to have been told of (`untold`), and that no
    /// retry has been answered from since (`is_answered`). Anything else is a new request
    /// and writes: the same words after an answer, or beside a write still being answered.
    /// Every file the write left must still read as it left it, since a save since then
    /// makes the words new.
    pub fn earlier_attempt(
        &self,
        digest: &str,
        cwd: &str,
        session: &str,
        bound: Option<&str>,
    ) -> Option<Op> {
        self.intents().into_iter().find(|op| {
            let unchanged = op.files.iter().all(|(rel, text)| {
                std::fs::read_to_string(self.root.join(rel)).ok().as_deref() == Some(text.as_str())
            });
            self.untold(op)
                && !self.is_answered(&op.key)
                && op.at_ms.is_some_and(|at| now_ms() - at <= RETRY_MS)
                && op.digest.as_deref() == Some(digest)
                && op.cwd.as_deref() == Some(cwd)
                && op.session.as_deref() == Some(session)
                && op.bound.as_deref() == bound
                && unchanged
        })
    }

    /// Writes, journals, commits and records the session state, each step skipped when it's
    /// already done, so applying twice is applying once. Once committed, the files are never
    /// written again, only the state files: a newer save may have landed since.
    pub fn apply(&self, op: &Op) -> Result<(), String> {
        self.apply_noting(op).map(|_| ())
    }

    /// `apply`, with what the reply should say about hand-off files the commit left out.
    pub fn apply_noting(&self, op: &Op) -> Result<Vec<String>, String> {
        self.refuse_unmerged()?;
        let mut op = op.clone();
        if let Ok(bytes) = std::fs::read(self.intent_path(&op.key))
            && let Ok(saved) = serde_json::from_slice::<Op>(&bytes)
            && saved.committed
        {
            op.committed = true;
        }
        self.save_intent(&op)?;
        if op.committed {
            self.apply_closes(&op)?;
            return self.write_states(&op).map(|()| Vec::new());
        }
        if let Some(lesson) = &op.lesson_close {
            self.apply_lesson(&op, lesson)?;
            op.committed = true;
            self.save_intent(&op)?;
            return self.write_states(&op).map(|()| Vec::new());
        }
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
            let path = self.root.join(journal_file(&line.day));
            let have = std::fs::read_to_string(&path).unwrap_or_default();
            if have.contains(&format!("e:{} ", line.id)) {
                continue;
            }
            let mut text = String::new();
            if line.replace {
                // The effort's earlier line today gives way, whichever machine wrote it:
                // one line per effort per day, the latest. Other efforts' lines stay.
                let tag = format!("<!-- {} ", line.effort);
                for l in have.lines().filter(|l| !l.contains(&tag)) {
                    text.push_str(l);
                    text.push('\n');
                }
            } else {
                text = have;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            text.push_str(&line.rendered());
            text.push('\n');
            // A union merge can have left both sides of a replacement; the latest stays.
            write_text(&path, &tidy_journal(&text))?;
        }
        self.ensure_attributes()?;
        // Only what changed: a first effort with no journal line yet has no journal folder,
        // and a path git doesn't know would fail the commit. Git decides, not the disk: a
        // path it has never known reports nothing, while a folder removed outright still
        // reports the files it tracked there, and those deletions are committed like any
        // other change. Hand edits under these paths ride along on purpose; anything else
        // the owner staged stays staged and out. A `.tmp` is a write that died before its
        // rename: never part of a commit. Lessons and drafts are written as plain files, so
        // riding along is the only way one is ever committed and reaches another machine.
        // One status for all of them, each entry put under the path it falls in: a git
        // call costs far more than reading its answer, most of all on Windows.
        // Hand-off files the save owns ride along by name, each one text under the size
        // limit, so a brief written here reaches the other machine without hand git. Another
        // worker's file in the same folder isn't named, so it stays out even when staged.
        let mut status = vec!["status", "--porcelain", "-z", "--untracked-files=all", "--"];
        status.extend(COMMITTED);
        let scope = op.handoffs.as_ref();
        let folder = scope.map(|h| literal(&h.folder));
        status.extend(folder.as_deref());
        status.push(NOT_TEMP);
        let prefix = self.git(&["rev-parse", "--show-prefix"])?;
        let status = self.git(&status)?;
        let entries = entries(&status, prefix.trim_end_matches('\n'));
        let changed = changed_under(&entries);
        let (carried, left) = match scope {
            Some(scope) => handoffs_changed(&self.root, &entries, scope),
            None => (Vec::new(), Vec::new()),
        };
        let mut notes: Vec<String> = handoffs::left_line(&left).into_iter().collect();
        let mut carried: Vec<String> = carried.iter().map(|p| literal(p)).collect();
        if !carried.is_empty() {
            let specs: Vec<&str> = carried.iter().map(String::as_str).collect();
            if let Err(why) = self.git_paths(&op.key, &["add", "-A"], &specs) {
                // A hand-off file is never why a save fails: the effort goes in without them,
                // and what this add staged before it stopped is staged no more.
                let _ = self.git_paths(&op.key, &["reset", "-q"], &specs);
                notes.push(format!(
                    "Hand-off files weren't committed, so they stay on this machine: {}.",
                    why.trim().trim_end_matches('.')
                ));
                carried.clear();
            }
        }
        if !changed.is_empty() || !carried.is_empty() {
            // With no path named, `add -A` would take everything.
            if !changed.is_empty() {
                let mut add = vec!["add", "-A", "--"];
                add.extend(&changed);
                add.push(NOT_TEMP);
                self.git(&add)?;
            }
            let mut message = format!("{}\n\nOp-Key: {}", op.message, op.key);
            if let Some(base) = &op.base {
                message += &format!("\nBase: {base}");
            }
            let mut paths: Vec<&str> = changed.clone();
            paths.extend(carried.iter().map(String::as_str));
            paths.push(NOT_TEMP);
            let commit = [
                "-c",
                "user.useConfigOnly=false",
                "commit",
                "--quiet",
                "--only",
                "-m",
                &message,
            ];
            self.git_paths(&op.key, &commit, &paths)?;
        }
        op.committed = true;
        self.save_intent(&op)?;
        self.apply_closes(&op)?;
        self.write_states(&op)?;
        Ok(notes)
    }

    /// `git <args>` on `specs`, read from a file in the state folder and not the command line:
    /// Windows caps a command line at 32,767 characters, and a first save over a big hand-off
    /// folder can name more, which would fail that save on every replay. Not stdin, which a
    /// notes git call holds as its running marker. Needs git 2.26. A hook that runs out of time
    /// leaves git to finish and its file behind; one left an hour is cleared by a later call.
    fn git_paths(&self, key: &str, args: &[&str], specs: &[&str]) -> Result<String, String> {
        let dir = self.state.join(PATHS);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let old = (entry.metadata().and_then(|m| m.modified()).ok())
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 3600);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let file = dir.join(key);
        let list: String = specs.iter().map(|s| format!("{s}\0")).collect();
        std::fs::write(&file, list).map_err(|e| format!("{}: {e}", file.display()))?;
        let from = format!("--pathspec-from-file={}", file.display());
        let ran = self.git(&[args, &[from.as_str(), "--pathspec-file-nul"]].concat());
        let _ = std::fs::remove_file(&file);
        ran
    }

    /// The session states, once the notes are committed.
    fn write_states(&self, op: &Op) -> Result<(), String> {
        for (path, state) in &op.states {
            let state = if let Some(after) = op.reply_after_ms {
                // Other tools can mark work or switch efforts while git commits. A reply
                // patches the latest matching checkpoint, never restores an old snapshot.
                let Some(mut current) = std::fs::read(path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                else {
                    continue;
                };
                if current["session"] != state["session"]
                    || current["effort"] != state["effort"]
                    || current["binding"] != state["binding"]
                    || current["delivered"].as_str() != op.base.as_deref()
                    || current["delivered_ms"].as_i64() != Some(after)
                {
                    continue;
                }
                current["delivered"] = state["delivered"].clone();
                current["delivered_ms"] = state["delivered_ms"].clone();
                current
            } else {
                state.clone()
            };
            write_text(path, &state.to_string())?;
        }
        Ok(())
    }

    /// The journal merges by union, so two machines appending the same day never conflict.
    /// Text checks out with LF on every machine, so git for Windows (whose system config sets
    /// `core.autocrlf=true`) doesn't rewrite the files the program wrote.
    fn ensure_attributes(&self) -> Result<(), String> {
        let path = self.root.join(".gitattributes");
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        // The day files sit on month shelves, so the rule reaches every depth; an older
        // `journal/*.md` line is left as it is, harmless.
        let union = have
            .lines()
            .any(|l| l.trim_start().starts_with("journal/**") && l.contains("merge=union"));
        let lf = have
            .lines()
            .any(|l| l.trim_start().starts_with("* ") && l.contains("eol=lf"));
        if union && lf {
            return Ok(());
        }
        let mut text = have;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        if !lf {
            text.push_str(EOL_RULE);
        }
        if !union {
            text.push_str("journal/**/*.md merge=union\n");
        }
        write_text(&path, &text)
    }
}

#[cfg(test)]
mod tests {
    use super::{changed_under, entries};

    #[test]
    fn status_entries_are_put_under_the_committed_path_they_fall_in() {
        let status = " M efforts/2026/a.md\0?? journal/\0R  lessons/new.md\0drafts/old.md\0";
        assert_eq!(
            changed_under(&entries(status, "")),
            ["efforts", "journal", "lessons", "drafts"]
        );
        assert_eq!(
            changed_under(&entries(" M .gitattributes\0", "")),
            [".gitattributes"]
        );
        // A name that only starts like a committed folder isn't in it.
        assert!(changed_under(&entries("?? effortsish.md\0", "")).is_empty());
        assert!(changed_under(&entries("", "")).is_empty());
    }

    /// Notes in a folder of a bigger repo: git names paths from that repo's top.
    #[test]
    fn a_notes_folder_inside_a_bigger_repo_is_read_from_its_own_folder() {
        let status =
            "?? sub/efforts/\0R  sub/journal/b.md\0sub/journal/a.md\0 M other/efforts/x.md\0";
        assert_eq!(
            changed_under(&entries(status, "sub/")),
            ["efforts", "journal"]
        );
        assert!(changed_under(&entries(status, "")).is_empty());
    }
}
