//! Launches the pinned Claude adapter. Login is whatever the client already has.
use crate::acp::{CLAUDE_ADAPTER, Launch, adapter_entry};
use crate::error::Result;
use serde_json::{Value, json};
use std::path::Path;

/// Claude Code reads this; the adapter passes its environment on.
const NO_UPDATES: (&str, &str) = ("DISABLE_AUTOUPDATER", "1");
const CLIENT_PATH: &str = "CLAUDE_CODE_EXECUTABLE";

/// `client` is the user-installed Claude Code; without it the adapter runs the copy
/// bundled with its SDK. `options` goes to the adapter as `_meta.claudeCode.options`, on `session/new` and
/// on resume: the route for what later steps deliver. The adapter owns some fields
/// and overrides them, such as `cwd` and `permissionMode`.
pub fn launch(
    adapters_dir: &Path,
    client: Option<&Path>,
    options: Option<Value>,
) -> Result<Launch> {
    let mut launch = Launch::node(&adapter_entry(adapters_dir, CLAUDE_ADAPTER)?);
    launch.env.push((NO_UPDATES.0.into(), NO_UPDATES.1.into()));
    if let Some(client) = client {
        launch.env.push((CLIENT_PATH.into(), client.into()));
    }
    launch.meta = options.map(|options| json!({ "claudeCode": { "options": options } }));
    Ok(launch)
}
