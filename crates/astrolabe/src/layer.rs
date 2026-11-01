//! The composer: what a launch adds to a session, and nothing a repo already supplies.
//! Pure. The input and `Delivery` name no provider. Each driver turns a `Delivery` into
//! its own form: `driver::claude::deliver` here, and the Codex driver's `CODEX_CONFIG`
//! after it. A Codex driver reads `Instructions::merged`, since its setting replaces
//! the repo's; Claude reads `Instructions::layer`, since a repo's files stay loaded.
use crate::error::{Error, Result};
use std::ffi::OsString;
use std::path::Path;

/// On `PATH` through `launch_env`. Never a full path: a repo's copy of a hook or server
/// must be the same string, or a client loads both, and hook trust hashes this text.
pub const PROGRAM: &str = "astrolabe";
pub const MCP_NAME: &str = "astrolabe";
pub const MCP_TOOLS: [&str; 1] = ["layer_check"];
/// Where hooks and the tool server find the core's files. Unset, the OS app-data folder.
pub const DATA_DIR_VAR: &str = "ASTROLABE_DATA_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
}

impl HookEvent {
    /// The argument to `astrolabe hook`.
    pub fn slug(self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub event: HookEvent,
    pub command: String,
}

impl Hook {
    pub fn astrolabe(event: HookEvent) -> Self {
        Self {
            event,
            command: format!("{PROGRAM} hook {}", event.slug()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Auto-approved, on every client.
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub instructions: String,
    pub hooks: Vec<Hook>,
    pub mcp: Vec<McpServer>,
    pub denies: Vec<String>,
    pub memory_off: bool,
}

impl Layer {
    /// Astrolabe's own hook and server. What goes in `denies` is step 4's.
    pub fn standard(instructions: &str, denies: &[&str]) -> Self {
        Self {
            instructions: instructions.to_owned(),
            hooks: vec![Hook::astrolabe(HookEvent::SessionStart)],
            mcp: vec![McpServer {
                name: MCP_NAME.to_owned(),
                command: PROGRAM.to_owned(),
                args: vec!["mcp".to_owned()],
                tools: MCP_TOOLS.map(str::to_owned).to_vec(),
            }],
            denies: denies.iter().map(|deny| (*deny).to_owned()).collect(),
            memory_off: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Holds the layer as its own config.
    Agent,
    Product,
}

/// What a repo's own config holds, as read by the caller. Whether a client honours it
/// is another matter: Codex drops an untrusted repo's config, which step 4's receipt
/// catches and step 5's trust lines prevent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSupplies {
    pub instructions: Option<String>,
    pub hooks: Vec<Hook>,
    /// `tools` is not compared. A client loads a second copy of a different name.
    pub mcp: Vec<McpServer>,
    pub denies: Vec<String>,
    pub memory_off: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    pub supplies: RepoSupplies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instructions {
    pub repo: Option<String>,
    pub layer: String,
}

impl Instructions {
    /// The repo's text first, then the layer's, which is left out if already there.
    pub fn merged(&self) -> String {
        match self.repo.as_deref().filter(|repo| !repo.trim().is_empty()) {
            Some(repo) if repo.contains(self.layer.trim()) => repo.to_owned(),
            Some(repo) => format!("{}\n\n{}", repo.trim_end(), self.layer),
            None => self.layer.clone(),
        }
    }
}

/// What the launch must add. Empty means the launch adds nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    pub instructions: Option<Instructions>,
    pub hooks: Vec<Hook>,
    pub mcp: Vec<McpServer>,
    pub denies: Vec<String>,
    pub memory_off: bool,
}

/// An identical hook, or this server under another name, is dropped on any target: either
/// would otherwise run twice. A same-named server, a deny and memory-off stay on a product
/// repo: the launch's server wins by name, a repeated deny costs nothing, and a product
/// repo's config can't be counted on. On an agent repo they are the repo's to supply.
/// Fails on a malformed deny, on any target, since a client would drop it and carry on.
pub fn compose(target: &Target, layer: &Layer) -> Result<Delivery> {
    for deny in &layer.denies {
        check_deny(deny)?;
    }
    let agent = target.kind == TargetKind::Agent;
    let supplies = &target.supplies;
    let mut hooks = Vec::new();
    for hook in &layer.hooks {
        if !supplies.hooks.contains(hook) && !hooks.contains(hook) {
            hooks.push(hook.clone());
        }
    }
    let wanted = |supplied: bool| !(agent && supplied);
    let supplied_server = |server: &McpServer| {
        supplies.mcp.iter().any(|theirs| {
            let same_name = theirs.name == server.name;
            let same_program = theirs.command == server.command && theirs.args == server.args;
            (same_program && !same_name) || (agent && same_name)
        })
    };
    Ok(Delivery {
        instructions: wanted(supplies.instructions.is_some()).then(|| Instructions {
            repo: supplies.instructions.clone(),
            layer: layer.instructions.clone(),
        }),
        hooks,
        mcp: (layer.mcp.iter())
            .filter(|server| !supplied_server(server))
            .cloned()
            .collect(),
        denies: (layer.denies.iter())
            .filter(|deny| wanted(supplies.denies.contains(deny)))
            .cloned()
            .collect(),
        memory_off: layer.memory_off && wanted(supplies.memory_off),
    })
}

/// Refuses what Claude Code 2.1.314 would skip with no effect on the session: `Tool` or
/// `Tool(content)` only. Stricter than the client where it costs nothing. A client's rule
/// grammar can grow, so passing here does not prove a rule took effect.
fn check_deny(rule: &str) -> Result<()> {
    let bad = |why: &str| Err(Error::InvalidLayer(format!("deny rule {rule:?}: {why}")));
    let (tool, content) = match rule.split_once('(') {
        Some((tool, rest)) => match rest.strip_suffix(')') {
            Some(content) => (tool, Some(content)),
            None => return bad("must end at the closing parenthesis"),
        },
        None => (rule, None),
    };
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if tool.is_empty() || !tool.chars().all(word) {
        return bad("needs a tool name");
    }
    if tool.starts_with("mcp__") {
        return match content {
            Some(_) => bad("an MCP rule takes no pattern"),
            None => Ok(()),
        };
    }
    if !tool.starts_with(|c: char| c.is_ascii_uppercase()) {
        return bad("a tool name starts with a capital");
    }
    let Some(content) = content else {
        return Ok(());
    };
    if content.trim().is_empty() {
        return bad("empty parentheses");
    }
    if content.contains("\\(") || content.contains("\\)") {
        return bad("an escaped parenthesis is ambiguous; use [(] or [)]");
    }
    if let Some(prefix) = content.strip_suffix(":*")
        && (tool != "Bash" || prefix.trim().is_empty())
    {
        return bad(":* needs Bash and a prefix before it");
    }
    Ok(())
}

/// For every launch on every target, since a repo's own hook needs them too: `PATH`
/// finds this `astrolabe`, the data directory is where it reports.
pub fn launch_env(
    program_dir: &Path,
    data_dir: &Path,
    inherited_path: Option<OsString>,
) -> std::result::Result<Vec<(OsString, OsString)>, std::env::JoinPathsError> {
    let inherited = inherited_path.unwrap_or_default();
    let paths = std::iter::once(program_dir.to_path_buf()).chain(std::env::split_paths(&inherited));
    Ok(vec![
        ("PATH".into(), std::env::join_paths(paths)?),
        (DATA_DIR_VAR.into(), data_dir.into()),
    ])
}
