//! The opt-in plugin: `astrolabe activate` and `astrolabe hook <event> --if-active`. Idle in
//! every session until that session switches on for an effort. A session is bound by its own
//! hook, never by a model: the pre-command hook sees `astrolabe activate …` with the session id
//! the app gave it and does the work there, outside any sandbox the command would run in. It
//! then rewrites the command to `astrolabe activate --show <ticket>`, which only prints what
//! the hook left. A model can type the command but not a ticket, so it can't name another
//! session. An accident guard like the rest: an agent can still delete its own state file.
use crate::hook::{self, comparable};
use crate::registry::AGENT;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Where activation state lives. Unset, `~/.local/state/astrolabe-layer`.
pub const STATE_VAR: &str = "ASTROLABE_LAYER_STATE";
/// Set at launch by a manager for a worker: switches on at session start.
pub const EFFORT_VAR: &str = "ASTROLABE_EFFORT";
const TICKET_LIFE_MS: i64 = 60_000;
/// A hook's text is cut near 10,000 characters on both clients (see `voice`).
const HOOK_LIMIT: usize = 9800;
const PROBE_NAME: &str = "astrolabe-guard-probe";

const INVOLVEMENT: [(&str, &str); 3] = [
    (
        "autonomous",
        "run to the end within authority, then report, because the owner chose to stay out of the loop for this effort",
    ),
    (
        "check in at plan",
        "show the plan and wait for the owner's go before changing files, because a plan is cheap to redirect and changed files aren't",
    ),
    (
        "each step",
        "report after each step and wait before the next, because the owner wants to steer each one",
    ),
];
const AUTHORITY: [(&str, &str); 3] = [
    (
        "read",
        "read and report; change no project files, because the owner hasn't handed this effort's changes to an agent",
    ),
    (
        "edit",
        "change files and commit locally on a branch, but don't push, because sharing the work is the owner's call here",
    ),
    (
        "push",
        "change, commit and push the effort's branch where the repo's own rules allow, because the owner allowed sharing for this effort",
    ),
];
/// Why and when work gets recorded, told at session start. Every rule says its reason, so a
/// newer model can judge the edge cases and a dead rule is easy to spot.
pub const RECORD_RULE: &str = "Work in this session gets an effort in the owner's notes, because those notes are the record of what's in flight and no app's memory carries across machines or providers. When the owner's request means a change, findings or a plan, first run one command alone, with the goal in its body (in PowerShell, the body as a here-string: `@'` … `'@` as the last argument, or piped in with `'@ | astrolabe effort new …`). Findings means they asked you to find out, look into, check, audit or compare something: that's an effort even when you report it in chat, because the owner wants to see it later in their notes. A question needs no effort only when answering takes no investigating, a line from what's already in front of you.\n  astrolabe effort new --title \"<short title>\" <<'ASTROLABE'\n  <the goal, one line>\n  ASTROLABE\nThen say your reading in one line (involvement · authority · one piece or multi-step) and proceed; add --involvement <autonomous | check in at plan | each step> or --authority <read | edit | push> only when the owner's words say so, and --initiative <name> only when the owner names one. Save before waiting on the owner, before a long run, and when a piece is done, because a closed window fires no hook: replace Current state (at most 12 lines, the owner's voice, Goal · Done · Decided · Verified · Next · Open, no paths or ids) with\n  astrolabe effort save [--status <state>] [--decision \"…\"] [--waits-for \"…\"] <<'ASTROLABE'\n  **Goal:** …\n  **Next:** …\n  ASTROLABE\n(PowerShell: the same with the body as a here-string, @' … '@, last or piped in.) Any other shape is refused with the accepted forms. States: proposed, ready, active, blocked (needs --waits-for; \"owner: <question>\" when it waits on them), done, abandoned (needs --reason). Under autonomous, mark done on your own checks; under check in at plan or each step, save with --waits-for \"owner: look at …\" and mark done after their look. A Stop-hook message about saving is the plugin, not the owner: act on it or say nothing changed, never treat it as a request.";

pub const GUARD_SCOPE: &str = "The guard is the plugin's pre-command hook. It reads each shell command this session runs (Claude's Bash and PowerShell tools, Codex's shell commands) and denies machine-wide kills, force pushes, hard resets, posting through gh, and recursive deletes outside the worktree, because those hit other people's work or can't be undone. It doesn't see file edits (Edit, Write, apply_patch), MCP tools, input typed into a shell that's already running, or anything at all if the app skipped the hook. Subagents of this session get the same guard but none of this context or authority, and their denials don't prove the guard for this session. It's an accident guard, not a sandbox.";
const NO_TICKET: &str = "No effort loaded: the plugin's hook didn't take this command, so it can't be bound to this session. That alone doesn't tell whether a guard runs here, so claim no protection until `astrolabe activate --status` says proven. Run the command alone, exactly as written, with nothing before or after it. If it still fails: in the notes checkout the plugin stays out, as that repo's own layer runs there; Astrolabe may be off for this machine (the owner runs `astrolabe on` in a terminal); on Codex, review and trust the plugin's hooks with /hooks; on Claude, check the plugin is enabled.";
/// What a refusal leaves running: it only ever comes from a hook, so the machine is on.
pub(crate) const STILL_ON: &str = "Astrolabe is still on for this machine: the guard still runs in this session, and the owner and voice notes from session start still apply. Only this effort's context and settings are missing.";
/// The machine switch's file in the state folder. Present means on.
const SWITCH: &str = "on";
/// The notes checkout and repos by name, in the state folder. Not the AppData database: a
/// packaged Windows app reads AppData from its own private copy, so it would miss a repo added
/// from a terminal.
/// One file per repo, `repos/<name>.json`: each add replaces only its own record, so two at
/// once can't lose each other's.
const REPOS: &str = "repos";

/// Whether Astrolabe is on for this machine. Off, the hooks do nothing.
pub fn machine_on(dir: &Path) -> bool {
    dir.join(SWITCH).is_file()
}

pub(crate) struct Repos(pub(crate) Map<String, Value>);

impl Repos {
    /// None written yet is no repos. A record that can't be read is an error, never skipped.
    fn read(dir: &Path) -> Result<Self, String> {
        let folder = dir.join(REPOS);
        let entries = match std::fs::read_dir(&folder) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self(Map::new())),
            Err(e) => return Err(format!("{}: {e}", folder.display())),
            Ok(entries) => entries,
        };
        let mut map = Map::new();
        for entry in entries {
            let path = entry
                .map_err(|e| format!("{}: {e}", folder.display()))?
                .path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let bad = || format!("{} isn't a repo record this program wrote", path.display());
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let record: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
            let (Some(name), Some(checkout)) =
                (record["name"].as_str(), record["checkout"].as_str())
            else {
                return Err(bad());
            };
            map.insert(name.to_owned(), checkout.into());
        }
        Ok(Self(map))
    }

    fn checkout(&self, name: &str) -> Option<PathBuf> {
        self.0.get(name)?.as_str().map(PathBuf::from)
    }
}

/// `astrolabe repo add` for the plugin, beside the database entry the notes repo's own layer
/// reads. This record is the plugin's authority.
pub fn add_repo(name: &str, checkout: &Path) -> Result<(), String> {
    let dir = state_dir().ok_or("Astrolabe has no home folder for its state")?;
    check_repo_name(name)?;
    let record = json!({"name": name, "checkout": checkout.display().to_string()});
    write_json(&dir.join(REPOS).join(format!("{name}.json")), &record)
}

/// A repo name is a file name in the plugin's records, so checked before anything is written.
pub fn check_repo_name(name: &str) -> Result<(), String> {
    let named = !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    if name.is_empty() || !named {
        return Err(format!(
            "repo name {name:?} must be letters, digits, '-', '_' or '.'"
        ));
    }
    Ok(())
}

/// The notes checkout as the plugin's own records name it, checked to be a git checkout.
fn notes_checked() -> Result<PathBuf, String> {
    let (_, notes) = notes_and_repos()?;
    common_dir(&notes).map(|_| notes.clone()).ok_or_else(|| {
        format!("the notes checkout on record, {}, isn't a git checkout here; run `astrolabe repo add agent <notes checkout>`", notes.display())
    })
}

/// Codex on Windows runs its shell commands in PowerShell, as Claude's PowerShell tool does.
fn in_powershell(provider: &str, tool: &Value) -> bool {
    tool == "PowerShell" || (cfg!(windows) && provider == "codex")
}

/// `astrolabe on | off | status`, for the machine. The owner's, from a terminal: the hook refuses
/// it inside a session.
pub fn machine(verb: &str) -> (String, bool) {
    let Some(dir) = state_dir() else {
        return (
            "Astrolabe has no home folder for its state.".to_owned(),
            false,
        );
    };
    let path = dir.join(SWITCH);
    let done = match verb {
        // Without the notes checkout on record the plugin can't stay out of it.
        "on" => notes_checked()
            .map_err(|why| format!("{why}. It's needed so the plugin stays out of the notes checkout, where that repo's own layer runs"))
            .and_then(|_| write_json(&path, &json!({"at_ms": now_ms()}))),
        "off" => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", path.display()))
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    };
    match (done, machine_on(&dir)) {
        (Err(why), _) => (format!("Astrolabe couldn't switch {verb}: {why}"), false),
        (Ok(()), true) => ("Astrolabe is on for this machine: every new session gets the owner and voice notes and the guard, and is told its repo's active efforts; none is loaded until the owner names one. That's the switch, not proof an app runs the hooks: in a session, `astrolabe activate --status` says.".to_owned(), true),
        (Ok(()), false) => ("Astrolabe is off for this machine: its hooks do nothing.".to_owned(), true),
    }
}

