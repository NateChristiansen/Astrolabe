//! Closing a lesson is a hook-owned write, with two commits in one recoverable intent.
use crate::effort::command::{Words, astrolabe_args, is_astrolabe, simple_commands};
use crate::notes::{LessonClose, Notes, Op, Pull};
use std::path::{Component, Path};

pub const FORM: &str = "astrolabe lesson close lessons/<name>.md --as \"<disposition>\"";

pub(crate) fn claims(command: &str) -> bool {
    simple_commands(command).iter().any(|words| {
        astrolabe_args(words).is_some_and(|args| match args {
            [verb, ..] if verb == "lesson" => true,
            [verb, sub, ..] => verb == "notes" && sub == "sync",
            _ => false,
        })
    })
}

/// A deliberately small grammar, shared by the POSIX and PowerShell hook inputs.
pub(crate) fn parse(command: &str) -> Result<Vec<String>, String> {
    if command.trim().contains(['\n', '\r']) {
        return Err("Run lesson closing and notes sync alone on one line".to_owned());
    }
    let mut words = Words::new(command);
    let mut args = Vec::new();
    args.extend(words.program()?);
    while let Some(word) = words.next_word()? {
        args.push(word);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match refs.as_slice() {
        [program, "notes", "sync"] if is_astrolabe(program) => Ok(args),
        [program, "lesson", "close", path, "--as", disposition] if is_astrolabe(program) => {
            validate_path(path)?;
            validate_disposition(disposition)?;
            Ok(args)
        }
        _ => Err(format!("Run {FORM}, or astrolabe notes sync, alone")),
    }
}

fn validate_disposition(text: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.contains(['\r', '\n']) {
        return Err("A lesson disposition must be one nonempty line".to_owned());
    }
    Ok(())
}

fn validate_path(rel: &str) -> Result<(), String> {
    let path = Path::new(rel);
    if !rel.starts_with("lessons/")
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || rel.contains('\\')
    {
        return Err("The lesson must be inside the notes lessons folder".to_owned());
    }
    if path.extension().is_none_or(|ext| ext != "md") {
        return Err("The lesson must be a Markdown file ending in .md".to_owned());
    }
    Ok(())
}

/// Rechecked during replay too: a symlink introduced since the intent must not escape.
pub(crate) fn safe_path(notes: &Notes, rel: &str) -> Result<std::path::PathBuf, String> {
    validate_path(rel)?;
    let root = notes.root.canonicalize().map_err(|e| e.to_string())?;
    let path = root.join(rel);
    let mut part = root.clone();
    for component in Path::new(rel).components() {
        part.push(component);
        if std::fs::symlink_metadata(&part).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("The lesson path must not contain a symbolic link".to_owned());
        }
    }
    Ok(path)
}

pub(crate) fn close(
    rel: &str,
    disposition: &str,
    key: &str,
    digest: &str,
    cwd: &Path,
) -> super::Done {
    match close_inner(rel, disposition, key, digest, cwd) {
        Ok(text) => super::Done { text, ok: true },
        Err(why) => super::Done {
            text: format!("Lesson not closed: {}.", why.trim_end_matches('.')),
            ok: false,
        },
    }
}

