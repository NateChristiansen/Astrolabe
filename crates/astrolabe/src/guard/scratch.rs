//! A session's own scratch folder: the one place outside the worktree where a recursive
//! delete is let through. What identifies it, on every app and system:
//! - a folder named exactly the session id the app gave the hook, which the command can't
//!   choose, so naming a folder "scratch" widens nothing;
//! - by its real location, links resolved, strictly inside the system temp folder or
//!   `/tmp`, so another session's folder, the shared parent and a link out don't qualify;
//! - existing, and owned by this user: on Unix the uid that owns `$HOME`; on Windows it's
//!   inside the per-user temp folder, which is already this user's alone.
//!
//! Each app's layout, and which apps make such a folder, is in `docs/hosts.md`. This reads
//! the file system, unlike the rest of the guard.
use std::path::{Path, PathBuf};

/// Whether `path`, as the guard normalized it, lies strictly inside this session's own
/// folder.
pub(super) fn owned(path: &str, session: &str, windows: bool) -> bool {
    let plausible = session.len() >= 8
        && session
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    let Some(real) = real_path(Path::new(path)).filter(|_| plausible) else {
        return false;
    };
    let real = super::normal(&real.to_string_lossy(), windows);
    let id = match windows {
        true => session.to_ascii_lowercase(),
        false => session.to_owned(),
    };
    let roots = temp_roots(windows);
    let parts: Vec<&str> = real.split('/').collect();
    (parts.iter().enumerate())
        .filter(|(at, part)| **part == id && at + 1 < parts.len())
        .any(|(at, _)| {
            let folder = parts[..=at].join("/");
            let under = |root: &String| folder.starts_with(&format!("{root}/"));
            roots.iter().any(under) && mine(Path::new(&folder))
        })
}

/// The path with every link resolved in the part that exists, the rest appended.
fn real_path(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            return Some(rest.iter().rev().fold(real, |path, part| path.join(part)));
        }
        rest.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

fn temp_roots(windows: bool) -> Vec<String> {
    let mut roots = vec![std::env::temp_dir()];
    if cfg!(unix) {
        roots.push("/tmp".into());
    }
    (roots.iter())
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .map(|root| super::normal(&root.to_string_lossy(), windows))
        .filter(|root| root.len() > 1)
        .collect()
}

#[cfg(unix)]
fn mine(folder: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let home = std::env::var_os("HOME").and_then(|home| std::fs::metadata(home).ok());
    let meta = std::fs::symlink_metadata(folder).ok();
    meta.zip(home)
        .is_some_and(|(meta, home)| meta.is_dir() && meta.uid() == home.uid())
}

#[cfg(not(unix))]
fn mine(folder: &Path) -> bool {
    std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.is_dir())
}
