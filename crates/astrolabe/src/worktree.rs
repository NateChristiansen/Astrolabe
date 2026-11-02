//! One git worktree per session, under the repo's convention: `<parent>/<repo>.worktrees/
//! <name>` on a branch of the same name. Creating one can take seconds and gigabytes on a
//! big repo, so it runs aside and says so while it runs. A dirty one is never removed.
use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

/// How often a creation in progress reports while git is quiet.
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Creating { elapsed: Duration },
    Ready(Worktree),
    Failed(String),
}

/// A creation under way. `poll` never waits longer than a tick, so a launch can show it.
pub struct Creating {
    started: Instant,
    outcome: Receiver<std::result::Result<Worktree, String>>,
    finished: bool,
}

impl Creating {
    /// None after the last report.
    pub fn poll(&mut self) -> Option<Progress> {
        if self.finished {
            return None;
        }
        match self.outcome.recv_timeout(PROGRESS_EVERY) {
            Err(RecvTimeoutError::Timeout) => Some(Progress::Creating {
                elapsed: self.started.elapsed(),
            }),
            Ok(Ok(worktree)) => {
                self.finished = true;
                Some(Progress::Ready(worktree))
            }
            Ok(Err(message)) => {
                self.finished = true;
                Some(Progress::Failed(message))
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                Some(Progress::Failed("worktree creation stopped".to_owned()))
            }
        }
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains("..")
        && !name.ends_with(".lock")
        && !name.ends_with('.')
}

/// Where a session of this name goes. Fails on a name that isn't safe as a folder and a
/// branch, or a repo with no folder name.
pub fn path_for(repo: &Path, name: &str) -> Result<PathBuf> {
    if !valid_name(name) {
        return Err(invalid(format!(
            "session name {name:?} can't name a worktree"
        )));
    }
    let repo = std::path::absolute(repo)?;
    let (Some(parent), Some(folder)) = (repo.parent(), repo.file_name()) else {
        return Err(invalid(format!("{} has no parent", repo.display())));
    };
    let mut holder = folder.to_owned();
    holder.push(".worktrees");
    Ok(parent.join(holder).join(name))
}

/// Starts `git worktree add` aside and returns at once.
pub fn create(repo: &Path, name: &str) -> Result<Creating> {
    let path = path_for(repo, name)?;
    if path.exists() {
        return Err(invalid(format!("{} already exists", path.display())));
    }
    let worktree = Worktree {
        repo: repo.to_path_buf(),
        path,
        branch: name.to_owned(),
    };
    let (sender, outcome) = mpsc::channel();
    thread::spawn(move || {
        let added = git(
            &worktree.repo,
            &[
                "worktree".as_ref(),
                "add".as_ref(),
                "--quiet".as_ref(),
                "-b".as_ref(),
                worktree.branch.as_ref(),
                worktree.path.as_os_str(),
            ],
        );
        let _ = sender.send(added.map(|_| worktree));
    });
    Ok(Creating {
        started: Instant::now(),
        outcome,
        finished: false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    Removed {
        /// Kept if it holds commits nothing else has, so no work goes with the folder.
        branch_kept: bool,
    },
    /// Left where it is. Says what is in it, for the owner.
    KeptDirty(String),
}

/// Removes a clean worktree. Ignored files don't count as dirty, but untracked ones do.
/// Removal has no `--force`, so git refuses too if the check here was overtaken.
pub fn close(worktree: &Worktree) -> Result<Closed> {
    let status =
        git(&worktree.path, &["status".as_ref(), "--porcelain".as_ref()]).map_err(invalid)?;
    if !status.trim().is_empty() {
        return Ok(Closed::KeptDirty(status));
    }
    git(
        &worktree.repo,
        &[
            "worktree".as_ref(),
            "remove".as_ref(),
            worktree.path.as_os_str(),
        ],
    )
    .map_err(invalid)?;
    let deleted = git(
        &worktree.repo,
        &["branch".as_ref(), "-d".as_ref(), worktree.branch.as_ref()],
    );
    Ok(Closed::Removed {
        branch_kept: deleted.is_err(),
    })
}

/// `close`, only when nothing would be lost: no changed, untracked, ignored or hidden file, and no
/// commit that another branch or a remote lacks. Any doubt keeps it. Never forces.
pub fn close_if_nothing_lost(worktree: &Worktree) -> Result<Closed> {
    let ask = |args: &[&str]| {
        let args: Vec<&std::ffi::OsStr> = args.iter().map(std::convert::AsRef::as_ref).collect();
        git(&worktree.path, &args).map_err(invalid)
    };
    // `status` can't see an edit under these flags, and git would then delete it.
    let flags = ask(&["ls-files", "-v"])?;
    let hidden = flags
        .lines()
        .filter(|line| line.starts_with(|tag: char| tag == 'S' || tag.is_lowercase()));
    let hidden: Vec<&str> = hidden.collect();
    if !hidden.is_empty() {
        let hidden = hidden.join(", ");
        return Ok(Closed::KeptDirty(format!(
            "git is told not to watch: {hidden}"
        )));
    }
    let files = ask(&[
        "status",
        "--porcelain",
        "--ignored",
        "--untracked-files=all",
    ])?;
    if !files.trim().is_empty() {
        return Ok(Closed::KeptDirty(format!(
            "it has files not committed: {}",
            files.trim()
        )));
    }
    let others = format!("--exclude={}", worktree.branch);
    let unique = ask(&[
        "rev-list",
        "HEAD",
        &worktree.branch,
        "--not",
        &others,
        "--branches",
        "--remotes",
    ])?;
    if !unique.trim().is_empty() {
        return Ok(Closed::KeptDirty(
            "it has commits no other branch or remote has".to_owned(),
        ));
    }
    close(worktree)
}

pub(crate) fn git(cwd: &Path, args: &[&std::ffi::OsStr]) -> Output {
    run_git(Command::new("git").args(args), cwd)
}

// Git for Windows maps this name to NUL too.
const NO_HOOKS: &str = "core.hooksPath=/dev/null";
type Output = std::result::Result<String, String>;

/// No global or system git config or hooks, for a commit no owner's hook should see.
pub(crate) fn isolated_git(cwd: &Path, args: &[&str]) -> Output {
    let mut git = Command::new("git");
    git.args(["-c", NO_HOOKS]).args(args);
    git.env("HOME", cwd).env("XDG_CONFIG_HOME", cwd);
    run_git(
        git.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1"),
        cwd,
    )
}

/// Keeps the owner's home, for a private remote's credentials, but no hook or template.
pub(crate) fn clone_without_hooks(url: &str, path: &Path, cwd: &Path) -> Output {
    let mut git = Command::new("git");
    git.args(["-c", NO_HOOKS, "clone", "--template="]);
    run_git(git.arg(url).arg(path), cwd)
}

fn run_git(command: &mut Command, cwd: &Path) -> Output {
    let output = command
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git could not start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn invalid(message: String) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    ))
}