fn close_inner(
    rel: &str,
    disposition: &str,
    key: &str,
    digest: &str,
    cwd: &Path,
) -> Result<String, String> {
    validate_path(rel)?;
    validate_disposition(disposition)?;
    let (_, notes) = super::notes_here()?;
    let held = notes.lock()?;
    let existing = notes.intents().into_iter().find(|op| {
        op.key == key
            || (op.result.is_none()
                && op.digest.as_deref() == Some(digest)
                && op.cwd.as_deref() == Some(crate::hook::comparable(cwd).as_str()))
    });
    let recovered_key = existing.as_ref().map(|op| op.key.clone());
    let key = recovered_key.as_deref().unwrap_or(key);
    if existing.is_none() && !notes.already_applied(key) {
        let path = safe_path(&notes, rel)?;
        std::fs::read_to_string(path).map_err(|_| "The lesson file is missing or unreadable")?;
    }
    // Validate before recovery or pull: malformed requests and an unmerged checkout
    // change nothing. Finish prior writes before moving their commits, as saves do.
    notes.refuse_unmerged()?;
    let failures = crate::application::reconcile(&notes);
    if !failures.is_empty() {
        return Err(format!(
            "Earlier notes writes need repair: {}",
            failures.join("; ")
        ));
    }
    let pulled = crate::application::pull(&notes);
    if !notes.already_applied(key) {
        // Use the refreshed lesson, including changes published by the other machine.
        let path = safe_path(&notes, rel)?;
        let original = std::fs::read_to_string(path)
            .map_err(|_| "The lesson file is missing or unreadable")?;
        let mut disposed = original.clone();
        if !disposed.is_empty() && !disposed.ends_with('\n') {
            disposed.push('\n');
        }
        disposed.push_str(disposition.trim());
        disposed.push('\n');
        let op = Op {
            reply_after_ms: None,
            key: key.to_owned(),
            lesson_close: Some(LessonClose {
                path: rel.to_owned(),
                original,
                disposed,
            }),
            files: Vec::new(),
            renames: Vec::new(),
            journal: Vec::new(),
            message: format!("Close lesson {rel}"),
            base: None,
            states: Vec::new(),
            committed: false,
            digest: Some(digest.to_owned()),
            result: None,
            cwd: Some(crate::hook::comparable(cwd)),
            session: None,
            bound: None,
            at_ms: None,
            rebaseline: None,
        };
        notes.apply(&op)?;
    }
    drop(held);
    let sync = if matches!(&pulled, Pull::Offline(why) if why == "no remote") {
        "sync unavailable: no remote configured"
    } else if notes.spawn_sync() {
        "background sync started"
    } else {
        "background sync could not start"
    };
    let name = rel.strip_prefix("lessons/").unwrap_or(rel);
    let mut text = format!("Closed lesson {name}: disposition committed, file removed; {sync}");
    if matches!(pulled, Pull::Diverged(_)) {
        text += ". Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    let _held = notes.lock()?;
    crate::application::finish(&notes, key, &text);
    Ok(text)
}

/// Without interception this reads a recent reply, and never writes notes.
pub fn plain(args: &[&str]) -> (String, bool) {
    if let ["close", path, "--as", disposition] = args {
        let digest = crate::notes::hash(&format!("lesson\0{path}\0{disposition}"));
        if let (Some(dir), Ok(cwd)) = (crate::activate::state_dir(), std::env::current_dir()) {
            if let Some((text, ok, _)) = crate::activate::shown_for(
                &dir,
                &digest,
                &crate::hook::comparable(&cwd),
                crate::notes::RECENT_MS,
            ) {
                return (
                    format!(
                        "{text}\n\nThis is the hook's recent result for this command; this invocation closed nothing."
                    ),
                    ok,
                );
            }
            let notes = Notes::new(Default::default(), dir);
            if let Some(op) = notes.intent_for(&digest, &crate::hook::comparable(&cwd)) {
                return match op.result {
                    Some(text) => (format!("{text}\n\nRecovered the hook's reply; this invocation closed nothing."), true),
                    None => ("The lesson close may still be finishing in the hook; this invocation closed nothing; check astrolabe notes status.".to_owned(), false),
                };
            }
        }
    }
    (
        format!("Nothing closed: the plugin's hook didn't take this command; run {FORM} alone."),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{claims, parse};

    #[test]
    fn a_windows_path_and_the_exe_name_are_the_program() {
        for program in [
            "astrolabe",
            "astrolabe.exe",
            "/Users/x/.local/bin/astrolabe",
            "C:\\Users\\x\\.local\\bin\\astrolabe",
            "C:\\Users\\x\\.local\\bin\\astrolabe.exe",
            "& 'C:\\Users\\x\\.local\\bin\\astrolabe.exe'",
        ] {
            let sync = format!("{program} notes sync");
            assert!(claims(&sync), "{sync}");
            let args = parse(&sync).unwrap_or_else(|why| panic!("{sync}: {why}"));
            assert_eq!(args[1..], ["notes", "sync"]);
            let close = format!("{program} lesson close lessons/a.md --as \"kept as a rule\"");
            let args = parse(&close).unwrap_or_else(|why| panic!("{close}: {why}"));
            assert_eq!(args[3..], ["lessons/a.md", "--as", "kept as a rule"]);
        }
        for program in ["notastrolabe", "astrolabe.exe.bak", "C:\\x\\astrolabe2"] {
            assert!(
                parse(&format!("{program} notes sync")).is_err(),
                "{program}"
            );
        }
    }
}
