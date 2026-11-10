//! Per-session guard evidence and the commands that establish it.
use super::context::GUARD_SCOPE;
use super::routing::{block_reason, provider};
use super::{SWITCH, hook_id, read_state, state_dir};
use crate::hook::{self};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) const PROBE_NAME: &str = "astrolabe-guard-probe";

/// The marker goes in the temp folder, which a sandboxed shell can still write, so a probe that
/// got past the hook leaves a trace.
/// Where probe and control markers go: the temp folder, the one place Codex's sandbox lets
/// a shell write on both systems (tested: not the state folder, on the Mac or under Windows'
/// sandbox). Made here, so a shell's `touch` finds it.
pub fn probe_dir() -> PathBuf {
    // Named for the state folder, so two installs (or two test fixtures) on one machine
    // never read each other's markers.
    let state = state_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let key: String = crate::notes::hash(&state).chars().take(8).collect();
    let dir = std::env::temp_dir().join(format!("astrolabe-probe-{key}"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub(super) fn marker(token: &str) -> PathBuf {
    probe_dir().join(token)
}

/// Made by an allowed command in the same shell, so a missing probe marker means the deny
/// held, not that the shell couldn't write there. One per shell.
pub(super) fn control_marker(token: &str, powershell: bool) -> PathBuf {
    let shell = if powershell { "powershell" } else { "posix" };
    probe_dir().join(format!("{token}-control-{shell}"))
}

pub(super) fn make_file(path: &Path, powershell: bool) -> String {
    let path = path.display();
    if powershell {
        format!("New-Item -ItemType File -Force '{path}'")
    } else {
        format!("touch '{path}'")
    }
}

/// Whether a hook input's command is PowerShell, by the guard's one rule.
pub(crate) fn powershell_input(input: &Value) -> bool {
    let tool = input["tool_name"].as_str().unwrap_or_default();
    crate::guard::powershell(tool, provider(input) == "codex")
}

/// The shell a hook input's command runs in, as the denial log records it.
pub fn shell_of(input: &Value) -> &'static str {
    if powershell_input(input) {
        "powershell"
    } else {
        "posix"
    }
}

/// What the session runs to prove its guard, in its own shell.
pub(super) fn proof_steps(token: &str, shells: &[bool]) -> String {
    let steps = |powershell: bool| {
        format!(
            "First an allowed control, which must run: {}\nThen the probe, which the hook must deny: {}\n",
            make_file(&control_marker(token, powershell), powershell),
            probe(token, powershell)
        )
    };
    let body = match shells {
        [one] => steps(*one),
        _ => format!(
            "In the shell you use. For the POSIX shell (Bash, zsh):\n{}For PowerShell:\n{}",
            steps(false),
            steps(true)
        ),
    };
    format!(
        "Prove the guard for this session, each command alone and exactly as written. {body}Then run `astrolabe activate --status`."
    )
}

/// The shells a session may use, for proof steps given at session start, before any tool
/// runs: Claude on Windows has both Bash and PowerShell tools.
pub(super) fn session_shells(provider: &str) -> Vec<bool> {
    if cfg!(windows) && provider == "claude" {
        vec![false, true]
    } else {
        vec![crate::guard::powershell("", provider == "codex")]
    }
}

/// In the session's shell, so that a probe the hook missed would leave its marker.
pub(super) fn probe(token: &str, powershell: bool) -> String {
    let make = make_file(&marker(token), powershell);
    if powershell {
        format!("{make}; Stop-Process -Name {PROBE_NAME}")
    } else {
        format!("{make}; pkill -0 {PROBE_NAME}")
    }
}

/// A session with no effort still has a probe: named for the session, proven from the
/// machine switch's time.
pub(crate) fn session_token(session: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(format!("astrolabe probe {session}"));
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Whether this session's own hook (not a child's), of this build and plugin, denied the
/// probe since `since_ms`, and the probe didn't run.
pub(super) fn proof(session: &str, token: &str, since_ms: i64, powershell: bool) -> String {
    if marker(token).exists() {
        return "Guard FAILED: the probe ran (its marker file exists), so no hook stopped it. Claim no protection and tell the owner.".to_owned();
    }
    let log = state_dir().map(|dir| dir.join(hook::DENIAL_LOG));
    let log = log
        .and_then(|log| std::fs::read_to_string(log).ok())
        .unwrap_or_default();
    let program = hook_id();
    // Each denial counts only for the shell it was sent to, and only in that shell's form.
    let denied_in = |shell: bool| {
        let name = if shell { "powershell" } else { "posix" };
        let form = probe(token, shell);
        (log.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok()))
        .any(|l| {
            l["session_id"] == session
                && l["agent_id"].is_null()
                && l["shell"] == name
                && l["command"] == form.as_str()
                && l["hook"] == program.as_str()
                && l["at_ms"].as_i64().is_some_and(|at| at >= since_ms)
        })
    };
    let proven: Vec<&str> = [false, true]
        .into_iter()
        .filter(|&shell| denied_in(shell) && control_marker(token, shell).exists())
        .map(|shell| {
            if shell {
                "PowerShell"
            } else {
                "the POSIX shell (Bash, zsh)"
            }
        })
        .collect();
    if !proven.is_empty() {
        return format!(
            "Guard proven for this session in {}: the hook denied the probe there, its marker file wasn't made, and the allowed control in the same shell made its own. {GUARD_SCOPE}",
            proven.join(" and ")
        );
    }
    let half = if denied_in(powershell) {
        " The hook denied the probe, but the allowed control's marker isn't there, so a missing probe marker doesn't yet show the deny held."
    } else {
        ""
    };
    format!(
        "Guard not proven yet:{half} Proof needs, in this session's own shell, by this build of the program and this version of the plugin's hooks, since it was set up: {}\nUntil then claim no protection.",
        proof_steps(token, &[powershell])
    )
}

/// Whether the hook denied this activation's probe, by this build, and the probe didn't run.
pub fn status(dir: &Path, session: &str, powershell: bool) -> String {
    // Proof counts only from the latest switch-on: off and on again starts it over.
    let on = std::fs::read(dir.join(SWITCH)).ok();
    let on = on.and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let since = on.and_then(|on| on["at_ms"].as_i64()).unwrap_or(i64::MAX);
    let state = match read_state(dir, session) {
        Ok(None) => {
            return format!(
                "No effort is set in this session. {}",
                proof(session, &session_token(session), since, powershell)
            );
        }
        Err(why) => {
            return format!(
                "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
            );
        }
        Ok(Some(state)) => state,
    };
    let token = state.token.as_deref().unwrap_or_default();
    let on = format!(
        "Astrolabe is on for {} in repo {}.",
        state.record_label(),
        state.repo.as_deref().unwrap_or_default()
    );
    if let Some(why) = block_reason(dir, &state, session) {
        return format!(
            "{on} {}",
            super::context::blocked(
                state.record_kind(),
                state.bound_key().unwrap_or_default(),
                &why,
                None
            )
        );
    }
    format!(
        "{on} {}",
        // Per session, from the machine's switch-on: the same hook in the same session,
        // whichever effort is loaded, so a proof made before the effort still counts.
        proof(session, token, since, powershell)
    )
}

#[cfg(test)]
mod tests {
    use super::super::routing::in_powershell;
    use super::*;
    use serde_json::json;

    /// For every input shape `docs/hosts.md` records, and Claude's PowerShell tool, the one
    /// rule answers as the routing rule it replaces did, so no session is told anything new.
    #[test]
    fn the_one_dialect_rule_agrees_with_the_old_one_on_recorded_shapes() {
        for tool in ["Bash", "PowerShell", "shell_command"] {
            for input in [
                json!({"tool_name": tool, "tool_input": {"command": "ls"}}),
                json!({"tool_name": tool, "turn_id": "t", "tool_input": {"command": "ls"}}),
                json!({"tool_name": tool, "transcript_path": "/x/rollout-1.jsonl"}),
            ] {
                let old = in_powershell(provider(&input), &input["tool_name"]);
                assert_eq!(powershell_input(&input), old, "{input}");
            }
        }
        let codex = json!({"turn_id": "t"});
        assert_eq!(session_shells("codex"), [powershell_input(&codex)]);
    }
}