pub fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(STATE_VAR) {
        return Some(dir.into());
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    Some(PathBuf::from(home).join(".local/state/astrolabe-layer"))
}

/// Names this build of the program and the plugin whose hook runs it, so a guard proof from
/// one doesn't certify another: the program's version and file time, the plugin's folder, and
/// a hash of its hooks and manifest. Both clients tell a plugin's hook its folder.
pub fn hook_id() -> String {
    use sha2::{Digest, Sha256};
    let exe = std::env::current_exe().ok();
    let modified = exe.and_then(|exe| exe.metadata().ok()?.modified().ok());
    let ms = modified.and_then(|at| at.duration_since(UNIX_EPOCH).ok());
    let root = std::env::var_os("CLAUDE_PLUGIN_ROOT").map(PathBuf::from);
    let mut hash = Sha256::new();
    for file in ["hooks/hooks.json", ".claude-plugin/plugin.json"] {
        let bytes = root
            .as_ref()
            .and_then(|root| std::fs::read(root.join(file)).ok());
        hash.update(bytes.unwrap_or_default());
        hash.update([0]);
    }
    let hash: String = hash
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let root = root.map_or("no plugin".to_owned(), |r| r.display().to_string());
    let ms = ms.map_or(0, |d| d.as_millis());
    format!("{} {ms} {root} {hash}", env!("CARGO_PKG_VERSION"))
}

fn now_ms() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    since.as_millis() as i64
}

pub(crate) fn random_hex() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|why| format!("no randomness: {why}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The effort note's fields this needs. The owner sets them; nothing here writes them in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effort {
    pub number: String,
    pub file: String,
    pub title: String,
    pub repos: Vec<String>,
    pub involvement: String,
    pub authority: String,
    pub current: String,
    /// The file as read, hashed: what a save compares against, from the same read.
    pub hash: String,
}

/// `efforts/<nnnn>-*.md`, its properties (YAML frontmatter, as Obsidian keeps them) and its
/// `## Current state` section. Every failure names what's missing.
pub fn read_effort(notes: &Path, number: &str) -> Result<Effort, String> {
    let n: u32 =
        (number.trim().parse()).map_err(|_| format!("effort {number:?} isn't a number"))?;
    let prefix = format!("{n:04}-");
    let dir = notes.join("efforts");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let names: Vec<String> = (entries.flatten())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(&prefix) && name.ends_with(".md"))
        .collect();
    let [name] = names.as_slice() else {
        return Err(match names.len() {
            0 => format!(
                "no effort {n}: no efforts/{prefix}*.md in {}",
                notes.display()
            ),
            _ => format!("more than one efforts/{prefix}*.md: {}", names.join(", ")),
        });
    };
    let file = format!("efforts/{name}");
    let text = std::fs::read_to_string(notes.join(&file)).map_err(|e| format!("{file}: {e}"))?;
    let hash = crate::notes::hash(&text);
    let text = text.replace("\r\n", "\n");
    let props = frontmatter(&text);
    let title = (props
        .iter()
        .find(|(k, _)| k == "title")
        .map(|(_, v)| v.as_str()))
    .or_else(|| text.lines().find_map(|line| line.strip_prefix("# ")));
    let get = |key: &str| {
        let found = props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str());
        found.filter(|v| !v.is_empty()).ok_or_else(|| {
            format!("{file} has no `{key}` property; the owner sets it in the note's properties at the top, as in `repo: <name>`, `involvement: check in at plan`, `authority: edit`")
        })
    };
    let one_of = |key: &str, allowed: &[(&str, &str)]| {
        let value = get(key)?;
        let known = allowed.iter().any(|(name, _)| *name == value);
        let names: Vec<&str> = allowed.iter().map(|(name, _)| *name).collect();
        known
            .then(|| value.to_owned())
            .ok_or_else(|| format!("{file}: {key} {value:?} isn't one of {}", names.join(", ")))
    };
    let repos: Vec<String> = get("repo")?
        .split(',')
        .map(|r| r.trim().to_owned())
        .collect();
    let named = |r: &String| {
        !r.is_empty() && (r.chars()).all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    if !repos.iter().all(named) || repos.iter().any(|r| r.starts_with('.')) {
        return Err(format!(
            "{file}: repo {:?} isn't a list of repo names",
            get("repo")?
        ));
    }
    let current = text.split("\n## Current state\n").nth(1).map(|rest| {
        let end = rest.find("\n## ").unwrap_or(rest.len());
        rest[..end].trim().to_owned()
    });
    Ok(Effort {
        number: n.to_string(),
        title: title.unwrap_or(name).trim().to_owned(),
        repos,
        involvement: one_of("involvement", &INVOLVEMENT)?,
        authority: one_of("authority", &AUTHORITY)?,
        current: current
            .filter(|c| !c.is_empty())
            .ok_or_else(|| format!("{file} has no `## Current state` section"))?,
        file,
        hash,
    })
}

/// What some Windows editors put before a file's text.
const BOM: char = '\u{feff}';

/// The plain properties at the top of a note: `key: value`, quoted or not, and a list as
/// `[a, b]` or `- a` lines, joined with commas. Nested YAML isn't read.
pub(crate) fn frontmatter(text: &str) -> Vec<(String, String)> {
    // A byte-order mark, as some Windows editors save, isn't part of the first line.
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Vec::new();
    }
    let unquote = |v: &str| {
        v.trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_owned()
    };
    let mut props: Vec<(String, String)> = Vec::new();
    for line in lines.take_while(|line| line.trim_end() != "---") {
        if let (Some(item), Some((_, value))) =
            (line.trim_start().strip_prefix("- "), props.last_mut())
        {
            if !value.is_empty() {
                value.push_str(", ");
            }
            value.push_str(&unquote(item));
        } else if let Some((key, value)) = line.split_once(':').filter(|_| !line.starts_with(' ')) {
            let value = value.trim();
            let value = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                Some(list) => list.split(',').map(unquote).collect::<Vec<_>>().join(", "),
                None => unquote(value),
            };
            props.push((key.trim().to_ascii_lowercase(), value));
        }
    }
    props
}

pub(crate) fn common_dir(dir: &Path) -> Option<String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(AsRef::as_ref);
    let out = crate::worktree::git(dir, &args).ok()?;
    Some(comparable(Path::new(out.trim())))
}

/// A git remote in one form for comparing, so the scp, ssh and https forms of one hosted repo
/// match: no scheme or `user@`, `host:path` as `host/path`, no trailing `.git` or `/`, the
/// host lowercased and the path's case kept. An scp path from the root (`host:/srv/game`)
/// stays apart from one from the home folder (`host:srv/game`), as an ssh server can serve
/// different repos at the two. None for anything that isn't a network remote: a local path
/// or a `file:` URL names a folder on one machine only, so that repo is registered instead.
fn normalize_remote(url: &str) -> Option<String> {
    const SCHEMES: [&str; 6] = ["ssh", "git", "http", "https", "git+ssh", "ssh+git"];
    let url = url.trim();
    if url.is_empty() || url.contains(char::is_whitespace) || url.contains('\\') {
        return None;
    }
    let (host, path, rooted) = match url.split_once("://") {
        Some((scheme, rest)) => {
            if !SCHEMES.contains(&scheme.to_ascii_lowercase().as_str()) {
                return None;
            }
            let (host, path) = rest.split_once('/')?;
            (host, path, false)
        }
        // `host:path`, unless the part before the colon is a folder or a Windows drive.
        None => {
            let (host, path) = url.split_once(':').filter(|(h, _)| !h.contains('/'))?;
            (host, path, path.starts_with('/'))
        }
    };
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let drive = host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic());
    let path = path.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if host.is_empty() || drive || path.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(if rooted {
        format!("{host}:/{path}")
    } else {
        format!("{host}/{path}")
    })
}

/// The URL each of this folder's remotes fetches from, by name: `git remote -v` adds a
/// partial clone's filter after its URL, so its lines aren't read. Only the first
/// `remote.<name>.url`, as git fetches from that one alone and pushes to the rest, so a
/// push-only URL can't make this folder another repo. A remote with no fetch URL (only a
/// `pushurl`) is skipped, as there's nothing it fetches; some git versions then give the
/// remote's own name, which never reads as a remote URL. A worktree shares its checkout's
/// config, so it has the same ones.
fn folder_remotes(dir: &Path) -> Result<Vec<String>, String> {
    let fail = |why: String| format!("the remotes of {} can't be read: {why}", dir.display());
    let names = crate::worktree::git(dir, &["remote".as_ref()]).map_err(fail)?;
    let mut urls: Vec<String> = Vec::new();
    for name in names.lines().map(str::trim).filter(|n| !n.is_empty()) {
        let args = ["remote", "get-url", name].map(AsRef::as_ref);
        let Ok(out) = crate::worktree::git(dir, &args) else {
            continue;
        };
        let url = out.trim();
        if !url.is_empty() && !urls.iter().any(|u| u == url) {
            urls.push(url.to_owned());
        }
    }
    Ok(urls)
}

