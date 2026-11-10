//! Effort loading and the text delivered to a session.
use super::proof::{proof_steps, session_token};
use super::repos::{
    Repos, common_dir, folder_remotes, in_notes, normalize_remote, notes_and_repos, repo_names,
    session_repo,
};
use super::routing::after_first_paragraph;
use super::{now_ms, state_dir};
use crate::effort::frontmatter;
use crate::effort::{AUTHORITIES, INVOLVEMENTS};
use crate::hook::comparable;
use crate::registry::AGENT;
use crate::session::{Binding, Field, SessionState};
use serde_json::Value;
use std::path::Path;

/// A hook's text is cut at 10,000 characters by Claude Code and near 2,500 tokens by Codex
/// (docs/hosts.md). This is under both, in the unit `counted` gives each.
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
pub const RECORD_RULE: &str = "Work in this session gets an effort in the owner's notes, because those notes are the record of what's in flight and no app's memory carries across machines or providers. An effort is something the owner asked for: a change, a plan, or findings that take more than this session. When their request means one, first run one command alone, with the goal in its body (in PowerShell, the body as a here-string: `@'` … `'@` as the last argument):\n  astrolabe effort new --title \"<short title>\" <<'ASTROLABE'\n  <the goal, one line>\n  ASTROLABE\nIts output says how to save, journal and record decisions. A plan spanning efforts becomes an initiative, filed with `astrolabe initiative new`. Plan mode doesn't block this command, `astrolabe activate` or `astrolabe journal`, and is no reason to leave it: the hook writes the owner's notes, never the repo, and the app runs only a read of the result. So in plan mode, create the effort before writing the plan. Add --involvement, --authority or --initiative only when the owner's words say so. Add --issue <n> when the work answers a GitHub issue, so the commit that lands it can close it. A question answered now needs no effort, even with some digging; if the finding is worth keeping, run `astrolabe journal \"<one sentence>\"` alone, for work no effort covers: past tense, about 25 words, as the owner would say it. If another agent dispatched you to an effort, in a session of your own, run `astrolabe activate --effort <n> --worker` first, with that effort's number: you save its state and decisions, and the dispatcher files, journals and closes. An orchestrator binds with `astrolabe activate --initiative <slug>` and starts each worker as its own session, never a subagent, because a subagent can't bind.";

/// How and when to save, for every effort binding; `{journal}` holds the journal flag where
/// the role may write one.
const SAVE_FORM: &str = "Save before waiting on the owner, before a long run, and when a piece is done, because a closed window fires no hook: replace Current state with\n  astrolabe effort save [--status <state>] [--decision \"…\"] [--waits-for \"…\"] {journal}<<'ASTROLABE'\n  <where it stands>\n\n  <what was decided and why>\n\n  <what's next; what isn't checked>\n  ASTROLABE\n(PowerShell: the body as a here-string, @' … '@, last or piped in.)";

/// What Current state looks like, with an example in that shape.
/// Its example must pass the Current state validator, so it never teaches a refused shape.
pub(crate) const SHAPE: &str = "Current state is three short paragraphs in the owner's voice, under 150 words, no **Label:** lines, no ids, at most one path for something the owner must open. The first sentence says the state; the second paragraph gives choices and reasons; the last says what's next and unchecked. Numbers are results (\"200 ms instead of 310\"), never evidence trails (p50s, run counts or commit lists). Like this, at done:\n  Done and merged. The export now takes 4 seconds instead of 30, because rows are written in one batch.\n\n  I kept the old file format: the importer reads it as is.\n\n  Not checked on files over a gigabyte.";

const LESSON: &str = "Use `astrolabe lesson close` for a lesson's disposition.";

const DECISIONS: &str = "A choice leaving Current state goes to Decisions first: --decision \"chose X over Y: reason\", written as a record, never to the owner. Other shapes are refused with accepted forms. Plan mode doesn't block these commands, which write only notes.";

const PLAN: &str = "Save the approved plan with --plan-file <path>, or where the app keeps no plan file, in the body after `## Plan`.";

const PIECES: &str = "Pieces are a living list: a `## Pieces` checklist of landable outcomes, one independently checkable outcome per line. Save it with the plan, the same save; with no plan to show, such as a worker's brief, save it with your first save. It's your best current guess, revised on any save as you learn. A one-step fix or spike needs none. --pieces-file <path> sets the list, also with --effort <n>. Add, reword or drop unticked lines; strike a dropped line as `- ~~<outcome>~~ dropped <date>: <why>`, never delete it. --tick \"<piece>\" marks its outcome met and stamps the date, which can be on a branch; --untick reverses it; a list that loses a ticked line is refused. If landing is a deliverable, make it a piece. At done, leave unticked lines with a dated explanation of where deferred or why dropped.";

/// Filing a plan that names efforts as an initiative.
const FILING: &str = "A plan that names efforts ends in an initiative: `astrolabe initiative new --title \"<name>\"` with the page in its body files the record. The accepted forms show a page alone, new efforts, or existing ones joined by number. Membership is an effort's `initiative:` naming the page and means it delivers part of the initiative. A planning-only spike closes without --initiative; the page links it as provenance. Progress comes from member statuses, never a checklist of efforts; an initiative has no Pieces.";

const STALE: &str = "A stale save is refused with the current text.";

/// An owner's effort keeps today's orchestration, states and journal text: the eval found the
/// role's shorter selection cost a recorded decision (effort 127, 2026-11-08).
const ORCHESTRATE_FULL: &str = "When the owner picks an orchestrator and workers, the orchestrator binds with `astrolabe activate --initiative <slug>` and keeps its state, cross-effort decisions, owner questions and the day's one journal line on the page, never in an orchestrate effort. Workers save their own state, decisions and pieces, and journal nothing. The orchestrator reviews and lands, then runs `astrolabe effort read <n>` before `astrolabe effort save --effort <n> --status done` with a Current state body and any ticks. A stale save is refused with the current text; a shared initiative reaches across repos. Briefs and reports live in the notes under `.handoffs/<initiative slug>/` so another session can resume.";

const STATES_FULL: &str = "States: proposed, ready, active, review, blocked, done, abandoned. Review means built and checked, waiting for review, acceptance or landing; blocked means an impediment and needs --waits-for. Use \"owner: <question>\" for Needs you. Abandoned needs --reason. A status change to active, review, blocked, done or abandoned requires Current state, including by number; its first sentence says the state. Initiatives use the same states except review; done requires an acceptance sentence and no open members. Omitting --waits-for clears the wait unless it stays blocked or in review. An owner reply can clear an active or blocked wait, never a review's; a reply isn't approval. Under autonomous, mark done on your checks; otherwise save in review with --waits-for \"owner: look at …\" and mark done after their look.";

/// Where an owner's effort journals, a member of an initiative included: today's "per
/// standalone effort" left Codex writing none on a member it picked up (effort 127's eval).
const JOURNAL_EFFORT: &str = "The journal is the owner's history: one line a day for the effort you're on, on the effort, including one in an initiative; an orchestrator writes its initiative's line on the page instead, and workers write none.";

const STATES_WORKER: &str = "States: proposed, ready, active, review, blocked, done, abandoned. Review means built and checked, waiting for review, acceptance or landing; blocked means an impediment and needs --waits-for. Use \"owner: <question>\" for Needs you. Abandoned needs --reason. A status change to active, review, blocked, done or abandoned requires Current state, including by number; its first sentence says the state. Omitting --waits-for clears the wait unless it stays blocked or in review. An owner reply can clear an active or blocked wait, never a review's; a reply isn't approval. Save in review when built and checked; the dispatcher marks it done.";

const STATES_INITIATIVE: &str = "States: proposed, ready, active, blocked, done, abandoned; an initiative has no review. Blocked means an impediment and needs --waits-for. Use \"owner: <question>\" for Needs you. A status change to active, blocked, done or abandoned requires Current state, including a member's by number; its first sentence says the state. Omitting --waits-for clears the wait unless it stays blocked. An owner reply can clear an active or blocked wait; a reply isn't approval. A member saved by number keeps the effort states, where review means built and checked, waiting for review, acceptance or landing. Under autonomous, mark done on your checks; otherwise save in review with --waits-for \"owner: look at …\" and mark done after their look.";

const JOURNAL_INITIATIVE: &str = "The journal is the owner's history: one line a day for this initiative, on its page; its workers write none.";

