//! Provider hook inputs, command dispatch and activation transitions.
use super::context::{Activation, WORKER_RULE, blocked, fresh, refused, switch_on};
use super::proof::{session_shells, status};
use super::tickets::{leave, leave_with, print_command, reissue, ticket_text, write_json};
use super::{EFFORT_VAR, machine_on, now_ms, random_hex, read_state, state_dir, state_path};
use crate::hook::{self};
use crate::session::SessionState;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Codex on Windows runs its shell commands in PowerShell, as Claude's PowerShell tool does.
pub(super) fn in_powershell(provider: &str, tool: &Value) -> bool {
    tool == "PowerShell" || (cfg!(windows) && provider == "codex")
}

/// What a switch changes, done only after its result is ready to show.
pub(super) enum Change {
    Keep,
    Write(Box<SessionState>),
    Remove,
}

pub(super) fn commit(dir: &Path, session: &str, change: &Change) -> Result<(), String> {
    let path = state_path(dir, session);
    if !matches!(change, Change::Keep) {
        let _ = std::fs::remove_file(fallback_block(dir, session));
    }
    match change {
        Change::Keep => Ok(()),
        Change::Write(state) => write_json(&path, state),
        Change::Remove => {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// `astrolabe activate <args>` for this session, run in its hook: what to show, and the
/// change to make once it's ready, with what to show instead if that fails.
pub(super) fn act(
    dir: &Path,
    args: &[&str],
    session: &str,
    provider: &str,
    cwd: &Path,
    powershell: bool,
) -> (String, bool, Change, String) {
    let previous = read_state(dir, session);
    match args {
        ["--status"] => (
            status(dir, session, powershell),
            true,
            Change::Keep,
            String::new(),
        ),
        ["--off"] => match previous {
            Ok(None) => ("This session has no effort set.".to_owned(), true, Change::Keep, String::new()),
            _ => (
                "This session has no effort now. The guard stays while Astrolabe is on for this machine.".to_owned(),
                true,
                Change::Remove,
                "Astrolabe could NOT drop this session's effort, so it stays as it was".to_owned(),
            ),
        },
        ["--effort", effort] | ["--effort", effort, "--worker"] => {
            let worker = args.len() == 3;
            let previous = previous.ok().flatten();
            // The published checkpoint, not a stale local copy: another machine may hold it.
            let refreshed = crate::record::refresh_notes();
            match switch_on(dir, effort, session, provider, cwd, previous.as_ref(), &[powershell]).map(Activation::mid_session) {
                Ok(mut activation) => {
                    let failed = refused(effort, "its state file couldn't be written", previous.as_ref());
                    if worker && !activation.state.worker.is_true() {
                        activation.state.worker.set(true);
                        activation.head += &format!("\n\n{WORKER_RULE}");
                    }
                    let mut text = activation.text();
                    if let Some(note) = refreshed {
                        text = after_first_paragraph(&text, &note);
                    }
                    (text, true, Change::Write(Box::new(activation.state)), failed)
                }
                Err(why) => (refused(effort, &why, previous.as_ref()), false, Change::Keep, String::new()),
            }
        }
        _ => ("Usage: astrolabe activate <--effort <n> [--worker]|--status|--off>, run alone so the hook can bind it.".to_owned(), false, Change::Keep, String::new()),
    }
}

/// A refresh note after the opening paragraph, never before it: that paragraph begins
/// "Astrolabe is on", the line a pickup checks before trusting the rest.
fn after_first_paragraph(text: &str, note: &str) -> String {
    match text.split_once("\n\n") {
        Some((first, rest)) => format!("{first}\n\n{note}\n\n{rest}"),
        None => format!("{text}\n\n{note}"),
    }
}

/// The shell tools a pre-command hook is sent for: Claude's two, and Codex's names for its
/// shell commands. Their input must carry the command.
pub(super) const SHELL_TOOLS: [&str; 6] = [
    "bash",
    "powershell",
    "shell",
    "shell_command",
    "exec_command",
    "local_shell",
];

/// Why a pre-command hook's input can't be read, if it can't: not a JSON object (empty,
/// truncated, or anything else), no tool named, a shell tool with no command, or a command
/// the guard would read that isn't text or a list of text. The command is looked for in the
/// field the guard judges, so the two can't disagree. Input for a tool the guard doesn't
/// check is readable and passes as before.
pub(super) fn unreadable(input: &Value) -> Option<&'static str> {
    if !input.is_object() {
        return Some("not a JSON object");
    }
    let Some(tool) = input["tool_name"].as_str().filter(|t| !t.is_empty()) else {
        return Some("no tool named");
    };
    // The guard's own exceptions: their `command` isn't a shell's.
    if tool.starts_with("mcp__") || tool == "apply_patch" {
        return None;
    }
    let shell = is_shell_tool(tool);
    match crate::guard::command_field(&input["tool_input"]) {
        None if shell => Some("no command for its shell tool"),
        Some(command) if !crate::guard::readable_command(command) => {
            Some("a command that isn't text or a list of text")
        }
        _ => None,
    }
}

pub(super) fn is_shell_tool(tool: &str) -> bool {
    SHELL_TOOLS.iter().any(|s| tool.eq_ignore_ascii_case(s))
}

/// The tool an input names, read from its text when it was too big to parse whole: the first
/// `"tool_name": "<name>"`, as the apps put it ahead of the tool's input.
pub(super) fn tool_named_in(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(1 << 16)]);
    let rest = text.split_once("\"tool_name\"")?.1.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let (name, _) = rest.split_once('"')?;
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c));
    (!name.is_empty() && plain).then(|| name.to_owned())
}

