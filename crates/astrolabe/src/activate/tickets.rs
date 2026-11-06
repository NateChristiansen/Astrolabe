//! Atomic hook replies and the read-only command that displays them.
use super::{now_ms, random_hex, state_dir};
use crate::hook::comparable;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) const TICKET_LIFE_MS: i64 = 60_000;

pub(super) const NO_TICKET: &str = "No effort loaded: the plugin's hook didn't take this command, so it can't be bound to this session. That alone doesn't tell whether a guard runs here, so claim no protection until `astrolabe activate --status` says proven. Run the command alone, exactly as written, with nothing before or after it. If it still fails: Astrolabe may be off for this machine (the owner runs `astrolabe on` in a terminal); on Codex, review and trust the plugin's hooks with /hooks; on Claude, check the plugin is enabled.";

/// Whole or not at all: written aside, then renamed.
pub(super) fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| e.to_string())?;
    crate::notes::write_text(path, &text)
}

/// What the hook did, left for the command to print. Old ones go as new ones come.
pub(super) fn leave(dir: &Path, text: &str, ok: bool) -> Result<String, String> {
    leave_with(dir, text, ok, None, None)
}

/// A ticket that also names the command it answers, so the plain command can find it.
pub(crate) fn leave_with(
    dir: &Path,
    text: &str,
    ok: bool,
    digest: Option<&str>,
    cwd: Option<&Path>,
) -> Result<String, String> {
    let shown = dir.join("shown");
    for entry in std::fs::read_dir(&shown).into_iter().flatten().flatten() {
        let age = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok()?.elapsed().ok());
        if age.is_some_and(|age| age.as_millis() as i64 > TICKET_LIFE_MS) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let ticket = random_hex()?;
    let cwd = cwd.map(comparable);
    let body = json!({"text": text, "ok": ok, "at_ms": now_ms(), "digest": digest, "cwd": cwd});
    write_json(&shown.join(format!("{ticket}.json")), &body)?;
    // The text alone too, for the read the app runs in place of the command.
    let printed = if text.ends_with('\n') {
        text.to_owned()
    } else {
        format!("{text}\n")
    };
    crate::notes::write_text(&ticket_text(&shown, &ticket), &printed)?;
    Ok(ticket)
}

/// The ticket's text file, which a plain `cat` prints.
pub(super) fn ticket_text(shown: &Path, ticket: &str) -> PathBuf {
    shown.join(format!("{ticket}.txt"))
}

/// A ticket's result replaced by a failure, in both the file the app prints and the one
/// the plain command looks up: a switch that failed after its ticket was left must not
/// print success.
pub(super) fn reissue(dir: &Path, ticket: &str, text: &str) -> Result<(), String> {
    let shown = dir.join("shown");
    let body = json!({"text": text, "ok": false, "at_ms": now_ms()});
    write_json(&shown.join(format!("{ticket}.json")), &body)?;
    crate::notes::write_text(&ticket_text(&shown, ticket), &format!("{text}\n"))
}

/// A path as one single-quoted word for the shell the app runs: nothing inside single
/// quotes is expanded, and a quote in the path is spelled the way that shell wants.
pub(super) fn quoted(path: &str, powershell: bool) -> String {
    if powershell {
        format!("'{}'", path.replace('\'', "''"))
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// The read the app runs: `cat` is in Claude's built-in read-only set; PowerShell's
/// `-LiteralPath` keeps `[` and `]` in a path from being read as wildcards, and `-Encoding
/// UTF8` keeps Windows PowerShell 5.1 from reading the UTF-8 text as ANSI.
pub(super) fn print_command(path: &str, powershell: bool) -> String {
    let path = quoted(path, powershell);
    if powershell {
        format!("Get-Content -Raw -Encoding UTF8 -LiteralPath {path}")
    } else {
        format!("cat {path}")
    }
}

/// The freshest ticket the hook left for a command with this digest, within `within_ms`.
pub(crate) fn shown_for(
    dir: &Path,
    digest: &str,
    cwd: &str,
    within_ms: i64,
) -> Option<(String, bool, i64)> {
    let mut best: Option<(i64, String, bool)> = None;
    for entry in std::fs::read_dir(dir.join("shown"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let at = body["at_ms"].as_i64().unwrap_or(0);
        if body["digest"] == digest
            && body["cwd"] == cwd
            && now_ms() - at <= within_ms
            && best.as_ref().is_none_or(|b| at > b.0)
        {
            best = Some((
                at,
                body["text"].as_str().unwrap_or_default().to_owned(),
                body["ok"] == true,
            ));
        }
    }
    best.map(|(at, text, ok)| (text, ok, now_ms() - at))
}

/// Only while fresh. Reads and nothing else, so it works inside a client's sandbox.
pub(super) fn show(dir: &Path, ticket: &str) -> Result<(String, bool), String> {
    let bad = || "that ticket isn't one the hook just issued".to_owned();
    if ticket.len() != 32 || !ticket.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let path = dir.join("shown").join(format!("{ticket}.json"));
    let bytes = std::fs::read(&path).map_err(|_| bad())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let fresh = (body["at_ms"].as_i64()).is_some_and(|at| now_ms() - at <= TICKET_LIFE_MS);
    let text = body["text"].as_str().filter(|_| fresh).ok_or_else(bad)?;
    Ok((text.to_owned(), body["ok"] == true))
}

/// `astrolabe activate --show <ticket>`, as the hook rewrote it: what to print, and whether
/// it worked. Anything else reaching here is a command no hook took.
pub fn command(args: &[&str]) -> (String, bool) {
    match (args, state_dir()) {
        (["--show", ticket], Some(dir)) => show(&dir, ticket)
            .unwrap_or_else(|why| (format!("No effort loaded: {why}. {NO_TICKET}"), false)),
        _ => (NO_TICKET.to_owned(), false),
    }
}

#[cfg(test)]
mod print_tests {
    use super::{print_command, quoted};

    #[test]
    fn a_path_with_a_quote_or_brackets_is_one_literal_word() {
        assert_eq!(quoted("/a b/c", false), "'/a b/c'");
        assert_eq!(quoted("/Users/O'Brien/x", false), "'/Users/O'\\''Brien/x'");
        assert_eq!(
            quoted("C:\\Users\\O'Brien\\x", true),
            "'C:\\Users\\O''Brien\\x'"
        );
        assert_eq!(
            print_command("/s/[work]/t.txt", false),
            "cat '/s/[work]/t.txt'"
        );
        assert_eq!(
            print_command("C:\\s\\[work]\\t.txt", true),
            "Get-Content -Raw -Encoding UTF8 -LiteralPath 'C:\\s\\[work]\\t.txt'"
        );
    }
}