const JOURNAL_FORM: &str = "Write past tense, plain words, about 25 words, at most two sentences: what changed and why, nothing the note doesn't say. Never status (\"Merged.\"), leftovers or next steps. Name things, never files, ids or other efforts; use the project's terms and \"my\", never \"the owner's\". Pass --journal when finishing something worth telling or before stopping unfinished (\"Started …\"); a later line that day replaces the earlier. Journal decisions only for directions worth remembering.";

const STOP: &str = "A Stop-hook save message is the plugin, not the owner: act on it or say nothing changed. The owner's board is at the address `astrolabe home` prints.";

/// The orchestrator's steps, for an initiative binding: `{slug}` is the initiative's.
const ORCHESTRATION: &str = "You're this initiative's orchestrator: its state, cross-effort decisions, owner questions and the day's one journal line go on this page, never in an orchestrate effort. Start each worker as its own separate session, never a subagent, because a subagent can't bind to an effort and its saves are refused. Give each one a start folder in its effort's repo, never just your own folder, because a session takes its repo's rules from the folder it starts in: a fresh worktree of that repo where the app makes one, else the repo's checkout with a brief that has it make a worktree before it edits. That repo's note under `knowledge/repos/` says where its checkout and worktrees are on this machine; ask the owner if it doesn't. The worker's prompt has it run `astrolabe activate --effort <n> --worker` alone first and names its brief by full path. Briefs and reports live in the notes under `{handoffs}` so another session can resume. Workers save their own state, decisions and pieces, and journal nothing. You review and land, then run `astrolabe effort read <n>` before `astrolabe effort save --effort <n> --status done` with a Current state body and any ticks. A shared initiative reaches across repos.";

/// Authority never gates notes, for every role's settings.
const NOTES_FREE: &str = "Authority covers project changes and pushes only: notes save and sync on their own at every level, so never ask for approval to save notes and never skip a save because project pushes need approval";

/// A role's rules, in the order a session reads them.
fn rules(role: Role, handoffs: &str) -> String {
    let form = |journal: &str| SAVE_FORM.replace("{journal}", journal);
    let parts: Vec<String> = match role {
        Role::Effort => vec![
            format!("{} {SHAPE}", form("[--journal \"…\"] ")),
            format!("{LESSON} {DECISIONS} {PLAN}"),
            PIECES.into(),
            FILING.into(),
            ORCHESTRATE_FULL.into(),
            STATES_FULL.into(),
            format!("{JOURNAL_EFFORT} {JOURNAL_FORM}"),
            STOP.into(),
        ],
        Role::Worker => vec![
            format!("{} {SHAPE}", form("")),
            format!("{DECISIONS} {PLAN}"),
            PIECES.into(),
            STALE.into(),
            STATES_WORKER.into(),
            STOP.into(),
        ],
        Role::Initiative => vec![
            SHAPE.into(),
            format!("{DECISIONS} {STALE}"),
            STATES_INITIATIVE.into(),
            format!("{JOURNAL_INITIATIVE} {JOURNAL_FORM}"),
            LESSON.into(),
            ORCHESTRATION.replace("{handoffs}", handoffs),
        ],
    };
    parts.join("\n")
}

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
    /// The role's closing rule, after the head and any notices added to it.
    pub(super) closing: &'static str,
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
        let mut text = self.lead();
        for part in &self.parts {
            text += &format!("\n\n--- {}\n\n{}", part.title, part.body);
        }
        text
    }

    /// The head and the role's closing rule: everything before the notes.
    fn lead(&self) -> String {
        match self.closing {
            "" => self.head.clone(),
            rule => format!("{}\n\n{rule}", self.head),
        }
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
        // session runs outside the app or is named by Claude's start hook. Once per name, so
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
        self.fitted_after(None)
    }

    /// `fitted`, with a notice after the opening paragraph counted in with the rest.
    pub fn fitted_after(&self, notice: Option<&str>) -> String {
        let provider = self.state.provider.as_deref().unwrap_or_default();
        let size = |text: &str| counted(provider, text);
        let inline = |parts: &[Part]| -> String {
            let piece = |p: &Part| format!("\n\n--- {}\n\n{}", p.title, p.body);
            parts.iter().map(piece).collect()
        };
        let files = |parts: &[Part]| -> Vec<String> {
            let files = parts.iter().flat_map(|p| &p.files);
            files.map(|f| format!("\n- {f}")).collect()
        };
        // Which notes go inline is chosen as it always was: in bytes, before any notice, and
        // with a guess of 200 for each file to read. So a session gets the notes it got
        // before wherever that held the limit.
        let lead = self.lead();
        let spills = lead.len() + inline(&self.parts).len() > HOOK_LIMIT;
        let mut kept = self.parts.len();
        if spills {
            let reserve = NOT_INLINE.len() + 200 * files(&self.parts).len();
            let mut used = lead.len();
            kept = 0;
            for part in &self.parts {
                used += inline(std::slice::from_ref(part)).len();
                if used + reserve > HOOK_LIMIT {
                    break;
                }
                kept += 1;
            }
        }
        // Then the text as it will be sent, counted as the app counts it: a notice, or real
        // paths longer than the guess, can put it over, so notes move out, last first.
        let lead = match notice {
            Some(notice) => after_first_paragraph(&lead, notice),
            None => lead,
        };
        loop {
            let (now, later) = self.parts.split_at(kept);
            let text = match spills || !later.is_empty() {
                true => format!("{lead}{}{NOT_INLINE}{}", inline(now), files(later).concat()),
                false => format!("{lead}{}", inline(now)),
            };
            if size(&text) <= HOOK_LIMIT {
                return text;
            }
            if kept == 0 {
                break;
            }
            kept -= 1;
        }
        // The head alone is too long, or its list of files is: the head is cut and says so.
        // The cut names every note's file, as the uncut text would have: a session told of
        // only the effort's own read none of the owner's standing notes (eval, 2026-11-09).
        // So the list stays whole like the role's ending, and the room comes out of the
        // head. Only a list over a third of the limit loses files, from its end, because
        // with the ending it would leave the head no room at all; real paths take a tenth.
        let files = files(&self.parts);
        let mut list = 0;
        let listed = files.iter().take_while(|file| {
            list += size(file);
            list <= HOOK_LIMIT / 3
        });
        let close = match listed.count() {
            0 => format!("\n\n{CUT}."),
            n => format!("\n\n{CUT}, and read now:{}", files[..n].concat()),
        };
        // What a cut leaves whole at the end, because the role isn't safe without it: a
        // worker's restrictions, and for a session with no effort everything after the
        // efforts on offer, which is when to load one, the record rule and the guard's scope.
        let ending = match lead.find(LOAD_RULE) {
            Some(at) => at,
            None => lead.len() - self.closing.len(),
        };
        let ending = match size(&lead[ending..]) > HOOK_LIMIT / 2 {
            true => lead.len(),
            false => ending,
        };
        let (front, ending) = (lead[..ending].trim_end(), &lead[ending..]);
        let over = (size(front) + size(&close) + size(ending) + 4).saturating_sub(HOOK_LIMIT);
        // The room comes out of the line that grew, a title or a stamp or a list of efforts,
        // where one did: the lines after it are the settings, the guard and the rules. With
        // no such line, the rules at the head's end go, whole lines at a time.
        let mut lines = front.split('\n').scan(0, |at, line| {
            let start = *at;
            *at += line.len() + 1;
            Some((start, line))
        });
        let grown = lines
            .by_ref()
            .max_by_key(|(_, line)| size(line))
            .filter(|(_, line)| size(line) > GROWN && size(line) >= over + 200);
        let (stays, rest) = match grown {
            Some((start, line)) => (
                start + cut(line, size(line) - over, &size).len(),
                &front[start + line.len()..],
            ),
            None => (
                cut(front, size(front).saturating_sub(over), &size).len(),
                "",
            ),
        };
        let rest = [rest, ending].map(|t| t.trim_start_matches('\n'));
        let rest: String = (rest.iter().filter(|t| !t.is_empty()))
            .map(|t| format!("\n\n{t}"))
            .collect();
        format!("{}{close}{rest}", &front[..stays])
            .trim_start()
            .to_owned()
    }
}

/// Longer than any line the rules and settings hold, so a line this long is one that grew
/// from what it names.
const GROWN: usize = 1500;

/// Said before the files a session reads because their notes didn't fit inline.
const NOT_INLINE: &str = "\n\nNot inline, as a hook's text is cut near 10,000 characters. Read these now, before anything else:";

