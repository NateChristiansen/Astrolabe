//! Repository observations used by stamps and save reminders.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// How many untracked files, and how many of their bytes, the print reads whole. Opening a
/// file is the cost (about 3 ms each on Windows with a virus scanner), so this caps a
/// save's wait at about a second however much is untracked.
const READ_FILES: u32 = 256;
const READ_BYTES: u64 = 8 << 20;

/// What a session's product repo looks like right now, for the stamp and the nudge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoFacts {
    pub branch: String,
    pub head: String,
    pub unpushed: Option<u32>,
    pub dirty: bool,
    /// HEAD, the content of `git diff HEAD` and each untracked file's name, size and modified
    /// time (and the first few files' contents), hashed: the same tree gives the same print,
    /// and a second edit to a dirty file changes it. In the
    /// notes checkout HEAD is left out, since there every save and pull moves it by the
    /// plugin's own hand; hand edits and new files still count.
    pub fingerprint: String,
}

impl RepoFacts {
    /// Facts of the repo at `cwd`, or None when it isn't a git checkout or couldn't be
    /// read whole (a hook's time for reads ran out, or git failed partway). None is "not
    /// known", which is not the same as nothing having changed: no fingerprint is stored
    /// from it, and a session with none stored is told nothing about its repo moving.
    pub fn read(cwd: &Path) -> Option<Self> {
        let args = ["rev-parse", "HEAD"].map(AsRef::as_ref);
        let head = crate::worktree::git(cwd, &args).ok()?.trim().to_owned();
        Self::read_from(cwd, head)
    }

    /// The rest, once HEAD is known. None when the status or the diff couldn't be read: an
    /// empty one would hash as a clean tree, which this may not be.
    fn read_from(cwd: &Path, head: String) -> Option<Self> {
        let git = |args: &[&str]| {
            let args: Vec<&OsStr> = args.iter().map(AsRef::as_ref).collect();
            crate::worktree::git(cwd, &args).ok()
        };
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
        let status = raw(&["status", "--porcelain", "-z", "--untracked-files=all"])?;
        let top = raw(&["rev-parse", "--show-toplevel"])
            .map(|t| path_of(t.strip_suffix(b"\n").unwrap_or(&t)))
            .unwrap_or_else(|| cwd.to_owned());
        let diff = git(&["diff", "HEAD"])?;
        use sha2::{Digest, Sha256};
        let mut print = Sha256::new();
        crate::deadline::stall("fingerprint-identity");
        if !crate::activate::notes_identity(cwd)? {
            print.update(head.as_bytes());
        }
        print.update(diff.as_bytes());
        // Untracked files count by name, size and modified time, and the first few by their
        // bytes too, so a quick same-size edit still shows. Reading every one doesn't scale:
        // a checkout holding a 2.5 GB node_modules took 43 s, past the hook's time limit, and
        // its saves were dropped (2026-11-06).
        let (mut files_left, mut bytes_left) = (READ_FILES, READ_BYTES);
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
            let path = top.join(path_of(name));
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            print.update(meta.len().to_le_bytes());
            print.update(modified.to_le_bytes());
            if files_left > 0 && meta.len() <= bytes_left {
                files_left -= 1;
                bytes_left -= meta.len();
                if let Ok(bytes) = std::fs::read(&path) {
                    print.update(&bytes);
                }
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
