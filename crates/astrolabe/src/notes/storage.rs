//! Recoverable note writes and their commit scope. Callers hold the notes lock.
use super::{
    EOL_RULE, INTENTS, Notes, Op, RECENT_MS, RepoFacts, journal_file, tidy_journal, write_text,
};
use std::path::PathBuf;

impl Notes {
    /// Save's intent/reconcile machinery owns both commits. Commit trailers are phase
    /// markers too, covering a crash after git commits but before the intent is updated.
    fn apply_lesson(&self, op: &Op, lesson: &super::LessonClose) -> Result<(), String> {
        if self.already_applied(&op.key) {
            return Ok(());
        }
        let path = crate::record::lesson::safe_path(self, &lesson.path)?;
        let literal = format!(":(literal){}", lesson.path);
        let first_key = format!("disposition-{}", op.key);
        let first = self.applied_commit(&first_key);
        let have = std::fs::read_to_string(&path).ok();
        if first.is_none() {
            if have.as_deref() != Some(&lesson.original)
                && have.as_deref() != Some(&lesson.disposed)
            {
                return Err(
                    "the lesson changed before its disposition could be committed".to_owned(),
                );
            }
            super::write_text(&path, &lesson.disposed)?;
            self.git(&["add", "--", &literal])?;
            let message = format!(
                "Record disposition for lesson {}\n\nOp-Key: {first_key}",
                lesson.path
            );
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
        } else if have.as_deref().is_some_and(|text| text != lesson.disposed) {
            return Err("the lesson changed after its disposition was committed".to_owned());
        }
        // An absent file is the crash window after removal, before its commit.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let message = format!("Remove closed lesson {}\n\nOp-Key: {}", lesson.path, op.key);
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
            if op.result.is_some() {
                let age = std::fs::metadata(self.intent_path(&op.key))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok());
                if age.is_some_and(|a| a.as_millis() as i64 > RECENT_MS) {
                    let _ = std::fs::remove_file(self.intent_path(&op.key));
                }
                continue;
            }
            match self.apply(&op) {
                Ok(()) => self.finish(
                    &op.key,
                    "Finished by a later run: this write landed; the reply for it was lost.",
                ),
                Err(why) => failed.push(format!("{}: {why}", op.key)),
            }
        }
        failed
    }

    /// Records what the hook showed for this op; the caller removes the intent once the
    /// ticket exists.
    pub fn finish(&self, key: &str, result: &str) {
        let path = self.intent_path(key);
        if let Ok(bytes) = std::fs::read(&path)
            && let Ok(mut op) = serde_json::from_slice::<Op>(&bytes)
        {
            op.result = Some(result.to_owned());
            let _ = write_text(&path, &serde_json::to_string(&op).unwrap_or_default());
        }
        self.changed();
    }

    pub fn forget(&self, key: &str) {
        let _ = std::fs::remove_file(self.intent_path(key));
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

    /// Writes, journals, commits and records the session state, each step skipped when it's
    /// already done, so applying twice is applying once. Once committed, the files are never
    /// written again, only the state files: a newer save may have landed since.
    pub fn apply(&self, op: &Op) -> Result<(), String> {
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
            return self.write_states(&op);
        }
        if let Some(lesson) = &op.lesson_close {
            self.apply_lesson(&op, lesson)?;
            op.committed = true;
            self.save_intent(&op)?;
            return self.write_states(&op);
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
        // and a path git doesn't know would fail the commit. Hand edits under these paths
        // ride along on purpose; anything else the owner staged stays staged and out.
        let mut changed = Vec::new();
        for path in ["efforts", "journal", "initiatives", ".gitattributes"] {
            if self.root.join(path).exists()
                && !self
                    .git(&["status", "--porcelain", "--", path])?
                    .trim()
                    .is_empty()
            {
                changed.push(path);
            }
        }
        if !changed.is_empty() {
            let mut add = vec!["add", "-A", "--"];
            add.extend(&changed);
            self.git(&add)?;
            let mut message = format!("{}\n\nOp-Key: {}", op.message, op.key);
            if let Some(base) = &op.base {
                message += &format!("\nBase: {base}");
            }
            let mut commit = vec![
                "-c",
                "user.useConfigOnly=false",
                "commit",
                "--quiet",
                "--only",
                "-m",
                &message,
                "--",
            ];
            commit.extend(&changed);
            self.git(&commit)?;
        }
        op.committed = true;
        self.save_intent(&op)?;
        // A save of a file two machines wrote unseen resolves that warning.
        for (rel, _) in &op.files {
            self.clear_stale(rel);
        }
        self.write_states(&op)
    }

    /// The session states, once the notes are committed; from the notes checkout with the
    /// baseline as the tree is now.
    fn write_states(&self, op: &Op) -> Result<(), String> {
        let print = op
            .rebaseline
            .as_deref()
            .map(|cwd| RepoFacts::read(cwd).map(|f| f.fingerprint));
        for (path, state) in &op.states {
            let mut rebaseline = true;
            let mut state = if let Some(after) = op.reply_after_ms {
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
                    || current["delivered"].as_str() != op.base.as_deref()
                    || current["delivered_ms"].as_i64() != Some(after)
                {
                    continue;
                }
                rebaseline = current["baseline"] == state["baseline"];
                current["delivered"] = state["delivered"].clone();
                current["delivered_ms"] = state["delivered_ms"].clone();
                current
            } else {
                state.clone()
            };
            if rebaseline && let Some(print) = &print {
                state["baseline"] = print.clone().into();
            }
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
