//! Whether Codex still runs Astrolabe's hooks on this machine, read from Codex's own files.
//!
//! Codex (0.164.0 on) stores, with each hook the owner trusts, a fingerprint of the whole
//! installed plugin folder, and skips a hook without saying so once the folder no longer
//! matches it. Reading that fingerprint and computing the same one from the installed copy
//! tells a confirmed loss from trust that holds. Codex's files are only read, never written.
//! The fingerprint follows `codex-rs/hooks/src/engine/plugin_fingerprint.rs` and the legacy
//! baseline `codex-rs/hooks/src/engine/plugin_trust.rs`, both unchanged from 0.164.0 to
//! 0.166.0.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

const PLUGIN: &str = "astrolabe";
const USAGE: &str = "usage: astrolabe codex-hooks [--codex-home <folder>] [--plugin <folder>]";

/// What Codex's stored trust says about Astrolabe's hooks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// Every hook's stored fingerprint matches the installed copy.
    Trusted,
    /// A stored fingerprint differs from the installed copy: Codex skips that hook.
    Lost,
    /// Installed, and no hook has ever been trusted.
    NotTrusted,
    /// The owner turned the plugin or one of its hooks off in Codex.
    Off,
    /// The files don't settle it, and why.
    Unverified(String),
}

impl Trust {
    /// What the owner reads, on the board and from `astrolabe status`.
    pub fn sentence(&self) -> String {
        match self {
            Trust::Trusted => "Codex runs Astrolabe's hooks here: their trust matches the installed plugin.".to_owned(),
            Trust::Lost => "Codex has stopped running Astrolabe's hooks here, guard included: the plugin changed since you trusted them. Review them in Codex (/hooks in the CLI, Settings → Hooks in the app).".to_owned(),
            Trust::NotTrusted => "Codex doesn't run Astrolabe's hooks here: they've never been trusted. Review them in Codex to turn them on.".to_owned(),
            Trust::Off => "Astrolabe's hooks are turned off in Codex here.".to_owned(),
            Trust::Unverified(why) => format!("Can't tell whether Codex runs Astrolabe's hooks here: {why}."),
        }
    }
}

/// Codex's folder on this machine: `CODEX_HOME`, else `~/.codex`.
pub fn home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Some(dir.into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".codex"))
}

/// This machine's answer, or None when Codex or Astrolabe's plugin isn't installed here.
pub fn here() -> Option<Trust> {
    read(&home()?, None)
}

/// `astrolabe codex-hooks`: the answer, and whether hooks run. `--plugin` names a folder to
/// check in place of the installed copy, as one about to be installed would be.
pub fn command(args: &[&str]) -> Result<(String, bool), String> {
    let (mut codex_home, mut plugin) = (None, None);
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let target = match *word {
            "--codex-home" => &mut codex_home,
            "--plugin" => &mut plugin,
            _ => return Err(USAGE.to_owned()),
        };
        match words.next() {
            Some(v) if !v.is_empty() && !v.starts_with("--") => *target = Some(PathBuf::from(v)),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let codex_home = codex_home
        .or_else(home)
        .ok_or("no home folder; pass --codex-home <folder>")?;
    Ok(match read(&codex_home, plugin.as_deref()) {
        Some(trust) => (trust.sentence(), trust == Trust::Trusted),
        None => (
            "Astrolabe's plugin isn't installed in Codex here.".to_owned(),
            false,
        ),
    })
}

/// The answer for the Codex folder `codex_home`, checking `plugin` in place of the installed
/// copy when given. None when Codex keeps no config there or doesn't list the plugin.
pub fn read(codex_home: &Path, plugin: Option<&Path>) -> Option<Trust> {
    let config = codex_home.join("config.toml");
    let text = match fs::read_to_string(&config) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => return Some(unverified(format!("its config can't be read ({e})"))),
    };
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return Some(unverified("its config isn't valid TOML"));
    };
    let ids: Vec<(String, bool)> = doc
        .get("plugins")
        .and_then(Item::as_table_like)
        .into_iter()
        .flat_map(|t| t.iter())
        .filter(|(id, _)| id.split_once('@').is_some_and(|(name, _)| name == PLUGIN))
        .map(|(id, item)| {
            let on = item.get("enabled").and_then(Item::as_bool) != Some(false);
            (id.to_owned(), on)
        })
        .collect();
    let id = match ids.as_slice() {
        [] => return None,
        [(_, false)] => return Some(Trust::Off),
        [(id, true)] => id.clone(),
        _ => return Some(unverified("more than one Astrolabe plugin is listed")),
    };
    let marketplace = id.split_once('@').map_or("", |(_, m)| m);
    let root = match plugin {
        Some(folder) => folder.to_path_buf(),
        None => match installed(codex_home, marketplace) {
            Ok(root) => root,
            Err(why) => return Some(unverified(why)),
        },
    };
    let entries = hook_states(&doc, &id);
    if entries.is_empty() {
        return Some(Trust::NotTrusted);
    }
    let current = match fingerprint(&root) {
        Ok(hash) => hash,
        Err(e) => {
            return Some(unverified(format!(
                "the installed plugin can't be read ({e})"
            )));
        }
    };
    let baseline = || fs::read_to_string(legacy_path(&config, &id)).ok();
    let mut answer = Trust::Trusted;
    for entry in entries {
        let found = match entry {
            Entry::Off => Trust::Off,
            Entry::Untrusted => Trust::NotTrusted,
            Entry::Odd => unverified("Codex stores its trust in a form this build doesn't know"),
            Entry::Content(stored) => compare(&stored, &current),
            // Trust given before Codex fingerprinted plugins: it pins a one-time baseline.
            Entry::Legacy => match baseline() {
                Some(stored) => compare(&stored, &current),
                None => unverified("its trust predates Codex's check of plugin contents"),
            },
        };
        if rank(&found) > rank(&answer) {
            answer = found;
        }
    }
    Some(answer)
}

