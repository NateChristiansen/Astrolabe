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
mod validation;
use validation::{
    AUTHORITIES, INVOLVEMENTS, STATUSES, as_current, check_journal, closes, issues, one_of,
    shift_plan_headings, split_plan,
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
    let initiative = canonical_membership(&notes.root, cmd.flag("initiative").unwrap_or(""))?;
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
    let text = format!("{head}\n\n{}", activation.text());
    crate::application::finish(&notes, key, &text);
    Ok(text)
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
    let mut file = EffortFile {
        title: format!("{number:04} {title}"),
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
    let number = state.bound_key().unwrap_or_default().to_owned();
    let rel = state.file.as_deref().unwrap_or_default().to_owned();
    let status = cmd
        .flag("status")
        .map(|s| one_of("status", s, &STATUSES))
        .transpose()?;
    // Each one given is recorded, in order: agents pass several, and keeping only the last
    // dropped the others without a word.
    let decisions: Vec<&str> = cmd
        .flags_all("decision")
        .into_iter()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .collect();
    // Given empty, it clears the condition; left out, it's cleared too unless the effort
    // stays blocked or in review (below).
    let waits_for = cmd.flag("waits-for").map(str::trim);
    let reason = cmd.flag("reason").map(str::trim).filter(|r| !r.is_empty());
    let plan_file = cmd.flag("plan-file");
    let journal_text = cmd.flag("journal").map(str::trim).filter(|j| !j.is_empty());
    let ticks = cmd.flags_all("tick");
    let unticks = cmd.flags_all("untick");
    let pieces_file = cmd.flag("pieces-file");
    // An empty value clears the grouping, so an effort can leave an initiative.
    let initiative = cmd.flag("initiative");
    let target = cmd
        .flag("effort")
        .map(|n| {
            n.trim()
                .parse::<u32>()
                .map_err(|_| format!("--effort takes an effort number, not {n:?}"))
        })
        .transpose()?;
    let page_binding = state.initiative_key().is_some() && target.is_none();
    let record = if page_binding {
        format!("initiative {number}")
    } else {
        format!("effort {number}")
    };
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
    if page_binding
        && (pieces_file.is_some()
            || !ticks.is_empty()
            || !unticks.is_empty()
            || plan_file.is_some()
            || body_marked
            || initiative.is_some())
    {
        return Err("an initiative has no pieces, ticks, plan replacement or initiative membership; save its Current state, Decisions, status, wait and journal only. Nothing was written".into());
    }
    if page_binding && status.as_deref() == Some("review") {
        return Err("an initiative has no review status. Nothing was written".into());
    }
    if plan_file.is_some() && body_marked {
        return Err(
            "the plan comes one way: --plan-file, or a `## Plan` section in the body, not both"
                .to_owned(),
        );
    }
    if body.is_none()
        && body_plan.is_none()
        && status.is_none()
        && decisions.is_empty()
        && waits_for.is_none()
        && plan_file.is_none()
        && journal_text.is_none()
        && ticks.is_empty()
        && unticks.is_empty()
        && pieces_file.is_none()
        && initiative.is_none()
    {
        return Err(format!(
            "nothing to save: give a Current state body, or a flag.\n{FORMS}"
        ));
    }
    if target.is_some()
        && (body_plan.is_some()
            || plan_file.is_some()
            || journal_text.is_some()
            || pieces_file.is_some())
    {
        return Err(format!(
            "with --effort <n>, --journal, --plan-file and --pieces-file are unavailable. Nothing was written.\n{FORMS}"
        ));
    }
    if target.is_some() && state.worker.is_true() {
        return Err(format!(
            "this session is a worker under {record}: it saves that effort only; the session that dispatched it marks the others. Nothing was written"
        ));
    }
    if !page_binding
        && state.initiative_key().is_none()
        && target.is_some_and(|t| t.to_string() == number)
    {
        return Err(format!(
            "{record} is this session's own: save it without --effort. Nothing was written"
        ));
    }
    if status.as_deref() == Some("blocked") && waits_for.is_none_or(str::is_empty) {
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
            "Already saved: this same tool call was done before (a retry), so nothing new was written to {record}."
        ));
    }
    let pulled = crate::application::pull(&notes);
    let initiative = initiative
        .map(|name| canonical_membership(&notes.root, name))
        .transpose()?;
    let initiative = initiative.as_deref();
    if let Some(target) = target {
        let change = Change {
            status: status.as_deref(),
            waits_for,
            reason,
            decisions: &decisions,
            initiative,
        };
        return save_target(
            &notes, cmd, key, session, provider, cwd, &state, target, &change, &pulled,
        );
    }
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
    require_transition_body(&file, status.as_deref(), current.as_deref())?;
    if page_binding && status.as_deref() == Some("done") {
        let pages = crate::effort::initiative_catalog(&notes.root)?;
        let page = crate::effort::resolve_initiative(&pages, &number)?;
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
    if let Some(p) = &pieces {
        file.set_pieces(p);
    }
    apply_ticks(&mut file, &ticks, &unticks)?;
    let before = Change {
        status: status.as_deref(),
        waits_for,
        reason,
        decisions: &decisions,
        initiative,
    }
    .apply(&mut file, &day)?;
    // The journal is the owner's history and only the agent's sentence goes in it; one per
    // effort per day, the later replacing the earlier.
    let journal = journal_text
        .map(|j| JournalLine {
            id: line_id(key, 0),
            effort: if page_binding {
                format!("initiative:{number}")
            } else {
                format!("{:04}", number.parse::<u32>().unwrap_or(0))
            },
            text: j.to_owned(),
            machine: notes::machine_name(),
            at_ms: notes::now_ms(),
            day: day.clone(),
            replace: true,
        })
        .into_iter()
        .collect();
    let text = if page_binding {
        for (key, value) in [
            ("status", "ready"),
            ("repo", state.repo.as_deref().unwrap_or("")),
            ("involvement", "check in at plan"),
            ("authority", "edit"),
            ("created", day.as_str()),
        ] {
            if file.prop(key).is_none() {
                file.set(key, value);
            }
        }
        file.set("updated", &notes::local_stamp());
        file.set(
            "saved_by",
            &format!("{} · {provider}", notes::machine_name()),
        );
        file.set("session", session);
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
    let message = if page_binding {
        match &status {
            Some(s) if *s != before => format!("initiative {number}: {s}"),
            _ => format!("initiative {number}: checkpoint"),
        }
    } else {
        match &status {
            Some(s) if *s != before => format!("{:04}: {s}", number.parse::<u32>().unwrap_or(0)),
            _ => format!("{:04}: checkpoint", number.parse::<u32>().unwrap_or(0)),
        }
    };
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
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
    let saved = saved_text(&pulled, notes.spawn_sync());
    let mut head = format!(
        "Saved {record}: status {}{}{}; {saved}.",
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
        }
    );
    if matches!(pulled, Pull::Diverged(_)) {
        head += " Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    for l in notes_lines {
        head += &format!(" {l}");
    }
    if status.as_deref() == Some("done")
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
        head += &format!(" {}", issue_report(checkout.as_deref(), issue));
    }
    head += &unfinished_pieces(&file);
    crate::application::finish(&notes, key, &head);
    Ok(head)
}

