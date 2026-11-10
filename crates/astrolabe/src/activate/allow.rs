//! A quoted approval is an agent's claim, bound by the hook to one exact command.
use super::{now_ms, read_state, state_path};
use crate::{guard, hook};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) const HINT: &str = "If the owner approved this exact command, run `astrolabe allow --once \"<command>\" --said \"<their words>\"` alone, then run it again within five minutes.";
pub(super) const SUCCESS: &str = "Recorded the claimed owner approval. Run that exact command once in this session and working directory within five minutes.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    command: String,
    said: String,
    session: String,
    cwd: String,
    minted_ms: i64,
    expires_ms: i64,
}

pub(crate) fn covered(reason: &str) -> bool {
    matches!(
        reason,
        "force-push" | "a gh call that posts or changes something"
    )
}

fn lock(dir: &Path, session: &str) -> Result<std::fs::File, String> {
    let file = std::fs::File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state_path(dir, session).with_extension("allow.lock"))
        .map_err(|e| e.to_string())?;
    match crate::deadline::lock(&file).map_err(|e| e.to_string())? {
        crate::deadline::Wait::Held => Ok(file),
        crate::deadline::Wait::Busy(waited) => Err(format!(
            "this session's approval file was busy for the {} s this hook could wait",
            crate::deadline::seconds(waited)
        )),
    }
}

fn lease_path(dir: &Path, session: &str) -> PathBuf {
    state_path(dir, session).with_extension("allow.json")
}

fn read_leases(dir: &Path, session: &str) -> Result<Vec<Lease>, String> {
    match std::fs::read(lease_path(dir, session)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

// Lease writers hold the allow lock; ordinary session writers never touch this file.
fn persist(dir: &Path, session: &str, leases: &[Lease]) -> Result<(), String> {
    let path = lease_path(dir, session);
    super::tickets::write_json(&path, &leases)?;
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn log(dir: &Path, event: &str, lease: &Lease) -> Result<(), String> {
    let line = json!({"at_ms": now_ms(), "event": event, "command": lease.command,
        "said": lease.said, "session_id": lease.session, "cwd": lease.cwd});
    let mut file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(dir.join(hook::DENIAL_LOG))
        .map_err(|e| e.to_string())?;
    file.write_all(format!("{line}\n").as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}

/// Dropping the session's effort cancels the approvals it was waiting to use, as it always has.
pub(super) fn cancel(dir: &Path, session: &str) -> Result<(), String> {
    let _held = lock(dir, session)?;
    match std::fs::remove_file(lease_path(dir, session)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

pub(super) fn mint(
    dir: &Path,
    input: &Value,
    command: &str,
    session: &str,
    powershell: bool,
) -> Result<(), String> {
    let dialect = if powershell {
        guard::Dialect::Ps
    } else {
        guard::Dialect::Sh
    };
    let parsed = guard::literal_words(command.trim(), dialect)?;
    let [program, verb, once, exact, said, quote] = parsed.as_slice() else {
        return Err("use astrolabe allow --once \"<exact command>\" --said \"<the owner's words>\", run alone".into());
    };
    if !crate::effort::command::is_astrolabe(program)
        || verb != "allow"
        || once != "--once"
        || said != "--said"
        || quote.trim().is_empty()
    {
        return Err("use --once and a nonempty --said quote, run alone".into());
    }
    let target = guard::literal_words(exact, dialect)?;
    let cwd = hook::folder(input).ok_or("the hook didn't name a working directory")?;
    let scope = guard::Scope {
        root: None,
        cwd: Some(hook::comparable(&cwd)),
    };
    let reason = guard::decide(
        input["tool_name"].as_str().unwrap_or("Bash"),
        &json!({"command":exact}),
        &scope,
    )
    .ok_or("the guard wouldn't deny that command, so it needs no lease")?;
    if !covered(&reason) {
        return Err(format!(
            "a lease cannot cover {reason}; only force pushes and GitHub writes are supported"
        ));
    }
    // A nested shell could hide a second operation behind the first covered denial.
    // This command deliberately accepts direct git/gh invocations only.
    let program = target
        .first()
        .map(|p| p.rsplit(['/', '\\']).next().unwrap_or(p));
    if !matches!(program, Some("git" | "git.exe" | "gh" | "gh.exe")) {
        return Err(
            "the approved command must invoke git or gh directly, without a wrapper".into(),
        );
    }
    let _held = lock(dir, session)?;
    // No effort is fine; a state file this program can't read is not.
    read_state(dir, session)?;
    let mut leases = read_leases(dir, session)?;
    let at = now_ms();
    let lease = Lease {
        command: exact.clone(),
        said: quote.clone(),
        session: session.into(),
        cwd: hook::comparable(&cwd),
        minted_ms: at,
        expires_ms: at + 300_000,
    };
    log(dir, "lease-minted", &lease)?;
    leases.push(lease);
    persist(dir, session, &leases)?;
    Ok(())
}

pub(crate) fn consume(dir: &Path, input: &Value, reason: &str) -> Result<bool, String> {
    if !covered(reason) || input["agent_id"].as_str().is_some_and(|s| !s.is_empty()) {
        return Ok(false);
    }
    let Some(session) = input["session_id"].as_str() else {
        return Ok(false);
    };
    let Some(command) = guard::command_field(&input["tool_input"]).and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(cwd) = hook::folder(input) else {
        return Ok(false);
    };
    let _held = lock(dir, session)?;
    // The hook already denies a session whose state it can't read; this holds if that changes.
    read_state(dir, session)?;
    let mut leases = read_leases(dir, session)?;
    let now = now_ms();
    let Some(index) = leases.iter().position(|lease| {
        lease.session == session
            && lease.command == command
            && lease.cwd == hook::comparable(&cwd)
            && now >= lease.minted_ms
            && now < lease.expires_ms
    }) else {
        return Ok(false);
    };
    let lease = leases.remove(index);
    persist(dir, session, &leases)?;
    log(dir, "lease-used", &lease)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::mint;
    use serde_json::json;

    /// A command taken as `astrolabe allow` is recorded; one that runs another program is
    /// refused for its form.
    #[test]
    fn a_windows_path_and_the_exe_name_are_the_program() {
        let dir = std::env::temp_dir().join(format!("astrolabe-allow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = json!({"cwd": dir, "tool_name": "Bash"});
        let rest = "allow --once \"git push --force\" --said \"Yes, run that.\"";
        for (program, powershell) in [
            ("astrolabe", false),
            ("astrolabe.exe", false),
            ("/Users/x/.local/bin/astrolabe", false),
            ("C:/Users/x/.local/bin/astrolabe.exe", false),
            ("C:\\Users\\x\\.local\\bin\\astrolabe", true),
            ("C:\\Users\\x\\.local\\bin\\astrolabe.exe", true),
        ] {
            let command = format!("{program} {rest}");
            assert_eq!(
                mint(&dir, &input, &command, "s1", powershell),
                Ok(()),
                "{command}"
            );
        }
        for program in ["notastrolabe", "astrolabe.exe.bak", "/x/astrolabe2"] {
            let command = format!("{program} {rest}");
            assert_eq!(
                mint(&dir, &input, &command, "s1", false),
                Err("use --once and a nonempty --said quote, run alone".to_owned()),
                "{command}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
