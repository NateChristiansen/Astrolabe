//! Repository observations used by stamps and save reminders.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// What a session's product repo looks like right now, for the stamp and the nudge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    pub branch: String,
    pub head: String,
    pub unpushed: Option<u32>,
    pub dirty: bool,
    /// HEAD, the content of `git diff HEAD` and the untracked files' contents, hashed: the
    /// same tree gives the same print, and a second edit to a dirty file changes it. In the
    /// notes checkout HEAD is left out, since there every save and pull moves it by the
    /// plugin's own hand; hand edits and new files still count.
    pub fingerprint: String,
}

impl RepoFacts {
    /// Facts of the repo at `cwd`, or None when it isn't a git checkout.
    pub fn read(cwd: &Path) -> Option<Self> {
        let git = |args: &[&str]| {
            let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
            crate::worktree::git(cwd, &args).ok()
        };
        let head = git(&["rev-parse", "HEAD"])?.trim().to_owned();
        let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])
            .map(|b| b.trim().to_owned())
            .unwrap_or_default();
        let unpushed =
            git(&["rev-list", "--count", "@{upstream}..HEAD"]).and_then(|n| n.trim().parse().ok());
        // `-z` names files as they are, never quoted, and relative to the repo root, which
        // is where they're read from whatever folder the session is in. Both stay bytes:
        // a name needn't be UTF-8, and a root may end in a space.
        let raw = |args: &[&str]| {
            let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
            crate::worktree::git_bytes(cwd, &args).ok()
        };
        let status =
            raw(&["status", "--porcelain", "-z", "--untracked-files=all"]).unwrap_or_default();
        let top = raw(&["rev-parse", "--show-toplevel"])
            .map(|t| path_of(t.strip_suffix(b"\n").unwrap_or(&t)))
            .unwrap_or_else(|| cwd.to_owned());
        let diff = git(&["diff", "HEAD"]).unwrap_or_default();
        use sha2::{Digest, Sha256};
        let mut print = Sha256::new();
        if !crate::activate::in_notes(cwd) {
            print.update(head.as_bytes());
        }
        print.update(diff.as_bytes());
        let mut entries = status.split(|b| *b == 0).filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            // A rename or copy carries its old name as the next entry.
            if entry
                .get(..2)
                .is_some_and(|xy| xy.contains(&b'R') || xy.contains(&b'C'))
            {
                entries.next();
            }
            let Some(name) = entry.strip_prefix(b"?? ") else {
                continue;
            };
            print.update(entry);
            if let Ok(bytes) = std::fs::read(top.join(path_of(name))) {
                print.update(&bytes);
            }
        }
        let fingerprint = print
            .finalize()
            .iter()
            .take(12)
            .map(|b| format!("{b:02x}"))
            .collect();
        Some(Self {
            branch,
            head: head.chars().take(7).collect(),
            unpushed,
            dirty: !status.is_empty(),
            fingerprint,
        })
    }
}

/// A path as git printed it: bytes on Unix, where a name needn't be UTF-8; UTF-8 elsewhere,
/// which is what git for Windows prints.
fn path_of(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}
