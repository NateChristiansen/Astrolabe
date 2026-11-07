//! `astrolabe effort new | save` and `astrolabe journal`, done inside the hook: the agent's
//! words become an effort note, at most one journal line and a commit; the session is bound to the effort; code stamps what it
//! can observe (time, machine, provider, session, branch, head, unpushed, dirty), without asking the agent to reconstruct them.
use crate::activate::{self, Repos, read_state, state_dir, state_path};
use crate::effort::{EffortCommand, EffortFile, FORMS, slug};
use crate::notes::{self, JournalLine, Notes, Op, Pull, RECENT_MS, RepoFacts};
use crate::session::{Field, SessionState};
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
mod validation;
use validation::{
    AUTHORITIES, INVOLVEMENTS, STATUSES, as_current, check_journal, one_of, shift_plan_headings,
    split_plan,
};

/// What the hook does for one effort command: what to show, whether it worked, and the
/// state written when it did.
pub struct Done {
    pub text: String,
    pub ok: bool,
}

/// The session's notes checkout and state folder, or why not.
pub(crate) fn notes_here() -> Result<(Repos, Notes), String> {
    let dir = state_dir().ok_or("Astrolabe has no home folder for its state")?;
    let (registry, root) = activate::notes_and_repos()?;
    Ok((registry, Notes::new(root, dir)))
}

/// The one repo note this folder belongs to, for a new effort.
fn repo_of(
    notes: &Notes,
    registry: &Repos,
    cwd: &Path,
    wanted: Option<&str>,
) -> Result<String, String> {
    let here = activate::common_dir(cwd).ok_or_else(|| {
        format!("this session's folder {} isn't in a git repo, so the effort can't say which repo to work out of", cwd.display())
    })?;
    // The notes checkout is the owner's seat, about every repo: the lead says which one the
    // work is for, since the folder can't.
    if activate::in_notes(cwd) {
        let Some(name) = wanted else {
            return Err("this session runs in the notes checkout, the owner's seat, so the effort can't take its repo from the folder: --repo <name> names the repo the work is for, as its note in knowledge/repos names it".to_owned());
        };
        activate::check_repo_name(name)?;
        if registry.checkout(name).is_none()
            && activate::note_remotes(&notes.root, name)?.is_empty()
        {
            return Err(format!(
                "--repo {name:?} is neither a repo note with a remote (knowledge/repos/{name}.md) nor registered on this machine; `astrolabe repo add {name}`, run from a folder of that repo, adds it"
            ));
        }
        return Ok(name.to_owned());
    }
    if let Some(name) = wanted {
        return Err(format!(
            "--repo is for the notes checkout, where the folder says nothing; here the repo is this folder's, so drop --repo {name:?}"
        ));
    }
    let mut names = activate::repo_names(&notes.root, registry, cwd, &here)?;
    names.retain(|n| n != crate::registry::AGENT);
    match names.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(activate::no_note(cwd)),
        many => Err(format!(
            "this folder matches more than one repo note ({}); the owner settles which",
            many.join(", ")
        )),
    }
}

/// The save's stamp. The repo facts are the session folder's; from the notes checkout they
/// would be the notes' own, which say nothing about the work, so there they're left blank.
fn stamp(file: &mut EffortFile, session: &str, provider: &str, facts: Option<&RepoFacts>) {
    file.set("updated", &notes::local_stamp());
    file.set(
        "saved_by",
        &format!("{} · {provider}", notes::machine_name()),
    );
    file.set("session", session);
    match facts {
        Some(f) => {
            file.set("branch", &f.branch);
            file.set("head", &f.head);
            file.set(
                "unpushed",
                &f.unpushed.map_or("unknown".to_owned(), |n| n.to_string()),
            );
            file.set("dirty", if f.dirty { "true" } else { "false" });
        }
        None => {
            for key in ["branch", "head", "unpushed", "dirty"] {
                file.set(key, "");
            }
        }
    }
}

