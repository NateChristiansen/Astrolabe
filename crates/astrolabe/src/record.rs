//! `astrolabe effort new | save` and `astrolabe journal`, done inside the hook: the agent's
//! words become an effort note, at most one journal line and a commit; the session is bound to the effort; code stamps what it
//! can observe (time, machine, provider, session, branch, head, unpushed, dirty), without asking the agent to reconstruct them.
use crate::activate::{self, Repos, read_state, state_dir, state_path};
use crate::effort::{EffortCommand, EffortFile, FORMS, InitiativePage, slug};
use crate::notes::{self, JournalLine, Notes, Op, Pull, RECENT_MS, RepoFacts};
use crate::session::{Binding, Field, SessionState};
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
pub mod lesson;
mod save;
mod validation;
use save::{
    Reply, SaveRequest, Saver, commit_message, edit_bound, saved_reply, stamp_initiative,
    stamp_saver,
};
use validation::{
    AUTHORITIES, INVOLVEMENTS, STATUSES, as_current, check_journal, closes, issues, one_of,
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

/// Explicit membership must name a page; omitted flags never enter this resolver.
fn canonical_membership(root: &Path, name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Ok(String::new());
    }
    let pages = crate::effort::initiative_catalog(root)?;
    crate::effort::resolve_initiative(&pages, name).map(|page| page.slug.clone())
}

/// The key a command is written under: its own, or its first attempt's when it's a retry.
enum Attempt {
    /// A retry of a command whose first attempt left its reply: that reply.
    Answered(String),
    Key(String),
}

/// A retry is a new tool call, so it comes with a new key. It's the same request as a write
/// this session attempted with the same command, in the same folder and on the same record,
/// that its session is known never to have been told of (see `Notes::earlier_attempt`). If
/// that attempt got as far as its reply, the retry gets that reply. If a later run finished
/// the write, no reply was ever made: the retry takes the first attempt's key and is told
/// what a repeat of that call is told.
fn attempt(notes: &Notes, cmd: &EffortCommand, key: &str, session: &str, cwd: &Path) -> Attempt {
    // The same tool call again is known by its key, as it always was.
    if notes.already_applied(key) {
        return Attempt::Key(key.to_owned());
    }
    let bound = bound_label(&notes.state, session);
    let first = notes.earlier_attempt(
        &cmd.digest(),
        &crate::hook::comparable(cwd),
        session,
        bound.as_deref(),
    );
    let Some(first) = first else {
        return Attempt::Key(key.to_owned());
    };
    // The retry's answer, if it works and goes out, is the first attempt's too.
    told().earlier = Some(first.key.clone());
    match first.result {
        Some(reply) if reply != notes::RECOVERED => Attempt::Answered(reply),
        _ => Attempt::Key(first.key),
    }
}

/// The writes this hook's answer tells the session about once it goes out, and before that
/// the earlier attempt a retry in this hook was matched to.
#[derive(Default)]
struct Told {
    writes: Vec<String>,
    earlier: Option<String>,
}

fn told() -> std::sync::MutexGuard<'static, Told> {
    static TOLD: std::sync::Mutex<Told> = std::sync::Mutex::new(Told {
        writes: Vec::new(),
        earlier: None,
    });
    TOLD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// This hook's command worked and its result is left for the session: its answer, once it
/// goes out, tells the session about the write with this key, and about the earlier
/// attempt it was a retry of. A command that failed tells the session of neither.
pub(crate) fn answers(key: &str) {
    let mut told = told();
    told.writes.push(key.to_owned());
    if let Some(earlier) = told.earlier.take() {
        told.writes.push(earlier);
    }
}

/// The hook's own answer is going out, so the session is told: each write it's told about
/// is marked answered, and the same words after this are a new request. Done as the answer
/// is printed and without the notes lock, which a sync the write started may hold by now.
/// Not called when the backstop answered first; those intents stay for the session's retry.
pub(crate) fn mark_answered() {
    if let Some(dir) = state_dir() {
        let notes = Notes::new(PathBuf::new(), dir);
        for key in &told().writes {
            notes.mark_answered(key);
        }
    }
}

/// After the answer: the marked intents go now if the notes lock is free, and otherwise
/// when a later write or sync holds it. Nothing waits here.
pub(crate) fn answered() {
    let writes = std::mem::take(&mut told().writes);
    if !writes.is_empty()
        && let Ok((_, notes)) = notes_here()
        && let Some(_held) = notes.try_lock()
    {
        for key in writes {
            notes.forget(&key);
        }
    }
}