/// The remotes a repo note lists under `remote:`, as written. One that can't be read refuses,
/// naming the file, because skipping it could match a folder to the wrong repo or to none.
fn parse_remotes(file: &str, text: &str) -> Result<Vec<String>, String> {
    let text = text.replace("\r\n", "\n");
    let props = frontmatter(&text);
    let Some((_, value)) = props.iter().find(|(k, _)| k == "remote") else {
        return Ok(Vec::new());
    };
    let bad = |what: String| {
        format!(
            "{file}: {what}; the owner fixes it in the note's properties, as in `remote: git@example.com:team/game.git` or a list of them, because a remote that can't be read could match this repo to the wrong folder or to none"
        )
    };
    if value.is_empty() {
        return Err(bad("`remote` has no URL".to_owned()));
    }
    let urls: Vec<String> = value.split(',').map(|u| u.trim().to_owned()).collect();
    match urls.iter().find(|u| normalize_remote(u).is_none()) {
        Some(url) => Err(bad(format!("remote {url:?} isn't a git remote URL"))),
        None => Ok(urls),
    }
}

/// `knowledge/repos/<name>.md`'s remotes. No note is no remotes.
fn note_remotes(notes: &Path, name: &str) -> Result<Vec<String>, String> {
    let file = format!("knowledge/repos/{name}.md");
    match std::fs::read_to_string(notes.join(&file)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{file}: {e}")),
        Ok(text) => parse_remotes(&file, &text),
    }
}

/// Every repo note that lists a remote: its name (the file's stem) and its remotes, normalized.
fn remote_notes(notes: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let dir = notes.join("knowledge/repos");
    let entries = match std::fs::read_dir(&dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
        Ok(entries) => entries,
    };
    let mut found = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let (Some(stem), Some("md")) = (
            path.file_stem().and_then(|s| s.to_str()),
            path.extension().and_then(|e| e.to_str()),
        ) else {
            continue;
        };
        let file = format!("knowledge/repos/{stem}.md");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{file}: {e}"))?;
        let remotes = parse_remotes(&file, &text)?;
        if remotes.is_empty() {
            continue;
        }
        check_repo_name(stem).map_err(|why| {
            format!("{file} lists a `remote:`, but its file name is the repo's name and {why}, so effort notes couldn't name it in `repo:`; the owner renames the note")
        })?;
        let remotes = remotes.iter().filter_map(|r| normalize_remote(r)).collect();
        found.push((stem.to_owned(), remotes));
    }
    Ok(found)
}

/// The repo names this folder is, as a checkout or a worktree: those registered on this
/// machine for its git repo (`here`, its common directory), and those whose repo note lists
/// one of its remotes. A name can be both, even for different folders; this folder is every
/// name it matches.
pub(crate) fn repo_names(
    notes: &Path,
    registry: &Repos,
    cwd: &Path,
    here: &str,
) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = (registry.0.keys())
        .filter(|name| {
            let checkout = registry.checkout(name);
            checkout.and_then(|c| common_dir(&c)).as_deref() == Some(here)
        })
        .cloned()
        .collect();
    let remotes: Vec<String> = (folder_remotes(cwd)?.iter())
        .filter_map(|r| normalize_remote(r))
        .collect();
    if !remotes.is_empty() {
        for (name, theirs) in remote_notes(notes)? {
            if theirs.iter().any(|r| remotes.contains(r)) {
                names.push(name);
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// The effort's repo this session's folder is in, as a checkout or a worktree of it.
fn session_repo(
    notes: &Path,
    registry: &Repos,
    repos: &[String],
    cwd: &Path,
) -> Result<String, String> {
    let here = common_dir(cwd).ok_or_else(|| {
        format!("this session's folder {} isn't in a git repo, so it can't be matched to the effort's repo", cwd.display())
    })?;
    let names = repo_names(notes, registry, cwd, &here)?;
    if let Some(name) = repos.iter().find(|r| names.contains(r)) {
        return Ok(name.clone());
    }
    let mut unknown = Vec::new();
    for name in repos {
        if registry.checkout(name).is_none() && note_remotes(notes, name)?.is_empty() {
            unknown.push(format!("`astrolabe repo add {name}`"));
        }
    }
    let mut why = format!(
        "this session's repo isn't the effort's ({}), so it's refused; switch on from a session in that repo, or the owner adds this one to the effort's `repo`",
        repos.join(", ")
    );
    if !unknown.is_empty() {
        why += &format!(
            ". Neither registered on this machine nor given a `remote:` in its repo note, so no folder can match it: {}, run from a folder of that repo, adds it",
            unknown.join(", ")
        );
    }
    Err(why)
}

/// Where a remote goes when `repo add` writes it: `origin` if the folder has one, else the
/// first it lists. Only remotes another machine could match.
fn chosen_remote(dir: &Path) -> Result<Option<String>, String> {
    let args = ["remote", "get-url", "origin"].map(AsRef::as_ref);
    let origin = crate::worktree::git(dir, &args).ok();
    let origin = origin.map(|o| o.trim().to_owned());
    let mut remotes = folder_remotes(dir)?;
    remotes.retain(|r| normalize_remote(r).is_some());
    Ok(match origin.filter(|o| remotes.contains(o)) {
        Some(origin) => Some(origin),
        None => remotes.into_iter().next(),
    })
}

/// `remotes` as the note's `remote:` property, put in place of the one it has, or added.
fn with_remotes(text: &str, remotes: &[String]) -> String {
    let crlf = text.contains("\r\n");
    // Kept, and the properties after it found as `frontmatter` finds them.
    let bom = text.starts_with(BOM);
    let text = text.trim_start_matches(BOM).replace("\r\n", "\n");
    let block = match remotes {
        [one] => format!("remote: {one}\n"),
        _ => format!(
            "remote:\n{}",
            remotes
                .iter()
                .map(|r| format!("  - {r}\n"))
                .collect::<String>()
        ),
    };
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let end = (lines.first().map(|l| l.trim_end()) == Some("---"))
        .then(|| lines.iter().skip(1).position(|l| l.trim_end() == "---"))
        .flatten()
        .map(|i| i + 1);
    let out = match end {
        None => format!("---\n{block}---\n\n{text}"),
        Some(end) => {
            let key = (1..end).find(|&i| {
                let line = &lines[i];
                !line.starts_with(' ')
                    && line
                        .split_once(':')
                        .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case("remote"))
            });
            let (from, to) = match key {
                Some(i) => {
                    let items = lines[i + 1..end]
                        .iter()
                        .take_while(|l| l.trim_start().starts_with("- "))
                        .count();
                    (i, i + 1 + items)
                }
                None => (end, end),
            };
            lines.splice(from..to, [block]);
            lines.concat()
        }
    };
    let out = if bom { format!("{BOM}{out}") } else { out };
    if crlf { out.replace('\n', "\r\n") } else { out }
}

/// `astrolabe repo add <name> [<checkout>]` by remote: when `folder` has a remote another
/// machine could match, writes it into `knowledge/repos/<name>.md` in the notes checkout,
/// making a stub note if there's none, and says what it wrote. None when the folder has no
/// such remote, so the caller registers it on this machine instead. Never commits.
pub fn add_repo_note(name: &str, folder: &Path) -> Result<Option<Vec<String>>, String> {
    check_repo_name(name)?;
    common_dir(folder).ok_or_else(|| format!("{} isn't in a git repo", folder.display()))?;
    let Some(remote) = chosen_remote(folder)? else {
        return Ok(None);
    };
    let notes = notes_checked().map_err(|why| {
        format!("{why}. It's needed because a repo with a remote is added as a note in the notes checkout, which every machine shares")
    })?;
    let file = format!("knowledge/repos/{name}.md");
    let path = notes.join(&file);
    let unsaved = "Not committed or pushed: the notes checkout is the owner's to commit.";
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(fail(e)),
        Ok(text) => Some(text),
    };
    let Some(text) = text else {
        let stub = format!(
            "---\nremote: {remote}\n---\n\n# {name}\n\nCreated by `astrolabe repo add`, so sessions in this repo match it by its remote on every machine.\n"
        );
        std::fs::create_dir_all(path.parent().unwrap_or(&notes)).map_err(fail)?;
        std::fs::write(&path, stub).map_err(fail)?;
        return Ok(Some(vec![
            format!(
                "Wrote {}, a new note for repo {name} with remote {remote}: a session in a checkout or worktree with that remote is {name} on any machine.",
                path.display()
            ),
            unsaved.to_owned(),
        ]));
    };
    let mut listed = parse_remotes(&file, &text)?;
    let ours: Vec<String> = (folder_remotes(folder)?.iter())
        .filter_map(|r| normalize_remote(r))
        .collect();
    if let Some(known) =
        (listed.iter()).find(|r| normalize_remote(r).is_some_and(|r| ours.contains(&r)))
    {
        return Ok(Some(vec![format!(
            "{} already lists {known}, a remote of this folder, so nothing was written.",
            path.display()
        )]));
    }
    let before = listed.join(", ");
    listed.push(remote.clone());
    std::fs::write(&path, with_remotes(&text, &listed)).map_err(fail)?;
    let said = if before.is_empty() {
        format!(
            "Added remote {remote} to {}, which listed none: a session with that remote is {name} on any machine.",
            path.display()
        )
    } else {
        format!(
            "Added remote {remote} to {}, which listed {before} and still does: a session with any of them is {name}.",
            path.display()
        )
    };
    Ok(Some(vec![said, unsaved.to_owned()]))
}

/// A checkout's main folder from any folder in it or its worktrees, for a registry record
/// that outlives a worktree.
pub fn main_checkout(folder: &Path) -> Result<PathBuf, String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(AsRef::as_ref);
    let common = crate::worktree::git(folder, &args)
        .map_err(|why| format!("{} isn't in a git repo: {why}", folder.display()))?;
    let common = PathBuf::from(common.trim());
    if common.file_name().is_some_and(|n| n == ".git")
        && let Some(parent) = common.parent()
    {
        return Ok(parent.to_path_buf());
    }
    let args = ["rev-parse", "--show-toplevel"].map(AsRef::as_ref);
    let top = crate::worktree::git(folder, &args)
        .map_err(|why| format!("{} has no checkout folder: {why}", folder.display()))?;
    Ok(PathBuf::from(top.trim()))
}