/// `astrolabe hook <event> --if-active`. Silent in a session that's off, except for its own
/// switch. Always exits 0, like the hooks it reuses.
pub fn gate(event: &str) {
    std::panic::set_hook(Box::new(|_| {}));
    let (bytes, over) = hook::read_stdin();
    let input: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let session = input["session_id"].as_str().filter(|s| !s.is_empty());
    // A child agent shares its parent's session id. It gets the guard, as every session does
    // while the machine is on, but no context or authority, can't switch, and doesn't count
    // toward the parent's proof.
    let child = input["agent_id"].as_str().is_some_and(|a| !a.is_empty());
    // Off for the machine: nothing at all, as if the plugin weren't there.
    if !state_dir().is_some_and(|dir| machine_on(&dir)) {
        return;
    }
    match event {
        "pre-command" => {
            // Too big to read whole: a tool the guard doesn't check still runs, as it would
            // have; anything else can't be checked.
            let named = over.then(|| tool_named_in(&bytes)).flatten();
            if named.as_deref().is_some_and(|t| !is_shell_tool(t)) {
                return;
            }
            // Not recorded as a denial, so it counts toward no session's proof.
            let what = match over {
                true => {
                    Some("more than the hook reads whole, for a shell tool or one it couldn't name")
                }
                false => unreadable(&input),
            };
            if let Some(what) = what {
                return hook::say(&hook::deny(&format!(
                    "Astrolabe couldn't read what the app sent its hook ({what}), so it can't tell which session this is or check the command, and it blocks the command rather than let it run unguarded. If this keeps happening, the app's hook input has changed or the plugin is broken; the owner can run `astrolabe off` in a terminal to stop Astrolabe on this machine until it's fixed."
                )));
            }
            let answer = std::panic::catch_unwind(|| switch_command(&input, session, child));
            if let Ok(Some(answer)) = answer {
                return hook::say(&answer);
            }
            // A commit is work the save should cover; a child's counts for its parent.
            if let (Some(session), Some(command)) =
                (session, crate::guard::command_field(&input["tool_input"]))
                && crate::nudge::commits(&crate::guard::command_text(command))
            {
                let _ = std::panic::catch_unwind(|| crate::nudge::mark(session, false));
            }
            // The plugin's denials go in its state folder, which every app reads the same.
            let dir = state_dir();
            let guard = || hook::guard(&bytes, dir.as_deref());
            let (Some(session), Some(dir)) = (session, dir.as_deref()) else {
                return guard();
            };
            match read_state(dir, session) {
                Ok(None) => guard(),
                Ok(Some(state)) => match block_reason(dir, &state, session) {
                    Some(why) => hook::say(&hook::deny(&blocked(
                        state.effort.as_deref().unwrap_or_default(),
                        &why,
                    ))),
                    None => guard(),
                },
                Err(why) => hook::say(&hook::deny(&format!(
                    "Astrolabe is on for this session but can't read its state ({why}), so it can't judge this command. Repair: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
                ))),
            }
        }
        "post-tool" => {
            let tool = input["tool_name"].as_str().unwrap_or_default();
            if let (Some(session), true) = (session, crate::nudge::EDIT_TOOLS.contains(&tool)) {
                let plan = crate::nudge::plan_edit(&input);
                let _ = std::panic::catch_unwind(|| crate::nudge::mark(session, plan));
            }
        }
        "stop" if !child => {
            let answer = std::panic::catch_unwind(|| crate::nudge::at_stop(&input, session?));
            if let Ok(Some(answer)) = answer {
                hook::say(&answer);
            }
        }
        "session-end" if !child => {
            if let Some(session) = session {
                let _ = std::panic::catch_unwind(|| crate::nudge::at_end(session));
            }
        }
        "session-start" if !child => {
            // Anything an earlier session left unpublished goes now, in the background.
            let _ = std::panic::catch_unwind(crate::nudge::push_pending);
            // And the home is up, so the owner's board is there before they look.
            let _ = std::panic::catch_unwind(|| {
                if let Ok((_, notes)) = crate::record::notes_here() {
                    crate::home::ensure_serving(&notes);
                }
            });
            let text = std::panic::catch_unwind(|| session_start(&input, session?));
            if let Ok(Some(text)) = text {
                let inner = json!({"hookEventName": "SessionStart", "additionalContext": text});
                hook::say(&json!({ "hookSpecificOutput": inner }));
            }
        }
        _ => {}
    }
}