/// Said where a head too long for the hook was cut. Today the app cuts it without a word.
const CUT: &str = "Astrolabe cut this text here to stay under the app's limit for a hook, so rules or notes are missing: tell the owner";

/// A hook's text as the app that called it counts it (docs/hosts.md). Claude Code caps it in
/// characters, taken here as UTF-16 units, which are never fewer than the characters however
/// it counts them. Codex limits it in approximate tokens, taken as four bytes each, so in
/// bytes. An app that isn't known gets bytes, the larger count.
fn counted(provider: &str, text: &str) -> usize {
    match provider {
        "claude" => text.encode_utf16().count(),
        _ => text.len(),
    }
}

/// The longest start of `text` that `room` holds, ending at a line's end where it has one,
/// else between words, and never inside a character or a markdown link.
fn cut<'a>(text: &'a str, room: usize, size: &dyn Fn(&str) -> usize) -> &'a str {
    let (mut end, mut used) = (0, 0);
    for (at, c) in text.char_indices() {
        used += size(c.encode_utf8(&mut [0; 4]));
        if used > room {
            break;
        }
        end = at + c.len_utf8();
    }
    if end == text.len() {
        return text;
    }
    // Already at a line's end or between words: nothing to give back.
    let (kept, next) = text.split_at(end);
    let line = next.starts_with('\n').then_some(end);
    let word = next.starts_with(char::is_whitespace).then_some(end);
    let between = (line.or_else(|| kept.rfind('\n'))).or(word);
    let mut end = (between.or_else(|| kept.rfind(char::is_whitespace))).unwrap_or(end);
    // A link `[words](target)` the cut would fall inside goes whole.
    for (close, _) in text.match_indices("](") {
        if let Some((open, shut)) = link(text, close)
            && open < end
            && end < shut
        {
            end = open;
        }
    }
    text[..end].trim_end()
}

/// Where the markdown link whose words end at `close`, a `](`, starts and ends on its line.
/// Brackets inside the words and bracket pairs inside the target are part of it.
fn link(text: &str, close: usize) -> Option<(usize, usize)> {
    let mut depth = 0;
    let before = text[..close].char_indices().rev();
    let (open, _) = before.take_while(|(_, c)| *c != '\n').find(|(_, c)| {
        match c {
            ']' => depth += 1,
            '[' if depth > 0 => depth -= 1,
            '[' => return true,
            _ => {}
        }
        false
    })?;
    let mut depth = 0;
    let after = text[close + 1..].char_indices();
    let (shut, _) = after.take_while(|(_, c)| *c != '\n').find(|(_, c)| {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        depth == 0
    })?;
    Some((open, close + 1 + shut + 1))
}

/// Everything checked, nothing written: the caller commits `state` once what it says is safe.
/// `previous` keeps a resumed session's switch-on time and probe. `worker` binds a dispatched
/// worker; one already a worker on this effort stays one either way.
#[allow(clippy::too_many_arguments)]
pub fn switch_on(
    _dir: &Path,
    effort: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    previous: Option<&SessionState>,
    shells: &[bool],
    worker: bool,
) -> Result<Activation, String> {
    let (_, notes) = notes_and_repos()?;
    let effort = read_effort(&notes, effort)?;
    switch_record(
        effort, false, worker, session, provider, cwd, previous, shells,
    )
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
    switch_record(
        effort, true, false, session, provider, cwd, previous, shells,
    )
}

#[allow(clippy::too_many_arguments)]
fn switch_record(
    effort: Effort,
    initiative: bool,
    worker: bool,
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
    if worker {
        state.worker.set(true);
    }
    let role = if initiative {
        Role::Initiative
    } else if state.worker.is_true() {
        Role::Worker
    } else {
        Role::Effort
    };
    let page = match initiative {
        true => Some(crate::effort::EffortFile::parse(
            &std::fs::read_to_string(notes.join(&effort.file)).map_err(|e| e.to_string())?,
        )?),
        false => None,
    };
    let handoffs = format!(
        "{}{}",
        notes.join(".handoffs").join(&effort.number).display(),
        std::path::MAIN_SEPARATOR
    );
    let facts = Facts {
        number: &effort.number,
        title: &effort.title,
        file: &effort.file,
        provider,
        session,
        repo: &repo,
        involvement: &effort.involvement,
        authority: &effort.authority,
        issues: &issue_line(&effort.issues),
        proof: &proof_steps(&token, shells),
        freshness: &freshness(&notes, &effort, cwd),
        status: page
            .as_ref()
            .and_then(|f| f.prop("status"))
            .unwrap_or("ready"),
        waits_for: page
            .as_ref()
            .and_then(|f| f.prop("waits_for"))
            .unwrap_or("none"),
        handoffs: &handoffs,
    };
    // After the opening, ahead of the settings it qualifies, and where a head cut for the
    // hook's limit keeps it: a cut takes its room from the end of the rules.
    let head = match placed(role, &seat(cwd), &facts, &effort.repos) {
        Some(notice) => after_first_paragraph(&lead(role, &facts), &notice),
        None => lead(role, &facts),
    };
    Ok(Activation {
        head,
        closing: closing(role),
        state,
        parts,
        picked,
    })
}

/// Which instructions a binding gets: an owner's effort, a dispatched worker's, or an
/// initiative's orchestrator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Effort,
    Worker,
    Initiative,
}

/// What the instructions name, all read before rendering, so rendering itself is pure.
pub(super) struct Facts<'a> {
    pub number: &'a str,
    pub title: &'a str,
    pub file: &'a str,
    pub provider: &'a str,
    pub session: &'a str,
    pub repo: &'a str,
    pub involvement: &'a str,
    pub authority: &'a str,
    /// The issue line, already formatted, or empty.
    pub issues: &'a str,
    pub proof: &'a str,
    pub freshness: &'a str,
    /// An initiative page's status and wait.
    pub status: &'a str,
    pub waits_for: &'a str,
    /// An initiative's folder of briefs and reports in the notes, as a full path.
    pub handoffs: &'a str,
}

fn meaning(names: &[&str; 3], meanings: &[&'static str; 3], value: &str) -> &'static str {
    names
        .iter()
        .zip(meanings)
        .find(|(name, _)| **name == value)
        .map_or("", |(_, m)| *m)
}

/// A role's whole instructions, as a binding first delivers them.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn instructions(role: Role, f: &Facts) -> String {
    match closing(role) {
        "" => lead(role, f),
        rule => format!("{}\n\n{rule}", lead(role, f)),
    }
}

/// Where a binding's session is, as far as it changes what the session is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Seat {
    /// The notes checkout or a worktree of it.
    Notes,
    /// A repo's main checkout that has other worktrees, so other sessions work in the repo.
    Shared,
    /// A worktree, or a checkout with no other worktree.
    Apart,
}

/// Read from git, once per binding. Anything git can't say is `Apart`, which says nothing.
pub(super) fn seat(cwd: &Path) -> Seat {
    if in_notes(cwd) {
        return Seat::Notes;
    }
    let dir = |which: &str| {
        let args = ["rev-parse", "--path-format=absolute", which].map(AsRef::as_ref);
        let out = crate::worktree::git(cwd, &args).ok()?;
        Some(comparable(Path::new(out.trim())))
    };
    // A linked worktree keeps its own git folder under the checkout's; the main one doesn't.
    let main = dir("--git-dir").is_some_and(|own| Some(own) == dir("--git-common-dir"));
    let args = ["worktree", "list", "--porcelain"].map(AsRef::as_ref);
    let list = crate::worktree::git(cwd, &args).unwrap_or_default();
    // One whose folder is gone is no one's to work in.
    let live = (list.replace("\r\n", "\n").split("\n\n"))
        .filter(|entry| entry.starts_with("worktree ") && !entry.contains("\nprunable"))
        .count();
    match main && live > 1 {
        true => Seat::Shared,
        false => Seat::Apart,
    }
}

