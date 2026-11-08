//! Effort loading and the text delivered to a session.
use super::proof::{proof_steps, session_token};
use super::repos::{
    Repos, common_dir, folder_remotes, in_notes, normalize_remote, notes_and_repos, repo_names,
    session_repo,
};
use super::{now_ms, state_dir};
use crate::effort::frontmatter;
use crate::effort::{AUTHORITIES, INVOLVEMENTS};
use crate::hook::comparable;
use crate::registry::AGENT;
use crate::session::{Binding, Field, SessionState};
use serde_json::Value;
use std::path::Path;

/// A hook's text is cut near 10,000 characters on both clients (see `voice`).
pub(super) const HOOK_LIMIT: usize = 9800;

/// What each of `INVOLVEMENTS` asks of an agent, in that list's order.
const INVOLVEMENT_MEANINGS: [&str; 3] = [
    "run to the end within authority, then report, because the owner chose to stay out of the loop for this effort",
    "show the plan and wait for the owner's go before changing files, because a plan is cheap to redirect and changed files aren't",
    "report after each step and wait before the next, because the owner wants to steer each one",
];

/// What each of `AUTHORITIES` lets an agent do, in that list's order.
const AUTHORITY_MEANINGS: [&str; 3] = [
    "read and report; change no project files, because the owner hasn't handed this effort's changes to an agent",
    "change files and commit locally on a branch, but don't push, because sharing the work is the owner's call here",
    "change, commit and push the effort's branch where the repo's own rules allow, because the owner allowed sharing for this effort",
];

/// Why and when work gets recorded, told at session start. Every rule says its reason, so a
/// newer model can judge the edge cases and a dead rule is easy to spot.
/// What every session needs: when work gets an effort, and how to start one.
pub const RECORD_RULE: &str = "Work in this session gets an effort in the owner's notes, because those notes are the record of what's in flight and no app's memory carries across machines or providers. An effort is something the owner asked for: a change, a plan, or findings that take more than this session. When their request means one, first run one command alone, with the goal in its body (in PowerShell, the body as a here-string: `@'` … `'@` as the last argument):\n  astrolabe effort new --title \"<short title>\" <<'ASTROLABE'\n  <the goal, one line>\n  ASTROLABE\nIts output says how to save, journal and record decisions. A plan spanning efforts becomes an initiative, filed with `astrolabe initiative new`. Plan mode doesn't block this command, `astrolabe activate` or `astrolabe journal`, and is no reason to leave it: the hook writes the owner's notes, never the repo, and the app runs only a read of the result. So in plan mode, create the effort before writing the plan. Add --involvement, --authority or --initiative only when the owner's words say so. Add --issue <n> when the work answers a GitHub issue, so the commit that lands it can close it. From the notes checkout, the owner's seat, `effort new` and `astrolabe journal` need --repo <name>, the repo the work is for, as its note in knowledge/repos names it. A question answered now needs no effort, even with some digging; if the finding is worth keeping, run `astrolabe journal \"<one sentence>\"` alone, for work no effort covers: past tense, about 25 words, as the owner would say it. If another agent dispatched you to an effort, run `astrolabe activate --effort <n> --worker` first, with that effort's number: you save its state and decisions, and the dispatcher files, journals and closes.";

