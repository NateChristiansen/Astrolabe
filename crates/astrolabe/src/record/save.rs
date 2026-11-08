//! A save's request, checked before the notes lock: everything `effort save` asks for, read
//! from its flags and body, with every refusal that needs no note. Nothing here reads the
//! notes or writes session state.
use super::validation::{
    STATUSES, as_current, check_journal, one_of, shift_plan_headings, split_plan,
};
use crate::effort::{EffortCommand, FORMS};
use crate::session::SessionState;
use std::path::{Path, PathBuf};

/// What one `effort save` asks for, refused already if it breaks a rule that needs no note.
pub(super) struct SaveRequest<'a> {
    /// The bound record's number and its file, from the session state.
    pub number: String,
    pub rel: String,
    /// "effort N" or "initiative N", as replies name the bound record.
    pub record: String,
    /// Bound to an initiative page and not addressing an effort by number.
    pub page_binding: bool,
    pub target: Option<u32>,
    pub status: Option<String>,
    pub decisions: Vec<&'a str>,
    pub waits_for: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub journal_text: Option<&'a str>,
    pub ticks: Vec<&'a str>,
    pub unticks: Vec<&'a str>,
    /// As given; resolved against the pages after the pull.
    pub initiative: Option<&'a str>,
    /// The plan as stored, and whether its headings were shifted.
    pub plan: Option<String>,
    pub plan_shifted: bool,
    pub pieces: Option<String>,
    /// The body's Current state as stored.
    pub current: Option<String>,
}

impl<'a> SaveRequest<'a> {
    /// The request, checked in the order agents see refusals; plan and pieces files are read
    /// relative to `cwd`.
    pub(super) fn parse(
        cmd: &'a EffortCommand,
        state: &SessionState,
        cwd: &Path,
    ) -> Result<Self, String> {
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
        // stays blocked or in review.
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
        Ok(Self {
            number,
            rel,
            record,
            page_binding,
            target,
            status,
            decisions,
            waits_for,
            reason,
            journal_text,
            ticks,
            unticks,
            initiative,
            plan,
            plan_shifted,
            pieces,
            current,
        })
    }
}