fn apply_ticks(file: &mut EffortFile, ticks: &[&str], unticks: &[&str]) -> Result<(), String> {
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
            file.tick(at, true);
        }
        for at in off {
            file.tick(at, false);
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
    key: &str,
    session: &str,
    provider: &str,
    cwd: &Path,
    state: &SessionState,
    target: u32,
    change: &Change<'_>,
    pulled: &Pull,
) -> Result<String, String> {
    let (rel, on_disk, mut file) = target_note(notes, state, target, cwd)?;
    let on_disk_hash = notes::content_hash(&on_disk);
    let current = cmd.body.as_deref().map(as_current).transpose()?;
    let ticks = cmd.flags_all("tick");
    let unticks = cmd.flags_all("untick");
    let delivered = state
        .read_efforts
        .as_ref()
        .and_then(|reads| reads.get(&target.to_string()));
    if (current.is_some() || !ticks.is_empty() || !unticks.is_empty()) && delivered.is_none() {
        return Err(format!(
            "run `astrolabe effort read {target}` first: a body or tick needs the note delivered in this session. Nothing was written.\n{FORMS}"
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
    require_transition_body(&file, change.status, current.as_deref())?;
    if let Some(current) = current {
        file.current = current;
    }
    apply_ticks(&mut file, &ticks, &unticks)?;
    let (day, _) = notes::local_day_time();
    let before = change.apply(&mut file, &day)?;
    file.set("updated", &notes::local_stamp());
    file.set(
        "saved_by",
        &format!("{} · {provider}", notes::machine_name()),
    );
    file.set("session", session);
    let text = file.render();
    let message = match change.status {
        Some(s) if s != before => format!("{target:04}: {s}"),
        _ => format!("{target:04}: checkpoint"),
    };
    let mut next = state.clone();
    if delivered.is_some() {
        next.deliver_effort(target, notes::content_hash(&text));
    }
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
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
        rebaseline: None,
    };
    notes.apply(&op)?;
    let saved = saved_text(pulled, notes.spawn_sync());
    let mut head = format!(
        "Saved effort {target}: status {}; {saved}. This session's {} is unchanged.",
        file.prop("status").unwrap_or("?"),
        state.record_label()
    );
    if matches!(pulled, Pull::Diverged(_)) {
        head += " Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    head += &unfinished_pieces(&file);
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
    if notes.already_applied(key) {
        return Ok("Already written: this same tool call was done before (a retry), so nothing new went in the journal.".to_owned());
    }
    let pulled = crate::application::pull(&notes);
    let (day, _) = notes::local_day_time();
    let op = Op {
        reply_after_ms: None,
        lesson_close: None,
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
        let positional = (i > 0 || first == "journal")
            && !(first == "effort" && args.first() == Some(&"read") && i == 1)
            && !is_flag
            && !wants_value;
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