/// What a session with an effort needs: how to save, journal and record decisions. Printed by
/// `effort new` and `activate`, so a session that never records doesn't carry it.
pub const SAVE_RULE: &str = "Save before waiting on the owner, before a long run, and when a piece is done, because a closed window fires no hook: replace Current state with\n  astrolabe effort save [--status <state>] [--decision \"…\"] [--waits-for \"…\"] [--journal \"…\"] <<'ASTROLABE'\n  <where it stands>\n\n  <what was decided and why>\n\n  <what's next; what isn't checked>\n  ASTROLABE\n(PowerShell: the body as a here-string, @' … '@, last or piped in.) Current state is three short paragraphs in the owner's voice, under 150 words, no **Label:** lines, no ids, at most one path for something the owner must open. The first sentence says the state; the second paragraph gives choices and reasons; the last says what's next and unchecked. Numbers are results (\"200 ms instead of 310\"), never evidence trails (p50s, run counts or commit lists). Like this, at done:\n  Done and merged. A Deck guest now sees each drawn piece about 200 ms late instead of 310, because merging no longer reads meshes back from the GPU.\n\n  I dropped the stand-in: it buys about a frame. Parallel shells on the guest made pieces later. Sending shells ahead would save under 11 ms.\n\n  Not playtested; it rides along in the next one. The last 200 ms is on the todo. One Deck run each way so far.\nUse `astrolabe lesson close` for a lesson's disposition. A choice leaving Current state goes to Decisions first: --decision \"chose X over Y: reason\", written as a record, never to the owner. Other shapes are refused with accepted forms. Plan mode doesn't block these commands, which write only notes. Save the approved plan with --plan-file <path>, or where the app keeps no plan file, in the body after `## Plan`.\nPieces come with the approved plan when it has more than one landable step: a `## Pieces` checklist, one independently checkable outcome per line. A one-step fix or spike needs none. --pieces-file <path> sets the list; --tick \"<piece>\" marks its outcome met, which can be on a branch; --untick reverses it. If landing is a deliverable, make it a piece. At done, leave unticked lines with a dated explanation of where deferred or why dropped.\nA plan that names efforts ends in an initiative: `astrolabe initiative new --title \"<name>\"` with the page in its body files the record. The accepted forms show a page alone, new efforts, or existing ones joined by number. Membership is an effort's `initiative:` naming the page and means it delivers part of the initiative. A planning-only spike closes without --initiative; the page links it as provenance. Progress comes from member statuses, never a checklist of efforts; an initiative has no Pieces.\nWhen the owner picks an orchestrator and workers, the orchestrator binds with `astrolabe activate --initiative <slug>` and keeps its state, cross-effort decisions, owner questions and the day's one journal line on the page, never in an orchestrate effort. Workers save their own state, decisions and pieces, and journal nothing. The orchestrator reviews and lands, then runs `astrolabe effort read <n>` before `astrolabe effort save --effort <n> --status done` with a Current state body and any ticks. A stale save is refused with the current text; a shared initiative reaches across repos. Briefs and reports live in the notes under `.agents/<initiative slug>/` so another session can resume.\nStates: proposed, ready, active, review, blocked, done, abandoned. Review means built and checked, waiting for review, acceptance or landing; blocked means an impediment and needs --waits-for. Use \"owner: <question>\" for Needs you. Abandoned needs --reason. A status change to active, review, blocked, done or abandoned requires Current state, including by number; its first sentence says the state. Initiatives use the same states except review; done requires an acceptance sentence and no open members. Omitting --waits-for clears the wait unless it stays blocked or in review. An owner reply can clear an active or blocked wait, never a review's; a reply isn't approval. Under autonomous, mark done on your checks; otherwise save in review with --waits-for \"owner: look at …\" and mark done after their look.\nThe journal is the owner's history: one line per initiative per day on its page, or per standalone effort on itself; workers write none. Write past tense, plain words, about 25 words, at most two sentences: what changed and why, nothing the note doesn't say. Never status (\"Merged.\"), leftovers or next steps. Name things, never files, ids or other efforts; use the project's terms and \"my\", never \"the owner's\". Pass --journal when finishing something worth telling or before stopping unfinished (\"Started …\"); a later line that day replaces the earlier. Journal decisions only for directions worth remembering.\nA Stop-hook save message is the plugin, not the owner: act on it or say nothing changed. The owner's board is at the address `astrolabe home` prints.";

/// What the app should call a session on an effort: its number without the zeros, then its
/// title without the number the note's heading leads with, so the owner sees who's on what.
pub fn session_name(number: &str, title: &str) -> String {
    let title = title.trim();
    let digits = title
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(title.len());
    let (lead, rest) = title.split_at(digits);
    // A number of its own, as the heading's "0083 " is, never the start of a word ("7zip").
    let apart = rest.is_empty() || rest.starts_with(char::is_whitespace);
    let same = !lead.is_empty() && apart && lead.parse::<u64>().ok() == number.parse::<u64>().ok();
    let title = if same { rest.trim_start() } else { title };
    match title.is_empty() {
        true => number.to_owned(),
        false => format!("{number} {title}"),
    }
}