/// One piece of the context, and what to read instead when it can't go inline.
struct Part {
    title: String,
    body: String,
    files: Vec<String>,
}

pub struct Activation {
    pub state: Value,
    head: String,
    parts: Vec<Part>,
}

fn owner_voice(notes: &Path) -> Result<Part, String> {
    Ok(Part {
        title: "owner and voice".to_owned(),
        body: crate::voice::read(notes)?,
        files: ["owner", "voice"]
            .map(|f| {
                notes
                    .join(format!("knowledge/{f}.md"))
                    .display()
                    .to_string()
            })
            .to_vec(),
    })
}

impl Activation {
    /// Everything, for a command's output.
    pub fn text(&self) -> String {
        let mut text = self.head.clone();
        for part in &self.parts {
            text += &format!("\n\n--- {}\n\n{}", part.title, part.body);
        }
        text
    }

    /// Under a hook's cut: what fits inline, in order, and the files to read for the rest.
    pub fn fitted(&self) -> String {
        let text = self.text();
        if text.len() <= HOOK_LIMIT {
            return text;
        }
        let (mut inline, mut later) = (self.head.clone(), Vec::new());
        let note = "\n\nNot inline, as a hook's text is cut near 10,000 characters. Read these now, before anything else:";
        let reserve = note.len()
            + self
                .parts
                .iter()
                .map(|p| p.files.len() * 200)
                .sum::<usize>();
        for part in &self.parts {
            let piece = format!("\n\n--- {}\n\n{}", part.title, part.body);
            if later.is_empty() && inline.len() + piece.len() + reserve <= HOOK_LIMIT {
                inline += &piece;
            } else {
                later.extend(part.files.iter().map(|f| format!("\n- {f}")));
            }
        }
        inline + note + &later.concat()
    }
}

/// The marker goes in the temp folder, which a sandboxed shell can still write, so a probe that
/// got past the hook leaves a trace.
/// Where probe and control markers go: the temp folder, the one place Codex's sandbox lets
/// a shell write on both systems (tested: not the state folder, on the Mac or under Windows'
/// sandbox). Made here, so a shell's `touch` finds it.
pub fn probe_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("astrolabe-probe");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn marker(token: &str) -> PathBuf {
    probe_dir().join(token)
}

/// Made by an allowed command in the same shell, so a missing probe marker means the deny
/// held, not that the shell couldn't write there. One per shell.
fn control_marker(token: &str, powershell: bool) -> PathBuf {
    let shell = if powershell { "powershell" } else { "posix" };
    probe_dir().join(format!("{token}-control-{shell}"))
}

fn make_file(path: &Path, powershell: bool) -> String {
    let path = path.display();
    if powershell {
        format!("New-Item -ItemType File -Force '{path}'")
    } else {
        format!("touch '{path}'")
    }
}

/// The shell a hook input's command runs in, as the denial log records it.
pub fn shell_of(input: &Value) -> &'static str {
    if in_powershell(provider(input), &input["tool_name"]) {
        "powershell"
    } else {
        "posix"
    }
}

/// What the session runs to prove its guard, in its own shell.
fn proof_steps(token: &str, shells: &[bool]) -> String {
    let steps = |powershell: bool| {
        format!(
            "First an allowed control, which must run: {}\nThen the probe, which the hook must deny: {}\n",
            make_file(&control_marker(token, powershell), powershell),
            probe(token, powershell)
        )
    };
    let body = match shells {
        [one] => steps(*one),
        _ => format!(
            "In the shell you use. For the POSIX shell (Bash, zsh):\n{}For PowerShell:\n{}",
            steps(false),
            steps(true)
        ),
    };
    format!(
        "Prove the guard for this session, each command alone and exactly as written. {body}Then run `astrolabe activate --status`."
    )
}

/// The shells a session may use, for proof steps given at session start, before any tool
/// runs: Claude on Windows has both Bash and PowerShell tools.
fn session_shells(provider: &str) -> Vec<bool> {
    if cfg!(windows) && provider == "claude" {
        vec![false, true]
    } else {
        vec![in_powershell(provider, &Value::Null)]
    }
}

/// In the session's shell, so that a probe the hook missed would leave its marker.
fn probe(token: &str, powershell: bool) -> String {
    let make = make_file(&marker(token), powershell);
    if powershell {
        format!("{make}; Stop-Process -Name {PROBE_NAME}")
    } else {
        format!("{make}; pkill -0 {PROBE_NAME}")
    }
}

pub(crate) fn state_path(dir: &Path, session: &str) -> PathBuf {
    let safe = |c: char| {
        if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '_'
        }
    };
    dir.join(format!(
        "{}.json",
        session.chars().map(safe).collect::<String>()
    ))
}

/// None when this session is off. An error when it's on but the file can't be read.
pub fn read_state(dir: &Path, session: &str) -> Result<Option<Value>, String> {
    let path = state_path(dir, session);
    match std::fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(state) if state["session"] == session && state["effort"].is_string() => {
                Ok(Some(state))
            }
            _ => Err(format!(
                "{} isn't a state file this program wrote",
                path.display()
            )),
        },
    }
}

/// Whole or not at all: written aside, then renamed.
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(fail)?;
    let aside = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&aside, value.to_string()).map_err(fail)?;
    std::fs::rename(&aside, path).map_err(fail)
}

/// What the hook did, left for the command to print. Old ones go as new ones come.
fn leave(dir: &Path, text: &str, ok: bool) -> Result<String, String> {
    leave_with(dir, text, ok, None, None)
}

/// A ticket that also names the command it answers, so the plain command can find it.
pub(crate) fn leave_with(
    dir: &Path,
    text: &str,
    ok: bool,
    digest: Option<&str>,
    cwd: Option<&Path>,
) -> Result<String, String> {
    let shown = dir.join("shown");
    for entry in std::fs::read_dir(&shown).into_iter().flatten().flatten() {
        let age = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok()?.elapsed().ok());
        if age.is_some_and(|age| age.as_millis() as i64 > TICKET_LIFE_MS) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let ticket = random_hex()?;
    let cwd = cwd.map(comparable);
    let body = json!({"text": text, "ok": ok, "at_ms": now_ms(), "digest": digest, "cwd": cwd});
    write_json(&shown.join(format!("{ticket}.json")), &body)?;
    Ok(ticket)
}