/// The record a session is on, as an op names it; None for a session on none.
fn bound_label(dir: &Path, session: &str) -> Option<String> {
    (read_state(dir, session).ok().flatten())
        .filter(|state| state.bound_key().is_some())
        .map(|state| state.record_label())
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
    let issue = cmd.flag("issue").map(issues).transpose()?;
    let lessons = lesson::named(&cmd.flags_all("lesson"))?;
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
    let key = match attempt(&notes, cmd, key, session, cwd) {
        Attempt::Answered(reply) => return Ok(reply),
        Attempt::Key(key) => key,
    };
    let key = key.as_str();
    if notes.already_applied(key) {
        // Done before, by this call or its interrupted first run: the result it had, or the
        // effort it bound. A write a later run finished has no result of its own to give.
        if let Some(result) = notes
            .intents()
            .into_iter()
            .find(|op| op.key == key)
            .and_then(|op| op.result)
            .filter(|result| result != notes::RECOVERED)
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
                let activation = activate::switch_on(&dir, &n, session, provider, cwd, previous.as_ref(), shells, false)?;
                notes::write_text(&state_path(&dir, session), &activation.state.to_string())?;
                Ok(format!("Effort {n} was already created by this same call (a retry); nothing new was written, and this session is bound to it.\n\n{}", activation.text()))
            }
            None => Err("this same tool call was already done (a retry); nothing new was written. Run `astrolabe activate --status` to see the session's effort".to_owned()),
        };
    }
    let pulled = crate::application::pull(&notes);
    lesson::require_open(&notes, &lessons)?;
    let initiative = canonical_membership(&notes.root, cmd.flag("initiative").unwrap_or(""))?;
    // Work on an initiative's member, filed as a new untagged effort, leaves the member
    // proposed and the board's count wrong. A title naming the initiative is that, nearly
    // always; any other new effort beside an open initiative only hears that it's there.
    let open = match initiative.is_empty() {
        true => crate::effort::roster::open_in_repo(&notes.root, &repo),
        false => Vec::new(),
    };
    if let Some(named) = open
        .iter()
        .find(|o| crate::effort::roster::names(title, &o.page))
    {
        return Err(format!(
            "the title names the initiative \"{}\", so it looks like work on one of its members. Nothing was written. If you were dispatched to a member, bind to it with `astrolabe activate --effort <n> --worker`; if this is a new part of the initiative, add --initiative {}; if it's unrelated, use a title that doesn't name the initiative.\n\n{}",
            named.page.file.title,
            named.page.slug,
            crate::effort::roster::summary(named)
        ));
    }
    let beside: Vec<String> = open
        .iter()
        .map(|o| format!(
            "Initiative \"{}\" is open in {repo}. If this work is one of its members, drop this effort with `astrolabe effort save --status abandoned --reason \"…\"` and bind to the member with `astrolabe activate --effort <n> --worker`.",
            o.page.file.title
        ))
        .collect();
    let number = notes.next_number();
    let (day, _) = notes::local_day_time();
    let file_name = format!("{}/{number:04}-{}.md", notes::effort_dir(&day), slug(title));
    let facts = RepoFacts::read(cwd);
    let mut file = effort_note(
        number,
        title,
        current,
        &Settings {
            status: &status,
            repo: &repo,
            initiative: &initiative,
            priority: cmd.flag("priority").unwrap_or("none"),
            involvement: &involvement,
            authority: &authority,
        },
        &day,
    );
    if let Some(issue) = &issue {
        file.set("issue", issue);
    }
    lesson::add_to(&mut file, &lessons);
    stamp(
        &mut file,
        session,
        provider,
        stamp_facts(cwd, facts.as_ref()),
    );
    let text = file.render();
    // The session's one probe token, so the proof from session start carries in.
    let token = activate::session_token(session);
    // The names the session carries from an effort it was on before, if any.
    let previous = read_state(&dir, session).ok().flatten();
    let state = SessionState {
        session: session.to_owned().into(),
        provider: provider.to_owned().into(),
        binding: Binding::effort(number.to_string()).into(),
        effort: number.to_string().into(),
        file: file_name.clone().into(),
        repo: repo.clone().into(),
        cwd: crate::hook::comparable(cwd).into(),
        at_ms: notes::now_ms().into(),
        token: token.into(),
        delivered: notes::content_hash(&text).into(),
        delivered_ms: notes::now_ms().into(),
        baseline: facts
            .as_ref()
            .map(|f| Field::Known(f.fingerprint.clone()))
            .unwrap_or(Field::Other(Value::Null)),
        last_save_ms: notes::now_ms().into(),
        mark: false.into(),
        saves: 0.into(),
        // From the heading the note gets, as a later load reads it. Filed for later, the
        // effort is bound but its work isn't taken up, so the session keeps the name it had.
        name: match status == "active" {
            true => {
                activate::session_name(&number.to_string(), &format!("{number:04} {title}")).into()
            }
            false => previous
                .as_ref()
                .map(|p| p.name.clone())
                .filter(|n| !n.is_missing())
                .unwrap_or(Field::Other(Value::Null)),
        },
        taken: (status == "active").into(),
        named: previous
            .as_ref()
            .map(|p| p.named.clone())
            .unwrap_or_default(),
        told: previous
            .as_ref()
            .map(|p| p.told.clone())
            .unwrap_or_default(),
        ..Default::default()
    };
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes: Vec::new(),
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
        session: Some(session.to_owned()),
        bound: Some(format!("effort {number}")),
        at_ms: Some(notes::now_ms()),
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
        false,
    )?
    .mid_session();
    // The agent was just told to rename its session: recorded, so loading this effort again
    // doesn't ask it to undo a rename the owner makes after.
    if activation.state.told != state.told
        && let Ok(Some(mut saved)) = read_state(&dir, session)
    {
        saved.told = activation.state.told.clone();
        let _ = notes::write_text(&state_path(&dir, session), &saved.to_string());
    }
    let mut head = format!(
        "Effort {number} created as {file_name}, status {status}, bound to this session and committed. Say your reading in one line (involvement {involvement} · authority {authority} · one piece or multi-step) and proceed.",
    );
    if let Pull::Diverged(why) = &pulled {
        head += &format!(
            " Notes diverged from origin before this ({}); the number may collide with the other machine's and be renumbered later.",
            why.lines().next().unwrap_or_default()
        );
    }
    for line in &beside {
        head += &format!("\n\n{line}");
    }
    if let Some(left) = previous
        .as_ref()
        .and_then(|p| activate::leaving(p, Some(&activation.state), cwd))
    {
        head += &format!("\n\n{left}");
    }
    let text = format!("{head}\n\n{}", activation.text());
    crate::application::finish(&notes, key, &text);
    Ok(text)
}

/// Refuses to rewrite a note whose header never closes. Every reader takes such a note as all
/// header, so it parses with no title or sections, and a write built from that would drop them.
fn closed_header(record: &str, text: &str) -> Result<(), String> {
    match crate::effort::header_bounds(text) {
        Some(at) if !at.closed() => Err(format!(
            "{record}'s header never closes: its first line is `---` and no later line is, so the whole note reads as properties and a write would drop its title and sections. Nothing was written. Tell the owner the note needs a line of just `---` after its last property"
        )),
        _ => Ok(()),
    }
}