pub(crate) fn provider(input: &Value) -> &'static str {
    // Codex adds a turn id to tool hooks and names its transcripts `rollout-…`; Claude does neither.
    let transcript = input["transcript_path"].as_str().unwrap_or_default();
    let name = Path::new(transcript).file_name().and_then(|n| n.to_str());
    let codex = input.get("turn_id").is_some() || name.is_some_and(|n| n.starts_with("rollout-"));
    if codex { "codex" } else { "claude" }
}

/// The hook's answer to `astrolabe activate …`: done here, for this session, and the command
/// rewritten to print it. Only a plain command is taken, so it's exactly what the hook read.
pub(super) fn switch_command(input: &Value, session: Option<&str>, child: bool) -> Option<Value> {
    let tool_input = &input["tool_input"];
    let key = if tool_input.get("command").is_some() {
        "command"
    } else {
        "cmd"
    };
    let command = tool_input[key].as_str()?.trim();
    let words: Vec<&str> = command.split_whitespace().collect();
    if matches!(words.as_slice(), ["astrolabe", "on" | "off", ..]) {
        return Some(hook::deny(
            "The machine switch is the owner's: they run `astrolabe on` or `astrolabe off` in a terminal, because it changes every session. To drop this session's effort, run `astrolabe activate --off`.",
        ));
    }
    // An effort command is done here, in the hook, bound to the session the app named, and
    // the app runs only a print of the result (see `record`).
    if crate::effort::EffortCommand::is_effort_command(command) {
        let session = session?;
        if child {
            return Some(hook::deny(
                "A subagent can't create or save an effort: one session writes an effort's record, the one the owner talks to. Report to it instead.",
            ));
        }
        // A worker was dispatched by another agent under its effort: it writes nothing to
        // the notes, because last night's thirteen lane records were read by no one and the
        // dispatcher reports for the lot (owner, 2026-11-04).
        if let Some(dir) = state_dir()
            && let Ok(Some(state)) = read_state(&dir, session)
            && state.worker.is_true()
        {
            return Some(hook::deny(&format!(
                "This session is a worker under effort {}: it creates, saves and journals nothing; the session that dispatched it writes the notes. Report to it instead.",
                state.effort.as_deref().unwrap_or_default()
            )));
        }
        let powershell = in_powershell(provider(input), &input["tool_name"]);
        let cwd = input["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let (text, ok, digest, op_key) = match crate::effort::EffortCommand::parse(command) {
            Ok(parsed) => {
                // The same tool call retried gets the same key, so it can't commit twice.
                let op_key = match input["tool_use_id"].as_str().filter(|id| !id.is_empty()) {
                    Some(id) => crate::notes::hash(&format!("{session} {id} {}", parsed.digest())),
                    None => random_hex().unwrap_or_else(|_| parsed.digest()),
                };
                let done = match parsed.verb.as_str() {
                    "new" => crate::record::new(
                        &parsed,
                        &op_key,
                        session,
                        provider(input),
                        &cwd,
                        &[powershell],
                    ),
                    "journal" => crate::record::journal(&parsed, &op_key, session, &cwd),
                    _ => crate::record::save(&parsed, &op_key, session, provider(input), &cwd),
                };
                (done.text, done.ok, Some(parsed.digest()), Some(op_key))
            }
            Err(why) => (
                format!("Effort command refused: {why}.\n{}", crate::effort::FORMS),
                false,
                None,
                None,
            ),
        };
        let left = state_dir()
            .ok_or_else(|| "no home folder".to_owned())
            .and_then(|dir| leave_with(&dir, &text, ok, digest.as_deref(), Some(&cwd)));
        return Some(match left {
            Ok(ticket) => {
                // The ticket now holds the result, so the intent that held it can go. A
                // failed one keeps its intent, which is its recovery plan.
                if let (Some(op_key), true) = (op_key, ok) {
                    crate::record::forget(&op_key);
                }
                rewrite_to_show(tool_input, key, &ticket, powershell)
            }
            Err(why) => hook::deny(&format!(
                "Astrolabe couldn't show this effort command's result ({why}); run `astrolabe notes status`."
            )),
        });
    }
    if words.get(..2) != Some(&["astrolabe", "activate"]) {
        return None;
    }
    if !(command.chars()).all(|c| c.is_ascii_alphanumeric() || " -_.".contains(c)) {
        return None;
    }
    if words.contains(&"--show") {
        return Some(hook::deny(
            "Astrolabe's tickets come only from its hook, so a typed one is refused. Run the command without --show.",
        ));
    }
    if child {
        return Some(hook::deny(
            "A subagent can't switch Astrolabe on or off: the switch is the parent session's. A subagent of a switched-on session gets the guard only.",
        ));
    }
    let session = session?;
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    let powershell = in_powershell(provider(input), &input["tool_name"]);
    let left = state_dir()
        .ok_or_else(|| "no home folder".to_owned())
        .and_then(|dir| {
            let (text, ok, change, failed) = act(
                &dir,
                &words[2..],
                session,
                provider(input),
                &cwd,
                powershell,
            );
            // The result first: a switch whose result can't be shown isn't made.
            let ticket = leave(&dir, &text, ok)?;
            if let Err(why) = commit(&dir, session, &change) {
                let text = format!("{failed}: {why}.");
                reissue(&dir, &ticket, &text).map_err(|_| text.clone())?;
            }
            Ok(ticket)
        });
    Some(match left {
        Ok(ticket) => rewrite_to_show(tool_input, key, &ticket, powershell),
        Err(why) => hook::deny(&format!(
            "Astrolabe couldn't finish this ({why}). Run `astrolabe activate --status` to see where this session stands; claim no new protection."
        )),
    })
}