/// The freshest ticket the hook left for a command with this digest, within `within_ms`.
pub(crate) fn shown_for(
    dir: &Path,
    digest: &str,
    cwd: &str,
    within_ms: i64,
) -> Option<(String, bool, i64)> {
    let mut best: Option<(i64, String, bool)> = None;
    for entry in std::fs::read_dir(dir.join("shown"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let at = body["at_ms"].as_i64().unwrap_or(0);
        if body["digest"] == digest
            && body["cwd"] == cwd
            && now_ms() - at <= within_ms
            && best.as_ref().is_none_or(|b| at > b.0)
        {
            best = Some((
                at,
                body["text"].as_str().unwrap_or_default().to_owned(),
                body["ok"] == true,
            ));
        }
    }
    best.map(|(at, text, ok)| (text, ok, now_ms() - at))
}

/// Only while fresh. Reads and nothing else, so it works inside a client's sandbox.
fn show(dir: &Path, ticket: &str) -> Result<(String, bool), String> {
    let bad = || "that ticket isn't one the hook just issued".to_owned();
    if ticket.len() != 32 || !ticket.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let path = dir.join("shown").join(format!("{ticket}.json"));
    let bytes = std::fs::read(&path).map_err(|_| bad())?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    let fresh = (body["at_ms"].as_i64()).is_some_and(|at| now_ms() - at <= TICKET_LIFE_MS);
    let text = body["text"].as_str().filter(|_| fresh).ok_or_else(bad)?;
    Ok((text.to_owned(), body["ok"] == true))
}

/// The notes checkout on record, for commands run outside a hook.
pub fn notes_root() -> Result<PathBuf, String> {
    notes_and_repos().map(|(_, notes)| notes)
}

pub(crate) fn notes_and_repos() -> Result<(Repos, PathBuf), String> {
    let dir = state_dir().ok_or("Astrolabe has no home folder for its state")?;
    let repos = Repos::read(&dir).map_err(|e| format!("this machine's Astrolabe repos: {e}"))?;
    let notes = repos.checkout(AGENT).ok_or(
        "no notes checkout is set on this machine; run `astrolabe repo add agent <notes checkout>`",
    )?;
    Ok((repos, notes))
}

/// Everything checked, nothing written: the caller commits `state` once what it says is safe.
/// `previous` keeps a resumed session's switch-on time and probe.
pub fn switch_on(
    _dir: &Path,
    effort: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    previous: Option<&Value>,
    shells: &[bool],
) -> Result<Activation, String> {
    let (registry, notes) = notes_and_repos()?;
    let effort = read_effort(&notes, effort)?;
    let repo = session_repo(&notes, &registry, &effort.repos, cwd)?;
    let read = |file: String| {
        let text = std::fs::read_to_string(notes.join(&file)).map_err(|e| format!("{file}: {e}"));
        Ok::<_, String>(Part {
            body: text?.trim().to_owned(),
            files: vec![notes.join(&file).display().to_string()],
            title: file,
        })
    };
    let owner_voice = owner_voice(&notes)?;
    let current = Part {
        title: format!("{}, Current state", effort.file),
        body: effort.current.clone(),
        // Never the whole file: the rest holds history and plans that no longer apply.
        files: vec![format!(
            "{}: only its `## Current state` section, up to the next `## ` heading. Not the rest of that file, which holds history and superseded plans.",
            notes.join(&effort.file).display()
        )],
    };
    let parts = vec![
        current,
        owner_voice,
        read(format!("knowledge/repos/{repo}.md"))?,
        read("knowledge/machines.md".to_owned())?,
    ];
    let same = previous.filter(|p| p["effort"] == effort.number.as_str());
    let token = match same.and_then(|p| p["token"].as_str()) {
        Some(token) => token.to_owned(),
        None => random_hex()?,
    };
    let at_ms = same
        .and_then(|p| p["at_ms"].as_i64())
        .unwrap_or_else(now_ms);
    // The effort as delivered, so a save can tell whether it would overwrite another's, and
    // the repo as it stands, so the Stop hook can tell whether work moved.
    let delivered = Some(effort.hash.clone());
    // A resumed session keeps its baseline, so changes made before it closed still count.
    let baseline = match same.and_then(|p| p["baseline"].as_str()) {
        Some(kept) => Some(kept.to_owned()),
        None => crate::notes::RepoFacts::read(cwd).map(|f| f.fingerprint),
    };
    let mut state = json!({
        "session": session, "provider": provider, "effort": effort.number, "file": effort.file,
        "repo": repo, "cwd": comparable(cwd), "at_ms": at_ms, "token": token,
        "delivered": delivered, "baseline": baseline, "mark": false,
    });
    for keep in ["mark", "acknowledged", "last_save_ms"] {
        if let Some(value) = same.and_then(|p| p.get(keep)) {
            state[keep] = value.clone();
        }
    }
    fn meaning(table: &[(&str, &'static str)], value: &str) -> &'static str {
        table
            .iter()
            .find(|(name, _)| *name == value)
            .map_or("", |(_, m)| *m)
    }
    let head = format!(
        "Astrolabe is on in this session for effort {n}, \"{title}\" ({provider} session {session}, repo {repo}). The plugin's hook bound it to the session id the app gave; other sessions are unaffected.\n\n\
        Settings from {file}, set by the owner. Agents never raise them, because they're the owner's call:\n\
        - involvement: {inv}: {inv_m}.\n\
        - authority: {auth}: {auth_m}. It's a ceiling inside {repo} and this effort's scope, and never covers posting or destructive actions, because those reach other people or can't be undone.\n\
        - A narrower current instruction from the owner wins, because it's the newest word.\n\
        - In this build these settings are context, not enforced, apart from the guard. Switching on wrote only Astrolabe's own state file; that isn't leave to edit project files.\n\n\
        {GUARD_SCOPE}\n\n\
        {proof}\n\
        Report the effort and what the status says. Until it says proven, claim no protection, and claim none for commands that ran before switch-on. To drop the effort from this session: `astrolabe activate --off`; the guard stays while Astrolabe is on for the machine.\n\n{freshness}\n\n{RECORD_RULE}",
        n = effort.number,
        freshness = freshness(&notes, &effort, cwd),
        title = effort.title,
        file = effort.file,
        inv = effort.involvement,
        inv_m = meaning(&INVOLVEMENT, &effort.involvement),
        auth = effort.authority,
        auth_m = meaning(&AUTHORITY, &effort.authority),
        proof = proof_steps(&token, shells),
    );
    Ok(Activation { state, head, parts })
}

/// What the stamp says against the repo here: who saved last and when, and what moved since.
/// Reported at the last published checkpoint; work after it on another machine isn't visible.
fn freshness(notes: &Path, effort: &Effort, cwd: &Path) -> String {
    let text = std::fs::read_to_string(notes.join(&effort.file)).unwrap_or_default();
    let file = crate::effort::EffortFile::parse(&text).unwrap_or_default();
    let (Some(saved_by), Some(updated)) = (file.prop("saved_by"), file.prop("updated")) else {
        return "This effort has no stamp yet: it was written by hand, not saved by a session."
            .to_owned();
    };
    let here = crate::notes::machine_name();
    let other = !saved_by.starts_with(&here);
    let mut text = format!("Last saved by {saved_by} at {updated}");
    if let (Some(branch), Some(head)) = (file.prop("branch"), file.prop("head")) {
        text += &format!(", on {branch} at {head}");
        if let Some(n) = file.prop("unpushed").filter(|n| *n != "0") {
            text += &format!(", {n} unpushed commit(s) then");
        }
        if file.prop("dirty") == Some("true") {
            text += ", tree dirty then";
        }
    }
    text.push('.');
    if other {
        text +=
            " That was another machine: anything after that save there isn't visible from here.";
    }
    if let Some(facts) = crate::notes::RepoFacts::read(cwd) {
        let same_branch = file.prop("branch") == Some(facts.branch.as_str());
        let moved = file.prop("head").is_some_and(|h| h != facts.head);
        text += &format!(
            " Here now: {} at {}{}{}.",
            facts.branch,
            facts.head,
            if moved && same_branch {
                ", moved since that save"
            } else {
                ""
            },
            if facts.dirty { ", tree dirty" } else { "" }
        );
    }
    let warnings =
        crate::notes::Notes::new(notes.to_path_buf(), state_dir().unwrap_or_default()).warnings();
    if !warnings.is_empty() {
        text += &format!(" Notes: {}", warnings.join(" "));
    }
    text
}

/// Whether the hook denied this activation's probe, by this build, and the probe didn't run.
/// A session with no effort still has a probe: named for the session, proven from the
/// machine switch's time.
fn session_token(session: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(format!("astrolabe probe {session}"));
    hash.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// Whether this session's own hook (not a child's), of this build and plugin, denied the
/// probe since `since_ms`, and the probe didn't run.
fn proof(session: &str, token: &str, since_ms: i64, powershell: bool) -> String {
    if marker(token).exists() {
        return "Guard FAILED: the probe ran (its marker file exists), so no hook stopped it. Claim no protection and tell the owner.".to_owned();
    }
    let log = state_dir().map(|dir| dir.join(hook::DENIAL_LOG));
    let log = log
        .and_then(|log| std::fs::read_to_string(log).ok())
        .unwrap_or_default();
    let program = hook_id();
    // Each denial counts only for the shell it was sent to, and only in that shell's form.
    let denied_in = |shell: bool| {
        let name = if shell { "powershell" } else { "posix" };
        let form = probe(token, shell);
        (log.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok()))
        .any(|l| {
            l["session_id"] == session
                && l["agent_id"].is_null()
                && l["shell"] == name
                && l["command"] == form.as_str()
                && l["hook"] == program.as_str()
                && l["at_ms"].as_i64().is_some_and(|at| at >= since_ms)
        })
    };
    let proven: Vec<&str> = [false, true]
        .into_iter()
        .filter(|&shell| denied_in(shell) && control_marker(token, shell).exists())
        .map(|shell| {
            if shell {
                "PowerShell"
            } else {
                "the POSIX shell (Bash, zsh)"
            }
        })
        .collect();
    if !proven.is_empty() {
        return format!(
            "Guard proven for this session in {}: the hook denied the probe there, its marker file wasn't made, and the allowed control in the same shell made its own. {GUARD_SCOPE}",
            proven.join(" and ")
        );
    }
    let half = if denied_in(powershell) {
        " The hook denied the probe, but the allowed control's marker isn't there, so a missing probe marker doesn't yet show the deny held."
    } else {
        ""
    };
    format!(
        "Guard not proven yet:{half} Proof needs, in this session's own shell, by this build of the program and this version of the plugin's hooks, since it was set up: {}\nUntil then claim no protection.",
        proof_steps(token, &[powershell])
    )
}

pub fn status(dir: &Path, session: &str, powershell: bool) -> String {
    // Proof counts only from the latest switch-on: off and on again starts it over.
    let on = std::fs::read(dir.join(SWITCH)).ok();
    let on = on.and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let since = on.and_then(|on| on["at_ms"].as_i64()).unwrap_or(i64::MAX);
    let state = match read_state(dir, session) {
        Ok(None) => {
            return format!(
                "No effort is set in this session. {}",
                proof(session, &session_token(session), since, powershell)
            );
        }
        Err(why) => {
            return format!(
                "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
            );
        }
        Ok(Some(state)) => state,
    };
    let token = state["token"].as_str().unwrap_or_default();
    let on = format!(
        "Astrolabe is on for effort {} in repo {}.",
        state["effort"].as_str().unwrap_or_default(),
        state["repo"].as_str().unwrap_or_default()
    );
    if let Some(why) = block_reason(dir, &state, session) {
        return format!(
            "{on} {}",
            blocked(state["effort"].as_str().unwrap_or_default(), &why)
        );
    }
    format!(
        "{on} {}",
        proof(
            session,
            token,
            state["at_ms"].as_i64().unwrap_or(i64::MAX).max(since),
            powershell
        )
    )
}

/// `astrolabe activate --show <ticket>`, as the hook rewrote it: what to print, and whether
/// it worked. Anything else reaching here is a command no hook took.
pub fn command(args: &[&str]) -> (String, bool) {
    match (args, state_dir()) {
        (["--show", ticket], Some(dir)) => show(&dir, ticket)
            .unwrap_or_else(|why| (format!("No effort loaded: {why}. {NO_TICKET}"), false)),
        _ => (NO_TICKET.to_owned(), false),
    }
}

/// What a switch changes, done only after its result is ready to show.
enum Change {
    Keep,
    Write(Value),
    Remove,
}

fn commit(dir: &Path, session: &str, change: &Change) -> Result<(), String> {
    let path = state_path(dir, session);
    if !matches!(change, Change::Keep) {
        let _ = std::fs::remove_file(fallback_block(dir, session));
    }
    match change {
        Change::Keep => Ok(()),
        Change::Write(state) => write_json(&path, state),
        Change::Remove => {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
        }
    }
}

/// `astrolabe activate <args>` for this session, run in its hook: what to show, and the
/// change to make once it's ready, with what to show instead if that fails.
fn act(
    dir: &Path,
    args: &[&str],
    session: &str,
    provider: &str,
    cwd: &Path,
    powershell: bool,
) -> (String, bool, Change, String) {
    let previous = read_state(dir, session);
    match args {
        ["--status"] => (
            status(dir, session, powershell),
            true,
            Change::Keep,
            String::new(),
        ),
        ["--off"] => match previous {
            Ok(None) => ("This session has no effort set.".to_owned(), true, Change::Keep, String::new()),
            _ => (
                "This session has no effort now. The guard stays while Astrolabe is on for this machine.".to_owned(),
                true,
                Change::Remove,
                "Astrolabe could NOT drop this session's effort, so it stays as it was".to_owned(),
            ),
        },
        ["--effort", effort] => {
            let previous = previous.ok().flatten();
            // The published checkpoint, not a stale local copy: another machine may hold it.
            let refreshed = crate::record::refresh_notes();
            match switch_on(dir, effort, session, provider, cwd, previous.as_ref(), &[powershell]) {
                Ok(activation) => {
                    let failed = refused(effort, "its state file couldn't be written", previous.as_ref());
                    let mut text = activation.text();
                    if let Some(note) = refreshed {
                        text = format!("{note}\n\n{text}");
                    }
                    (text, true, Change::Write(activation.state), failed)
                }
                Err(why) => (refused(effort, &why, previous.as_ref()), false, Change::Keep, String::new()),
            }
        }
        _ => ("Usage: astrolabe activate <--effort <n>|--status|--off>, run alone so the hook can bind it.".to_owned(), false, Change::Keep, String::new()),
    }
}

fn refused(effort: &str, why: &str, previous: Option<&Value>) -> String {
    let kept = match previous {
        Some(p) => format!(
            "Effort {} stays loaded as it was.",
            p["effort"].as_str().unwrap_or_default()
        ),
        None => "No effort is loaded in this session.".to_owned(),
    };
    format!("Effort {effort} NOT loaded: {why}. {kept} {STILL_ON}")
}

/// The shell tools a pre-command hook is sent for: Claude's two, and Codex's names for its
/// shell commands. Their input must carry the command.
const SHELL_TOOLS: [&str; 6] = [
    "bash",
    "powershell",
    "shell",
    "shell_command",
    "exec_command",
    "local_shell",
];

/// Why a pre-command hook's input can't be read, if it can't: not a JSON object (empty,
/// truncated, or anything else), no tool named, a shell tool with no command, or a command
/// the guard would read that isn't text or a list of text. The command is looked for in the
/// field the guard judges, so the two can't disagree. Input for a tool the guard doesn't
/// check is readable and passes as before.
fn unreadable(input: &Value) -> Option<&'static str> {
    if !input.is_object() {
        return Some("not a JSON object");
    }
    let Some(tool) = input["tool_name"].as_str().filter(|t| !t.is_empty()) else {
        return Some("no tool named");
    };
    // The guard's own exceptions: their `command` isn't a shell's.
    if tool.starts_with("mcp__") || tool == "apply_patch" {
        return None;
    }
    let shell = is_shell_tool(tool);
    match crate::guard::command_field(&input["tool_input"]) {
        None if shell => Some("no command for its shell tool"),
        Some(command) if !crate::guard::readable_command(command) => {
            Some("a command that isn't text or a list of text")
        }
        _ => None,
    }
}

fn is_shell_tool(tool: &str) -> bool {
    SHELL_TOOLS.iter().any(|s| tool.eq_ignore_ascii_case(s))
}

/// The tool an input names, read from its text when it was too big to parse whole: the first
/// `"tool_name": "<name>"`, as the apps put it ahead of the tool's input.
fn tool_named_in(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(1 << 16)]);
    let rest = text.split_once("\"tool_name\"")?.1.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let (name, _) = rest.split_once('"')?;
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c));
    (!name.is_empty() && plain).then(|| name.to_owned())
}