/// From the notes checkout the save's own commit changes what the fingerprint sees (a journal
/// edited by hand is committed with it), so the op takes the nudge's baseline again after it.
fn rebaseline(cwd: &Path) -> Option<PathBuf> {
    activate::in_notes(cwd).then(|| cwd.to_path_buf())
}

/// The facts for a stamp: none from the notes checkout.
fn stamp_facts<'a>(cwd: &Path, facts: Option<&'a RepoFacts>) -> Option<&'a RepoFacts> {
    (!activate::in_notes(cwd)).then_some(facts).flatten()
}

fn line_id(key: &str, n: usize) -> String {
    format!("{}{n}", &key[..key.len().min(10)])
}

/// `astrolabe effort new`, in the hook.
pub fn new(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    shells: &[bool],
) -> Done {
    match new_inner(cmd, key, session, provider, cwd, shells) {
        Ok(text) => Done { text, ok: true },
        Err(why) => Done {
            text: format!("Effort NOT created: {why}. {}", activate::STILL_ON),
            ok: false,
        },
    }
}

fn new_inner(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    shells: &[bool],
) -> Result<String, String> {
    let title = cmd
        .flag("title")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or("--title is needed")?;
    let goal = cmd
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| format!("the goal goes in the body, one line is enough.\n{FORMS}"))?;
    // Refused before the lock, reconcile and pull: a refused goal changes nothing.
    let current = as_current(goal)?;
    let status = one_of("status", cmd.flag("status").unwrap_or("active"), &STATUSES)?;
    if status == "blocked" || status == "done" || status == "abandoned" {
        return Err(format!("a new effort can't start {status}"));
    }
    // Defaults are owner.md's default dispatch: a branch and local commits, a plan shown first
    // when the work needs one. Never push unless the owner said so.
    let involvement = one_of(
        "involvement",
        cmd.flag("involvement").unwrap_or("check in at plan"),
        &INVOLVEMENTS,
    )?;
    let authority = one_of(
        "authority",
        cmd.flag("authority").unwrap_or("edit"),
        &AUTHORITIES,
    )?;
    let (registry, notes) = notes_here()?;
    let repo = repo_of(&notes, &registry, cwd, cmd.flag("repo"))?;
    let dir = notes.state.clone();
    let _held = notes.lock()?;
    // Leftovers first: one of them may be this very call, interrupted.
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    if notes.already_applied(key) {
        // Done before, by this call or its interrupted first run: the result it had, or the
        // effort it bound.
        if let Some(result) = notes
            .intents()
            .into_iter()
            .find(|op| op.key == key)
            .and_then(|op| op.result)
        {
            return Ok(result);
        }
        // The effort that commit created, not whatever this session is bound to now.
        let created = notes
            .applied_commit(key)
            .and_then(|sha| notes.effort_in_commit(&sha));
        return match created {
            Some(n) => {
                let n = n.to_string();
                let previous = read_state(&dir, session).ok().flatten();
                let activation = activate::switch_on(&dir, &n, session, provider, cwd, previous.as_ref(), shells)?;
                notes::write_text(&state_path(&dir, session), &activation.state.to_string())?;
                Ok(format!("Effort {n} was already created by this same call (a retry); nothing new was written, and this session is bound to it.\n\n{}", activation.text()))
            }
            None => Err("this same tool call was already done (a retry); nothing new was written. Run `astrolabe activate --status` to see the session's effort".to_owned()),
        };
    }
    let pulled = crate::application::pull(&notes);
    let number = notes.next_number();
    let (day, _) = notes::local_day_time();
    let file_name = format!("{}/{number:04}-{}.md", notes::effort_dir(&day), slug(title));
    let facts = RepoFacts::read(cwd);
    let mut file = EffortFile {
        title: format!("{number} {title}"),
        current,
        ..Default::default()
    };
    file.title = format!("{number:04} {title}");
    for (k, v) in [
        ("status", status.as_str()),
        ("repo", repo.as_str()),
        ("initiative", cmd.flag("initiative").unwrap_or("")),
        ("priority", cmd.flag("priority").unwrap_or("none")),
        ("involvement", involvement.as_str()),
        ("authority", authority.as_str()),
        ("waits_for", ""),
        ("created", day.as_str()),
    ] {
        file.set(k, v);
    }
    stamp(
        &mut file,
        session,
        provider,
        stamp_facts(cwd, facts.as_ref()),
    );
    let text = file.render();
    // The session's one probe token, so the proof from session start carries in.
    let token = activate::session_token(session);
    let state = SessionState {
        session: session.to_owned().into(),
        provider: provider.to_owned().into(),
        effort: number.to_string().into(),
        file: file_name.clone().into(),
        repo: repo.clone().into(),
        cwd: crate::hook::comparable(cwd).into(),
        at_ms: notes::now_ms().into(),
        token: token.into(),
        delivered: notes::content_hash(&text).into(),
        baseline: facts
            .as_ref()
            .map(|f| Field::Known(f.fingerprint.clone()))
            .unwrap_or(Field::Other(Value::Null)),
        last_save_ms: notes::now_ms().into(),
        mark: false.into(),
        saves: 0.into(),
        ..Default::default()
    };
    let op = Op {
        key: key.to_owned(),
        files: vec![(file_name.clone(), text)],
        renames: Vec::new(),
        // No journal line: starting is state, not history. The agent writes one with
        // `--journal` when something worth telling has happened.
        journal: Vec::new(),
        message: format!("{number:04}: started"),
        base: None,
        states: vec![(state_path(&dir, session), state.to_value())],
        committed: false,
        digest: Some(cmd.digest()),
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        rebaseline: rebaseline(cwd),
    };
    notes.apply(&op)?;
    notes.spawn_sync();
    let activation = activate::switch_on(
        &dir,
        &number.to_string(),
        session,
        provider,
        cwd,
        read_state(&dir, session).ok().flatten().as_ref(),
        shells,
    )?
    .mid_session();
    let mut head = format!(
        "Effort {number} created as {file_name}, status {status}, bound to this session and committed. Say your reading in one line (involvement {involvement} · authority {authority} · one piece or multi-step) and proceed.",
    );
    if let Pull::Diverged(why) = &pulled {
        head += &format!(
            " Notes diverged from origin before this ({}); the number may collide with the other machine's and be renumbered later.",
            why.lines().next().unwrap_or_default()
        );
    }
    let text = format!("{head}\n\n{}", activation.text());
    crate::application::finish(&notes, key, &text);
    Ok(text)
}

