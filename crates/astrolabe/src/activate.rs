//! Activation facade. A machine switch enables the guard; loading an effort binds context
//! to the session named by the provider hook. The hook performs the transition and leaves
//! a ticket for the rewritten command to read.
pub use crate::session::read_state;
pub(crate) use crate::session::state_path;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
pub(crate) mod allow;
mod context;
mod housekeeping;
mod proof;
mod repos;
mod routing;
mod tickets;

#[cfg(test)]
pub(crate) use context::SHAPE;
pub use context::{
    Activation, Effort, GUARD_SCOPE, RECORD_RULE, WORKER_RULE, read_effort, session_name, switch_on,
};
pub(crate) use context::{STILL_ON, no_note};
pub(crate) use proof::session_token;
pub use proof::{probe_dir, shell_of, status};
use repos::notes_checked;
pub(crate) use repos::{Repos, common_dir, in_notes, note_remotes, notes_and_repos, repo_names};
pub use repos::{add_repo, add_repo_note, check_repo_name, main_checkout, notes_root};
pub use routing::gate;
pub(crate) use routing::{block_reason, provider};
pub use tickets::command;
pub(crate) use tickets::shown_for;
use tickets::write_json;

/// Where activation state lives. Unset, `~/.local/state/astrolabe-layer`.
pub const STATE_VAR: &str = "ASTROLABE_LAYER_STATE";

/// Set at launch to an effort's number: binds the session to it at session start, as an
/// ordinary binding without a worker's restrictions.
pub const EFFORT_VAR: &str = "ASTROLABE_EFFORT";

/// The machine switch's file in the state folder. Present means on.
const SWITCH: &str = "on";

/// Whether Astrolabe is on for this machine. Off, the hooks do nothing.
pub fn machine_on(dir: &Path) -> bool {
    dir.join(SWITCH).is_file()
}

/// `astrolabe on | off | status`, for the machine. The owner's, from a terminal: the hook refuses
/// it inside a session.
pub fn machine(verb: &str) -> (String, bool) {
    let Some(dir) = state_dir() else {
        return (
            "Astrolabe has no home folder for its state.".to_owned(),
            false,
        );
    };
    let path = dir.join(SWITCH);
    let done = match verb {
        // Without the notes checkout on record nothing can be read or saved.
        "on" => notes_checked()
            .map_err(|why| format!("{why}. It's needed because that's where the notes are: every save, the board and the owner's seat read it"))
            .and_then(|_| write_json(&path, &json!({"at_ms": now_ms()}))),
        "off" => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", path.display()))
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    };
    match (done, machine_on(&dir)) {
        (Err(why), _) => (format!("Astrolabe couldn't switch {verb}: {why}"), false),
        (Ok(()), true) => ("Astrolabe is on for this machine: every new session gets the owner and voice notes and the guard, and is told its repo's active efforts; none is loaded until the owner names one. That's the switch, not proof an app runs the hooks: in a session, `astrolabe activate --status` says.".to_owned(), true),
        (Ok(()), false) => ("Astrolabe is off for this machine: its hooks do nothing.".to_owned(), true),
    }
}

pub fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(STATE_VAR) {
        return Some(dir.into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".local/state/astrolabe-layer"))
}

/// Names this build of the program and the plugin whose hook runs it, so a guard proof from
/// one doesn't certify another: the program's version and file time, the plugin's folder, and
/// a hash of its hooks and manifest. Both clients tell a plugin's hook its folder.
pub fn hook_id() -> String {
    use sha2::{Digest, Sha256};
    let exe = std::env::current_exe().ok();
    let modified = exe.and_then(|exe| exe.metadata().ok()?.modified().ok());
    let ms = modified.and_then(|at| at.duration_since(UNIX_EPOCH).ok());
    let root = std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from);
    let mut hash = Sha256::new();
    for file in ["hooks/hooks.json", ".claude-plugin/plugin.json"] {
        let bytes = root
            .as_ref()
            .and_then(|root| std::fs::read(root.join(file)).ok());
        hash.update(bytes.unwrap_or_default());
        hash.update([0]);
    }
    let hash: String = hash
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let root = root.map_or("no plugin".to_owned(), |r| r.display().to_string());
    let ms = ms.map_or(0, |d| d.as_millis());
    format!("{} {ms} {root} {hash}", env!("CARGO_PKG_VERSION"))
}

fn now_ms() -> i64 {
    crate::notes::now_ms()
}

pub(crate) fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|why| format!("no randomness: {why}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