/// `astrolabe hook <event> --if-active`. Silent in a session that's off, except for its own
/// switch. Always exits 0, like the hooks it reuses.
pub fn gate(event: &str) {
    std::panic::set_hook(Box::new(|_| {}));
    let (bytes, over) = hook::read_stdin();
    let input: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let session = input["session_id"].as_str().filter(|s| !s.is_empty());
    // A child agent shares its parent's session id. It gets the guard, as every session does
    // while the machine is on, but no context or authority, can't switch, and doesn't count
    // toward the parent's proof.
    let child = input["agent_id"].as_str().is_some_and(|a| !a.is_empty());
    // Off for the machine: nothing at all, as if the plugin weren't there. In the notes
    // checkout its own layer runs, so the plugin stays out there too (a trial exception).
    if !state_dir().is_some_and(|dir| machine_on(&dir)) || in_notes(&input) {
        return;
    }
    match event {
        "pre-command" => {
            // Too big to read whole: a tool the guard doesn't check still runs, as it would
            // have; anything else can't be checked.
            let named = over.then(|| tool_named_in(&bytes)).flatten();
            if named.as_deref().is_some_and(|t| !is_shell_tool(t)) {
                return;
            }
            // Not recorded as a denial, so it counts toward no session's proof.
            let what = match over {
                true => {
                    Some("more than the hook reads whole, for a shell tool or one it couldn't name")
                }
                false => unreadable(&input),
            };
            if let Some(what) = what {
                return hook::say(&hook::deny(&format!(
                    "Astrolabe couldn't read what the app sent its hook ({what}), so it can't tell which session this is or check the command, and it blocks the command rather than let it run unguarded. If this keeps happening, the app's hook input has changed or the plugin is broken; the owner can run `astrolabe off` in a terminal to stop Astrolabe on this machine until it's fixed."
                )));
            }
            let answer = std::panic::catch_unwind(|| switch_command(&input, session, child));
            if let Ok(Some(answer)) = answer {
                return hook::say(&answer);
            }
            // A commit is work the save should cover; a child's counts for its parent.
            if let (Some(session), Some(command)) =
                (session, crate::guard::command_field(&input["tool_input"]))
                && crate::nudge::commits(&crate::guard::command_text(command))
            {
                let _ = std::panic::catch_unwind(|| crate::nudge::mark(session));
            }
            // The plugin's denials go in its state folder, which every app reads the same.
            let dir = state_dir();
            let guard = || hook::guard(&bytes, dir.as_deref());
            let (Some(session), Some(dir)) = (session, dir.as_deref()) else {
                return guard();
            };
            match read_state(dir, session) {
                Ok(None) => guard(),
                Ok(Some(state)) => match block_reason(dir, &state, session) {
                    Some(why) => hook::say(&hook::deny(&blocked(
                        state["effort"].as_str().unwrap_or_default(),
                        &why,
                    ))),
                    None => guard(),
                },
                Err(why) => hook::say(&hook::deny(&format!(
                    "Astrolabe is on for this session but can't read its state ({why}), so it can't judge this command. Repair: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
                ))),
            }
        }
        "post-tool" => {
            let tool = input["tool_name"].as_str().unwrap_or_default();
            if let (Some(session), true) = (session, crate::nudge::EDIT_TOOLS.contains(&tool)) {
                let _ = std::panic::catch_unwind(|| crate::nudge::mark(session));
            }
        }
        "stop" if !child => {
            let answer = std::panic::catch_unwind(|| crate::nudge::at_stop(&input, session?));
            if let Ok(Some(answer)) = answer {
                hook::say(&answer);
            }
        }
        "session-end" if !child => {
            if let Some(session) = session {
                let _ = std::panic::catch_unwind(|| crate::nudge::at_end(session));
            }
        }
        "session-start" if !child => {
            // Anything an earlier session left unpublished goes now, in the background.
            let _ = std::panic::catch_unwind(crate::nudge::push_pending);
            let text = std::panic::catch_unwind(|| session_start(&input, session?));
            if let Ok(Some(text)) = text {
                let inner = json!({"hookEventName": "SessionStart", "additionalContext": text});
                hook::say(&json!({ "hookSpecificOutput": inner }));
            }
        }
        _ => {}
    }
}