fn unverified(why: impl Into<String>) -> Trust {
    Trust::Unverified(why.into())
}

/// The worst finding across hooks wins: one skipped hook is a loss.
fn rank(trust: &Trust) -> u8 {
    match trust {
        Trust::Trusted => 0,
        Trust::NotTrusted => 1,
        Trust::Off => 2,
        Trust::Unverified(_) => 3,
        Trust::Lost => 4,
    }
}

/// A stored fingerprint in a form this build doesn't know says nothing about a loss.
fn compare(stored: &str, current: &str) -> Trust {
    let known = stored.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    });
    if !known {
        unverified("Codex stores its trust in a form this build doesn't know")
    } else if stored == current {
        Trust::Trusted
    } else {
        Trust::Lost
    }
}

enum Entry {
    Content(String),
    Legacy,
    Untrusted,
    Off,
    Odd,
}

/// Each of the plugin's hooks Codex keeps a state for, keyed `<id>:<hooks file>:<event>:…`.
fn hook_states(doc: &DocumentMut, id: &str) -> Vec<Entry> {
    let prefix = format!("{id}:");
    let Some(states) = doc
        .get("hooks")
        .and_then(|h| h.get("state"))
        .and_then(Item::as_table_like)
    else {
        return Vec::new();
    };
    states
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, item)| {
            let Some(state) = item.as_table_like() else {
                return Entry::Odd;
            };
            if state.get("enabled").and_then(Item::as_bool) == Some(false) {
                return Entry::Off;
            }
            match (
                state.get("trusted_hash").map(|i| i.as_str()),
                state.get("trusted_content_hash").map(|i| i.as_str()),
            ) {
                (None, _) => Entry::Untrusted,
                (Some(Some(_)), Some(Some(content))) => Entry::Content(content.to_owned()),
                (Some(Some(_)), None) => Entry::Legacy,
                _ => Entry::Odd,
            }
        })
        .collect()
}

/// The one installed copy: `plugins/cache/<marketplace>/astrolabe/<version>`.
fn installed(codex_home: &Path, marketplace: &str) -> Result<PathBuf, String> {
    let parent = codex_home
        .join("plugins")
        .join("cache")
        .join(marketplace)
        .join(PLUGIN);
    let mut copies: Vec<PathBuf> = fs::read_dir(&parent)
        .map_err(|_| "Codex lists the plugin but keeps no installed copy of it".to_owned())?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    match copies.len() {
        1 => Ok(copies.remove(0)),
        0 => Err("Codex lists the plugin but keeps no installed copy of it".to_owned()),
        _ => Err("more than one installed copy of the plugin is kept".to_owned()),
    }
}

/// Where Codex pins the one-time baseline for trust given before it fingerprinted plugins.
fn legacy_path(config: &Path, id: &str) -> PathBuf {
    let mut identity = Sha256::new();
    for part in [config.as_os_str().as_encoded_bytes(), id.as_bytes()] {
        identity.update((part.len() as u64).to_le_bytes());
        identity.update(part);
    }
    let parent = config.parent().unwrap_or(Path::new("."));
    parent
        .join(".hook-trust")
        .join(format!("{:x}", identity.finalize()))
}

/// Codex's fingerprint of a plugin folder: a walk in name order hashing each entry's path
/// relative to the folder, in the platform's own form, then its kind, and a file's length
/// and bytes. Python caches, virtual environments and package metadata are skipped.
pub fn fingerprint(root: &Path) -> io::Result<String> {
    if !fs::symlink_metadata(root)?.is_dir() {
        return Err(io::Error::other("plugin root is not a directory"));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut hash = Sha256::new();
    hash.update(b"codex-plugin-bundle-v1");
    let mut buffer = [0_u8; 64 * 1024];
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        if relative
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|name| {
                matches!(name, "__pycache__" | ".venv")
                    || name.ends_with(".pyc")
                    || name.ends_with(".egg-info")
            })
        {
            continue;
        }
        let name = relative.as_os_str().as_encoded_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            hash.update(b"directory");
            let mut entries = fs::read_dir(&path)?.collect::<io::Result<Vec<_>>>()?;
            entries.sort_by_key(fs::DirEntry::file_name);
            pending.extend(entries.iter().rev().map(fs::DirEntry::path));
        } else if metadata.is_file() {
            hash.update(b"file");
            let length = metadata.len();
            hash.update(length.to_le_bytes());
            let mut file = fs::File::open(&path)?.take(length.saturating_add(1));
            let mut read = 0_u64;
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                read += count as u64;
                hash.update(&buffer[..count]);
            }
            if read != length {
                return Err(io::Error::other("a plugin file changed while it was read"));
            }
        } else {
            return Err(io::Error::other(format!(
                "{} is neither a file nor a folder",
                path.display()
            )));
        }
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}