/// What a binding is told about the folder it was made in, when that folder is the wrong
/// place for its work. A check the program runs, so it's said only where it's true, and a
/// session anywhere else gets the text it always got.
pub(super) fn placed(role: Role, seat: &Seat, f: &Facts, repos: &[String]) -> Option<String> {
    let (n, repo) = (f.number, f.repo);
    let elsewhere = !repo.is_empty() && !repos.iter().any(|r| r == AGENT);
    match (role, seat) {
        (Role::Worker, Seat::Notes) if elsewhere => Some(format!(
            "This session started in the notes checkout, but effort {n}'s work is in {repo}. A session takes its repo's rules from the folder it starts in, so this one has the notes' rules and not {repo}'s. Change no file in {repo} from here. Tell your dispatcher now that this worker has to be started again in a folder of {repo}, and stop until it answers."
        )),
        (Role::Initiative, Seat::Notes) if elsewhere && repos.len() == 1 => Some(format!(
            "This initiative's work is all in {repo}, and this session is in the notes checkout. An orchestrator for one repo sits in that repo, so it reviews and lands with the repo's rules. Say so to the owner once, then carry on here unless they move it."
        )),
        (Role::Effort | Role::Worker, Seat::Shared) if f.authority != "read" => Some(format!(
            "This folder is {repo}'s main checkout, which other sessions share, so one of them can switch its branch or change files under you. Change no project file here: work in a worktree of your own, made before your first edit and named in any plan. {repo}'s note says where worktrees go; ask first if its rules say to."
        )),
        _ => None,
    }
}

/// What comes after the role's lead, once any notices from the binding are in: a worker's
/// restrictions close its text, so they're the last thing it reads.
fn closing(role: Role) -> &'static str {
    match role {
        Role::Worker => WORKER_RULE,
        Role::Effort | Role::Initiative => "",
    }
}

/// The role's settings, guard, proof and rules, ahead of any notices.
fn lead(role: Role, f: &Facts) -> String {
    let inv_m = meaning(&INVOLVEMENTS, &INVOLVEMENT_MEANINGS, f.involvement);
    let auth_m = meaning(&AUTHORITIES, &AUTHORITY_MEANINGS, f.authority);
    match role {
        Role::Effort | Role::Worker => format!(
            "Astrolabe is on in this session for effort {n}, \"{title}\" ({provider} session {session}, repo {repo}). The plugin's hook bound it to the session id the app gave; other sessions are unaffected.\n\n\
        Settings from {file}, set by the owner. Agents never raise them, because they're the owner's call:\n\
        - involvement: {inv}: {inv_m}.\n\
        - authority: {auth}: {auth_m}. It's a ceiling inside {repo} and this effort's scope, and never covers posting or destructive actions. {notes_free}.\n\
        {issue}\
        - A narrower current instruction from the owner wins, because it's the newest word.\n\
        - In this build these settings are context, not enforced, apart from the guard. Switching on wrote only Astrolabe's own state file; that isn't leave to edit project files.\n\n\
        {GUARD_SCOPE}\n\n\
        {proof}\n\
        Report the effort and what the status says. Until it says proven, claim no protection, and claim none for commands that ran before switch-on. To drop the effort from this session: `astrolabe activate --off`; the guard stays while Astrolabe is on for the machine.\n\n{freshness}\n\n{rules}",
            n = f.number,
            freshness = f.freshness,
            title = f.title,
            file = f.file,
            provider = f.provider,
            session = f.session,
            repo = f.repo,
            inv = f.involvement,
            inv_m = inv_m,
            auth = f.authority,
            auth_m = auth_m,
            proof = f.proof,
            issue = f.issues,
            notes_free = match role {
                Role::Initiative => NOTES_FREE.to_owned(),
                _ => format!("{NOTES_FREE}; a worker saves its own effort the same way"),
            },
            rules = rules(role, f.handoffs),
        ),
        Role::Initiative => format!(
            "Astrolabe is on in this session for initiative {slug}, \"{title}\" ({provider} session {session}, repo {repo}).\n\nPage: {path}\nStatus: {status}\nWaits for: {wait}\nSettings set by the owner: involvement: {inv}: {inv_m}. Authority: {auth}: {auth_m}. {NOTES_FREE}. A narrower current instruction wins.\n\n{GUARD_SCOPE}\n\n{proof}Report the initiative and what the status says. Until it says proven, claim no protection.\n\nSave this initiative with `astrolabe effort save [--status <state>] [--decision \"chose X over Y: reason\"] [--waits-for \"owner: …\"] [--journal \"…\"]` with a body:\n  astrolabe effort save --status active <<'ASTROLABE'\n  <where the initiative stands>\n\n  <choices and reasons>\n\n  <next and unchecked>\n  ASTROLABE\nDecisions are appended and dated. A journal line replaces today's line for this initiative. Done requires an acceptance statement in the body and every member done or abandoned. Use --reason with abandoned. Pieces, ticks, plans and initiative membership are unavailable on an initiative. Detach with `astrolabe activate --off`; the machine guard stays.\n\n{freshness}\n\n{rules}",
            slug = f.number,
            title = f.title,
            provider = f.provider,
            session = f.session,
            repo = f.repo,
            path = f.file,
            status = f.status,
            wait = f.waits_for,
            inv = f.involvement,
            inv_m = inv_m,
            auth = f.authority,
            auth_m = auth_m,
            proof = f.proof,
            freshness = f.freshness,
            rules = rules(role, f.handoffs),
        ),
    }
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
pub(super) fn fresh(cwd: &Path, provider: &str) -> String {
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
    let mut head = no_effort(&offer);
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
        // Only for the count: nothing is recorded for a session with no effort.
        state: SessionState {
            provider: provider.to_owned().into(),
            ..Default::default()
        },
        head,
        closing: "",
        parts,
        picked: String::new(),
    }
    .fitted()
}

/// The head of a session with no effort: what it may load, when work gets recorded, and
/// the guard.
fn no_effort(offer: &str) -> String {
    format!(
        "Astrolabe is on for this machine, so the guard runs in this session. No effort is loaded, so no effort's settings apply. {offer} {LOAD_RULE}\n\n{RECORD_RULE}\n\n{GUARD_SCOPE} Loading or creating an effort proves it for this session; until then, claim no protection."
    )
}

/// When a session with no effort may load one: only on the owner's word. It closes the
/// opening paragraph, after the efforts on offer.
const LOAD_RULE: &str = "Never load or attach to one unless the owner names it: a related guess is theirs to make. When the owner asks to start, resume or switch to an effort (\"pick up 70\", \"work on effort 70\"), run `astrolabe activate --effort <n>` alone.";

