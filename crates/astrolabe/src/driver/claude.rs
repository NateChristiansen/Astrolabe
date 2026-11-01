//! Launches the pinned Claude adapter. Login is whatever the client already has.
use crate::acp::{CLAUDE_ADAPTER, Launch, ReceiptAt, adapter_entry};
use crate::error::Result;
use crate::layer::{Delivery, Hook, HookEvent};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// SessionStart fires during the launch, before the session is ready, even with no prompt.
pub const RECEIPT_AT: ReceiptAt = ReceiptAt::BeforeReady;

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

/// The added directory's file, for Claude Code to read as instructions.
const INSTRUCTIONS_FILE: &str = "CLAUDE.md";
/// Claude Code ignores instruction files in added directories without this.
const READ_ADDED_INSTRUCTIONS: &str = "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD";

/// A `Delivery` in Claude's form. Pass `options` to `launch` on a new session and on
/// every resume: nothing in it outlives its process.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeDelivery {
    /// None when the launch adds nothing.
    pub options: Option<Value>,
    /// For `write_instructions`, before the launch.
    pub instructions: Option<(PathBuf, String)>,
}

impl ClaudeDelivery {
    pub fn write_instructions(&self) -> Result<()> {
        if let Some((file, text)) = &self.instructions {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(file, text)?;
        }
        Ok(())
    }
}

/// Sets no `settingSources`: production stays on the adapter's default, so the user's own
/// settings and plugins load alongside the layer. Pure. `instructions_dir` is added to the session as a whole, so it should hold nothing
/// else. Only the layer's text goes in it: a repo's own instructions stay loaded.
pub fn deliver(delivery: &Delivery, instructions_dir: &Path) -> ClaudeDelivery {
    let mut settings = Map::new();
    if !delivery.hooks.is_empty() {
        settings.insert("hooks".into(), hooks(&delivery.hooks));
    }
    let allow: Vec<String> = (delivery.mcp.iter())
        .flat_map(|server| server.tools.iter().map(move |tool| (server, tool)))
        .map(|(server, tool)| format!("mcp__{}__{tool}", server.name))
        .collect();
    let mut permissions = Map::new();
    if !delivery.denies.is_empty() {
        permissions.insert("deny".into(), json!(delivery.denies));
    }
    if !allow.is_empty() {
        permissions.insert("allow".into(), json!(allow));
    }
    if !permissions.is_empty() {
        settings.insert("permissions".into(), permissions.into());
    }
    if delivery.memory_off {
        settings.insert("autoMemoryEnabled".into(), json!(false));
    }

    let mut options = Map::new();
    if !settings.is_empty() {
        options.insert("settings".into(), settings.into());
    }
    if !delivery.mcp.is_empty() {
        let servers: Map<String, Value> = (delivery.mcp.iter())
            .map(|server| {
                let entry = json!({
                    "type": "stdio",
                    "command": server.command,
                    "args": server.args,
                });
                (server.name.clone(), entry)
            })
            .collect();
        options.insert("mcpServers".into(), servers.into());
    }
    let instructions = delivery.instructions.as_ref().map(|instructions| {
        options.insert("additionalDirectories".into(), json!([instructions_dir]));
        options.insert("env".into(), json!({ READ_ADDED_INSTRUCTIONS: "1" }));
        (
            instructions_dir.join(INSTRUCTIONS_FILE),
            instructions.layer.clone(),
        )
    });
    ClaudeDelivery {
        options: (!options.is_empty()).then(|| options.into()),
        instructions,
    }
}

fn hooks(hooks: &[Hook]) -> Value {
    let mut by_event: Map<String, Value> = Map::new();
    for hook in hooks {
        let name = match hook.event {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::PreCommand => "PreToolUse",
            HookEvent::ConfigChange => "ConfigChange",
        };
        let entry = json!({ "hooks": [{ "type": "command", "command": hook.command }] });
        let list = by_event.entry(name).or_insert_with(|| json!([]));
        list.as_array_mut().expect("built above").push(entry);
    }
    by_event.into()
}