/// A worker binding to the effort it was dispatched to. A closed effort is refused, and a
/// proposed or ready one is marked active, so the board shows the work started whether or
/// not the worker ever saves. Gives what to tell the worker and the note's new hash when it
/// changed. Workers once built under planning notes and new untagged efforts while their
/// members sat proposed (Deck frame time, 2026-11-08).
/// `delivered` is the note's hash as the binding read it: a note changed since then is
/// refused, so the worker never saves over text it wasn't shown.
pub fn take_up(
    number: &str,
    delivered: &str,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
) -> Result<(String, Option<String>), String> {
    let n: u32 =
        (number.trim().parse()).map_err(|_| format!("effort {number:?} isn't a number"))?;
    let (_, notes) = notes_here()?;
    let _held = notes.lock()?;
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    let rel = (notes::effort_files(&notes.root).into_iter())
        .find(|(m, _)| *m == n)
        .map(|(_, rel)| rel)
        .ok_or_else(|| format!("no effort {n}"))?;
    let on_disk =
        std::fs::read_to_string(notes.root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
    if notes::content_hash(&on_disk) != delivered {
        return Err("its note changed while this binding read it; run the same command again to take up the current text".to_owned());
    }
    let mut file = EffortFile::parse(&on_disk)?;
    let status = file.prop("status").unwrap_or("").to_owned();
    let repo = (file.prop("repo").unwrap_or("").split(','))
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    let open = crate::effort::roster::open_in_repo(&notes.root, &repo);
    let others = || {
        open.iter()
            .map(crate::effort::roster::summary)
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    if matches!(status.as_str(), "done" | "abandoned") {
        let mut why = format!(
            "it's {status}, and a worker takes up open work only. If the work goes on, the session that dispatched you reopens it or names the effort that holds the work"
        );
        if !open.is_empty() {
            why += &format!(".\n\n{}", others());
        }
        return Err(why);
    }
    let mut notice = Vec::new();
    if file.prop("initiative").is_none_or(str::is_empty) && !open.is_empty() {
        notice.push(format!(
            "This effort isn't a member of the initiative open in {repo}. If the work you were dispatched to is one of its members, bind to that member instead with `astrolabe activate --effort <n> --worker`.\n\n{}",
            others()
        ));
    }
    if !matches!(status.as_str(), "proposed" | "ready") {
        return Ok((notice.join("\n\n"), None));
    }
    file.set("status", "active");
    let facts = RepoFacts::read(cwd);
    stamp(
        &mut file,
        session,
        provider,
        stamp_facts(cwd, facts.as_ref()),
    );
    let text = file.render();
    let hash = notes::content_hash(&text);
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes: Vec::new(),
        key: format!("{key}-take-up"),
        files: vec![(rel.clone(), text)],
        renames: Vec::new(),
        journal: Vec::new(),
        message: format!("{n:04}: active (worker bound)"),
        base: Some(notes::content_hash(&on_disk)),
        states: Vec::new(),
        committed: false,
        digest: None,
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        session: None,
        bound: None,
        at_ms: None,
        rebaseline: None,
    };
    notes.apply(&op)?;
    notice.insert(
        0,
        format!("Effort {n} was {status}; binding a worker marked it active, so the board shows the work started. Replace its Current state at your first save."),
    );
    let notice = notice.join("\n\n");
    crate::application::finish(&notes, &op.key, &notice);
    notes.spawn_sync();
    Ok((notice, Some(hash)))
}

/// The properties a new effort note starts with.
struct Settings<'a> {
    status: &'a str,
    repo: &'a str,
    initiative: &'a str,
    priority: &'a str,
    involvement: &'a str,
    authority: &'a str,
}

/// A new effort note before its stamp: the title with its number, the goal as Current state,
/// and the properties in the order the board reads them.
fn effort_note(
    number: u32,
    title: &str,
    current: String,
    settings: &Settings<'_>,
    day: &str,
) -> EffortFile {
    // The goal is the first Current state, and stays as the lead under the title once
    // Current state is rewritten.
    let mut file = EffortFile {
        title: format!("{number:04} {title}"),
        lead: current.clone(),
        current,
        ..Default::default()
    };
    for (k, v) in [
        ("status", settings.status),
        ("repo", settings.repo),
        ("initiative", settings.initiative),
        ("priority", settings.priority),
        ("involvement", settings.involvement),
        ("authority", settings.authority),
        ("waits_for", ""),
        ("created", day),
    ] {
        file.set(k, v);
    }
    file
}

/// `astrolabe initiative new`, in the hook: file a page alone, with new proposed efforts or
/// around existing efforts in one commit. The session's binding stays unchanged; a
/// planning-only spike closes as provenance linked from the page, outside its membership.
pub fn initiative(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
) -> Done {
    match initiative_inner(cmd, key, session, provider, cwd) {
        Ok(text) => Done { text, ok: true },
        Err(why) => Done {
            text: format!("Initiative NOT filed: {why}. {}", activate::STILL_ON),
            ok: false,
        },
    }
}