/// `astrolabe effort save`, in the hook.
pub fn save(cmd: &EffortCommand, key: &str, session: &str, provider: &str, cwd: &Path) -> Done {
    match save_inner(cmd, key, session, provider, cwd) {
        Ok(text) => Done { text, ok: true },
        Err(why) => Done {
            text: format!("Effort NOT saved: {why}"),
            ok: false,
        },
    }
}

fn save_inner(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
) -> Result<String, String> {
    let (_registry, notes) = notes_here()?;
    let dir = notes.state.clone();
    let mut state = read_state(&dir, session)?.ok_or(
        "no effort is bound to this session. Start one with `astrolabe effort new`, or load the one the owner named with `astrolabe activate --effort <n>`",
    )?;
    let number = state.effort.as_deref().unwrap_or_default().to_owned();
    let rel = state.file.as_deref().unwrap_or_default().to_owned();
    let status = cmd
        .flag("status")
        .map(|s| one_of("status", s, &STATUSES))
        .transpose()?;
    let decision = cmd
        .flag("decision")
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let waits_for = cmd
        .flag("waits-for")
        .map(str::trim)
        .filter(|w| !w.is_empty());
    let reason = cmd.flag("reason").map(str::trim).filter(|r| !r.is_empty());
    let plan_file = cmd.flag("plan-file");
    let journal_text = cmd.flag("journal").map(str::trim).filter(|j| !j.is_empty());
    let ticks = cmd.flags_all("tick");
    let unticks = cmd.flags_all("untick");
    let pieces_file = cmd.flag("pieces-file");
    // Conflicts are checked on the pieces they name, not the words: "Tether" and
    // "Tether cap" are one item.
    if let Some(j) = journal_text {
        check_journal(j)?;
    }
    // The reason is stored with the status it explains, so one given alone would be dropped.
    if reason.is_some() && status.as_deref() != Some("abandoned") {
        return Err("--reason goes with --status abandoned, where it's recorded under Decisions; for any other choice use --decision \"chose X over Y: reason\". Nothing was written".to_owned());
    }
    let (body, body_plan, body_marked) = split_plan(cmd.body.as_deref());
    if plan_file.is_some() && body_marked {
        return Err(
            "the plan comes one way: --plan-file, or a `## Plan` section in the body, not both"
                .to_owned(),
        );
    }
    if body.is_none()
        && body_plan.is_none()
        && status.is_none()
        && decision.is_none()
        && waits_for.is_none()
        && plan_file.is_none()
        && journal_text.is_none()
        && ticks.is_empty()
        && unticks.is_empty()
        && pieces_file.is_none()
    {
        return Err(format!(
            "nothing to save: give a Current state body, or a flag.\n{FORMS}"
        ));
    }
    if status.as_deref() == Some("blocked") && waits_for.is_none() {
        return Err("blocked needs --waits-for \"<what it waits on>\": the home view shows what could be picked up next by that condition, and v1 had blocked work nobody could tell how to unblock".to_owned());
    }
    if status.as_deref() == Some("abandoned") && reason.is_none() {
        return Err("abandoned needs --reason \"<why>\"".to_owned());
    }
    let read_file = |flag: &str, p: &str| {
        let path = if Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            cwd.join(p)
        };
        std::fs::read_to_string(&path).map_err(|e| format!("--{flag} {}: {e}", path.display()))
    };
    let plan = plan_file
        .map(|p| read_file("plan-file", p))
        .transpose()?
        .or(body_plan);
    let pieces = pieces_file
        .map(|p| read_file("pieces-file", p))
        .transpose()?;
    // The file's sections are read back by their `## ` headings, so a heading inside one
    // would split it on the next read; checked after trimming, as it's stored. A plan is
    // the app's own file (Claude's plans use `## `), so its headings are demoted a level
    // rather than refused; the short agent-written sections are refused.
    let is_heading = |l: &str| l.trim_start().starts_with("## ");
    for (what, text) in [
        ("Current state", body.as_deref()),
        ("Pieces", pieces.as_deref()),
    ] {
        if let Some(line) = text.and_then(|t| t.lines().find(|l| is_heading(l))) {
            return Err(format!(
                "{what} can't hold a `## ` heading ({}), because the effort file's sections are read by those; use ### or deeper. Nothing was written",
                line.trim()
            ));
        }
    }
    // As stored: trimmed whole, so a first-line indent is gone; then headings are shifted
    // with their endings and the rest untouched.
    let (plan, plan_shifted) = match plan.map(|t| shift_plan_headings(t.trim())) {
        Some((p, shifted)) => (Some(p), shifted),
        None => (None, false),
    };
    // Checked before the lock and the hash: a refused body writes nothing either way.
    let current = body.as_deref().map(as_current).transpose()?;
    let _held = notes.lock()?;
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    if notes.already_applied(key) {
        return Ok(format!(
            "Already saved: this same tool call was done before (a retry), so nothing new was written to effort {number}."
        ));
    }
    let pulled = crate::application::pull(&notes);
    let path = notes.root.join(&rel);
    let on_disk = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
    let on_disk_hash = notes::content_hash(&on_disk);
    let mut file = EffortFile::parse(&on_disk)?;
    let Some(delivered) = state.delivered.as_deref().map(str::to_owned) else {
        return Err("this session's state has no record of the effort as delivered to it, so a save can't tell whether it would overwrite another's. Load the effort again with `astrolabe activate --effort <n>`".to_owned());
    };
    if delivered != on_disk_hash {
        // Delivered anew, so the next save from this text goes through.
        state.delivered.set(on_disk_hash.clone());
        let _ = notes::write_text(&state_path(&dir, session), &state.to_string());
        let pieces = file
            .pieces()
            .map(|p| format!("\n\nIts Pieces now:\n\n{p}"))
            .unwrap_or_default();
        return Err(format!(
            "effort {number} changed since it was delivered to this session (saved by {} at {}). Nothing was overwritten. Its Current state now:\n\n{}{pieces}\n\nSave again from this.",
            file.prop("saved_by").unwrap_or("?"),
            file.prop("updated").unwrap_or("?"),
            file.current
        ));
    }
    let facts = RepoFacts::read(cwd);
    let (day, _) = notes::local_day_time();
    let mut notes_lines = Vec::new();
    if plan_shifted {
        notes_lines.push(
            "The plan's headings were shifted so its top level is `###`, because the effort file's sections are read by `## `."
                .to_owned(),
        );
    }
    if let Some(current) = current {
        file.current = current;
    }
    if let Some(plan) = plan {
        file.plan = Some(plan.trim().to_owned());
    }
    if let Some(d) = decision {
        file.add_decision(&format!("{day}: {d}"));
    }
    // Why the work stopped goes with the record of choices, dated, so it outlives later saves
    // and a revival; Current state is replaced on every save.
    if let Some(r) = reason {
        file.add_decision(&format!("{day}: abandoned: {r}"));
    }
    if let Some(p) = &pieces {
        file.set_pieces(p);
    }
    if !ticks.is_empty() || !unticks.is_empty() {
        let on: Vec<usize> = ticks
            .iter()
            .map(|l| file.piece_at(l))
            .collect::<Result<_, _>>()?;
        let off: Vec<usize> = unticks
            .iter()
            .map(|l| file.piece_at(l))
            .collect::<Result<_, _>>()?;
        if let Some(i) = on.iter().position(|i| off.contains(i)) {
            return Err(format!("{:?} is both ticked and unticked", ticks[i]));
        }
        for label in &ticks {
            file.tick(label, true)?;
        }
        for label in &unticks {
            file.tick(label, false)?;
        }
    }
    if let Some(w) = waits_for {
        file.set("waits_for", w);
    }
    let before = file.prop("status").unwrap_or("").to_owned();
    if let Some(s) = &status {
        file.set("status", s);
        if s != "blocked" && waits_for.is_none() {
            file.set("waits_for", "");
        }
    }
    // The journal is the owner's history and only the agent's sentence goes in it; one per
    // effort per day, the later replacing the earlier.
    let journal = journal_text
        .map(|j| JournalLine {
            id: line_id(key, 0),
            effort: format!("{:04}", number.parse::<u32>().unwrap_or(0)),
            text: j.to_owned(),
            machine: notes::machine_name(),
            at_ms: notes::now_ms(),
            day: day.clone(),
            replace: true,
        })
        .into_iter()
        .collect();
    stamp(
        &mut file,
        session,
        provider,
        stamp_facts(cwd, facts.as_ref()),
    );
    let text = file.render();
    let now = notes::now_ms();
    let mut next_state = state.clone();
    next_state.saved(
        notes::content_hash(&text),
        facts.as_ref().map(|f| f.fingerprint.clone()),
        now,
    );
    let message = match &status {
        Some(s) if *s != before => format!("{:04}: {s}", number.parse::<u32>().unwrap_or(0)),
        _ => format!("{:04}: checkpoint", number.parse::<u32>().unwrap_or(0)),
    };
    let op = Op {
        key: key.to_owned(),
        files: vec![(rel.clone(), text)],
        renames: Vec::new(),
        journal,
        message,
        base: Some(on_disk_hash),
        states: vec![(state_path(&dir, session), next_state.to_value())],
        committed: false,
        digest: Some(cmd.digest()),
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        rebaseline: rebaseline(cwd),
    };
    notes.apply(&op)?;
    // Every save, so the other machine's board follows it: a checkpoint held for a batching
    // window sat unpushed for as long as its session stayed quiet.
    notes.spawn_sync();
    let mut head = format!(
        "Saved effort {number}: status {}{}{}; committed, push started.",
        file.prop("status").unwrap_or("?"),
        if cmd.body.is_some() {
            ", Current state replaced"
        } else {
            ""
        },
        if journal_text.is_some() {
            ", journal line written"
        } else {
            ""
        },
    );
    if matches!(pulled, Pull::Diverged(_)) {
        head += " Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    for l in notes_lines {
        head += &format!(" {l}");
    }
    crate::application::finish(&notes, key, &head);
    Ok(head)
}

