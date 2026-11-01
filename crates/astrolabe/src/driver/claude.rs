//! Launches the pinned Claude adapter. Login is whatever the client already has.
use crate::acp::{CLAUDE_ADAPTER, Launch, adapter_entry};
use crate::error::Result;
use serde_json::{Value, json};
use std::path::Path;

/// Claude Code reads this; the adapter passes its environment on.
const NO_UPDATES: (&str, &str) = ("DISABLE_AUTOUPDATER", "1");
const CLIENT_PATH: &str = "CLAUDE_CODE_EXECUTABLE";
/// Set in the environment of a process that a Claude Code session started.
const NESTED_MARK: &str = "CLAUDECODE";
/// What such a session sets for its own children, seen in the environment of one.
/// Started from inside a session, a launch would otherwise call itself nested, send
/// its traffic to the parent's endpoint, take on the parent's account and session
/// identity, and expect the parent's host to refresh its login. Removed only then: on
/// an ordinary launch `ANTHROPIC_BASE_URL` may be the user's own gateway. A login of
/// the user's own (the keychain, `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`) is
/// left alone either way.
const NESTED_ENV: [&str; 11] = [
    "ANTHROPIC_BASE_URL",
    "CLAUDECODE",
    "CLAUDE_CODE_ACCOUNT_UUID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_SDK_HAS_HOST_AUTH_REFRESH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_USER_EMAIL",
];

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
    if std::env::var_os(NESTED_MARK).is_some() {
        launch.env_remove = NESTED_ENV.iter().map(Into::into).collect();
    }
    if let Some(client) = client {
        launch.env.push((CLIENT_PATH.into(), client.into()));
    }
    launch.meta = options.map(|options| json!({ "claudeCode": { "options": options } }));
    Ok(launch)
}