fn initiative_inner(
    cmd: &EffortCommand,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
) -> Result<String, String> {
    let title = cmd
        .flag("title")
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or("--title is needed")?;
    if title.contains(['\n', '\r']) {
        return Err("an initiative title must be one line".into());
    }
    let body = cmd
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| {
            format!("the page goes in the body, with its `## Efforts` section.\n{FORMS}")
        })?;
    // Refused before the lock, reconcile and pull: a refused body changes nothing.
    let page = InitiativePage::parse(body)?;
    // Managed-section ambiguity is refused before any recoverable write is planned.
    EffortFile::parse(body)?;
    let goals: Vec<String> = page
        .efforts
        .iter()
        .map(|(t, goal)| as_current(goal).map_err(|why| format!("effort {t:?}: {why}")))
        .collect::<Result<_, _>>()?;
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
    let page_rel = format!("initiatives/{}.md", slug(title));
    let _held = notes.lock()?;
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    let key = match attempt(&notes, cmd, key, session, cwd) {
        Attempt::Answered(reply) => return Ok(reply),
        Attempt::Key(key) => key,
    };
    let key = key.as_str();
    if notes.already_applied(key) {
        let result = notes
            .intents()
            .into_iter()
            .find(|op| op.key == key)
            .and_then(|op| op.result);
        return Ok(result.unwrap_or_else(|| format!(
            "Initiative {title:?} was already filed by this same call (a retry); nothing new was written. Its page is {page_rel}"
        )));
    }
    let pulled = crate::application::pull(&notes);
    if notes.root.join(&page_rel).exists() {
        return Err(format!(
            "{page_rel} exists already: an initiative is filed once, and a later effort joins it with --initiative {title:?} on `effort new` or `effort save`"
        ));
    }
    let pages = crate::effort::initiative_catalog(&notes.root)?;
    let page_slug = slug(title);
    for existing in &pages {
        let aliases = [
            crate::effort::initiative_name(&existing.slug),
            crate::effort::initiative_name(&existing.file.title),
        ];
        if [
            crate::effort::initiative_name(title),
            crate::effort::initiative_name(&page_slug),
        ]
        .iter()
        .any(|a| aliases.contains(a))
        {
            return Err(format!(
                "ambiguous initiative title or slug: {title:?} collides with {}",
                existing.path
            ));
        }
    }
    let start = notes.next_number();
    let (day, _) = notes::local_day_time();
    let facts = RepoFacts::read(cwd);
    let settings = Settings {
        status: "proposed",
        repo: &repo,
        initiative: &page_slug,
        priority: "none",
        involvement: &involvement,
        authority: &authority,
    };
    let mut files = Vec::new();
    let mut filed = Vec::new();
    let mut lines = Vec::new();
    for (i, ((name, _), goal)) in page.efforts.iter().zip(goals).enumerate() {
        let number = start + i as u32;
        let rel = format!("{}/{number:04}-{}.md", notes::effort_dir(&day), slug(name));
        let mut file = effort_note(number, name, goal, &settings, &day);
        stamp(
            &mut file,
            session,
            provider,
            stamp_facts(cwd, facts.as_ref()),
        );
        files.push((rel.clone(), file.render()));
        lines.push(format!("{number:04} {name}"));
        filed.push((number, rel));
    }
    // Validate every join before writing any file, and leave every field except membership alone.
    for number in &page.joins {
        let matches: Vec<_> = notes
            .numbers()
            .into_iter()
            .filter(|(n, _)| n == number)
            .collect();
        let [(_, rel)] = matches.as_slice() else {
            return Err(format!("effort {number} is missing or ambiguous"));
        };
        let text =
            std::fs::read_to_string(notes.root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        let file = EffortFile::parse(&text)?;
        if let Some(member) = file.prop("initiative") {
            let member = crate::effort::initiative_name(member);
            if member != crate::effort::initiative_name(title)
                && member != crate::effort::initiative_name(&page_slug)
            {
                return Err(format!(
                    "effort {number} already belongs to another initiative: {member}"
                ));
            }
        }
        // A header that never closes has no line to write the membership before, so the
        // join would be reported without happening.
        if crate::effort::header_bounds(&text).is_some_and(|at| !at.closed()) {
            return Err(format!(
                "effort {number}'s header never closes: its first line is `---` and no later line is, so there's no place to write its membership. Nothing was written. Tell the owner the note needs a line of just `---` after its last property"
            ));
        }
        // A raw property edit keeps the joined effort's Current state, stamps and all prose unchanged.
        let joined = crate::effort::set_initiative_property(&text, &page_slug);
        files.push((rel.clone(), joined));
        lines.push(format!("{number:04} {}", file.name()));
        filed.push((*number, rel.clone()));
    }
    let mut page_text = page.render(title, &repo, &day, &filed);
    // Joining uses existing effort titles, not the control paragraphs, in the prose list.
    if !page.joins.is_empty() {
        let mut page_for_join = page.clone();
        page_for_join.efforts = files
            .iter()
            .map(|(_, text)| {
                let f = EffortFile::parse(text).expect("join validated");
                (f.name().to_owned(), String::new())
            })
            .collect();
        page_text = page_for_join.render(title, &repo, &day, &filed);
    }
    let mut page_file = EffortFile::parse(&page_text)?;
    page_file.set("involvement", &involvement);
    page_file.set("authority", &authority);
    page_file.set("updated", &notes::local_stamp());
    page_file.set(
        "saved_by",
        &format!("{} · {provider}", notes::machine_name()),
    );
    page_file.set("session", session);
    files.push((page_rel.clone(), page_file.render_initiative()));
    let count = filed.len();
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes: Vec::new(),
        key: key.to_owned(),
        files,
        renames: Vec::new(),
        // No journal line: filing is state, not history; the planning session's own line
        // tells the plan.
        journal: Vec::new(),
        message: format!("initiative {title}: {count} efforts filed"),
        base: None,
        // The session stays on its own effort.
        states: Vec::new(),
        committed: false,
        digest: Some(cmd.digest()),
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        session: Some(session.to_owned()),
        bound: bound_label(&notes.state, session),
        at_ms: Some(notes::now_ms()),
        rebaseline: rebaseline(cwd),
    };
    notes.apply(&op)?;
    let saved = saved_text(&pulled, notes.spawn_sync());
    let bound = read_state(&notes.state, session)
        .ok()
        .flatten()
        .map(|s| s.record_label());
    let action = if page.joins.is_empty() {
        "created"
    } else {
        "joined"
    };
    let mut text = format!(
        "Initiative {title:?} filed as {page_rel} with {count} efforts: {count} {action}; {saved}.\n{}\n{}\nActivate it with `astrolabe activate --initiative {page_slug}`. A planning-only effort closes without --initiative; the page links it as provenance.",
        lines.join("\n"),
        match bound {
            Some(binding) => format!("This session stays on {binding}; its binding is unchanged."),
            None => "This session has no binding; it is unchanged.".to_owned(),
        }
    );
    if let Pull::Diverged(why) = &pulled {
        text += &format!(
            " Notes diverged from origin before this ({}); the numbers may collide with the other machine's and be renumbered later.",
            why.lines().next().unwrap_or_default()
        );
    }
    crate::application::finish(&notes, key, &text);
    Ok(text)
}