/// Whether this hook fired in the registered notes checkout or one of its worktrees, by the
/// registry and git's common directory.
fn in_notes(input: &Value) -> bool {
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let Some(here) = cwd.as_deref().and_then(common_dir) else {
        return false;
    };
    let notes = notes_and_repos().ok().map(|(_, notes)| notes);
    notes
        .and_then(|notes| common_dir(&notes))
        .is_some_and(|notes| notes == here)
}

pub(crate) fn provider(input: &Value) -> &'static str {
    // Codex adds a turn id to tool hooks and names its transcripts `rollout-…`; Claude does neither.
    let transcript = input["transcript_path"].as_str().unwrap_or_default();
    let name = Path::new(transcript).file_name().and_then(|n| n.to_str());
    let codex = input.get("turn_id").is_some() || name.is_some_and(|n| n.starts_with("rollout-"));
    if codex { "codex" } else { "claude" }
}

/// The hook's answer to `astrolabe activate …`: done here, for this session, and the command
/// rewritten to print it. Only a plain command is taken, so it's exactly what the hook read.
fn switch_command(input: &Value, session: Option<&str>, child: bool) -> Option<Value> {
    let tool_input = &input["tool_input"];
    let key = if tool_input.get("command").is_some() {
        "command"
    } else {
        "cmd"
    };
    let command = tool_input[key].as_str()?.trim();
    let words: Vec<&str> = command.split_whitespace().collect();
    if matches!(words.as_slice(), ["astrolabe", "on" | "off", ..]) {
        return Some(hook::deny(
            "The machine switch is the owner's: they run `astrolabe on` or `astrolabe off` in a terminal, because it changes every session. To drop this session's effort, run `astrolabe activate --off`.",
        ));
    }
    // An effort command is done here, in the hook, bound to the session the app named, and
    // the app runs only a print of the result (see `record`).
    if crate::effort::EffortCommand::is_effort_command(command) {
        let session = session?;
        if child {
            return Some(hook::deny(
                "A subagent can't create or save an effort: one session writes an effort's record, the one the owner talks to. Report to it instead.",
            ));
        }
        let powershell = in_powershell(provider(input), &input["tool_name"]);
        let cwd = input["cwd"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| ".".into());
        let (text, ok, digest, op_key) = match crate::effort::EffortCommand::parse(command) {
            Ok(parsed) => {
                // The same tool call retried gets the same key, so it can't commit twice.
                let op_key = match input["tool_use_id"].as_str().filter(|id| !id.is_empty()) {
                    Some(id) => crate::notes::hash(&format!("{session} {id} {}", parsed.digest())),
                    None => random_hex().unwrap_or_else(|_| parsed.digest()),
                };
                let done = match parsed.verb.as_str() {
                    "new" => crate::record::new(
                        &parsed,
                        &op_key,
                        session,
                        provider(input),
                        &cwd,
                        &[powershell],
                    ),
                    _ => crate::record::save(&parsed, &op_key, session, provider(input), &cwd),
                };
                (done.text, done.ok, Some(parsed.digest()), Some(op_key))
            }
            Err(why) => (
                format!("Effort command refused: {why}.\n{}", crate::effort::FORMS),
                false,
                None,
                None,
            ),
        };
        let left = state_dir()
            .ok_or_else(|| "no home folder".to_owned())
            .and_then(|dir| leave_with(&dir, &text, ok, digest.as_deref(), Some(&cwd)));
        return Some(match left {
            Ok(ticket) => {
                // The ticket now holds the result, so the intent that held it can go. A
                // failed one keeps its intent, which is its recovery plan.
                if let (Some(op_key), true) = (op_key, ok) {
                    crate::record::forget(&op_key);
                }
                rewrite_to_show(tool_input, key, &ticket, powershell)
            }
            Err(why) => hook::deny(&format!(
                "Astrolabe couldn't show this effort command's result ({why}); run `astrolabe notes status`."
            )),
        });
    }
    if words.get(..2) != Some(&["astrolabe", "activate"]) {
        return None;
    }
    if !(command.chars()).all(|c| c.is_ascii_alphanumeric() || " -_.".contains(c)) {
        return None;
    }
    if words.contains(&"--show") {
        return Some(hook::deny(
            "Astrolabe's tickets come only from its hook, so a typed one is refused. Run the command without --show.",
        ));
    }
    if child {
        return Some(hook::deny(
            "A subagent can't switch Astrolabe on or off: the switch is the parent session's. A subagent of a switched-on session gets the guard only.",
        ));
    }
    let session = session?;
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    let powershell = in_powershell(provider(input), &input["tool_name"]);
    let left = state_dir()
        .ok_or_else(|| "no home folder".to_owned())
        .and_then(|dir| {
            let (text, ok, change, failed) = act(
                &dir,
                &words[2..],
                session,
                provider(input),
                &cwd,
                powershell,
            );
            // The result first: a switch whose result can't be shown isn't made.
            let ticket = leave(&dir, &text, ok)?;
            if let Err(why) = commit(&dir, session, &change) {
                let path = dir.join("shown").join(format!("{ticket}.json"));
                let body =
                    json!({"text": format!("{failed}: {why}."), "ok": false, "at_ms": now_ms()});
                write_json(&path, &body).map_err(|_| format!("{failed}: {why}"))?;
            }
            Ok(ticket)
        });
    Some(match left {
        Ok(ticket) => rewrite_to_show(tool_input, key, &ticket, powershell),
        Err(why) => hook::deny(&format!(
            "Astrolabe couldn't finish this ({why}). Run `astrolabe activate --status` to see where this session stands; claim no new protection."
        )),
    })
}

/// The command the app runs instead: this program by its full path, printing the ticket.
fn rewrite_to_show(tool_input: &Value, key: &str, ticket: &str, powershell: bool) -> Value {
    // This program by its full path: a login shell's PATH can find another copy.
    let exe = std::env::current_exe().map(|exe| exe.display().to_string());
    let exe = exe.ok().filter(|exe| !exe.contains('\''));
    let exe = exe.map_or("astrolabe".to_owned(), |exe| format!("'{exe}'"));
    // PowerShell reads a quoted path followed by arguments as an error, so call it.
    let call = if powershell { "& " } else { "" };
    let mut updated = tool_input.clone();
    updated[key] = format!("{call}{exe} activate --show {ticket}").into();
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "updatedInput": updated,
    }})
}

/// Where a failed reload's block goes when the state file can't take it: the temp folder.
/// Named for the state folder too, as another folder's sessions are another switch.
fn fallback_block(dir: &Path, session: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let folder = Sha256::digest(dir.display().to_string());
    let folder: String = folder.iter().take(6).map(|b| format!("{b:02x}")).collect();
    let name = state_path(Path::new(""), session);
    std::env::temp_dir().join(format!("astrolabe-blocked-{folder}-{}", name.display()))
}

/// Why this session's shell commands are blocked, if a reload failed.
fn block_reason(dir: &Path, state: &Value, session: &str) -> Option<String> {
    if let Some(why) = state["reload_failed"].as_str() {
        return Some(why.to_owned());
    }
    let bytes = std::fs::read(fallback_block(dir, session)).ok()?;
    let body: Value = serde_json::from_slice(&bytes).ok()?;
    (body["effort"] == state["effort"])
        .then(|| body["why"].as_str().unwrap_or("unknown").to_owned())
}

