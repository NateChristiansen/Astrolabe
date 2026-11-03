//! The opt-in plugin: `astrolabe activate` and `astrolabe hook <event> --if-active`. Idle in
//! every session until that session switches on for an effort. A session is bound by its own
//! hook, never by a model: the pre-command hook sees `astrolabe activate …` with the session id
//! the app gave it and does the work there, outside any sandbox the command would run in. It
//! then rewrites the command to `astrolabe activate --show <ticket>`, which only prints what
//! the hook left. A model can type the command but not a ticket, so it can't name another
//! session. An accident guard like the rest: an agent can still delete its own state file.
use crate::hook::{self, comparable};
use crate::registry::Registry;
use serde_json::{Value, json};
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
pub const GUARD_SCOPE: &str = "The guard is the plugin's pre-command hook. It reads each shell command this session runs (Claude's Bash and PowerShell tools, Codex's shell commands) and denies machine-wide kills, force pushes, hard resets, posting through gh, and recursive deletes outside the worktree, because those hit other people's work or can't be undone. It doesn't see file edits (Edit, Write, apply_patch), MCP tools, input typed into a shell that's already running, or anything at all if the app skipped the hook. It's an accident guard, not a sandbox.";
const NO_TICKET: &str = "Astrolabe NOT switched on: the plugin's hook didn't take this command, so it can't be bound to this session and no guard is running for it. Run the command alone, exactly as written, with nothing before or after it. If it still fails: on Codex, review and trust the plugin's hooks with /hooks; on Claude, check the plugin is enabled.";

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

fn random_hex() -> Result<String, String> {
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
}

/// `efforts/<nnnn>-*.md`, its properties on the lines under the title as `key: value`, split
/// by `·`, and its `## Current state` section. Every failure names what's missing.
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
    let text = text.replace("\r\n", "\n");
    let head = text.split("\n## ").next().unwrap_or_default();
    let title = head.lines().find_map(|line| line.strip_prefix("# "));
    let props: Vec<(String, String)> = (head.lines())
        .filter(|line| !line.starts_with('#'))
        .flat_map(|line| line.split('·'))
        .filter_map(|part| part.split_once(':'))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let get = |key: &str| {
        let found = props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str());
        found.filter(|v| !v.is_empty()).ok_or_else(|| {
            format!("{file} has no `{key}` property; the owner sets it on the line under the title, as in `repo: <name> · involvement: check in at plan · authority: edit`")
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
    })
}

fn common_dir(dir: &Path) -> Option<String> {
    let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"].map(AsRef::as_ref);
    let out = crate::worktree::git(dir, &args).ok()?;
    Some(comparable(Path::new(out.trim())))
}