/// `astrolabe effort save`, in the hook.
pub fn save(cmd: &EffortCommand, key: &str, session: &str, provider: &str, cwd: &Path) -> Done {
    match save_inner(cmd, key, session, provider, cwd) {
        Ok(text) => Done { text, ok: true },
        Err(why) if why.contains(LESSONS_UNFINISHED) => Done {
            text: why,
            ok: false,
        },
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
    let (registry, notes) = notes_here()?;
    let dir = notes.state.clone();
    let mut state = read_state(&dir, session)?.ok_or(
        "no effort is bound to this session. Start one with `astrolabe effort new`, or load the one the owner named with `astrolabe activate --effort <n>`",
    )?;
    let mut req = SaveRequest::parse(cmd, &state, cwd)?;
    let _held = notes.lock()?;
    let leftovers = crate::application::reconcile(&notes);
    if !leftovers.is_empty() {
        return Err(format!(
            "earlier notes writes are still unfinished and failed again, so nothing new is written until they're repaired: {}",
            leftovers.join("; ")
        ));
    }
    let key = match attempt(&notes, cmd, key, session, cwd) {
        Attempt::Answered(reply) => return Ok(reply),
        Attempt::Key(key) => key,
    };
    let key = key.as_str();
    if notes.already_applied(key) {
        return Ok(format!(
            "Already saved: this same tool call was done before (a retry), so nothing new was written to {}.",
            req.record
        ));
    }
    let pulled = crate::application::pull(&notes);
    // The pull, or the recovery before it, can renumber this session's effort, when another
    // machine's effort had the number first. The session's state follows the file, so the
    // request is read again from where the effort is now, not from the path it had.
    let mut notes_lines = Vec::new();
    if let Some(moved) = read_state(&dir, session)?.filter(|now| now.file != state.file) {
        let was = state.bound_key().unwrap_or_default().to_owned();
        state = moved;
        req = SaveRequest::parse(cmd, &state, cwd)?;
        let now = state.bound_key().unwrap_or_default();
        notes_lines.push(format!(
            "This save's pull renumbered the effort from {was} to {now}, because another machine's effort had {was} first; it's effort {now} from here on."
        ));
    }
    let record = &req.record;
    let initiative = req
        .initiative
        .map(|name| canonical_membership(&notes.root, name))
        .transpose()?;
    let initiative = initiative.as_deref();
    if let Some(target) = req.target {
        return save_target(
            &notes, cmd, &req, key, session, provider, cwd, &state, target, initiative, &pulled,
        );
    }
    let rel = &req.rel;
    let path = notes.root.join(rel);
    let on_disk = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
    closed_header(record, &on_disk)?;
    let on_disk_hash = notes::content_hash(&on_disk);
    let mut file = EffortFile::parse(&on_disk)?;
    let Some(delivered) = state.delivered.as_deref().map(str::to_owned) else {
        return Err("this session's state has no record of the effort as delivered to it, so a save can't tell whether it would overwrite another's. Load the effort again with `astrolabe activate --effort <n>`".to_owned());
    };
    if delivered != on_disk_hash {
        // Delivered anew, so the next save from this text goes through.
        state.delivered.set(on_disk_hash.clone());
        state.delivered_ms.set(notes::now_ms());
        let _ = notes::write_text(&state_path(&dir, session), &state.to_string());
        let pieces = file
            .pieces()
            .map(|p| format!("\n\nIts Pieces now:\n\n{p}"))
            .unwrap_or_default();
        return Err(format!(
            "{record} changed since it was delivered to this session (saved by {} at {}). Nothing was overwritten. Its Current state now:\n\n{}{pieces}\n\nSave again from this.",
            file.prop("saved_by").unwrap_or("?"),
            file.prop("updated").unwrap_or("?"),
            file.current
        ));
    }
    let status = req.status.as_deref();
    require_transition_body(&file, status, req.current.as_deref())?;
    if req.page_binding && status == Some("done") {
        let pages = crate::effort::initiative_catalog(&notes.root)?;
        let page = crate::effort::resolve_initiative(&pages, &req.number)?;
        let open: Vec<_> = crate::effort::initiative_members(&notes.root, page)?
            .into_iter()
            .filter(|(_, f)| !matches!(f.prop("status"), Some("done" | "abandoned")))
            .map(|(_, f)| f.title)
            .collect();
        if !open.is_empty() {
            return Err(format!(
                "initiative cannot be done while members are open: {}. Nothing was written",
                open.join(", ")
            ));
        }
    }
    let facts = RepoFacts::read(cwd);
    let (day, _) = notes::local_day_time();
    if req.plan_shifted {
        notes_lines.push(
            "The plan's headings were shifted so its top level is `###`, because the effort file's sections are read by `## `."
                .to_owned(),
        );
    }
    let waited = file.prop("waits_for").unwrap_or_default().to_owned();
    lesson::require_open(&notes, &newly_named(&file, &req))?;
    let before = edit_bound(&mut file, &req, initiative, &day)?;
    if let Some(wait) = newly_needs_owner(&file, &before, &waited) {
        notes_lines.push(validation::needs_you_note(record, wait));
    }
    let lesson_closes = match !req.page_binding && marks_done(status, &before) {
        true => {
            let number = req.number.parse().unwrap_or(0);
            let (closes, lines) = lesson::closes(&notes, &file, number, &day);
            notes_lines.extend(lines);
            closes
        }
        false => Vec::new(),
    };
    let machine = notes::machine_name();
    let journal = req
        .journal_line(key, &machine, notes::now_ms(), &day)
        .into_iter()
        .collect();
    let now = notes::local_stamp();
    let saver = Saver {
        now: &now,
        machine: &machine,
        provider,
        session,
    };
    let text = if req.page_binding {
        stamp_initiative(&mut file, state.repo.as_deref().unwrap_or(""), &day, &saver);
        file.render_initiative()
    } else {
        stamp(
            &mut file,
            session,
            provider,
            stamp_facts(cwd, facts.as_ref()),
        );
        file.render()
    };
    let now = notes::now_ms();
    let mut next_state = state.clone();
    next_state.saved(
        notes::content_hash(&text),
        facts.as_ref().map(|f| f.fingerprint.clone()),
        now,
    );
    let message = commit_message(req.page_binding, &req.number, status, &before);
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes,
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
        session: Some(session.to_owned()),
        bound: Some(state.record_label()),
        at_ms: Some(notes::now_ms()),
        rebaseline: rebaseline(cwd),
    };
    apply_save(&notes, &op, record)?;
    // Every save, so the other machine's board follows it: a checkpoint held for a batching
    // window sat unpushed for as long as its session stayed quiet.
    let sync_started = notes.spawn_sync();
    let issue = if status == Some("done")
        && before != "done"
        && let Some(issue) = file.prop("issue").filter(|i| !i.is_empty())
    {
        // Main of the effort's own repo: from the notes checkout, its registered checkout;
        // elsewhere, this folder only when it's that repo, never another's history.
        let repos: Vec<&str> = (file.prop("repo").unwrap_or("").split(','))
            .map(str::trim)
            .collect();
        let checkout = if activate::in_notes(cwd) {
            repos.first().and_then(|r| registry.checkout(r))
        } else {
            repo_of(&notes, &registry, cwd, None)
                .ok()
                .filter(|r| repos.contains(&r.as_str()))
                .map(|_| cwd.to_path_buf())
        };
        Some(issue_report(checkout.as_deref(), issue))
    } else {
        None
    };
    let mut head = saved_reply(&Reply {
        record,
        status: file.prop("status").unwrap_or("?"),
        body_replaced: cmd.body.is_some(),
        journal_written: req.journal_text.is_some(),
        pulled: &pulled,
        sync_started,
        unchanged: None,
        notes_lines: &notes_lines,
        issue: issue.as_deref(),
        unfinished: &unfinished_pieces(&file),
    });
    // The page's own save shows every member where it stands: an orchestrator once saved its
    // page seven times about landed work while every member still read proposed.
    if req.page_binding
        && let Ok(pages) = crate::effort::initiative_catalog(&notes.root)
        && let Ok(page) = crate::effort::resolve_initiative(&pages, &req.number)
        && let Ok(members) = crate::effort::initiative_members(&notes.root, page)
    {
        let open = crate::effort::roster::Open {
            page: page.clone(),
            members,
        };
        head += &format!(
            "\n\n{}\nIf one of these moved, save it by number: `astrolabe effort read <n>`, then `astrolabe effort save --effort <n> …`.",
            crate::effort::roster::summary(&open)
        );
    }
    crate::application::finish(&notes, key, &head);
    Ok(head)
}