/// The command the app runs instead: a plain read of the ticket's text. A read-only command
/// runs in every mode, where Claude's plan mode ignores a hook's allow for anything else
/// and its classifier denied this program's own print; the hook has written by then, so a
/// denied print would hide a write that happened. This program's `--show` is the fallback
/// when there's no state folder to name.
pub(super) fn rewrite_to_show(
    tool_input: &Value,
    key: &str,
    ticket: &str,
    powershell: bool,
) -> Value {
    let text = state_dir().map(|dir| ticket_text(&dir.join("shown"), ticket));
    let text = text.map(|p| p.display().to_string());
    let print = match text {
        Some(path) => print_command(&path, powershell),
        None => {
            // This program by its full path: a login shell's PATH can find another copy.
            let exe = std::env::current_exe().map(|exe| exe.display().to_string());
            let exe = exe.ok().filter(|exe| !exe.contains('\''));
            let exe = exe.map_or("astrolabe".to_owned(), |exe| format!("'{exe}'"));
            // PowerShell reads a quoted path followed by arguments as an error, so call it.
            let call = if powershell { "& " } else { "" };
            format!("{call}{exe} activate --show {ticket}")
        }
    };
    let mut updated = tool_input.clone();
    updated[key] = print.into();
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "updatedInput": updated,
    }})
}

/// Where a failed reload's block goes when the state file can't take it: the temp folder.
/// Named for the state folder too, as another folder's sessions are another switch.
pub(super) fn fallback_block(dir: &Path, session: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let folder = Sha256::digest(dir.display().to_string());
    let folder: String = folder.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let name = state_path(Path::new(""), session);
    std::env::temp_dir().join(format!("astrolabe-blocked-{folder}-{}", name.display()))
}

