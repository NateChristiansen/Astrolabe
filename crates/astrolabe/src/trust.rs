//! The only user-config writes: Codex's project and hook lines, in a marked block, shown and
//! agreed first. Claude's folder trust is its own dialog; this never writes that file.
use crate::error::{Error, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const BEGIN: &str = "# BEGIN astrolabe before=";
const END: &str = "# END astrolabe";

pub type Ask<'a> = &'a mut dyn FnMut(&str) -> bool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Homes {
    pub codex_config: PathBuf,
    pub claude_json: PathBuf,
}

impl Homes {
    pub fn from_env() -> Result<Self> {
        let var = |name| std::env::var_os(name).map(PathBuf::from);
        let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
        let home = home.ok_or_else(|| Error::Setup("no home folder is set".to_owned()))?;
        let codex = var("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
        let claude_json = var("CLAUDE_CONFIG_DIR")
            .unwrap_or(home)
            .join(".claude.json");
        let codex_config = codex.join("config.toml");
        Ok(Self {
            codex_config,
            claude_json,
        })
    }
}

/// A folder's plain name, without the `\\?\` that canonicalizing adds on Windows.
pub fn native(path: &Path) -> String {
    let path = crate::hook::comparable(path);
    strip_verbatim(&path)
}

/// `\\?\C:\x` to `C:\x`, and `\\?\UNC\host\x` to `\\host\x`.
pub fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

/// As each client writes them. Windows: Codex lowercased, Claude forward slashes, case kept.
pub fn keys(repo: &str, windows: bool) -> (String, String) {
    if !windows {
        return (repo.to_owned(), repo.to_owned());
    }
    (repo.to_lowercase(), repo.replace('\\', "/"))
}

fn toml_key(key: &str) -> String {
    let single = key.contains('\\') && !key.contains('\'');
    if single {
        format!("'{key}'")
    } else {
        json!(key).to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexPlan {
    pub project: String,
    pub hooks: Vec<(String, String)>,
}

impl CodexPlan {
    pub fn lines(&self) -> Vec<String> {
        let project = format!("[projects.{}]", toml_key(&self.project));
        let mut lines = vec![project, "trust_level = \"trusted\"".to_owned()];
        for (key, hash) in &self.hooks {
            lines.push(format!("[hooks.state.{}]", toml_key(key)));
            lines.push(format!("trusted_hash = \"{hash}\""));
        }
        lines
    }
}

fn toml_string(s: &str) -> Option<(String, &str)> {
    if let Some(rest) = s.strip_prefix('\'') {
        let (value, rest) = rest.split_once('\'')?;
        return Some((value.to_owned(), rest));
    }
    s.strip_prefix('"')?;
    let mut escaped = false;
    let (end, _) = s.char_indices().skip(1).find(|(_, c)| {
        let close = *c == '"' && !escaped;
        escaped = *c == '\\' && !escaped;
        close
    })?;
    Some((serde_json::from_str(&s[..=end]).ok()?, &s[end + 1..]))
}

/// Trusted projects and hook lines, in either quoting. Whole tables only, not inline forms.
pub fn scan(text: &str) -> (Vec<String>, Vec<(String, String)>) {
    let (mut projects, mut hooks) = (Vec::new(), Vec::new());
    let (mut project, mut hook) = (None, None);
    let table = |line: &str, prefix: &str| {
        let (key, rest) = toml_string(line.strip_prefix(prefix)?)?;
        rest.trim_start().starts_with(']').then_some(key)
    };
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            project = table(line, "[projects.");
            hook = table(line, "[hooks.state.");
        }
        let value = line
            .split_once('=')
            .and_then(|(_, v)| toml_string(v.trim()));
        let Some((value, _)) = value else { continue };
        if let (Some(key), true) = (&project, line.starts_with("trust_level")) {
            if value == "trusted" {
                projects.push(key.clone());
            }
        } else if let (Some(key), true) = (&hook, line.starts_with("trusted_hash")) {
            hooks.push((key.clone(), value));
        }
    }
    (projects, hooks)
}

pub fn file_hash(path: &Path) -> String {
    let hash = |bytes: Vec<u8>| format!("sha256:{:x}", Sha256::digest(bytes));
    std::fs::read(path).map_or("no file".to_owned(), hash)
}

fn read_text(path: &Path) -> Result<String> {
    let not_utf8 = |_| Error::Setup(format!("{} is not UTF-8", path.display()));
    String::from_utf8(std::fs::read(path)?).map_err(not_utf8)
}

/// The byte range, with the ending before it, and the recorded hash. Whole lines only.
pub fn find_block(text: &str) -> Option<(usize, usize, String)> {
    let (mut at, mut found) = (0, None);
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']);
        match (&found, bare.strip_prefix(BEGIN)) {
            (_, Some(hash)) => found = Some((at, hash.to_owned())),
            (Some((begin, hash)), _) if bare == END => {
                let start = begin - usize::from(text[..*begin].ends_with('\n'));
                let start = start - usize::from(text[..start].ends_with('\r'));
                return Some((start, at + line.len(), hash.clone()));
            }
            _ => {}
        }
        at += line.len();
    }
    None
}

/// The key in any case or slash style, or a TOML unicode escape, which could spell anything.
pub fn mentions(text: &str, key: &str) -> bool {
    let fold = |s: &str| s.replace("\\\\", "/").replace('\\', "/").to_lowercase();
    let (folded, key) = (fold(text), fold(key));
    let path_char = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.');
    let named = folded.match_indices(&key).any(|(at, _)| {
        let next = folded[at + key.len()..].chars().next();
        !next.is_some_and(path_char)
    });
    let escaped = |line: &str| {
        let line = line.trim_start_matches(['\u{feff}', ' ', '\t', '[']);
        let bare = line.replace("\\\\", "");
        let ours = line.starts_with("projects") || line.starts_with("hooks");
        ours && line.contains('"') && (bare.contains("\\u") || bare.contains("\\U"))
    };
    named || text.lines().any(escaped)
}

/// Appends from bytes, in the file's own line ending. Any trace of these keys is refused, in
/// any state or form, as two tables under one key break TOML. A missing file is refused, so removal restores it.
pub fn apply_codex(path: &Path, plan: &CodexPlan, ask: Ask) -> Result<String> {
    let name = path.display();
    let text = read_text(path)?;
    let (projects, hooks) = scan(&text);
    let project = projects.contains(&plan.project);
    let all = plan.hooks.iter().all(|hook| hooks.contains(hook));
    if project && all {
        return Ok(format!("{name}: already trusted"));
    }
    let mut keys = plan.hooks.iter().map(|(key, _)| key.as_str());
    if mentions(&text, &plan.project) || keys.any(|key| mentions(&text, key)) {
        return Err(Error::Setup(format!(
            "{name} already mentions these keys, or holds a line this can't read. Nothing written. Run doctor remove, or take them out by hand: {:?}",
            plan.lines()
        )));
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = plan.lines().join(eol);
    let hash = file_hash(path);
    let block = format!("{eol}{BEGIN}{hash}{eol}{body}{eol}{END}{eol}");
    if !ask(&format!("Append to {name}:\n{block}")) {
        return Ok(format!("{name}: nothing written"));
    }
    if file_hash(path) != hash {
        return Err(Error::Setup(format!("{name} changed while asking")));
    }
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(block.as_bytes())?;
    Ok(format!("{name}: written, now {}", file_hash(path)))
}

/// Exact reversal of the block, read back against the hash it recorded.
pub fn remove_codex(path: &Path, ask: Ask) -> Result<String> {
    let name = path.display();
    let text = read_text(path)?;
    let Some((start, end, before)) = find_block(&text) else {
        return Ok(format!("{name}: no astrolabe block"));
    };
    if !ask(&format!("Remove from {name}:\n{}", &text[start..end])) {
        return Ok(format!("{name}: nothing removed"));
    }
    if read_text(path)? != text {
        return Err(Error::Setup(format!("{name} changed while asking")));
    }
    let aside = path.with_extension(format!("astrolabe-{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    let mut file = options.write(true).create_new(true).open(&aside)?;
    file.write_all(format!("{}{}", &text[..start], &text[end..]).as_bytes())?;
    drop(file);
    std::fs::set_permissions(&aside, std::fs::metadata(path)?.permissions())?;
    std::fs::rename(&aside, path)?;
    let now = file_hash(path);
    let same = now == before;
    Ok(format!(
        "{name}: removed. Byte-identical to before: {same} ({before}, now {now})"
    ))
}

pub fn claude_state(path: &Path, key: &str) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let entry = root.get("projects")?.get(key)?;
    Some(entry["hasTrustDialogAccepted"].as_bool().unwrap_or(false))
}