/// The effort's repo this session's folder is in, as a checkout or a worktree of it.
fn session_repo(registry: &Registry, repos: &[String], cwd: &Path) -> Result<String, String> {
    let here = common_dir(cwd).ok_or_else(|| {
        format!("this session's folder {} isn't in a git repo, so it can't be matched to the effort's repo", cwd.display())
    })?;
    let mut unknown = Vec::new();
    for name in repos {
        match registry.checkout(name) {
            Ok(checkout) if common_dir(&checkout).as_ref() == Some(&here) => {
                return Ok(name.clone());
            }
            Ok(_) => {}
            Err(_) => unknown.push(format!("`astrolabe repo add {name} <checkout>`")),
        }
    }
    let mut why = format!(
        "this session's repo isn't the effort's ({}), so it's refused; switch on from a session in that repo, or the owner adds this one to the effort's `repo`",
        repos.join(", ")
    );
    if !unknown.is_empty() {
        why += &format!(". Not registered on this machine: {}", unknown.join(", "));
    }
    Err(why)
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
fn marker(token: &str) -> PathBuf {
    std::env::temp_dir().join(format!("astrolabe-probe-{token}"))
}

fn probe(token: &str) -> String {
    let marker = marker(token);
    format!("touch '{}'; pkill -0 {PROBE_NAME}", marker.display())
}

fn state_path(dir: &Path, session: &str) -> PathBuf {
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
    let body = json!({"text": text, "ok": ok, "at_ms": now_ms()});
    write_json(&shown.join(format!("{ticket}.json")), &body)?;
    Ok(ticket)
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

fn notes_and_registry() -> Result<(Registry, PathBuf), String> {
    let data = hook::data_dir().ok_or("no Astrolabe data folder can be named")?;
    let registry = Registry::open(&data.join("astrolabe.db"))
        .map_err(|e| format!("this machine's Astrolabe config: {e}"))?;
    let notes = (registry.agent_repo()).map_err(|e| format!("this machine's notes path: {e}"))?;
    let notes = notes.ok_or(
        "no notes checkout is set on this machine; run `astrolabe repo add agent <notes checkout>`",
    )?;
    Ok((registry, notes))
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
) -> Result<Activation, String> {
    let (registry, notes) = notes_and_registry()?;
    let effort = read_effort(&notes, effort)?;
    let repo = session_repo(&registry, &effort.repos, cwd)?;
    let read = |file: String| {
        let text = std::fs::read_to_string(notes.join(&file)).map_err(|e| format!("{file}: {e}"));
        Ok::<_, String>(Part {
            body: text?.trim().to_owned(),
            files: vec![notes.join(&file).display().to_string()],
            title: file,
        })
    };
    let owner_voice = Part {
        title: "owner and voice".to_owned(),
        body: crate::voice::read(&notes)?,
        files: ["owner", "voice"]
            .map(|f| {
                notes
                    .join(format!("knowledge/{f}.md"))
                    .display()
                    .to_string()
            })
            .to_vec(),
    };
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
    let state = json!({
        "session": session, "provider": provider, "effort": effort.number, "file": effort.file,
        "repo": repo, "cwd": comparable(cwd), "at_ms": at_ms, "token": token,
    });
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
        Prove the guard for this session: run exactly this, alone: {probe}\n\
        The hook must deny it. Then run `astrolabe activate --status`, and report the effort and what it says. Until it says proven, claim no protection, and claim none for commands that ran before switch-on. To switch off: `astrolabe activate --off`.",
        n = effort.number,
        title = effort.title,
        file = effort.file,
        inv = effort.involvement,
        inv_m = meaning(&INVOLVEMENT, &effort.involvement),
        auth = effort.authority,
        auth_m = meaning(&AUTHORITY, &effort.authority),
        probe = probe(&token),
    );
    Ok(Activation { state, head, parts })
}

/// Whether the hook denied this activation's probe, by this build, and the probe didn't run.
pub fn status(dir: &Path, session: &str) -> String {
    let state = match read_state(dir, session) {
        Ok(None) => return "Astrolabe is off in this session.".to_owned(),
        Err(why) => {
            return format!(
                "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off`, then switch on again."
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
    if let Some(why) = state["reload_failed"].as_str() {
        return format!(
            "{on} {}",
            blocked(state["effort"].as_str().unwrap_or_default(), why)
        );
    }
    if marker(token).exists() {
        return format!(
            "{on} Guard FAILED: the probe ran (its marker file exists), so no hook stopped it. Claim no protection and tell the owner."
        );
    }
    let probe = probe(token);
    let log = hook::data_dir().map(|data| data.join(hook::DENIAL_LOG));
    let log = log
        .and_then(|log| std::fs::read_to_string(log).ok())
        .unwrap_or_default();
    let since = state["at_ms"].as_i64().unwrap_or(i64::MAX);
    let program = hook_id();
    let denied = (log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok()))
    .any(|l| {
        l["session_id"] == session
            && l["command"] == probe.as_str()
            && l["hook"] == program.as_str()
            && l["at_ms"].as_i64().is_some_and(|at| at >= since)
    });
    if denied {
        format!(
            "{on} Guard proven for this session and effort: the hook denied the probe, and its marker file wasn't made. {GUARD_SCOPE}"
        )
    } else {
        format!(
            "{on} Guard not proven yet: no hook denial of this session's probe since switch-on, by this build of the program and this version of the plugin's hooks. Run exactly this, alone: {probe}\nthen `astrolabe activate --status` again. Until then claim no protection."
        )
    }
}

/// `astrolabe activate --show <ticket>`, as the hook rewrote it: what to print, and whether
/// it worked. Anything else reaching here is a command no hook took.
pub fn command(args: &[&str]) -> (String, bool) {
    match (args, state_dir()) {
        (["--show", ticket], Some(dir)) => show(&dir, ticket).unwrap_or_else(|why| {
            (
                format!("Astrolabe NOT switched on: {why}. {NO_TICKET}"),
                false,
            )
        }),
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
) -> (String, bool, Change, String) {
    let previous = read_state(dir, session);
    match args {
        ["--status"] => (status(dir, session), true, Change::Keep, String::new()),
        ["--off"] => match previous {
            Ok(None) => ("Astrolabe was already off in this session.".to_owned(), true, Change::Keep, String::new()),
            _ => (
                "Astrolabe is off in this session; its guard no longer runs here.".to_owned(),
                true,
                Change::Remove,
                "Astrolabe could NOT switch off, so it stays on here, guard included".to_owned(),
            ),
        },
        ["--effort", effort] => {
            let previous = previous.ok().flatten();
            match switch_on(dir, effort, session, provider, cwd, previous.as_ref()) {
                Ok(activation) => {
                    let failed = refused(effort, "its state file couldn't be written", previous.as_ref());
                    (activation.text(), true, Change::Write(activation.state), failed)
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
            "Effort {} stays on as it was.",
            p["effort"].as_str().unwrap_or_default()
        ),
        None => "This session stays off.".to_owned(),
    };
    format!(
        "Astrolabe NOT switched on for effort {effort}: {why}. {kept} No new protection is claimed."
    )
}

/// `astrolabe hook <event> --if-active`. Silent in a session that's off, except for its own
/// switch. Always exits 0, like the hooks it reuses.
pub fn gate(event: &str) {
    std::panic::set_hook(Box::new(|_| {}));
    let bytes = hook::read_bytes();
    let input: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let session = input["session_id"].as_str().filter(|s| !s.is_empty());
    // Child agents start off: they share their parent's session id but not its switch.
    let child = input["agent_id"].as_str().is_some_and(|a| !a.is_empty());
    match event {
        "pre-command" => {
            let answer = std::panic::catch_unwind(|| switch_command(&input, session, child));
            if let Ok(Some(answer)) = answer {
                return hook::say(&answer);
            }
            let (Some(session), false, Some(dir)) = (session, child, state_dir()) else {
                return;
            };
            match read_state(&dir, session) {
                Ok(None) => {}
                Ok(Some(state)) => match state["reload_failed"].as_str() {
                    Some(why) => hook::say(&hook::deny(&blocked(
                        state["effort"].as_str().unwrap_or_default(),
                        why,
                    ))),
                    None => hook::guard(&bytes),
                },
                Err(why) => hook::say(&hook::deny(&format!(
                    "Astrolabe is on for this session but can't read its state ({why}), so it can't judge this command. Repair: run `astrolabe activate --off`, then switch on again."
                ))),
            }
        }
        "session-start" if !child => {
            let text = std::panic::catch_unwind(|| session_start(&input, session?));
            if let Ok(Some(text)) = text {
                let inner = json!({"hookEventName": "SessionStart", "additionalContext": text});
                hook::say(&json!({ "hookSpecificOutput": inner }));
            }
        }
        _ => {}
    }
}

fn provider(input: &Value) -> &'static str {
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
            "A subagent can't switch Astrolabe on or off: it starts off until its own launch binds an effort, so the parent session's switch stays the parent's.",
        ));
    }
    let session = session?;
    let cwd = input["cwd"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into());
    let left = state_dir()
        .ok_or_else(|| "no home folder".to_owned())
        .and_then(|dir| {
            let (text, ok, change, failed) = act(&dir, &words[2..], session, provider(input), &cwd);
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
        Ok(ticket) => {
            // This program by its full path: a login shell's PATH can find another copy.
            let exe = std::env::current_exe().map(|exe| exe.display().to_string());
            let exe = exe.ok().filter(|exe| !exe.contains('\''));
            let exe = exe.map_or("astrolabe".to_owned(), |exe| format!("'{exe}'"));
            let call = if input["tool_name"] == "PowerShell" {
                "& "
            } else {
                ""
            };
            let mut updated = tool_input.clone();
            updated[key] = format!("{call}{exe} activate --show {ticket}").into();
            json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": updated,
            }})
        }
        Err(why) => hook::deny(&format!(
            "Astrolabe couldn't finish this ({why}). Run `astrolabe activate --status` to see where this session stands; claim no new protection."
        )),
    })
}

