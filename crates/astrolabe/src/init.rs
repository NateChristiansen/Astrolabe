//! `astrolabe init`: an agent repo from the seed, with the real layer as its own config, so
//! it loads alone in a plain client once trusted. Hook strings hold no path or version, so
//! an upgrade keeps their trust hashes. A test holds `layer/` to `Layer::protected`.
use crate::error::{Error, Result};
use crate::registry::{AGENT, Registry};
use crate::worktree::{clone_without_hooks, git, isolated_git};
use std::path::{Path, PathBuf};

pub const LAYER: [(&str, &str); 3] = [
    (
        ".claude/settings.json",
        include_str!("../layer/claude-settings.json"),
    ),
    (".mcp.json", include_str!("../layer/mcp.json")),
    (
        ".codex/config.toml",
        include_str!("../layer/codex-config.toml"),
    ),
];
const SEED: [(&str, &str); 4] = [
    ("AGENTS.md", include_str!("../../../seed/AGENTS.md")),
    ("README.md", include_str!("../../../seed/README.md")),
    (
        "knowledge/owner.md",
        include_str!("../../../seed/knowledge/owner.md"),
    ),
    (
        "knowledge/voice.md",
        include_str!("../../../seed/knowledge/voice.md"),
    ),
];
const KEPT: &str = "agents drafts efforts initiatives journal lessons outcomes templates";

/// Each layer file: `Some(true)` real, `Some(false)` missing or the seed's placeholder, `None`
/// the repo's own. Line endings don't count.
pub fn layer_state(repo: &Path) -> Vec<(&'static str, Option<bool>)> {
    let state = |file: &str, wanted: &str| match std::fs::read_to_string(repo.join(file)) {
        Ok(text) if text.replace("\r\n", "\n") == wanted => Some(true),
        Ok(text) if text.contains("PLACEHOLDER") || text.contains("\"hooks\": {}") => Some(false),
        Ok(_) => None,
        Err(_) => Some(false),
    };
    LAYER
        .iter()
        .map(|(file, text)| (*file, state(file, text)))
        .collect()
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    Ok(std::fs::write(path, text)?)
}

/// Replaces missing and placeholder files only, and registers the repo.
fn finish(registry: &Registry, repo: &Path) -> Result<Vec<String>> {
    let mut said = vec![format!("agent repo {}", repo.display())];
    for ((file, text), (_, state)) in LAYER.iter().zip(layer_state(repo)) {
        if state == Some(false) {
            write(&repo.join(file), text)?;
        }
        let what = if state.is_none() {
            "the repo's own, left alone"
        } else {
            "real"
        };
        said.push(format!("{file}: {what}"));
    }
    registry.add(AGENT, repo)?;
    Ok(said)
}

pub fn create(registry: &Registry, path: &Path) -> Result<Vec<String>> {
    crate::doctor::refuse(path)?;
    if std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(Error::Setup(format!("{} is not empty", path.display())));
    }
    for (file, text) in SEED {
        write(&path.join(file), text)?;
    }
    for folder in KEPT.split(' ') {
        write(&path.join(folder).join(".gitkeep"), "")?;
    }
    let run = |args: &[&str]| isolated_git(path, args).map_err(Error::Setup);
    let identity = |key: &str| {
        let args = ["config".as_ref(), key.as_ref()];
        let value = git(path, &args).unwrap_or_default();
        format!("{key}={}", value.trim())
    };
    let (name, email) = (identity("user.name"), identity("user.email"));
    run(&["init", "-q", "--template=", "-b", "main"])?;
    let mut said = finish(registry, path)?;
    run(&["add", "."])?;
    let text = "Create agent repo";
    let commit = ["-c", &name, "-c", &email, "commit", "-qm", text];
    if let Err(error) = run(&commit) {
        said.push(format!("not committed: {error}"));
    }
    Ok(said)
}

/// A second device. Files it replaces stay uncommitted. It keeps the owner's home, which holds
/// the credentials a private remote needs, but runs no hook or template from it.
pub fn clone(registry: &Registry, url: &str, path: Option<&str>) -> Result<Vec<String>> {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next();
    let named = last.unwrap_or(url).trim_end_matches(".git");
    let path = std::path::absolute(PathBuf::from(path.unwrap_or(named)))?;
    crate::doctor::refuse(&path)?;
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    clone_without_hooks(url, &path, parent).map_err(Error::Setup)?;
    finish(registry, &path)
}