/// A fresh session while the machine is on: the effort if its repo has exactly one active,
/// else the owner and voice notes and the guard, and why there's no effort.
fn fresh(session: &str, cwd: &Path, provider: &str) -> String {
    let (registry, notes) = match notes_and_repos() {
        Ok(found) => found,
        Err(why) => {
            return format!(
                "Astrolabe is on for this machine and the guard runs here, but owner and voice weren't delivered: {why}. Without the notes checkout on record the plugin also can't tell whether this is the notes repo, so it may be running beside that repo's own layer. Tell the owner.\n\n{GUARD_SCOPE}"
            );
        }
    };
    // Never loaded on its own: the owner may be here for something else (owner, 2026-11-03).
    let say = |n: &str| format!("say \"pick up {n}\" to load it");
    let offer = match active_efforts(&notes, &registry, cwd) {
        Ok((repos, _)) if repos.is_empty() => no_note(cwd),
        Ok((_, efforts)) if efforts.len() == 1 => {
            let (n, title) = &efforts[0];
            format!(
                "This repo's one active effort is {n}, \"{title}\"; {}.",
                say(n)
            )
        }
        Ok((repos, efforts)) if efforts.is_empty() => {
            format!("No active effort names this repo ({}).", repos.join(", "))
        }
        Ok((_, efforts)) => {
            let list: Vec<String> = efforts
                .iter()
                .map(|(n, t)| format!("{n}, \"{t}\""))
                .collect();
            format!(
                "This repo's active efforts are {}; {}.",
                list.join("; "),
                say("<n>").replace("load it", "load one")
            )
        }
        Err(why) => format!("Can't list efforts: {why}."),
    };
    let mut head = format!(
        "Astrolabe is on for this machine, so the guard runs in this session. No effort is loaded, so no effort's settings apply. {offer} Never load or attach to one unless the owner names it: a related guess is theirs to make. When the owner asks to start, resume or switch to an effort (\"pick up 70\", \"work on effort 70\"), run `astrolabe activate --effort <n>` alone.\n\n{RECORD_RULE}\n\n{GUARD_SCOPE}\n\n{}\nUntil it says proven, claim no protection.",
        proof_steps(&session_token(session), &session_shells(provider))
    );
    let parts = match owner_voice(&notes) {
        Ok(part) => vec![part],
        Err(why) => {
            head += &format!("\n\nOwner and voice NOT delivered: {why}. Tell the owner.");
            Vec::new()
        }
    };
    Activation {
        state: Value::Null,
        head,
        parts,
    }
    .fitted()
}

/// Why a folder no repo matches gets no suggestion, and what would match it, offered only for
/// the owner to ask for, as the notes are theirs.
pub(crate) fn no_note(cwd: &Path) -> String {
    let remotes = folder_remotes(cwd).unwrap_or_default();
    let remotes: Vec<&String> = (remotes.iter())
        .filter(|r| normalize_remote(r).is_some())
        .collect();
    if remotes.is_empty() {
        return "This repo has no remote another machine could match and isn't registered on this machine, so it has no note in the notes and no effort can be suggested here. Only if the owner asks: `astrolabe repo add <name>`, run from this folder, registers it on this machine, `<name>` being what effort notes call it in `repo:`.".to_owned();
    }
    let remotes: Vec<&str> = remotes.iter().map(|r| r.as_str()).collect();
    format!(
        "This repo has no note in the notes: no `knowledge/repos/<name>.md` lists its remote ({}) under `remote:`, and it isn't registered on this machine, so no effort can be suggested here. Only if the owner asks: `astrolabe repo add <name>`, run from this folder, writes that note with this remote, `<name>` being what effort notes call it in `repo:`; it doesn't commit the notes.",
        remotes.join(", ")
    )
}

/// An effort to suggest: its number and title.
type Offer = (String, String);

/// The repo names this folder is, by registry or by remote, and the active efforts naming one
/// of them. No names is no efforts.
fn active_efforts(
    notes: &Path,
    registry: &Repos,
    cwd: &Path,
) -> Result<(Vec<String>, Vec<Offer>), String> {
    let here = common_dir(cwd).ok_or_else(|| {
        format!(
            "this session's folder {} isn't in a git repo",
            cwd.display()
        )
    })?;
    let mut repos = repo_names(notes, registry, cwd, &here)?;
    repos.retain(|name| name != AGENT);
    if repos.is_empty() {
        return Ok((repos, Vec::new()));
    }
    let dir = notes.join("efforts");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut efforts: Vec<Offer> = Vec::new();
    let unreadable = |what: String| {
        format!("the efforts for this repo can't all be read ({what}), so none is listed")
    };
    for entry in entries {
        let entry = entry.map_err(|e| unreadable(format!("{}: {e}", dir.display())))?;
        let Ok(name) = entry.file_name().into_string() else {
            return Err(unreadable(format!(
                "a name in {} that isn't text",
                dir.display()
            )));
        };
        let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || !name.ends_with(".md") {
            continue;
        }
        let text = std::fs::read_to_string(dir.join(&name))
            .map_err(|e| unreadable(format!("efforts/{name}: {e}")))?
            .replace("\r\n", "\n");
        let props = frontmatter(&text);
        let get = |key: &str| {
            props
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        let names_here =
            get("repo").is_some_and(|r| r.split(',').any(|r| repos.contains(&r.trim().to_owned())));
        if get("status") == Some("active") && names_here {
            let title = get("title").map(str::to_owned);
            let title = title.or_else(|| {
                text.lines()
                    .find_map(|l| l.strip_prefix("# "))
                    .map(|t| t.trim().to_owned())
            });
            efforts.push((
                digits.trim_start_matches('0').to_owned(),
                title.unwrap_or_else(|| name.clone()),
            ));
        }
    }
    efforts.sort_by_key(|(n, _)| n.parse::<u32>().unwrap_or(u32::MAX));
    Ok((repos, efforts))
}

fn blocked(effort: &str, why: &str) -> String {
    format!(
        "Astrolabe stays on for effort {effort} in this session, guard included, but its context couldn't be reloaded: {why}. Shell commands are blocked until that's repaired, because work here needs the effort's current context. Repair it, then run `astrolabe activate --effort {effort}`; or drop the effort with `astrolabe activate --off` (the guard keeps running)."
    )
}

/// A resumed session keeps its switch with fresh context; a launch may switch on by
/// `ASTROLABE_EFFORT`; any other session stays silent.
fn session_start(input: &Value, session: &str) -> Option<String> {
    let cwd = input["cwd"].as_str().map(PathBuf::from);
    let cwd = cwd.or_else(|| std::env::current_dir().ok())?;
    let launched = std::env::var(EFFORT_VAR)
        .ok()
        .filter(|e| !e.trim().is_empty());
    let Some(dir) = state_dir() else {
        return launched.map(|e| refused(&e, "no home folder for its state", None));
    };
    let state = read_state(&dir, session);
    // Another machine may have saved since: the published checkpoint, not a stale copy. The
    // refresh may renumber this session's effort, so the state is read again after it.
    let refreshed = match &state {
        Ok(Some(_)) => crate::record::refresh_notes(),
        _ => None,
    };
    let state = match state {
        Ok(Some(_)) => read_state(&dir, session),
        other => other,
    };
    match state {
        Ok(Some(state)) => {
            let effort = state["effort"].as_str().unwrap_or_default().to_owned();
            let provider = state["provider"].as_str().unwrap_or("claude").to_owned();
            // No tool at session start: the provider's usual shell.
            let shells = session_shells(&provider);
            Some(
                match switch_on(
                    &dir,
                    &effort,
                    session,
                    &provider,
                    &cwd,
                    Some(&state),
                    &shells,
                ) {
                    Ok(activation) => {
                        match commit(&dir, session, &Change::Write(activation.state.clone())) {
                            Ok(()) => match refreshed {
                                Some(note) => format!("{note}\n\n{}", activation.fitted()),
                                None => activation.fitted(),
                            },
                            Err(why) => format!(
                                "Astrolabe stays on for effort {effort}, guard included, but couldn't save its reload: {why}. Tell the owner."
                            ),
                        }
                    }
                    Err(why) => {
                        // Context failed, protection mustn't: stay on, and block until repaired.
                        let mut state = state.clone();
                        state["reload_failed"] = why.clone().into();
                        let saved = write_json(&state_path(&dir, session), &state);
                        let body = json!({"effort": effort, "why": why, "at_ms": now_ms()});
                        match saved.or_else(|e| {
                            write_json(&fallback_block(&dir, session), &body)
                                .map_err(|f| format!("{e}; {f}"))
                        }) {
                            Ok(()) => blocked(&effort, &why),
                            Err(e) => format!(
                                "Astrolabe stays on for effort {effort} in this session, guard included, but its context couldn't be reloaded: {why}. It also couldn't record a block on shell commands ({e}), so only the guard is on and other commands are NOT blocked. Treat this session's effort context as missing: tell the owner, repair it and run `astrolabe activate --effort {effort}`, or drop the effort with `astrolabe activate --off` (the guard keeps running)."
                            ),
                        }
                    }
                },
            )
        }
        Ok(None) => {
            let Some(effort) = launched else {
                return Some(fresh(session, &cwd, provider(input)));
            };
            let shells = session_shells(provider(input));
            Some(
                match switch_on(&dir, &effort, session, provider(input), &cwd, None, &shells) {
                    Ok(activation) => {
                        match write_json(&state_path(&dir, session), &activation.state) {
                            Ok(()) => activation.fitted(),
                            Err(why) => refused(
                                &effort,
                                &format!("its state file couldn't be written: {why}"),
                                None,
                            ),
                        }
                    }
                    Err(why) => refused(&effort, &format!("{why} (from {EFFORT_VAR})"), None),
                },
            )
        }
        Err(why) => Some(format!(
            "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off` (the guard keeps running), then load the effort again."
        )),
    }
}