/// Said to a session that picks up an effort mid-session, so its name shows at once. Codex
/// hooks can't name a thread, but the Codex app gives its agents a tool that can; a thread
/// started outside the app has no such tool and isn't listed there, so it skips this. A Claude
/// hook names a session only when a prompt is sent, so the Claude desktop app's own tool names
/// it sooner, and the hook still names it where there's no such tool.
fn rename(provider: &str, name: &str) -> Option<String> {
    let (what, tool, otherwise) = match provider {
        "codex" => ("thread", "`set_thread_title` in the Codex app", ""),
        "claude" => (
            "session",
            "`set_session_title` in the Claude desktop app",
            ": the plugin names it at the owner's next message",
        ),
        _ => return None,
    };
    Some(format!(
        "Rename this {what} to \"{name}\" now with the app's {what}-rename tool ({tool}), so the owner can see which effort it's on. If you have no such tool, skip this and don't mention it{otherwise}."
    ))
}

/// Said to a worker on binding: what it may not do, and why.
pub const WORKER_RULE: &str = "This session is a worker under that effort, dispatched by another agent: it saves that effort's state, decisions and pieces as above, and nothing else in the owner's notes. No journal line, no new effort, no other effort's record and no lesson close, because the dispatcher reviews, lands and marks it done, writes the one journal line a day for its initiative, and triages lessons. Report your handoff where the dispatcher said, or in chat. The guard and the effort's settings apply as above.";

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
    /// The GitHub issues it answers, as numbers; empty for most.
    pub issues: Vec<String>,
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
    let one_of = |key: &str, allowed: &[&str]| {
        let value = get(key)?;
        allowed
            .contains(&value)
            .then(|| value.to_owned())
            .ok_or_else(|| {
                format!(
                    "{file}: {key} {value:?} isn't one of {}",
                    allowed.join(", ")
                )
            })
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
        involvement: one_of("involvement", &INVOLVEMENTS)?,
        authority: one_of("authority", &AUTHORITIES)?,
        issues: (props.iter())
            .find(|(k, _)| k == "issue")
            .map(|(_, v)| {
                (v.split(','))
                    .map(|n| n.trim().trim_start_matches('#').to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
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
    /// The effort's name, which the session takes only when it takes up the effort's work.
    pub(super) picked: String,
}

/// One of the owner's notes, whole. Owner goes to every session at its start; whole voice to one
/// that loads or creates an effort, since saves and journal lines are written in it.
pub(super) fn owner_note(notes: &Path, name: &str) -> Result<Part, String> {
    let file = format!("knowledge/{name}.md");
    // A checkout with Windows line endings would otherwise hide voice's `## Rules` heading
    // from the split below, and every session on that machine would go without them.
    let body = std::fs::read_to_string(notes.join(&file))
        .map_err(|e| format!("{file} in {}: {e}", notes.display()))?
        .replace("\r\n", "\n");
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

    /// Taken up by name, as `activate --effort` does: the session takes the effort's name.
    pub fn picked_up(mut self) -> Self {
        self.state.name.set(self.picked.clone());
        self.state.taken.set(true);
        self
    }

    /// For an effort loaded or created mid-session: the session already has the owner note and
    /// the guard's scope from its start, so they aren't sent again. The repo's page is sent
    /// again: nothing records which pages a start sent, and the session may have started in the
    /// notes checkout or another repo, so a repeat is the safe side of not knowing.
    pub fn mid_session(mut self) -> Self {
        self.parts.retain(|p| p.title != "knowledge/owner.md");
        self.head = self.head.replace(&format!("{GUARD_SCOPE}\n\n"), "");
        // Only here: a resumed session was named when it picked the effort up, and a launched
        // worker runs outside the app or is named by Claude's start hook. Once per name, so
        // picking the same effort up again doesn't undo the owner's rename.
        let provider = self.state.provider.as_deref().unwrap_or_default();
        if let Some(name) = self.state.name.as_deref()
            && self.state.named.as_deref() != Some(name)
            && self.state.told.as_deref() != Some(name)
            && let Some(line) = rename(provider, name)
        {
            self.head += &format!("\n\n{line}");
            // Told, not named: Claude's hook still gives the name where the app has no tool.
            let name = name.to_owned();
            self.state.told.set(name);
        }
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
    let (_, notes) = notes_and_repos()?;
    let effort = read_effort(&notes, effort)?;
    switch_record(effort, false, session, provider, cwd, previous, shells)
}

/// Initiative activation shares the same eligibility, delivery and guard lifecycle.
pub fn switch_initiative(
    name: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    previous: Option<&SessionState>,
    shells: &[bool],
) -> Result<Activation, String> {
    if previous.is_some_and(|s| s.worker.is_true()) {
        return Err(
            "a worker cannot bind to an initiative; it saves its dispatched effort only".into(),
        );
    }
    let (_, notes) = notes_and_repos()?;
    let pages = crate::effort::initiative_catalog(&notes)?;
    let page = crate::effort::resolve_initiative(&pages, name)?;
    let setting = |key: &str, default: &str, allowed: &[&str]| -> Result<String, String> {
        let value = page.file.prop(key).unwrap_or(default);
        if !allowed.contains(&value) {
            return Err(format!("{}: unknown {key} {value:?}", page.path));
        }
        Ok(value.to_owned())
    };
    let effort = Effort {
        number: page.slug.clone(),
        file: page.path.clone(),
        title: page.file.title.clone(),
        repos: page
            .file
            .prop("repo")
            .unwrap_or("")
            .split(',')
            .map(|r| r.trim().to_owned())
            .collect(),
        involvement: setting("involvement", "check in at plan", &INVOLVEMENTS)?,
        authority: setting("authority", "edit", &AUTHORITIES)?,
        issues: Vec::new(),
        current: page.file.current.clone(),
        hash: crate::notes::content_hash(&page.text),
    };
    switch_record(effort, true, session, provider, cwd, previous, shells)
}

fn switch_record(
    effort: Effort,
    initiative: bool,
    session: &str,
    provider: &str,
    cwd: &Path,
    previous: Option<&SessionState>,
    shells: &[bool],
) -> Result<Activation, String> {
    let (registry, notes) = notes_and_repos()?;
    let repo = session_repo(&notes, &registry, &effort.repos, cwd)?;
    let read = |file: String| {
        let text = std::fs::read_to_string(notes.join(&file)).map_err(|e| format!("{file}: {e}"));
        Ok::<_, String>(Part {
            body: text?.replace("\r\n", "\n").trim().to_owned(),
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
    let mut parts = vec![current];
    if !initiative {
        let text = std::fs::read_to_string(notes.join(&effort.file))
            .map_err(|e| format!("{}: {e}", effort.file))?;
        let file = crate::effort::EffortFile::parse(&text)?;
        if let Some(pieces) = file.pieces().filter(|p| !p.trim().is_empty()) {
            parts.push(Part {
                title: format!("{}, Pieces", effort.file),
                body: pieces.to_owned(),
                files: vec![format!(
                    "{}: only its `## Pieces` section, up to the next `## ` heading.",
                    notes.join(&effort.file).display()
                )],
            });
        }
        // Missing legacy pages or a broken catalog must not prevent effort activation.
        if let Some(name) = file.prop("initiative").filter(|n| !n.is_empty())
            && let Ok(pages) = crate::effort::initiative_catalog(&notes)
            && let Ok(page) = crate::effort::resolve_initiative(&pages, name)
        {
            parts.push(Part {
                title: "Initiative".to_owned(),
                body: format!("[{}]({})", page.file.title, page.path),
                files: vec![format!(
                    "{}: the initiative's title and path only.",
                    notes.join(&page.path).display()
                )],
            });
        }
    }
    parts.extend([
        owner_note(&notes, "voice")?,
        owner_note(&notes, "owner")?,
        read(format!("knowledge/repos/{repo}.md"))?,
        read("knowledge/machines.md".to_owned())?,
    ]);
    let same = previous.filter(|p| {
        p.bound_key() == Some(effort.number.as_str()) && p.initiative_key().is_some() == initiative
    });
    let picked = if initiative {
        effort.title.clone()
    } else {
        session_name(&effort.number, &effort.title)
    };
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
        binding: if initiative {
            Binding::initiative(effort.number.clone())
        } else {
            Binding::effort(effort.number.clone())
        }
        .into(),
        effort: if initiative {
            Field::Missing
        } else {
            effort.number.clone().into()
        },
        file: effort.file.clone().into(),
        repo: repo.clone().into(),
        cwd: comparable(cwd).into(),
        at_ms: at_ms.into(),
        token: token.clone().into(),
        delivered_ms: now_ms().into(),
        delivered: delivered
            .map(Field::Known)
            .unwrap_or(Field::Other(Value::Null)),
        baseline: baseline
            .map(Field::Known)
            .unwrap_or(Field::Other(Value::Null)),
        mark: false.into(),
        // Kept through a resume only for an effort filed for later: one taken up takes its
        // name as it reads now, renumbered or retitled.
        name: match same.filter(|p| p.taken.as_ref() == Some(&false)) {
            Some(kept) => kept.name.clone(),
            None => picked.clone().into(),
        },
        taken: same.map_or(true.into(), |p| p.taken.clone()),
        // Whatever effort it was on: the name the session carries is the same one.
        named: previous.map(|p| p.named.clone()).unwrap_or_default(),
        told: previous.map(|p| p.told.clone()).unwrap_or_default(),
        // Unknown fields survive a resume, so a newer writer can share the record.
        extra: same.map(|p| p.extra.clone()).unwrap_or_default(),
        ..Default::default()
    };
    if let Some(previous) = same {
        if previous.binding.as_ref().is_some() {
            state.binding = previous.binding.clone();
        }
        if !previous.mark.is_missing() {
            state.mark = previous.mark.clone();
        }
        state.acknowledged = previous.acknowledged.clone();
        state.last_save_ms = previous.last_save_ms.clone();
        state.saves = previous.saves.clone();
        state.read_efforts = previous.read_efforts.clone();
        state.worker = previous.worker.clone();
    }
    fn meaning(names: &[&str; 3], meanings: &[&'static str; 3], value: &str) -> &'static str {
        names
            .iter()
            .zip(meanings)
            .find(|(name, _)| **name == value)
            .map_or("", |(_, m)| *m)
    }
    let head = format!(
        "Astrolabe is on in this session for effort {n}, \"{title}\" ({provider} session {session}, repo {repo}). The plugin's hook bound it to the session id the app gave; other sessions are unaffected.\n\n\
        Settings from {file}, set by the owner. Agents never raise them, because they're the owner's call:\n\
        - involvement: {inv}: {inv_m}.\n\
        - authority: {auth}: {auth_m}. It's a ceiling inside {repo} and this effort's scope, and never covers posting or destructive actions, because those reach other people or can't be undone. Authority covers project changes and pushes only: notes save and sync on their own at every level, so never ask for approval to save notes and never skip a save because project pushes need approval; a worker saves its own effort the same way.\n\
        {issue}\
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
        inv_m = meaning(&INVOLVEMENTS, &INVOLVEMENT_MEANINGS, &effort.involvement),
        auth = effort.authority,
        auth_m = meaning(&AUTHORITIES, &AUTHORITY_MEANINGS, &effort.authority),
        proof = proof_steps(&token, shells),
        issue = issue_line(&effort.issues),
    );
    let head = if initiative {
        let file = crate::effort::EffortFile::parse(
            &std::fs::read_to_string(notes.join(&effort.file)).map_err(|e| e.to_string())?,
        )?;
        format!(
            "Astrolabe is on in this session for initiative {slug}, \"{title}\" ({provider} session {session}, repo {repo}).\n\nPage: {path}\nStatus: {status}\nWaits for: {wait}\nSettings set by the owner: involvement: {inv}: {inv_m}. Authority: {auth}: {auth_m}. Notes save and sync independently of project authority. A narrower current instruction wins.\n\n{GUARD_SCOPE}\n\n{proof}Report the initiative and what the status says. Until it says proven, claim no protection.\n\nSave this initiative with `astrolabe effort save [--status <state>] [--decision \"chose X over Y: reason\"] [--waits-for \"owner: …\"] [--journal \"…\"]` with a body:\n  astrolabe effort save --status active <<'ASTROLABE'\n  <where the initiative stands>\n\n  <choices and reasons>\n\n  <next and unchecked>\n  ASTROLABE\nDecisions are appended and dated. A journal line replaces today's line for this initiative. Done requires an acceptance statement in the body and every member done or abandoned. Use --reason with abandoned. Pieces, ticks, plans and initiative membership are unavailable on an initiative. Detach with `astrolabe activate --off`; the machine guard stays.\n\n{freshness}",
            slug = effort.number,
            title = effort.title,
            path = effort.file,
            status = file.prop("status").unwrap_or("ready"),
            wait = file.prop("waits_for").unwrap_or("none"),
            inv = effort.involvement,
            inv_m = meaning(&INVOLVEMENTS, &INVOLVEMENT_MEANINGS, &effort.involvement),
            auth = effort.authority,
            auth_m = meaning(&AUTHORITIES, &AUTHORITY_MEANINGS, &effort.authority),
            proof = proof_steps(&token, shells),
            freshness = freshness(&notes, &effort, cwd),
        )
    } else {
        head
    };
    // A worker stays a worker through resume and reload, and is told so each time.
    let mut head = head;
    if state.worker.is_true() {
        head += &format!("\n\n{WORKER_RULE}");
    }
    Ok(Activation {
        state,
        head,
        parts,
        picked,
    })
}

/// The effort's GitHub issues, and the keyword that closes each when its commit reaches main.
/// Nothing else closes them: the landing script pushes straight to main, with no pull request.
fn issue_line(issues: &[String]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let tags: Vec<String> = issues.iter().map(|n| format!("#{n}")).collect();
    let fixes: Vec<String> = issues.iter().map(|n| format!("`Fixes #{n}`")).collect();
    format!(
        "- issue: {}: this work answers it on GitHub. The commit that lands it says {} on a line of its own, because only that keyword reaching main closes the issue; marking the effort done reports any it hasn't.\n",
        tags.join(", "),
        fixes.join(" and ")
    )
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
            "{} {} stays loaded as it was.",
            if p.initiative_key().is_some() {
                "Initiative"
            } else {
                "Effort"
            },
            p.bound_key().unwrap_or_default()
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
    // The repo's own page holds how work lands there, so every session in the repo needs it,
    // not only one that loads an effort. Matched apart from the efforts, so an unreadable
    // effort can't cost the session its repo's rules. The notes checkout has no repo of its own.
    let here: Vec<String> = match common_dir(cwd) {
        Some(dir) if !seat => repo_names(&notes, &registry, cwd, &dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|name| name != AGENT)
            .collect(),
        _ => Vec::new(),
    };
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
    // Last, so a page too long for the cut goes as a file to read without taking voice's
    // short rules with it. A repo with no page adds nothing: its rules live in the repo.
    for name in &here {
        let file = format!("knowledge/repos/{name}.md");
        if let Ok(text) = std::fs::read_to_string(notes.join(&file)) {
            parts.push(Part {
                body: text.replace("\r\n", "\n").trim().to_owned(),
                files: vec![notes.join(&file).display().to_string()],
                title: file,
            });
        }
    }
    Activation {
        state: SessionState::default(),
        head,
        parts,
        picked: String::new(),
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

/// The repair command uses the bound record's kind, including when no block could be saved.
pub(super) fn blocked(kind: &str, key: &str, why: &str, recording_error: Option<&str>) -> String {
    if let Some(error) = recording_error {
        return format!(
            "Astrolabe stays on for {kind} {key} in this session, guard included, but its context couldn't be reloaded: {why}. It also couldn't record a block on shell commands ({error}), so only the guard is on and other commands are NOT blocked. Treat this session's {kind} context as missing: tell the owner, repair it and run `astrolabe activate --{kind} {key}`, or drop the {kind} with `astrolabe activate --off` (the guard keeps running)."
        );
    }
    format!(
        "Astrolabe stays on for {kind} {key} in this session, guard included, but its context couldn't be reloaded: {why}. Shell commands are blocked until that's repaired, because work here needs the {kind}'s current context. Repair it, then run `astrolabe activate --{kind} {key}`; or drop the {kind} with `astrolabe activate --off` (the guard keeps running)."
    )
}