/// The ticket is written; the intent that held the result can go.
pub fn forget(key: &str) {
    if let Ok((_, notes)) = notes_here() {
        notes.forget(key);
    }
}

/// `astrolabe journal "…"`, in the hook: one line in today's day file for a finding with no
/// effort, tagged with the repo. A session with an effort passes `--journal` on its save
/// instead, so the line is the effort's.
pub fn journal(cmd: &EffortCommand, key: &str, session: &str, cwd: &Path) -> Done {
    match journal_inner(cmd, key, session, cwd) {
        Ok(text) => Done { text, ok: true },
        Err(why) => Done {
            text: format!("Journal line NOT written: {why}"),
            ok: false,
        },
    }
}

fn journal_inner(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    cwd: &Path,
) -> Result<String, String> {
    let sentence = cmd.body.as_deref().map(str::trim).unwrap_or_default();
    check_journal(sentence)?;
    let (registry, notes) = notes_here()?;
    let dir = notes.state.clone();
    let bound = read_state(&dir, session)?;
    if let Some(n) = bound.as_ref().and_then(|s| s.effort.as_deref()) {
        return Err(format!(
            "this session has effort {n}; its journal line goes on the effort: `astrolabe effort save --journal \"…\"`"
        ));
    }
    let repo = repo_of(&notes, &registry, cwd, cmd.flag("repo"))?;
    let _held = notes.lock()?;
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    if notes.already_applied(key) {
        return Ok("Already written: this same tool call was done before (a retry), so nothing new went in the journal.".to_owned());
    }
    let pulled = crate::application::pull(&notes);
    let (day, _) = notes::local_day_time();
    let op = Op {
        key: key.to_owned(),
        files: Vec::new(),
        renames: Vec::new(),
        journal: vec![JournalLine {
            id: line_id(key, 0),
            effort: repo.clone(),
            text: sentence.to_owned(),
            machine: notes::machine_name(),
            at_ms: notes::now_ms(),
            day: day.clone(),
            replace: false,
        }],
        message: format!("journal: {repo}"),
        base: None,
        states: Vec::new(),
        committed: false,
        digest: Some(cmd.digest()),
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        rebaseline: None,
    };
    notes.apply(&op)?;
    notes.spawn_sync();
    let mut head = format!("Journal line written for {day} and committed.");
    if matches!(pulled, Pull::Diverged(_)) {
        head += " Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    crate::application::finish(&notes, key, &head);
    Ok(head)
}

/// The plain `astrolabe effort …` or `astrolabe journal …` command, when no hook rewrote it: it explains what happened
/// instead of doing anything, because only the hook knows the session. A recent ticket for the
/// same command means it was done; a recent intent means it's still finishing.
pub fn plain(first: &str, args: &[&str], body: Option<&str>) -> (String, bool) {
    let mut words = vec!["astrolabe".to_owned(), first.to_owned()];
    // PowerShell hands the here-string over as one argument: the one that isn't a flag or a
    // flag's value, or any with line breaks.
    let mut body = body.map(str::to_owned);
    let mut wants_value = false;
    for (i, a) in args.iter().enumerate() {
        let is_flag = a.starts_with("--");
        let positional = (i > 0 || first == "journal") && !is_flag && !wants_value;
        if a.contains('\n') || positional {
            body = Some((*a).to_owned());
        } else if a.contains(' ') || a.is_empty() {
            words.push(format!("\"{a}\""));
        } else {
            words.push((*a).to_owned());
        }
        wants_value = is_flag;
    }
    let mut text = words.join(" ");
    if let Some(body) = body.as_deref().filter(|b| !b.trim().is_empty()) {
        text += &format!(" <<'ASTROLABE'\n{}\nASTROLABE", body.trim());
    }
    let parsed = match EffortCommand::parse(&text) {
        Ok(p) => p,
        Err(why) => return (format!("Effort command refused: {why}.\n{FORMS}"), false),
    };
    let digest = parsed.digest();
    let Some(dir) = state_dir() else {
        return (
            "Astrolabe has no home folder for its state.".to_owned(),
            false,
        );
    };
    let here = std::env::current_dir()
        .map(|c| crate::hook::comparable(&c))
        .unwrap_or_default();
    if let Some((text, ok, age_ms)) = activate::shown_for(&dir, &digest, &here, RECENT_MS) {
        return (
            format!(
                "{text}\n\n(This is the result of a command with this exact text, run in this folder {} s ago through the hook. If that was another session, this one's command wasn't taken by the hook and recorded nothing.)",
                age_ms / 1000
            ),
            ok,
        );
    }
    let notes = Notes::new(std::path::PathBuf::new(), dir);
    if let Some(op) = notes.intent_for(&digest, &here) {
        return match op.result {
            Some(result) => (
                format!("{result}\n\n(Recovered from the hook's record of this command: its reply was lost on the way.)"),
                true,
            ),
            None => ("This write may still be finishing in the hook; run `astrolabe notes status` in a moment to see.".to_owned(), false),
        };
    }
    (
        format!(
            "NOT recorded: the plugin's hook didn't take this command, so nothing was written. Run it alone, exactly as written. If it fails again while `astrolabe activate --status` says this session is on for an effort, the hook most likely ran out of the time the app gives it: stop retrying and tell the owner. Otherwise Astrolabe may be off for this machine (the owner runs `astrolabe on` in a terminal); on Codex, review and trust the plugin's hooks with /hooks; on Claude, check the plugin is enabled.\n{FORMS}"
        ),
        false,
    )
}

/// `astrolabe efforts [--repo <name>] [--all] --json`: what the home view and a manager read,
/// without opening every file by hand.
pub fn list(repo: Option<&str>, all: bool) -> Result<String, String> {
    let (_registry, root) = activate::notes_and_repos()?;
    let notes = Notes::new(root, state_dir().unwrap_or_default());
    serde_json::to_string(&rows_in(&notes, repo, all)).map_err(|e| e.to_string())
}

pub use crate::board::{lessons_in, rows_in};

/// Before a pickup: the notes brought up to date, so the effort read is the published one.
pub fn refresh_notes() -> Option<String> {
    let (_registry, notes) = notes_here().ok()?;
    let (leftovers, pulled) = crate::application::refresh(&notes);
    let mut lines = Vec::new();
    match pulled {
        Pull::Pulled => {}
        Pull::Offline(why) => lines.push(format!("Notes not refreshed from origin ({}): this is the local copy.", why.lines().next().unwrap_or_default())),
        Pull::Diverged(why) => lines.push(format!("Notes diverged from origin ({}): this is the local copy; resolve in the notes checkout.", why.lines().next().unwrap_or_default())),
    }
    if !leftovers.is_empty() {
        lines.push(format!(
            "Unfinished notes writes failed again: {}.",
            leftovers.join("; ")
        ));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}