/// How a done save's reply starts when its own commit landed and a lesson close after it
/// didn't: the effort is saved, so saving it again would only add a commit.
const LESSONS_UNFINISHED: &str = ", but closing its lessons stopped: ";

/// The write, and when only its lesson closes failed, a reply that says the save landed: the
/// intent stays without a result, so the next notes write finishes the closes.
fn apply_save(notes: &Notes, op: &Op, record: &str) -> Result<(), String> {
    match notes.apply(op) {
        Ok(()) => Ok(()),
        Err(why) if !op.lesson_closes.is_empty() && notes.already_applied(&op.key) => {
            let why = match why.trim() {
                "" => "git refused a commit".to_owned(),
                why => why.trim_end_matches('.').to_owned(),
            };
            Err(format!(
                "Saved {record}{LESSONS_UNFINISHED}{why}. Don't save it again: the next notes write, or `astrolabe notes sync`, finishes the close without repeating a disposition."
            ))
        }
        Err(why) => Err(why),
    }
}

/// A save that moves the effort to done from any other status: the one that closes its lessons.
fn marks_done(status: Option<&str>, before: &str) -> bool {
    status == Some("done") && before != "done"
}

/// The lessons a save names that the effort doesn't yet: only those must exist now.
fn newly_named(file: &EffortFile, req: &SaveRequest) -> Vec<String> {
    let have = lesson::of(file);
    (req.lessons.iter())
        .filter(|l| !have.contains(l))
        .cloned()
        .collect()
}

fn apply_ticks(
    file: &mut EffortFile,
    ticks: &[&str],
    unticks: &[&str],
    day: &str,
) -> Result<(), String> {
    if !ticks.is_empty() || !unticks.is_empty() {
        let on: Vec<usize> = ticks
            .iter()
            .map(|l| file.piece_at(l, true))
            .collect::<Result<_, _>>()?;
        let off: Vec<usize> = unticks
            .iter()
            .map(|l| file.piece_at(l, false))
            .collect::<Result<_, _>>()?;
        if let Some(i) = on.iter().position(|i| off.contains(i)) {
            return Err(format!("{:?} is both ticked and unticked", ticks[i]));
        }
        for at in on {
            file.tick(at, true, day);
        }
        for at in off {
            file.tick(at, false, day);
        }
    }
    Ok(())
}

fn require_transition_body(
    file: &EffortFile,
    status: Option<&str>,
    body: Option<&str>,
) -> Result<(), String> {
    if let Some(status @ ("active" | "review" | "blocked" | "done" | "abandoned")) = status
        && file.prop("status") != Some(status)
        && body.is_none_or(|b| b.trim().is_empty())
    {
        return Err(format!(
            "changing status to {status} needs a Current state body so status moves with its text. Nothing was written.\n{FORMS}"
        ));
    }
    Ok(())
}

/// The wait a save just put under Needs you: the record shows there now, and before the save
/// it didn't, or showed another ask. What the board shows, not the stored words: a closed
/// record never shows, so one reopened with the wait an old note left on it is new there.
fn newly_needs_owner<'a>(file: &'a EffortFile, before: &str, waited: &str) -> Option<&'a str> {
    let shows = |status: &str, wait: &str| {
        !matches!(status, "done" | "abandoned") && wait.starts_with("owner:")
    };
    let wait = file.prop("waits_for").unwrap_or_default();
    let now = shows(file.prop("status").unwrap_or_default(), wait);
    (now && !(shows(before, waited) && wait == waited)).then_some(wait)
}

fn unfinished_pieces(file: &EffortFile) -> String {
    if file.prop("status") != Some("done") {
        return String::new();
    }
    let lines: Vec<_> = file.unticked_pieces().collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("\nUnticked pieces:\n{}", lines.join("\n"))
    }
}

/// The part of a save that changes an effort's state rather than its text: what a session
/// writes beside Current state, whether bound or addressed by number.
struct Change<'a> {
    status: Option<&'a str>,
    waits_for: Option<&'a str>,
    reason: Option<&'a str>,
    decisions: &'a [&'a str],
    initiative: Option<&'a str>,
}

impl Change<'_> {
    /// Applies the change; gives the status from before it.
    fn apply(&self, file: &mut EffortFile, day: &str) -> Result<String, String> {
        for d in self.decisions {
            file.add_decision(&format!("{day}: {d}"));
        }
        // Why the work stopped goes with the record of choices, dated, so it outlives later
        // saves and a revival; Current state is replaced on every save.
        if let Some(r) = self.reason {
            file.add_decision(&format!("{day}: abandoned: {r}"));
        }
        if let Some(i) = self.initiative {
            file.set("initiative", i);
        }
        let before = file.prop("status").unwrap_or("").to_owned();
        if let Some(s) = self.status {
            file.set("status", s);
        }
        // A wait is set by the save made just before waiting, so a later save means the work
        // moved on: one without --waits-for clears it unless blocked or in review.
        // Before this, only a status change cleared it, and an agent saving progress after
        // the owner answered left the effort in Needs you.
        let blocked = file.prop("status") == Some("blocked");
        if blocked && self.waits_for == Some("") {
            return Err("a blocked effort keeps what it waits on: give the new condition, or a status that isn't blocked. Nothing was written".to_owned());
        }
        // A closed record waits on nothing. Read from the file, not the flag: a wait added to
        // an effort already done, with no --status, put a finished effort in Needs you.
        if let Some(closed @ ("done" | "abandoned")) = file.prop("status")
            && self.waits_for.is_some_and(|w| !w.is_empty())
        {
            return Err(format!(
                "a {closed} record waits on nothing: while something still waits, keep it open (active, review or blocked) with --status; otherwise save without --waits-for. Nothing was written"
            ));
        }
        match self.waits_for {
            Some(w) => file.set("waits_for", w),
            None if !matches!(file.prop("status"), Some("blocked" | "review")) => {
                file.set("waits_for", "")
            }
            None => {}
        }
        Ok(before)
    }
}