fn blocked(effort: &str, why: &str) -> String {
    format!(
        "Astrolabe stays on for effort {effort} in this session, guard included, but its context couldn't be reloaded: {why}. Shell commands are blocked until that's repaired, because work here needs the effort's current context. Repair it, then run `astrolabe activate --effort {effort}`; or switch off with `astrolabe activate --off`."
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
    match read_state(&dir, session) {
        Ok(Some(state)) => {
            let effort = state["effort"].as_str().unwrap_or_default().to_owned();
            let provider = state["provider"].as_str().unwrap_or("claude").to_owned();
            Some(
                match switch_on(&dir, &effort, session, &provider, &cwd, Some(&state)) {
                    Ok(activation) => {
                        match write_json(&state_path(&dir, session), &activation.state) {
                            Ok(()) => activation.fitted(),
                            Err(why) => format!(
                                "Astrolabe stays on for effort {effort}, guard included, but couldn't save its reload: {why}. Tell the owner."
                            ),
                        }
                    }
                    Err(why) => {
                        // Context failed, protection mustn't: stay on, and block until repaired.
                        let mut state = state.clone();
                        state["reload_failed"] = why.clone().into();
                        let _ = write_json(&state_path(&dir, session), &state);
                        blocked(&effort, &why)
                    }
                },
            )
        }
        Ok(None) => {
            let effort = launched?;
            Some(
                match switch_on(&dir, &effort, session, provider(input), &cwd, None) {
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
            "Astrolabe state for this session can't be read ({why}). Shell commands are blocked until it's repaired: run `astrolabe activate --off`, then switch on again."
        )),
    }
}
