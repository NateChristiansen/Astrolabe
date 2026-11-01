//! Launch the pinned adapter using the existing login, including on Windows.
//! No per-launch `CODEX_HOME` is created and no credentials are copied. This also
//! avoids adding a long home path to Windows AF_UNIX socket names. An inherited
//! custom home remains the caller's responsibility; future isolated homes must
//! use a short root such as `%LOCALAPPDATA%\astrolabe\c\<id>`.
use crate::acp::{CODEX_ADAPTER, Launch, adapter_entry};
use crate::layer::{Delivery, Hook, HookEvent, RepoInstructions};
use crate::{Error, Result};
use serde_json::{Map, Value, json};
use std::io;
use std::path::Path;

/// A session-flags entry returned by this machine's app-server `hooks/list`.
/// Keep the key verbatim: Windows and Unix use different synthetic paths.
#[derive(Debug, Clone)]
pub struct SessionFlagKey {
    pub key: String,
    pub current_hash: String,
    pub command: String,
}

fn valid_hash(hash: &str) -> bool {
    hash.strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn valid_key(key: &str, suffix: &str) -> bool {
    key == format!("/<session-flags>/config.toml{suffix}")
        || key == format!(r"C:\<session-flags>\config.toml{suffix}")
}

impl SessionFlagKey {
    /// Read a full JSON-RPC hooks/list response. Repo and plugin entries are excluded.
    pub fn from_hooks_list(response: &Value) -> Result<Vec<Self>> {
        let data = response["result"]["data"]
            .as_array()
            .ok_or_else(|| invalid_config("hooks/list response has no result.data"))?;
        let mut keys = Vec::new();
        for group in data {
            let hooks = group["hooks"]
                .as_array()
                .ok_or_else(|| invalid_config("hooks/list response has no hooks array"))?;
            for hook in hooks {
                let key = hook["key"].as_str().unwrap_or("");
                if !key.contains("<session-flags>") {
                    continue;
                }
                if !key.starts_with("/<session-flags>/config.toml:")
                    && !key.starts_with(r"C:\<session-flags>\config.toml:")
                {
                    return Err(invalid_config("malformed session-flags key prefix"));
                }
                let command = hook["command"]
                    .as_str()
                    .ok_or_else(|| invalid_config("session hook has no command"))?;
                let hash = hook["currentHash"].as_str().unwrap_or("");
                if !valid_hash(hash) {
                    return Err(invalid_config("session hook has no SHA-256 currentHash"));
                }
                keys.push(Self {
                    key: key.to_owned(),
                    current_hash: hash.to_owned(),
                    command: command.to_owned(),
                });
            }
        }
        Ok(keys)
    }
}

#[derive(Debug, Clone)]
pub struct CodexDelivery {
    /// The layer's CODEX_CONFIG entries, before launch defaults.
    pub config: Value,
    pub mcp_servers: Vec<Value>,
}

impl CodexDelivery {
    pub fn launch(&self, adapters_dir: &Path, client_path: Option<&Path>) -> Result<Launch> {
        let mut spec = launch(adapters_dir, client_path, &self.config)?;
        spec.mcp_servers.clone_from(&self.mcp_servers);
        Ok(spec)
    }
}

/// Translate a composed layer. Trust comes from hooks/list on the selected client,
/// with these exact hook declarations passed as session flags. No path or hash is
/// guessed. An empty delivery needs no discovery and adds no config or MCP entries.
/// RepoSupplies must describe config the client actually honours; never populate it
/// from an untrusted repo merely because a config file exists there.
pub fn deliver(delivery: &Delivery, session_flag_key: &[SessionFlagKey]) -> Result<CodexDelivery> {
    let mut config = Map::new();
    if let Some(instructions) = &delivery.instructions {
        if instructions.repo == RepoInstructions::Unknown {
            return Err(invalid_config(
                "state effective repo instructions as text or none",
            ));
        }
        config.insert(
            "developer_instructions".into(),
            json!(instructions.merged()),
        );
    }
    // Creation-only: these flags disable memory for a new thread, not an existing
    // thread resumed here. The real new-thread probe read memory_mode = disabled.
    // Its flat dotted keys are deliberate; hooks below uses a table value.
    if delivery.memory_off {
        config.insert("features.memories".into(), json!(false));
        config.insert("memories.generate_memories".into(), json!(false));
        config.insert("memories.use_memories".into(), json!(false));
    }
    if !delivery.hooks.is_empty() {
        let mut hooks = hook_declarations(&delivery.hooks);
        let mut state = Map::new();
        for (index, hook) in delivery.hooks.iter().enumerate() {
            let label = match hook.event {
                HookEvent::SessionStart => "session_start",
            };
            let suffix = format!(":{label}:{index}:0");
            let matches: Vec<_> = session_flag_key
                .iter()
                .filter(|key| {
                    valid_key(&key.key, &suffix)
                        && valid_hash(&key.current_hash)
                        && key.command == hook.command
                })
                .collect();
            let [key] = matches.as_slice() else {
                return Err(invalid_config(
                    "hooks/list must contain exactly one matching session hook key and hash",
                ));
            };
            // A syntactically valid but stale hash cannot be verified here. Step 4's
            // SessionStart receipt must catch a hook that did not actually run.
            state.insert(key.key.clone(), json!({ "trusted_hash": key.current_hash }));
        }
        hooks.insert("state".into(), state.into());
        config.insert("hooks".into(), hooks.into());
        config.insert("features.hooks".into(), json!(true));
    }
    let mut mcp_servers = Vec::new();
    for server in &delivery.mcp {
        mcp_servers.push(json!({ "name": server.name, "command": server.command,
            "args": server.args, "env": [] }));
        // codex-acp 2.5.0 replaces nested mcp_servers when translating ACP servers.
        // Dotted config overrides survive that replacement. Codex splits paths
        // literally, so tools are a table value and server names cannot contain dots.
        if !server.tools.is_empty() {
            if server.name.is_empty()
                || !server
                    .name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            {
                return Err(invalid_config(
                    "Codex MCP server names must use letters, digits, underscores or hyphens",
                ));
            }
            let tools: Map<String, Value> = server
                .tools
                .iter()
                .map(|tool| (tool.clone(), json!({"approval_mode":"approve"})))
                .collect();
            config.insert(format!("mcp_servers.{}.tools", server.name), tools.into());
        }
    }
    Ok(CodexDelivery {
        config: config.into(),
        mcp_servers,
    })
}

/// Shared by app-server discovery and delivery: identical session hook declarations.
pub fn hook_declarations(hooks: &[Hook]) -> Map<String, Value> {
    let mut config = Map::new();
    for hook in hooks {
        let event = match hook.event {
            HookEvent::SessionStart => "SessionStart",
        };
        config
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks": [{"type": "command", "command": hook.command}]}));
    }
    config
}

fn invalid_config(message: &str) -> Error {
    Error::Io(io::Error::new(io::ErrorKind::InvalidInput, message))
}

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
