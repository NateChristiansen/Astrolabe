//! Effort loading and the text delivered to a session.
use super::proof::{proof_steps, session_token};
use super::repos::{
    Repos, common_dir, folder_remotes, in_notes, normalize_remote, notes_and_repos, repo_names,
    session_repo,
};
use super::{now_ms, state_dir};
use crate::effort::frontmatter;
use crate::hook::comparable;
use crate::registry::AGENT;
use crate::session::{Field, SessionState};
use serde_json::Value;
use std::path::Path;

/// A hook's text is cut near 10,000 characters on both clients (see `voice`).
pub(super) const HOOK_LIMIT: usize = 9800;

pub(super) const INVOLVEMENT: [(&str, &str); 3] = [
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

pub(super) const AUTHORITY: [(&str, &str); 3] = [
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
/// What every session needs: when work gets an effort, and how to start one.
pub const RECORD_RULE: &str = "Work in this session gets an effort in the owner's notes, because those notes are the record of what's in flight and no app's memory carries across machines or providers. An effort is something the owner asked for: a change, a plan, or findings that take more than this session. When their request means one, first run one command alone, with the goal in its body (in PowerShell, the body as a here-string: `@'` … `'@` as the last argument):\n  astrolabe effort new --title \"<short title>\" <<'ASTROLABE'\n  <the goal, one line>\n  ASTROLABE\nIts output says how to save, journal and record decisions. Plan mode doesn't block this command, `astrolabe activate` or `astrolabe journal`, and is no reason to leave it: the hook writes the owner's notes, never the repo, and the app runs only a read of the result. So in plan mode, create the effort before writing the plan. Add --involvement, --authority or --initiative only when the owner's words say so. From the notes checkout, the owner's seat, `effort new` and `astrolabe journal` need --repo <name>, the repo the work is for, as its note in knowledge/repos names it. A question answered now needs no effort, even with some digging; if the finding is worth keeping, run `astrolabe journal \"<one sentence>\"` alone, for work no effort covers: past tense, about 25 words, as the owner would say it. If another agent dispatched you under its effort, run `astrolabe activate --effort <n> --worker` first and write nothing to the notes: the dispatcher reports for you.";

/// What a session with an effort needs: how to save, journal and record decisions. Printed by
/// `effort new` and `activate`, so a session that never records doesn't carry it.
pub const SAVE_RULE: &str = "Save before waiting on the owner, before a long run, and when a piece is done, because a closed window fires no hook: replace Current state with\n  astrolabe effort save [--status <state>] [--decision \"…\"] [--waits-for \"…\"] [--journal \"…\"] <<'ASTROLABE'\n  <where it stands>\n\n  <what was decided and why>\n\n  <what's next; what isn't checked>\n  ASTROLABE\n(PowerShell: the same with the body as a here-string, @' … '@, last or piped in.) Current state is three short paragraphs in the owner's voice, under 150 words, no **Label:** lines, no ids, at most one path and only for the thing the owner has to open. The first sentence says the state in plain words; the second paragraph gives each choice with its reason; the last says what's next and what isn't checked. Numbers only when they're the result (\"200 ms instead of 310\"), never the evidence trail (no p50s, run counts or commit lists). Like this, at done:\n  Done and merged. A Deck guest now sees each drawn piece about 200 ms late instead of 310, because merging no longer reads meshes back from the GPU.\n\n  I dropped the stand-in: it buys about a frame. Parallel shells on the guest made pieces later. Sending shells ahead would save under 11 ms.\n\n  Not playtested; it rides along in the next one. The last 200 ms is on the todo. One Deck run each way so far.\nUse `astrolabe lesson close` for a lesson's disposition. A choice that leaves Current state goes to Decisions first: --decision \"chose X over Y: reason\", written as a record, never to the owner. Any other shape is refused with the accepted forms. Plan mode doesn't block these commands and is no reason to leave it: the hook writes the owner's notes, never the repo, and the app runs only a read of the result. So in plan mode create the effort first, and when the owner approves the plan, save it on the effort: --plan-file <path> with the plan file the app named, or, where the app keeps no plan file (Codex), the plan in the body after a line `## Plan`, below the Current state paragraphs. Work with more than one piece keeps them as a `## Pieces` checklist in the effort, not as efforts of their own: `--pieces-file <path>` sets the list, `--tick \"<piece>\"` marks one landed. The journal is the owner's history, read days later beside the effort's note: one line per effort per day, as they would say it, past tense, plain words, about 25 words, two sentences at most, like \"Deck guests see drawn pieces 200 ms late now, down from 310, by skipping the GPU readback. Dropped the stand-in and send-ahead.\" It says what changed, and why when the why matters, and nothing the note doesn't say, since the line outlives the session. Never where it stands (no \"Merged.\", no leftover, no next step): the board shows status. It names things, never files, ids or other efforts, in the game's own terms (\"tool presses\", not \"tool presses on Hot Wheels\"), and says \"my\", never \"the owner's\". Pass --journal on the save that finishes something worth telling, or before you stop with it unfinished (\"Started …\"); a later line the same day replaces the earlier, so small things batch, and a decision gets a line only when it's a direction they'd want to remember. States: proposed, ready, active, blocked (needs --waits-for; \"owner: <question>\" when it waits on them), done, abandoned (needs --reason). Under autonomous, mark done on your own checks; under check in at plan or each step, save with --waits-for \"owner: look at …\" and mark done after their look. A Stop-hook message about saving is the plugin, not the owner: act on it or say nothing changed, never treat it as a request. The owner's home is served on this machine at the address `astrolabe home` prints (localhost:4747 unless set), started by the plugin at any change and stopped when idle: the board, the notes tree beside it, each note as a page, following every save. `astrolabe home --open` opens it; `--theme dark` and `--palette iris` set its look per machine. The same page is written as home.html at the notes checkout root for a machine with no server.";

/// Said to a worker on binding: what it may not do, and why.
pub const WORKER_RULE: &str = "This session is a worker under that effort, dispatched by another agent: it creates, saves and journals nothing in the owner's notes, because one session writes an effort's record and the dispatcher reports for its workers. Report your handoff where the dispatcher said, or in chat. The guard and the effort's settings apply as above.";

pub const GUARD_SCOPE: &str = "The guard is the plugin's pre-command hook. It reads each shell command this session runs (Claude's Bash and PowerShell tools, Codex's shell commands) and denies machine-wide kills, force pushes, hard resets, posting through gh, and recursive deletes outside the worktree, because those hit other people's work or can't be undone; an owner-approved force push or GitHub write can be let through once with `astrolabe allow`. It doesn't see file edits (Edit, Write, apply_patch), MCP tools, input typed into a shell that's already running, or anything at all if the app skipped the hook. Subagents of this session get the same guard but none of this context or authority, and their denials don't prove the guard for this session. It's an accident guard, not a sandbox.";

/// What a refusal leaves running: it only ever comes from a hook, so the machine is on.
pub(crate) const STILL_ON: &str = "Astrolabe is still on for this machine: the guard still runs in this session, and the owner and voice notes from session start still apply. Only this effort's context and settings are missing.";

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
    if !dir.is_dir() {
        return Err(format!("{}: no such folder", dir.display()));
    }
    let names: Vec<String> = crate::notes::effort_files(notes)
        .into_iter()
        .filter(|(m, _)| *m == n)
        .map(|(_, rel)| rel)
        .collect();
    let [file] = names.as_slice() else {
        return Err(match names.len() {
            0 => format!(
                "no effort {n}: no {prefix}*.md under efforts/ in {}",
                notes.display()
            ),
            _ => format!(
                "more than one {prefix}*.md under efforts/: {}",
                names.join(", ")
            ),
        });
    };
    let file = file.clone();
    let text = std::fs::read_to_string(notes.join(&file)).map_err(|e| format!("{file}: {e}"))?;
    let hash = crate::notes::content_hash(&text);
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
    // The same reading a save and the board make, so a fenced example isn't the section.
    let current = crate::effort::EffortFile::parse(&text)
        .map(|f| f.current)
        .ok();
    Ok(Effort {
        number: n.to_string(),
        title: title
            .unwrap_or_else(|| file.rsplit('/').next().unwrap_or(&file))
            .trim()
            .to_owned(),
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

/// One piece of the context, and what to read instead when it can't go inline.
pub(super) struct Part {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) files: Vec<String>,
}

pub struct Activation {
    pub state: SessionState,
    pub(super) head: String,
    pub(super) parts: Vec<Part>,
}

/// One of the owner's notes, whole. Owner goes to every session at its start; whole voice to one
/// that loads or creates an effort, since saves and journal lines are written in it.
pub(super) fn owner_note(notes: &Path, name: &str) -> Result<Part, String> {
    let file = format!("knowledge/{name}.md");
    let body = std::fs::read_to_string(notes.join(&file))
        .map_err(|e| format!("{file} in {}: {e}", notes.display()))?;
    let lead = match name {
        "voice" => "Samples in it teach style only and are never a source of facts.\n\n",
        _ => "",
    };
    Ok(Part {
        body: format!("{lead}{}", body.trim()),
        files: vec![notes.join(&file).display().to_string()],
        title: file,
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

    /// For an effort loaded or created mid-session: the session already has the owner note and
    /// the guard's scope from its start, so they aren't sent again.
    pub fn mid_session(mut self) -> Self {
        self.parts.retain(|p| p.title != "knowledge/owner.md");
        self.head = self.head.replace(&format!("{GUARD_SCOPE}\n\n"), "");
        self
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

/// Everything checked, nothing written: the caller commits `state` once what it says is safe.
/// `previous` keeps a resumed session's switch-on time and probe.
pub fn switch_on(
    _dir: &Path,
    effort: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    previous: Option<&SessionState>,
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
        owner_note(&notes, "voice")?,
        owner_note(&notes, "owner")?,
        read(format!("knowledge/repos/{repo}.md"))?,
        read("knowledge/machines.md".to_owned())?,
    ];
    let same = previous.filter(|p| p.effort.as_deref() == Some(effort.number.as_str()));
    // One probe per session: the guard is the same hook in the same session whichever effort
    // is loaded, so the proof from session start carries into an effort created or picked up
    // later. A new token per effort made the proof stale and asked for a probe named in tool
    // output, which Claude declined (PC, 2026-11-03).
    let token = match same.and_then(|p| p.token.as_deref()) {
        Some(token) => token.to_owned(),
        None => session_token(session),
    };
    let at_ms = same
        .and_then(|p| p.at_ms.as_ref().copied())
        .unwrap_or_else(now_ms);
    // The effort as delivered, so a save can tell whether it would overwrite another's, and
    // the repo as it stands, so the Stop hook can tell whether work moved.
    let delivered = Some(effort.hash.clone());
    // A resumed session keeps its baseline, so changes made before it closed still count.
    let baseline = match same.and_then(|p| p.baseline.as_deref()) {
        Some(kept) => Some(kept.to_owned()),
        None => crate::notes::RepoFacts::read(cwd).map(|f| f.fingerprint),
    };
    let mut state = SessionState {
        session: session.to_owned().into(),
        provider: provider.to_owned().into(),
        effort: effort.number.clone().into(),
        file: effort.file.clone().into(),
        repo: repo.clone().into(),
        cwd: comparable(cwd).into(),
        at_ms: at_ms.into(),
        token: token.clone().into(),
        delivered: delivered
            .map(Field::Known)
            .unwrap_or(Field::Other(Value::Null)),
        baseline: baseline
            .map(Field::Known)
            .unwrap_or(Field::Other(Value::Null)),
        mark: false.into(),
        // Unknown fields survive a resume, so a newer writer can share the record.
        extra: same.map(|p| p.extra.clone()).unwrap_or_default(),
        ..Default::default()
    };
    if let Some(previous) = same {
        if !previous.mark.is_missing() {
            state.mark = previous.mark.clone();
        }
        state.acknowledged = previous.acknowledged.clone();
        state.last_save_ms = previous.last_save_ms.clone();
        state.worker = previous.worker.clone();
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
        - authority: {auth}: {auth_m}. It's a ceiling inside {repo} and this effort's scope, and never covers posting or destructive actions, because those reach other people or can't be undone. Authority covers project changes and pushes only: notes save and sync on their own at every level, so never ask for approval to save notes and never skip a save because project pushes need approval; a worker leaves that saving to its dispatcher.\n\
        - A narrower current instruction from the owner wins, because it's the newest word.\n\
        - In this build these settings are context, not enforced, apart from the guard. Switching on wrote only Astrolabe's own state file; that isn't leave to edit project files.\n\n\
        {GUARD_SCOPE}\n\n\
        {proof}\n\
        Report the effort and what the status says. Until it says proven, claim no protection, and claim none for commands that ran before switch-on. To drop the effort from this session: `astrolabe activate --off`; the guard stays while Astrolabe is on for the machine.\n\n{freshness}\n\n{SAVE_RULE}",
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
    // A worker stays a worker through resume and reload, and is told so each time.
    let mut head = head;
    if state.worker.is_true() {
        head += &format!("\n\n{WORKER_RULE}");
    }
    Ok(Activation { state, head, parts })
}

/// What the stamp says against the repo here: who saved last and when, and what moved since.
/// Reported at the last published checkpoint; work after it on another machine isn't visible.
pub(super) fn freshness(notes: &Path, effort: &Effort, cwd: &Path) -> String {
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

pub(super) fn refused(effort: &str, why: &str, previous: Option<&SessionState>) -> String {
    let kept = match previous {
        Some(p) => format!(
            "Effort {} stays loaded as it was.",
            p.effort.as_deref().unwrap_or_default()
        ),
        None => "No effort is loaded in this session.".to_owned(),
    };
    format!("Effort {effort} NOT loaded: {why}. {kept} {STILL_ON}")
}

/// A fresh session while the machine is on: the effort if its repo has exactly one active,
/// else the owner and voice notes and the guard, and why there's no effort.
pub(super) fn fresh(cwd: &Path) -> String {
    let (registry, notes) = match notes_and_repos() {
        Ok(found) => found,
        Err(why) => {
            return format!(
                "Astrolabe is on for this machine and the guard runs here, but owner and voice weren't delivered: {why}. Without the notes checkout on record nothing can be saved either. Tell the owner.\n\n{GUARD_SCOPE}"
            );
        }
    };
    // Never loaded on its own: the owner may be here for something else (owner, 2026-11-03).
    let say = |n: &str| format!("say \"pick up {n}\" to load it");
    let seat = in_notes(cwd);
    let offer = match active_efforts(&notes, &registry, cwd) {
        Ok((repos, _)) if repos.is_empty() => no_note(cwd),
        Ok((_, efforts)) if seat && efforts.is_empty() => no_note(cwd),
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
    let offer = match seat && !offer.starts_with("This is the notes checkout") {
        true => format!("{} {offer}", no_note(cwd)),
        false => offer,
    };
    let mut head = format!(
        "Astrolabe is on for this machine, so the guard runs in this session. No effort is loaded, so no effort's settings apply. {offer} Never load or attach to one unless the owner names it: a related guess is theirs to make. When the owner asks to start, resume or switch to an effort (\"pick up 70\", \"work on effort 70\"), run `astrolabe activate --effort <n>` alone.\n\n{RECORD_RULE}\n\n{GUARD_SCOPE} Loading or creating an effort proves it for this session; until then, claim no protection."
    );
    let mut parts = match owner_note(&notes, "owner") {
        Ok(part) => vec![part],
        Err(why) => {
            head += &format!("\n\nOwner NOT delivered: {why}. Tell the owner.");
            Vec::new()
        }
    };
    // Every session talks to the owner, so voice's rules come along; the samples wait for a
    // session that records.
    if let Ok(part) = owner_note(&notes, "voice") {
        let rules = part.body.split("\n## ").find(|s| s.starts_with("Rules\n"));
        if let Some(rules) = rules {
            parts.push(Part {
                body: format!(
                    "How the owner writes, for anything they read:\n{}",
                    rules.trim_start_matches("Rules\n").trim_end()
                ),
                files: part.files,
                title: "knowledge/voice.md, its rules".into(),
            });
        }
    }
    Activation {
        state: SessionState::default(),
        head,
        parts,
    }
    .fitted()
}

/// Why a folder no repo matches gets no suggestion, and what would match it, offered only for
/// the owner to ask for, as the notes are theirs.
pub(crate) fn no_note(cwd: &Path) -> String {
    if in_notes(cwd) {
        return "This is the notes checkout, the owner's seat: any effort loads here with \"pick up N\" whatever its repo, and a new one names the repo its work is for with --repo <name>, as its note in knowledge/repos names it.".to_owned();
    }
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
pub(super) fn active_efforts(
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
    // The notes checkout's own efforts (`repo: agent`, as the seeded "Fill in your notes" is)
    // are offered there, at the owner's seat, and nowhere else.
    if !in_notes(cwd) {
        repos.retain(|name| name != AGENT);
    }
    if repos.is_empty() {
        return Ok((repos, Vec::new()));
    }
    let dir = notes.join("efforts");
    if !dir.is_dir() {
        return Err(format!("{}: no such folder", dir.display()));
    }
    let mut efforts: Vec<Offer> = Vec::new();
    let unreadable = |what: String| {
        format!("the efforts for this repo can't all be read ({what}), so none is listed")
    };
    for (_, rel) in crate::notes::effort_files(notes) {
        let name = rel.rsplit('/').next().unwrap_or(&rel).to_owned();
        let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
        let text = std::fs::read_to_string(notes.join(&rel))
            .map_err(|e| unreadable(format!("{rel}: {e}")))?
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

pub(super) fn blocked(effort: &str, why: &str) -> String {
    format!(
        "Astrolabe stays on for effort {effort} in this session, guard included, but its context couldn't be reloaded: {why}. Shell commands are blocked until that's repaired, because work here needs the effort's current context. Repair it, then run `astrolabe activate --effort {effort}`; or drop the effort with `astrolabe activate --off` (the guard keeps running)."
    )
}