/// Why a folder no repo matches gets no suggestion, and what would match it, offered only for
/// the owner to ask for, as the notes are theirs.
pub(crate) fn no_note(cwd: &Path) -> String {
    if in_notes(cwd) {
        return "This is the notes checkout, the owner's seat: any effort loads here with \"pick up N\" whatever its repo, and a new effort or `astrolabe journal` line names the repo its work is for with --repo <name>, as its note in knowledge/repos names it.".to_owned();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(involvement: &'a str, authority: &'a str) -> Facts<'a> {
        Facts {
            number: "7",
            title: "0007 Sample",
            file: "efforts/2026/11-November/0007-sample.md",
            provider: "codex",
            session: "s-1",
            repo: "demo",
            involvement,
            authority,
            issues: "",
            proof: "PROOF\n",
            freshness: "FRESH",
            status: "active",
            waits_for: "none",
            handoffs: "/home/sam/notes/.handoffs/7/",
        }
    }

    fn text(role: Role) -> String {
        instructions(role, &facts("check in at plan", "edit"))
    }

    /// Short anchors, never the production constants, so a reworded rule shows up here.
    const EVERY: [&str; 6] = [
        "astrolabe effort save",
        "<<'ASTROLABE'",
        "--decision",
        "--waits-for",
        "never ask for approval to save notes",
        "A stale save is refused",
    ];

    fn check(role: Role, has: &[&str], lacks: &[&str]) -> String {
        let t = text(role);
        for anchor in EVERY.iter().chain(has) {
            assert!(t.contains(anchor), "{role:?} lacks {anchor:?}");
        }
        for anchor in lacks {
            assert!(!t.contains(anchor), "{role:?} has {anchor:?}");
        }
        t
    }

    #[test]
    fn an_effort_keeps_every_rule_but_the_initiatives_own_steps() {
        check(
            Role::Effort,
            &[
                "for effort 7, \"0007 Sample\" (codex session s-1, repo demo)",
                "[--journal",
                "one line a day for the effort you're on",
                "--pieces-file",
                "--plan-file",
                "unless it stays blocked or in review",
                "astrolabe lesson close",
                "astrolabe initiative new",
                "A planning-only spike closes without --initiative",
                "Progress comes from member statuses",
                "astrolabe activate --initiative <slug>",
                "`astrolabe effort read <n>`",
                "`.handoffs/<initiative slug>/`",
                "Stop-hook",
                "PROOF\n\nReport the effort",
                "FRESH",
            ],
            &[
                "This session is a worker",
                "You're this initiative's orchestrator",
            ],
        );
    }

    #[test]
    fn a_worker_gets_its_own_saving_and_closes_on_its_restrictions() {
        let t = check(
            Role::Worker,
            &[
                "--pieces-file",
                "--plan-file",
                "the dispatcher marks it done",
                "Stop-hook",
                "a worker saves its own effort the same way",
                "No journal line, no new effort",
            ],
            &[
                "--journal",
                "astrolabe initiative new",
                "astrolabe lesson close",
                "astrolabe activate --initiative",
                "astrolabe effort read",
                "Under autonomous, mark done",
            ],
        );
        assert!(t.ends_with(closing(Role::Worker)));
        assert!(closing(Role::Worker).starts_with("This session is a worker under that effort"));
        assert_eq!(closing(Role::Effort), "");
        assert_eq!(closing(Role::Initiative), "");
    }

    #[test]
    fn an_initiative_gets_orchestration_and_its_journal_but_no_pieces_or_plans() {
        check(
            Role::Initiative,
            &[
                "for initiative 7, \"0007 Sample\" (codex session s-1, repo demo)",
                "Page: efforts/2026/11-November/0007-sample.md\nStatus: active\nWaits for: none",
                "an initiative has no review",
                "one line a day for this initiative, on its page",
                "astrolabe lesson close",
                "own separate session, never a subagent",
                "`astrolabe activate --effort <n> --worker` alone first",
                "Give each one a start folder in its effort's repo, never just your own folder",
                "a fresh worktree of that repo where the app makes one, else the repo's checkout with a brief that has it make a worktree before it edits",
                "That repo's note under `knowledge/repos/` says where its checkout and worktrees are on this machine; ask the owner if it doesn't.",
                "names its brief by full path",
                "under `/home/sam/notes/.handoffs/7/` so another session can resume",
                "`astrolabe effort read <n>` before `astrolabe effort save --effort <n> --status done`",
                "Done requires an acceptance statement",
                "PROOF\nReport the initiative",
                "FRESH",
            ],
            &[
                "--pieces-file",
                "--plan-file",
                "astrolabe initiative new",
                "This session is a worker",
            ],
        );
    }

    #[test]
    fn every_role_states_each_setting_with_its_meaning() {
        for role in [Role::Effort, Role::Worker, Role::Initiative] {
            for (i, inv) in INVOLVEMENTS.iter().enumerate() {
                for (j, auth) in AUTHORITIES.iter().enumerate() {
                    let t = instructions(role, &facts(inv, auth));
                    assert!(
                        t.contains(&format!("{inv}: {}", INVOLVEMENT_MEANINGS[i])),
                        "{role:?} {inv}"
                    );
                    assert!(
                        t.contains(&format!("{auth}: {}", AUTHORITY_MEANINGS[j])),
                        "{role:?} {auth}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_initiative_head_fits_a_hook_with_long_settings_and_proof() {
        // `fitted` never cuts the head, so the initiative's must fit on its own.
        let (title, proof, fresh) = ("T".repeat(200), "P".repeat(1500), "F".repeat(600));
        let mut f = facts("check in at each step", "push");
        (f.title, f.proof, f.freshness, f.number) =
            (&title, &proof, &fresh, "a-long-initiative-slug");
        let t = instructions(Role::Initiative, &f);
        assert!(t.len() < HOOK_LIMIT, "{} characters", t.len());
    }

    #[test]
    fn an_issue_line_sits_with_the_effort_settings() {
        let mut f = facts("autonomous", "edit");
        f.issues = "- issue line\n";
        let t = instructions(Role::Effort, &f);
        assert!(t.contains("the same way.\n- issue line\n- A narrower"));
    }

    /// A hook's text as the app that called it counts it (docs/hosts.md), written out here
    /// apart from the code under test: Claude Code in characters, taken as UTF-16 units, and
    /// Codex in approximate tokens, taken as bytes.
    fn counted(provider: &str, text: &str) -> usize {
        match provider {
            "claude" => text.encode_utf16().count(),
            _ => text.len(),
        }
    }

    const APPS: [&str; 2] = ["claude", "codex"];

    /// Every binding a session can start with; `None` is a session with no effort.
    const ROLES: [Option<Role>; 4] = [
        Some(Role::Effort),
        Some(Role::Worker),
        Some(Role::Initiative),
        None,
    ];

    /// What a session can't be safe without, however its text was cut: what it's bound to,
    /// the owner's settings, the guard's scope, how to prove it and not to claim it before,
    /// a worker's restrictions, and for a session with no effort when work gets recorded.
    fn mandatory(role: Option<Role>) -> Vec<&'static str> {
        let guard = "The guard is the plugin's pre-command hook.";
        let unproven = "Until it says proven, claim no protection";
        match role {
            Some(Role::Effort | Role::Worker) => {
                let mut lines = vec![
                    "Astrolabe is on in this session for effort 7,",
                    "- involvement: check in at plan: ",
                    "- authority: edit: ",
                    guard,
                    "PROOF",
                    "Report the effort and what the status says.",
                    unproven,
                ];
                if role == Some(Role::Worker) {
                    lines.push(WORKER_RULE);
                }
                lines
            }
            Some(Role::Initiative) => vec![
                "Astrolabe is on in this session for initiative 7,",
                "Settings set by the owner: involvement: check in at plan: ",
                guard,
                "PROOF",
                "Report the initiative and what the status says.",
                unproven,
            ],
            None => vec![
                "Astrolabe is on for this machine, so the guard runs in this session.",
                "Never load or attach to one unless the owner names it: a related guess is theirs to make. When the owner asks to start, resume or switch to an effort (\"pick up 70\", \"work on effort 70\"), run `astrolabe activate --effort <n>` alone.",
                RECORD_RULE,
                GUARD_SCOPE,
                "Loading or creating an effort proves it for this session; until then, claim no protection.",
            ],
        }
    }

    /// A session's text before its notes, as `switch_record` and `fresh` build it.
    fn session(provider: &str, role: Option<Role>, f: &Facts, parts: Vec<Part>) -> Activation {
        Activation {
            state: SessionState {
                provider: provider.to_owned().into(),
                ..Default::default()
            },
            head: match role {
                Some(role) => lead(role, f),
                None => no_effort(f.freshness),
            },
            closing: role.map_or("", closing),
            parts,
            picked: String::new(),
        }
    }

    fn part(title: &str, body: &str, file: &str) -> Part {
        Part {
            title: title.to_owned(),
            body: body.to_owned(),
            files: vec![file.to_owned()],
        }
    }

    /// The notes an effort's session gets, each `size` long, in a notes folder named `notes`.
    fn notes(notes: &str, size: usize) -> Vec<Part> {
        let body = "word ".repeat(size / 5);
        let file = |name: &str| format!("{notes}/{name}");
        vec![
            part(
                "efforts/0007-sample.md, Current state",
                &body,
                &format!(
                    "{}: only its `## Current state` section, up to the next `## ` heading. Not the rest of that file, which holds history and superseded plans.",
                    file("efforts/0007-sample.md")
                ),
            ),
            part("knowledge/voice.md", &body, &file("knowledge/voice.md")),
            part("knowledge/owner.md", &body, &file("knowledge/owner.md")),
            part(
                "knowledge/repos/demo.md",
                &body,
                &file("knowledge/repos/demo.md"),
            ),
            part(
                "knowledge/machines.md",
                &body,
                &file("knowledge/machines.md"),
            ),
        ]
    }

    /// `fitted` as it was before it checked its own result, kept to show what it returned.
    fn before(a: &Activation) -> String {
        let text = a.text();
        if text.len() <= HOOK_LIMIT {
            return text;
        }
        let (mut inline, mut later) = (a.lead(), Vec::new());
        let note = "\n\nNot inline, as a hook's text is cut near 10,000 characters. Read these now, before anything else:";
        let reserve = note.len() + a.parts.iter().map(|p| p.files.len() * 200).sum::<usize>();
        for part in &a.parts {
            let piece = format!("\n\n--- {}\n\n{}", part.title, part.body);
            if later.is_empty() && inline.len() + piece.len() + reserve <= HOOK_LIMIT {
                inline += &piece;
            } else {
                later.extend(part.files.iter().map(|f| format!("\n- {f}")));
            }
        }
        inline + note + &later.concat()
    }

    fn within(provider: &str, text: &str, what: &str) {
        let size = counted(provider, text);
        assert!(size <= HOOK_LIMIT, "{what}: {size} for {provider}");
    }

    /// Every note reaches the session: inline, or as a file it's told to read. Only a list
    /// of files too long to keep whole, over a third of the limit, loses any, from its end.
    fn carries(provider: &str, a: &Activation, text: &str, what: &str) {
        let entry = |p: &Part| format!("\n- {}", p.files[0]);
        let list: usize = a.parts.iter().map(|p| counted(provider, &entry(p))).sum();
        let cut = text.contains("Astrolabe cut this text here");
        for (n, part) in a.parts.iter().enumerate() {
            if cut && list > HOOK_LIMIT / 3 && n > 0 {
                continue;
            }
            let inline = format!("\n\n--- {}\n\n{}", part.title, part.body);
            assert!(
                text.contains(&entry(part)) || text.contains(&inline),
                "{what}: {} is neither inline nor named",
                part.files[0]
            );
        }
    }

    #[test]
    fn a_cut_text_names_every_note_the_uncut_text_does() {
        // A stamp that lists more unpushed work than a hook's text can carry.
        let stamp = format!(
            "Last saved by workbench at 2026-11-09T14:02-07:00, on main at 00a76cc, {}tree dirty then.",
            "an unpushed commit, ".repeat(600)
        );
        let folder = "/home/sam/workspace/notes";
        for provider in APPS {
            for role in ROLES {
                let mut f = facts("check in at plan", "edit");
                f.freshness = &stamp;
                let mut parts = notes(folder, 400);
                parts.insert(1, part("efforts/0007-sample.md, Pieces", "- [ ] one", &format!("{folder}/efforts/0007-sample.md: only its `## Pieces` section, up to the next `## ` heading.")));
                parts.insert(
                    2,
                    part(
                        "Initiative",
                        "[Audit](initiatives/audit.md)",
                        &format!(
                            "{folder}/initiatives/audit.md: the initiative's title and path only."
                        ),
                    ),
                );
                let a = session(provider, role, &f, parts);
                let t = a.fitted();
                let what = format!("{role:?}, {provider}");
                within(provider, &t, &what);
                assert!(t.contains("Astrolabe cut this text here"), "{what}: {t}");
                // All seven, in the order the uncut text gives them, straight after the cut.
                let list: String = a
                    .parts
                    .iter()
                    .map(|p| format!("\n- {}", p.files[0]))
                    .collect();
                assert!(
                    t.contains(&format!("tell the owner, and read now:{list}\n\n")),
                    "{what}: {t}"
                );
                carries(provider, &a, &t, &what);
                for line in mandatory(role) {
                    assert!(t.contains(line), "{what} lacks {line:?}");
                }
            }
        }
    }

    #[test]
    fn a_head_too_long_for_the_hook_is_cut_and_points_at_the_effort_file() {
        // Warnings from the notes have no bound of their own, and they sit in the head.
        let fresh = "A note write is unfinished. ".repeat(350);
        for provider in APPS {
            for role in ROLES {
                let mut f = facts("check in at plan", "edit");
                f.freshness = &fresh;
                let a = session(provider, role, &f, notes("/home/sam/notes", 400));
                let t = a.fitted();
                within(provider, &t, &format!("{role:?}"));
                carries(provider, &a, &t, &format!("{role:?}"));
                for line in mandatory(role) {
                    assert!(t.contains(line), "{role:?} lacks {line:?}");
                }
                assert!(
                    t.contains("\n\nAstrolabe cut this text here to stay under the app's limit for a hook, so rules or notes are missing: tell the owner, and read now:\n- /home/sam/notes/efforts/0007-sample.md: only its `## Current state` section"),
                    "{role:?}: {t}"
                );
                // A worker's restrictions still close its text.
                assert!(
                    role != Some(Role::Worker) || t.ends_with(WORKER_RULE),
                    "{t}"
                );
            }
        }
    }

    #[test]
    fn a_line_of_any_length_with_nowhere_to_break_still_fits() {
        // No space to cut at, in text that takes three bytes a character: a cut inside a
        // character would panic. As a title it's in the opening line, ahead of everything.
        let long = "題".repeat(20_000);
        for provider in APPS {
            for role in ROLES {
                let mut f = facts("check in at plan", "edit");
                // A session with no effort has no title: its opening names the efforts.
                match role {
                    Some(_) => f.title = &long,
                    None => f.freshness = &long,
                }
                let a = session(provider, role, &f, Vec::new());
                let t = a.fitted();
                within(provider, &t, &format!("{role:?}"));
                carries(provider, &a, &t, &format!("{role:?}"));
                for line in mandatory(role) {
                    assert!(t.contains(line), "{role:?} lacks {line:?}");
                }
                assert!(
                    t.contains("\n\nAstrolabe cut this text here to stay under the app's limit for a hook, so rules or notes are missing: tell the owner.\n\n"),
                    "{role:?}: {t}"
                );
            }
        }
    }

    #[test]
    fn file_paths_longer_than_the_guess_still_fit() {
        // Each path is longer than the 200 the choice of what goes inline allows for.
        let folder = format!("/home/sam/{}", "a-long-folder-name/".repeat(30));
        for provider in APPS {
            for role in ROLES {
                let f = facts("check in at plan", "edit");
                let a = session(provider, role, &f, notes(&folder, 3000));
                let t = a.fitted();
                within(provider, &t, &format!("{role:?}"));
                carries(provider, &a, &t, &format!("{role:?}"));
                for line in mandatory(role) {
                    assert!(t.contains(line), "{role:?} lacks {line:?}");
                }
                assert!(
                    t.contains("Read these now") || t.contains("read now:"),
                    "{role:?}: {t}"
                );
                // The effort's own state is inline or its file is named, whatever else went.
                assert!(
                    t.contains("\n\n--- efforts/0007-sample.md, Current state\n\n")
                        || t.contains(&format!("\n- {folder}/efforts/0007-sample.md: only its")),
                    "{role:?}: {t}"
                );
            }
        }
    }

    #[test]
    fn the_limit_is_counted_as_the_calling_app_counts_it() {
        let owner = || {
            vec![part(
                "knowledge/owner.md",
                "OWNER",
                "/home/sam/notes/knowledge/owner.md",
            )]
        };
        // A head over the limit in bytes and under it in characters: Claude Code gets what
        // it always got, and Codex, limited in bytes, gets it cut.
        let fresh = "語".repeat(2400);
        let mut f = facts("check in at plan", "edit");
        f.freshness = &fresh;
        let claude = session("claude", None, &f, owner());
        let was = before(&claude);
        assert!(was.len() > HOOK_LIMIT && was.encode_utf16().count() < HOOK_LIMIT);
        assert_eq!(claude.fitted(), was);
        let t = session("codex", None, &f, owner()).fitted();
        within("codex", &t, "three-byte text");
        assert!(t.contains("Astrolabe cut this text here"), "{t}");
        // A character outside the first plane counts as two for Claude Code, so a head under
        // the limit in characters is still over it.
        let fresh = "🧭".repeat(4000);
        f.freshness = &fresh;
        let claude = session("claude", None, &f, owner());
        assert!(before(&claude).chars().count() < HOOK_LIMIT);
        let t = claude.fitted();
        within("claude", &t, "two-unit characters");
        assert!(t.contains("Astrolabe cut this text here"), "{t}");
    }

    #[test]
    fn a_cut_never_falls_inside_a_markdown_link() {
        // Spaces inside the link's words and its target, so a cut between words could land
        // in either.
        let link = "[the plan for this effort](efforts/2026/11 November/0007 sample.md)";
        // And brackets inside the words, and a bracket pair inside the target.
        let nested = "[see [the plan] for this effort](efforts/0007 sample.md)";
        let paired = "[the plan for this effort](efforts/0007 sample (the first) again.md)";
        let bytes = |t: &str| t.len();
        for link in [link, nested, paired] {
            let text = format!("before it {link} after it");
            let (from, to) = (text.find('[').unwrap(), text.rfind(')').unwrap() + 1);
            // Every room there is, so the limit falls on each part of the link in turn.
            for room in 0..=text.len() {
                let kept = cut(&text, room, &bytes);
                assert!(kept.len() <= room && text.starts_with(kept));
                match room {
                    // On any part of the link, the cut backs off to before it.
                    r if (from..to).contains(&r) => assert_eq!(kept, "before it", "{room}"),
                    r if r < from => assert!(!kept.contains('['), "{room}: {kept:?}"),
                    _ => assert!(kept.contains(link), "{room}: {kept:?}"),
                }
            }
        }
        // Without a link, the same rooms cut between its words.
        let plain = format!("before it {} after it", link.replace(['[', ']'], "|"));
        assert_eq!(cut(&plain, 20, &bytes), "before it |the plan");
        // And through a session's text: a link in the line that grew, moved a character at
        // a time across the place the limit falls.
        let (mut whole, mut gone) = (0, 0);
        for provider in APPS {
            for lead in 6350..6850 {
                let fresh = format!("{} {link} {}", "x".repeat(lead), "y ".repeat(2500));
                let mut f = facts("check in at plan", "edit");
                f.freshness = &fresh;
                let t = session(provider, None, &f, Vec::new()).fitted();
                within(provider, &t, "a link at the limit");
                let cut = t.split("\n\nAstrolabe cut this text here").next().unwrap();
                match cut.contains(link) {
                    true => whole += 1,
                    false => gone += 1,
                }
                assert!(
                    cut.contains(link) || !cut.contains("[the"),
                    "{lead}: {}",
                    &cut[cut.len() - 120..]
                );
            }
        }
        assert!(whole > 100 && gone > 100, "{whole} whole, {gone} gone");
    }

    #[test]
    fn a_notice_after_the_opening_is_fitted_with_the_rest() {
        let note = "Notes not refreshed from origin (offline): this is the local copy.";
        let f = facts("check in at plan", "edit");
        for provider in APPS {
            for role in ROLES {
                let file = "/home/sam/notes/knowledge/owner.md";
                let owner = |body: &str| vec![part("knowledge/owner.md", body, file)];
                // Exactly at the limit without the note, so adding it afterwards went over.
                let small = session(provider, role, &f, owner(""));
                let room = HOOK_LIMIT - small.text().len();
                let full = session(provider, role, &f, owner(&"x".repeat(room)));
                assert_eq!(full.fitted().len(), HOOK_LIMIT);
                let t = full.fitted_after(Some(note));
                within(provider, &t, &format!("{role:?} with a note"));
                let opening = full.head.split("\n\n").next().unwrap();
                assert!(
                    t.starts_with(&format!("{opening}\n\n{note}\n\n"))
                        && t.ends_with(&format!("Read these now, before anything else:\n- {file}")),
                    "{role:?}: {t}"
                );
            }
        }
    }

    /// A small repeatable source of sizes, so a failure names its case.
    fn sizes(seed: u64) -> impl FnMut(usize) -> usize {
        let mut state = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        move |most: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize % (most + 1)
        }
    }

    #[test]
    fn every_role_fits_with_long_settings_proof_paths_notes_and_any_text() {
        let texts = ["word ", "語の ", "né ", "🧭 "];
        for seed in 0..400 {
            let mut size = sizes(seed);
            let fill = texts[size(3)];
            let title = fill.repeat(size(60));
            let proof = format!("PROOF {}\n", "touch '/tmp/probe' ".repeat(size(120)));
            let fresh = fill.repeat(size(600));
            let issues = format!("- issue: {}\n", "#12, ".repeat(size(40)));
            let folder = format!("/home/{}", "フォルダ-folder/".repeat(size(40)));
            let mut f = facts("check in at plan", "edit");
            (f.title, f.proof, f.freshness, f.issues) = (&title, &proof, &fresh, &issues);
            let mut parts = notes(&folder, size(4000));
            for n in 0..size(12) {
                let body = fill.repeat(size(800));
                parts.push(part(
                    &format!("note {n}"),
                    &body,
                    &format!("{folder}/{n}.md"),
                ));
            }
            for provider in APPS {
                for role in ROLES {
                    let what = format!("seed {seed}, {role:?}, {provider}");
                    let a = session(provider, role, &f, notes(&folder, 0));
                    let mut a = Activation {
                        parts: Vec::new(),
                        ..a
                    };
                    a.parts = parts
                        .iter()
                        .map(|p| part(&p.title, &p.body, &p.files[0]))
                        .collect();
                    let t = a.fitted();
                    within(provider, &t, &what);
                    carries(provider, &a, &t, &what);
                    for line in mandatory(role) {
                        assert!(t.contains(line), "{what} lacks {line:?}");
                    }
                    // Whole, or it says what to read or that it was cut.
                    assert!(
                        t == a.text()
                            || t.contains("Read these now")
                            || t.contains("cut this text here"),
                        "{what}"
                    );
                }
            }
        }
    }

    #[test]
    fn text_that_fitted_before_is_the_same_bytes() {
        let note = "Notes not refreshed from origin (offline): this is the local copy.";
        let (mut whole, mut listed) = (0, 0);
        for seed in 0..400 {
            let mut size = sizes(seed);
            let fill = ["word ", "語の ", "né ", "🧭 "][size(3)];
            let fresh = fill.repeat(size(80));
            let mut f = facts("check in at plan", "edit");
            f.freshness = &fresh;
            let bodies: Vec<String> = (0..5).map(|_| fill.repeat(size(400))).collect();
            for role in ROLES {
                for provider in APPS {
                    let mut a = session(provider, role, &f, notes("/home/sam/notes", 0));
                    for (part, body) in a.parts.iter_mut().zip(&bodies) {
                        part.body = body.clone();
                    }
                    for notice in [None, Some(note)] {
                        // As it was: fitted, then any notice put after the opening.
                        let was = match notice {
                            Some(note) => after_first_paragraph(&before(&a), note),
                            None => before(&a),
                        };
                        if counted(provider, &was) > HOOK_LIMIT {
                            continue;
                        }
                        let what = format!("seed {seed}, {role:?}, {provider}, {notice:?}");
                        assert_eq!(a.fitted_after(notice), was, "{what}");
                        match was.contains("Read these now") {
                            true => listed += 1,
                            false => whole += 1,
                        }
                    }
                }
            }
        }
        assert!(
            whole > 500 && listed > 500,
            "{whole} whole, {listed} listed"
        );
    }

    #[test]
    fn no_line_of_the_rules_is_long_enough_to_pass_for_one_that_grew() {
        for role in ROLES {
            let f = facts("check in at each step", "push");
            let a = session("claude", role, &f, Vec::new());
            for line in a.lead().lines() {
                assert!(line.len() < GROWN, "{role:?}: {} in {line:?}", line.len());
            }
        }
    }

    /// Facts as long as a real session's: a full session id, a dated effort path, proof
    /// commands in a temp folder and a stamp from another machine.
    fn real<'a>(proof: &'a str) -> Facts<'a> {
        Facts {
            number: "197",
            title: "0197 Hold the context cap",
            file: "efforts/2026/11-November/0197-hold-the-context-cap.md",
            provider: "claude",
            session: "0b6f1c7e-58a1-4c9e-9d55-3f2a7e1c4b90",
            repo: "astrolabe",
            involvement: "check in at plan",
            authority: "edit",
            issues: "",
            proof,
            freshness: "Last saved by workbench · codex at 2026-11-09T14:02-07:00, on main at 00a76cc. That was another machine: anything after that save there isn't visible from here. Here now: main at 13ee99e, moved since that save.",
            status: "active",
            waits_for: "none",
            handoffs: "/home/sam/workspace/notes/.handoffs/audit-fixes/",
        }
    }

    /// The sizes the report on this cap quotes; run with `--nocapture` to print them.
    #[test]
    fn every_roles_head_leaves_room_to_name_its_notes() {
        let folder = "/var/folders/9h/45zln1jx7bs6wkl9b2l21m7w0000gn/T/astrolabe-probe-2691581b";
        let token = "ddf10eeca5ad75677778f464f0e891ba";
        let posix = format!(
            "First an allowed control, which must run: touch '{folder}/{token}-control-posix'\nThen the probe, which the hook must deny: touch '{folder}/{token}'; pkill -0 astrolabe-guard-probe\n"
        );
        let one = format!(
            "Prove the guard for this session, each command alone and exactly as written. {posix}Then run `astrolabe activate --status`."
        );
        // Claude Code on Windows is given both shells' steps.
        let both = format!(
            "Prove the guard for this session, each command alone and exactly as written. In the shell you use. For the POSIX shell (Bash, zsh):\n{posix}For PowerShell:\nFirst an allowed control, which must run: New-Item -ItemType File -Force '{folder}/{token}-control-powershell'\nThen the probe, which the hook must deny: New-Item -ItemType File -Force '{folder}/{token}'; Stop-Process -Name astrolabe-guard-probe\nThen run `astrolabe activate --status`."
        );
        let list = "\n\nNot inline, as a hook's text is cut near 10,000 characters. Read these now, before anything else:".len();
        for (shells, proof) in [("one shell", &one), ("two shells", &both)] {
            for role in ROLES {
                let f = real(proof);
                let mut parts = notes("/home/sam/workspace/notes", 0);
                if role.is_some_and(|r| r != Role::Initiative) {
                    parts.insert(1, part("efforts/2026/11-November/0197-hold-the-context-cap.md, Pieces", "", "/home/sam/workspace/notes/efforts/2026/11-November/0197-hold-the-context-cap.md: only its `## Pieces` section, up to the next `## ` heading."));
                    parts.insert(2, part("Initiative", "", "/home/sam/workspace/notes/initiatives/audit-fixes.md: the initiative's title and path only."));
                }
                if role.is_none() {
                    parts.truncate(3);
                }
                let a = session("claude", role, &f, parts);
                let head = counted("claude", &a.lead());
                let files: usize = a.parts.iter().map(|p| p.files[0].len() + 3).sum();
                let guessed = list + 200 * a.parts.len();
                eprintln!(
                    "{role:?}, {shells}: head {head}, left {}, the list of {} files takes {} (guessed {guessed}), left for notes inline {}",
                    HOOK_LIMIT - head,
                    a.parts.len(),
                    list + files,
                    (HOOK_LIMIT - head).saturating_sub(guessed),
                );
                assert!(
                    head + list + files <= HOOK_LIMIT,
                    "{role:?}, {shells}: the head leaves no room to name its notes"
                );
                // The same with the longest notice about where the session is: an owner's
                // effort on Windows, resumed in a shared main checkout, is the tight one.
                let repos = ["astrolabe".to_owned()];
                let told = [Seat::Notes, Seat::Shared]
                    .iter()
                    .filter_map(|seat| placed(role?, seat, &f, &repos))
                    .map(|notice| counted("claude", &notice) + 2)
                    .max()
                    .unwrap_or(0);
                eprintln!(
                    "{role:?}, {shells}: told where it is, head {}, left {} once its {} files are named",
                    head + told,
                    HOOK_LIMIT.saturating_sub(head + told + list + files),
                    a.parts.len(),
                );
                assert!(
                    head + told + list + files <= HOOK_LIMIT,
                    "{role:?}, {shells}: a notice about its folder leaves no room to name its notes"
                );
            }
        }
    }

    #[test]
    fn a_binding_in_the_wrong_folder_is_told_and_any_other_is_told_nothing() {
        let (demo, both, notes) = (
            vec!["demo".to_owned()],
            vec!["demo".to_owned(), "other".to_owned()],
            vec!["demo".to_owned(), AGENT.to_owned()],
        );
        let f = facts("check in at plan", "edit");
        let told = |role, seat, repos: &[String]| placed(role, &seat, &f, repos);
        let said = |role, seat, repos: &[String], opening: &str| {
            let notice = told(role, seat, repos).unwrap_or_default();
            assert!(notice.starts_with(opening), "{role:?} {seat:?}: {notice:?}");
        };
        said(
            Role::Worker,
            Seat::Notes,
            &demo,
            "This session started in the notes checkout, but effort 7's work is in demo.",
        );
        said(
            Role::Initiative,
            Seat::Notes,
            &demo,
            "This initiative's work is all in demo, and this session is in the notes checkout.",
        );
        for role in [Role::Effort, Role::Worker] {
            said(
                role,
                Seat::Shared,
                &demo,
                "This folder is demo's main checkout, which other sessions share,",
            );
        }
        // The notes are the owner's seat for any effort, an orchestrator's across repos, and a
        // worker's when its effort is the notes' own. An orchestrator edits nothing.
        for (role, seat, repos) in [
            (Role::Effort, Seat::Notes, &demo),
            (Role::Worker, Seat::Notes, &notes),
            (Role::Initiative, Seat::Notes, &both),
            (Role::Initiative, Seat::Notes, &notes),
            (Role::Initiative, Seat::Shared, &demo),
            (Role::Effort, Seat::Apart, &demo),
            (Role::Worker, Seat::Apart, &demo),
            (Role::Initiative, Seat::Apart, &demo),
        ] {
            assert_eq!(told(role, seat, repos), None, "{role:?} {seat:?} {repos:?}");
        }
        // Read authority changes no file, so a shared checkout costs it nothing.
        let read = facts("check in at plan", "read");
        for role in [Role::Effort, Role::Worker] {
            assert_eq!(placed(role, &Seat::Shared, &read, &demo), None, "{role:?}");
        }
        let push = facts("autonomous", "push");
        assert!(placed(Role::Effort, &Seat::Shared, &push, &demo).is_some());
    }

    /// An effort's and a worker's text as main had it before any binding was told where it
    /// is (fbc24b4), by length and SHA-256: a session no notice fires for gets these bytes.
    #[test]
    fn an_effort_and_a_worker_with_no_notice_get_the_text_they_always_got() {
        use sha2::{Digest, Sha256};
        for (role, authority, length, hash) in [
            (
                Role::Effort,
                "read",
                7362,
                "a0c3b440e1d64b04b735089cf20e22a03375baa1fbf34ca514a91baaf8a0965f",
            ),
            (
                Role::Effort,
                "edit",
                7365,
                "7bef85b9d2d8377bb652ad920d1401cdf39e3671d64bca4cf80b789d2762dcb9",
            ),
            (
                Role::Effort,
                "push",
                7382,
                "6c6aa3f4e495c9670e9ed93e1b0c8f792a24f3749a7295dbee23d48cd6962cbd",
            ),
            (
                Role::Worker,
                "read",
                5761,
                "00a5501b5e0315bff1e7630339b7f5eb380364a9eccecfc97f2ae9891e99fc2d",
            ),
            (
                Role::Worker,
                "edit",
                5764,
                "fb8d8942cb3e4dbb1d0c5f8f44ed5dddd812e9739b40039ec6ad5ce912d5f540",
            ),
            (
                Role::Worker,
                "push",
                5781,
                "96977b4c93e643e59b421ceb780f3b347177670b0dde427e1c55e5f67cb14259",
            ),
        ] {
            let t = instructions(role, &facts("check in at plan", authority));
            let now = format!("{:x}", Sha256::digest(t.as_bytes()));
            assert_eq!(
                (t.len(), now.as_str()),
                (length, hash),
                "{role:?} {authority}"
            );
        }
    }

    #[test]
    fn a_head_cut_for_the_limit_keeps_what_it_was_told_about_its_folder() {
        // No one line grew, so the room comes from the end of the rules.
        let fresh = "A note write is unfinished. ".repeat(50);
        let folder = format!("/home/{}notes", "a-long-folder/".repeat(12));
        let repos = ["demo".to_owned()];
        for provider in APPS {
            for (role, seat) in [
                (Role::Effort, Seat::Shared),
                (Role::Worker, Seat::Shared),
                (Role::Worker, Seat::Notes),
            ] {
                let mut f = facts("check in at plan", "edit");
                f.freshness = &fresh;
                let notice = placed(role, &seat, &f, &repos).unwrap();
                let mut a = session(provider, Some(role), &f, notes(&folder, 400));
                for n in 0..30 {
                    let file = format!("{folder}/knowledge/{n}.md");
                    a.parts.push(part(&format!("note {n}"), "", &file));
                }
                a.head = after_first_paragraph(&a.head, &notice);
                let t = a.fitted();
                within(provider, &t, &format!("{role:?}"));
                assert!(t.contains("cut this text here"), "{role:?}: {t}");
                let opening = a.head.split("\n\n").next().unwrap();
                assert!(
                    t.starts_with(&format!("{opening}\n\n{notice}\n\nSettings from")),
                    "{role:?}: {t}"
                );
                for line in mandatory(Some(role)) {
                    assert!(t.contains(line), "{role:?} lacks {line:?}");
                }
            }
        }
    }
}