/// An explicit read delivers the whole note without changing the session binding.
pub fn read(cmd: &EffortCommand, session: &str, cwd: &Path) -> Done {
    match read_inner(cmd, session, cwd) {
        Ok(text) => Done { text, ok: true },
        Err(why) => Done {
            text: format!("Effort NOT read: {why}"),
            ok: false,
        },
    }
}

fn read_inner(cmd: &EffortCommand, session: &str, cwd: &Path) -> Result<String, String> {
    let (_, notes) = notes_here()?;
    let _held = notes.lock()?;
    let mut state =
        read_state(&notes.state, session)?.ok_or("no record is bound to this session")?;
    let target: u32 = cmd
        .flag("effort")
        .ok_or("read needs a number")?
        .parse()
        .map_err(|_| "read needs a number")?;
    if state.worker.is_true()
        && (state.initiative_key().is_some()
            || state.bound_key() != Some(target.to_string().as_str()))
    {
        return Err(
            "a worker can read only its own effort; report to the session that dispatched it"
                .into(),
        );
    }
    crate::application::pull(&notes);
    let (_, text, file) = target_note(&notes, &state, target, cwd)?;
    let hash = notes::content_hash(&text);
    state.deliver_effort(target, hash.clone());
    if state.initiative_key().is_none() && state.bound_key() == Some(target.to_string().as_str()) {
        state.delivered.set(hash);
        state.delivered_ms.set(notes::now_ms());
    }
    notes::write_text(&state_path(&notes.state, session), &state.to_string())?;
    Ok(format!("Read effort {target}: {}\n\n{text}", file.title))
}

/// Repo access is identical for reading and saving; membership is resolved from disk,
/// never from the save's proposed initiative flag.
fn target_note(
    notes: &Notes,
    state: &SessionState,
    target: u32,
    cwd: &Path,
) -> Result<(String, String, EffortFile), String> {
    let (_, rel) = notes
        .numbers()
        .into_iter()
        .find(|(n, _)| *n == target)
        .ok_or_else(|| format!("no effort {target} in the notes"))?;
    let text = std::fs::read_to_string(notes.root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
    let file = EffortFile::parse(&text)?;
    let mine = state.repo.as_deref().unwrap_or_default();
    let theirs = file.prop("repo").unwrap_or_default();
    if theirs != mine && !activate::in_notes(cwd) {
        let pages = crate::effort::initiative_catalog(&notes.root)?;
        let target_page = file
            .prop("initiative")
            .and_then(|name| crate::effort::resolve_initiative(&pages, name).ok());
        let bound =
            std::fs::read_to_string(notes.root.join(state.file.as_deref().unwrap_or_default()))
                .map_err(|e| format!("cannot read the bound record: {e}"))?;
        let bound = EffortFile::parse(&bound)?;
        let source_page = state
            .initiative_key()
            .or_else(|| bound.prop("initiative"))
            .and_then(|name| crate::effort::resolve_initiative(&pages, name).ok());
        if !matches!((source_page, target_page), (Some(a), Some(b)) if a.slug == b.slug) {
            return Err(format!(
                "effort {target} is for repo {theirs:?}, not this session's {mine:?}, and they share no initiative. Nothing was written"
            ));
        }
    }
    Ok((rel, text, file))
}

/// A dispatcher can write text and ticks once the note was delivered by number.
#[allow(clippy::too_many_arguments)]
fn save_target(
    notes: &Notes,
    cmd: &EffortCommand,
    req: &SaveRequest,
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    state: &SessionState,
    target: u32,
    initiative: Option<&str>,
    pulled: &Pull,
) -> Result<String, String> {
    let (rel, on_disk, mut file) = target_note(notes, state, target, cwd)?;
    closed_header(&format!("effort {target}"), &on_disk)?;
    let on_disk_hash = notes::content_hash(&on_disk);
    let current = req.current.as_deref();
    let delivered = state
        .read_efforts
        .as_ref()
        .and_then(|reads| reads.get(&target.to_string()));
    if (current.is_some()
        || req.pieces.is_some()
        || !req.ticks.is_empty()
        || !req.unticks.is_empty())
        && delivered.is_none()
    {
        return Err(format!(
            "run `astrolabe effort read {target}` first: a body, pieces list or tick needs the note delivered in this session. Nothing was written.\n{FORMS}"
        ));
    }
    if delivered.is_some_and(|hash| hash != &on_disk_hash) {
        let mut next = state.clone();
        next.deliver_effort(target, on_disk_hash.clone());
        notes::write_text(&state_path(&notes.state, session), &next.to_string())?;
        return Err(format!(
            "effort {target} changed since it was delivered to this session. Nothing was overwritten. Its note now:\n\n{on_disk}\nSave again from this."
        ));
    }
    let status = req.status.as_deref();
    require_transition_body(&file, status, current)?;
    let (day, _) = notes::local_day_time();
    let waited = file.prop("waits_for").unwrap_or_default().to_owned();
    lesson::require_open(notes, &newly_named(&file, req))?;
    let before = edit_bound(&mut file, req, initiative, &day)?;
    let mut needs_you: Vec<String> = newly_needs_owner(&file, &before, &waited)
        .map(|wait| validation::needs_you_note(&format!("effort {target}"), wait))
        .into_iter()
        .collect();
    let lesson_closes = match marks_done(status, &before) {
        true => {
            let (closes, lines) = lesson::closes(notes, &file, target, &day);
            needs_you.extend(lines);
            closes
        }
        false => Vec::new(),
    };
    let now = notes::local_stamp();
    let machine = notes::machine_name();
    stamp_saver(
        &mut file,
        &Saver {
            now: &now,
            machine: &machine,
            provider,
            session,
        },
    );
    let text = file.render();
    let message = commit_message(false, &target.to_string(), status, &before);
    let mut next = state.clone();
    if delivered.is_some() {
        next.deliver_effort(target, notes::content_hash(&text));
    }
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes,
        key: key.to_owned(),
        files: vec![(rel, text)],
        renames: Vec::new(),
        journal: Vec::new(),
        message,
        base: Some(on_disk_hash),
        // Only the by-number hash advances; the bound record was not saved.
        states: vec![(state_path(&notes.state, session), next.to_value())],
        committed: false,
        digest: Some(cmd.digest()),
        result: None,
        cwd: Some(crate::hook::comparable(cwd)),
        session: Some(session.to_owned()),
        bound: Some(state.record_label()),
        at_ms: Some(notes::now_ms()),
        rebaseline: None,
    };
    apply_save(notes, &op, &format!("effort {target}"))?;
    let sync_started = notes.spawn_sync();
    let head = saved_reply(&Reply {
        record: &format!("effort {target}"),
        status: file.prop("status").unwrap_or("?"),
        body_replaced: false,
        journal_written: false,
        pulled,
        sync_started,
        unchanged: Some(&state.record_label()),
        notes_lines: &needs_you,
        issue: None,
        unfinished: &unfinished_pieces(&file),
    });
    crate::application::finish(notes, key, &head);
    Ok(head)
}

