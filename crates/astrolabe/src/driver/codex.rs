//! Launch the pinned adapter using the existing login, including on Windows.
//! No per-launch `CODEX_HOME` is created and no credentials are copied. This also
//! avoids adding a long home path to Windows AF_UNIX socket names. An inherited
//! custom home remains the caller's responsibility; future isolated homes must
//! use a short root such as `%LOCALAPPDATA%\astrolabe\c\<id>`.
use crate::acp::{CODEX_ADAPTER, Launch, adapter_entry};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::io;
use std::path::Path;

/// Build a launch without starting a process or modifying user configuration.
/// Overrides are already-composed session config; layer composition belongs to
/// the caller. `client_path` selects the user-installed executable via CODEX_PATH;
/// step 5's doctor supplies it. None leaves the adapter's inherited/bundled choice.
/// We request `check_for_update_on_startup = false` in CODEX_CONFIG, but 2.5.0
/// forwards this as session config, not app-server startup arguments. Whether it
/// disables startup update checks is unverified.
pub fn launch(
    adapters_dir: &Path,
    client_path: Option<&Path>,
    overrides: &Value,
) -> Result<Launch> {
    let overrides = overrides.as_object().ok_or_else(|| {
        Error::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Codex launch config must be a JSON object",
        ))
    })?;
    let mut config = json!({
        "model": "gpt-6.1-terra",
        "model_reasoning_effort": "low",
    });
    config.as_object_mut().unwrap().extend(overrides.clone());
    config["check_for_update_on_startup"] = json!(false);
    let mut launch = Launch::node(&adapter_entry(adapters_dir, CODEX_ADAPTER)?);
    launch
        .env
        .push(("CODEX_CONFIG".into(), config.to_string().into()));
    if let Some(path) = client_path {
        launch.env.push(("CODEX_PATH".into(), path.into()));
    }
    Ok(launch)
}

/// Codex reject ends the current turn. The caller should offer this action,
/// answer with the request's reject option, wait for `ClientEvent::TurnEnded`,
/// then send the user's replacement as a new prompt in the same session.
/// A failed prompt or exited process must be surfaced instead of continuing.
pub const REJECT_AND_SAY_WHAT_INSTEAD: &str = "Reject and say what instead";