/// Why this session's shell commands are blocked, if a reload failed.
pub(super) fn block_reason(dir: &Path, state: &SessionState, session: &str) -> Option<String> {
    if let Some(why) = state.reload_failed.as_deref() {
        return Some(why.to_owned());
    }
    let bytes = std::fs::read(fallback_block(dir, session)).ok()?;
    let body: Value = serde_json::from_slice(&bytes).ok()?;
    (body["effort"].as_str() == state.effort.as_deref())
        .then(|| body["why"].as_str().unwrap_or("unknown").to_owned())
}

/// A resumed session keeps its switch with fresh context; a launch may switch on by
/// `ASTROLABE_EFFORT`; any other session stays silent.
pub(super) fn session_start(input: &Value, session: &str) -> Option<String> {
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let cwd = cwd.or_else(|| std::env::current_dir().ok())?;
    let launched = std::env::var(EFFORT_VAR)
        .ok()
        .filter(|e| !e.trim().is_empty());
    let Some(dir) = state_dir() else {
        return launched.map(|e| refused(&e, "no home folder for its state", None));
    };
    let state = read_state(&dir, session);
    // Another machine may have saved since: the published checkpoint, not a stale copy. The
    // refresh may renumber this session's effort, so the state is read again after it.
    let refreshed = match &state {
        Ok(Some(_)) => crate::record::refresh_notes(),
        _ => None,
    };
    let state = match state {
        Ok(Some(_)) => read_state(&dir, session),
        other => other,
    };
    match state {
        Ok(Some(state)) => {
            let effort = state.effort.as_deref().unwrap_or_default().to_owned();
            let provider = state.provider.as_deref().unwrap_or("claude").to_owned();
            // No tool at session start: the provider's usual shell.
            let shells = session_shells(&provider);
            Some(
                match switch_on(
                    &dir,
                    &effort,
                    session,
                    &provider,
                    &cwd,
                    Some(&state),
                    &shells,
                ) {
                    Ok(activation) => {
                        match commit(
                            &dir,
                            session,
                            &Change::Write(Box::new(activation.state.clone())),
                        ) {
                            Ok(()) => match refreshed {
                                Some(note) => after_first_paragraph(&activation.fitted(), &note),
                                None => activation.fitted(),
                            },
                            Err(why) => format!(
                                "Astrolabe stays on for effort {effort}, guard included, but couldn't save its reload: {why}. Tell the owner."
                            ),
                        }
                    }
                    Err(why) => {
                        // Context failed, protection mustn't: stay on, and block until repaired.
                        let mut state = state.clone();
                        state.reload_failed.set(why.clone());
                        let saved = write_json(&state_path(&dir, session), &state);
                        let body = json!({"effort": effort, "why": why, "at_ms": now_ms()});
                        match saved.or_else(|e| {
                            write_json(&fallback_block(&dir, session), &body)
                                .map_err(|f| format!("{e}; {f}"))
                        }) {
                            Ok(()) => blocked(&effort, &why),
                            Err(e) => format!(
                                "Astrolabe stays on for effort {effort} in this session, guard included, but its context couldn't be reloaded: {why}. It also couldn't record a block on shell commands ({e}), so only the guard is on and other commands are NOT blocked. Treat this session's effort context as missing: tell the owner, repair it and run `astrolabe activate --effort {effort}`, or drop the effort with `astrolabe activate --off` (the guard keeps running)."
                            ),
                        }
                    }
                },
            )
        }
        Ok(None) => {
            let Some(effort) = launched else {
                return Some(fresh(&cwd));
            };
            let shells = session_shells(provider(input));
            Some(
                match switch_on(&dir, &effort, session, provider(input), &cwd, None, &shells) {
                    Ok(activation) => {
                        match write_json(&state_path(&dir, session), &activation.state) {
                            Ok(()) => activation.fitted(),
                            Err(why) => refused(
                                &effort,
                                &format!("its state file couldn't be written: {why}"),
                                None,
                            ),
                        }
                    }
                    Err(why) => refused(&effort, &format!("{why} (from {EFFORT_VAR})"), None),
                },
            )
        }
        Err(why) => Some(format!(
            "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
        )),
    }
}