/// Whether main here already holds a commit that closes each issue the effort answers, read
/// from the last fetch: an effort can be done while the issue it answers stays open, because
/// the issue number never reached the landed commit (2026-11-07: five fixed issues sat open).
fn issue_report(checkout: Option<&Path>, issue: &str) -> String {
    // Split on any comma: a hand-edited `5,6` is two issues, as the session context reads it.
    let issues: Vec<&str> = (issue.split(','))
        .map(|n| n.trim().trim_start_matches('#'))
        .filter(|n| !n.is_empty())
        .collect();
    let tag = |ns: &[&str]| {
        ns.iter()
            .map(|n| format!("#{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let Some(cwd) = checkout else {
        return format!(
            "This effort answers issue {}, but this folder isn't a checkout of its repo, so nothing checked whether main closes it; make sure GitHub does.",
            tag(&issues)
        );
    };
    let os = |s: &str| std::ffi::OsString::from(s);
    let main = ["refs/remotes/origin/HEAD", "refs/remotes/origin/main"]
        .into_iter()
        .find(|r| {
            crate::worktree::git(
                cwd,
                &[&os("rev-parse"), &os("--verify"), &os("--quiet"), &os(r)],
            )
            .is_ok()
        });
    let log =
        main.map(|r| crate::worktree::git(cwd, &[&os("log"), &os("--format=%B%x00"), &os(r)]));
    let Some(Ok(log)) = log else {
        return format!(
            "This effort answers issue {}, but there's no origin main here to check it against; make sure GitHub closes it.",
            tag(&issues)
        );
    };
    let messages: Vec<&str> = log.split('\0').collect();
    let (closed, open): (Vec<&str>, Vec<&str>) =
        (issues.iter().copied()).partition(|n| messages.iter().any(|m| closes(m, n)));
    let mut said = Vec::new();
    if !closed.is_empty() {
        said.push(format!(
            "Main here has a commit that closes {}.",
            tag(&closed)
        ));
    }
    if !open.is_empty() {
        let fixes: Vec<String> = open.iter().map(|n| format!("`Fixes #{n}`")).collect();
        said.push(format!(
            "No commit on main here, as of the last fetch, closes {}, so GitHub won't close it on its own. Land a commit that says {}, or ask the owner before closing it by hand.",
            tag(&open),
            fixes.join(" and ")
        ));
    }
    said.join(" ")
}

/// The ticket is written; the intent that held the result can go.
pub fn forget(key: &str) {
    // Another save may already be scanning recovery intents. Removing one outside the
    // lock can make that scan report a missing file as a broken, unfinished write.
    if let Ok((_, notes)) = notes_here()
        && let Ok(_held) = notes.lock()
    {
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
    if let Some(state) = bound.as_ref().filter(|s| s.bound_key().is_some()) {
        return Err(format!(
            "this session has {}; its journal line goes on the {}: `astrolabe effort save --journal \"…\"`",
            state.record_label(),
            state.record_kind()
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
    let key = match attempt(&notes, cmd, key, session, cwd) {
        Attempt::Answered(reply) => return Ok(reply),
        Attempt::Key(key) => key,
    };
    let key = key.as_str();
    if notes.already_applied(key) {
        return Ok("Already written: this same tool call was done before (a retry), so nothing new went in the journal.".to_owned());
    }
    let pulled = crate::application::pull(&notes);
    let (day, _) = notes::local_day_time();
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
        lesson_closes: Vec::new(),
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
        session: Some(session.to_owned()),
        bound: None,
        at_ms: Some(notes::now_ms()),
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
        let positional = (i > 0 || first == "journal")
            && !(first == "effort" && args.first() == Some(&"read") && i == 1)
            && !is_flag
            && !wants_value;
        if a.contains('\n') || positional {
            body = Some((*a).to_owned());
        } else if a.is_empty()
            || a.contains(|c: char| c.is_whitespace() || "\"'$`\\;|&<>()".contains(c))
        {
            // The shell's quoting is gone by here, so every argument a bare word can't
            // hold is quoted again: an apostrophe with no space beside it too. One holding
            // a character no quoted value may is refused below, as the hook refused it.
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
    let notes = Notes::new(std::path::PathBuf::new(), dir.clone());
    if let Some((text, ok, age_ms)) = activate::shown_for(&dir, &digest, &here, RECENT_MS) {
        // Shown here, the session has its answer, whatever its hook managed to say.
        notes.mark_shown(&digest, &here);
        return (
            format!(
                "{text}\n\n(This is the result of a command with this exact text, run in this folder {} s ago through the hook. If that was another session, this one's command wasn't taken by the hook and recorded nothing.)",
                age_ms / 1000
            ),
            ok,
        );
    }
    if let Some(op) = notes.intent_for(&digest, &here) {
        return match op.result {
            Some(result) => {
                notes.mark_shown(&digest, &here);
                (
                format!("{result}\n\n(Recovered from the hook's record of this command: its reply was lost on the way.)"),
                true,
            )
            }
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

/// How a save's reply says where its commit went: kept here when there's no remote, else
/// whether the background push could start.
fn saved_text(pulled: &Pull, sync_started: bool) -> &'static str {
    if matches!(pulled, Pull::Offline(why) if why == "no remote") {
        "saved locally; sync unavailable: no remote configured"
    } else if sync_started {
        "committed, background sync started"
    } else {
        "committed; background sync could not start"
    }
}
