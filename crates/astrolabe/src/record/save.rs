//! A save's request, checked before the notes lock: everything `effort save` asks for, read
//! from its flags and body, with every refusal that needs no note. Nothing here reads the
//! notes or writes session state.
use super::validation::{
    STATUSES, as_current, check_journal, check_wait, one_of, shift_plan_headings, split_plan,
};
use super::{Change, apply_ticks, line_id, saved_text};
use crate::effort::{EffortCommand, EffortFile, FORMS};
use crate::notes::{JournalLine, Pull};
use crate::session::SessionState;
use std::path::{Path, PathBuf};

/// What one `effort save` asks for, refused already if it breaks a rule that needs no note.
#[derive(Default)]
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
    /// A checklist the plan carried under its own Pieces heading, lifted out of it: the
    /// effort's list only when it has none yet, else a copy that's dropped.
    pub plan_pieces: Option<String>,
    /// The body's Current state as stored.
    pub current: Option<String>,
    /// Lessons this save adds to the effort's list, checked for shape; whether each exists is
    /// read after the pull.
    pub lessons: Vec<String>,
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
        let lessons = super::lesson::named(&cmd.flags_all("lesson"))?;
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
                || initiative.is_some()
                || !lessons.is_empty())
        {
            return Err("an initiative has no pieces, ticks, plan replacement, lessons or initiative membership; save its Current state, Decisions, status, wait and journal only. Nothing was written".into());
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
            && lessons.is_empty()
        {
            return Err(format!(
                "nothing to save: give a Current state body, or a flag.\n{FORMS}"
            ));
        }
        if target.is_some()
            && (body_plan.is_some() || plan_file.is_some() || journal_text.is_some())
        {
            return Err(format!(
                "with --effort <n>, --journal and --plan-file are unavailable. Nothing was written.\n{FORMS}"
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
        if let Some(wait) = waits_for {
            check_wait(wait, state.worker.is_true().then_some(record.as_str()))?;
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
        // A pieces file headed with the section's own `## Pieces` line is the list itself:
        // the heading is dropped rather than refused, as the effort file adds it back.
        let pieces = pieces_file
            .map(|p| read_file("pieces-file", p))
            .transpose()?
            .map(|t| without_pieces_heading(&t));
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
                if what == "Current state" && line.trim() == "## Pieces" {
                    return Err("Pieces belong in a separate checklist file. Write it first, then run alone in PowerShell: astrolabe effort save --pieces-file \"pieces.txt\". Keep Current state as paragraphs. Nothing was written".into());
                }
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
        // A plan file often carries its own Pieces checklist. The effort keeps one list, so
        // the plan's is lifted out of it, and becomes the list only for an effort that has
        // none and a save that gives none.
        let (plan, plan_pieces) = match plan.map(|p| lift_plan_pieces(&p)) {
            Some((p, lifted)) => (Some(p), lifted),
            None => (None, None),
        };
        let plan_pieces = plan_pieces.filter(|_| pieces.is_none());
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
            plan_pieces,
            current,
            lessons,
        })
    }
}

impl SaveRequest<'_> {
    /// The state change it makes beside the text, with membership as resolved after the pull.
    pub(super) fn change<'b>(&'b self, initiative: Option<&'b str>) -> Change<'b> {
        Change {
            status: self.status.as_deref(),
            waits_for: self.waits_for,
            reason: self.reason,
            decisions: &self.decisions,
            initiative,
        }
    }

    /// The journal line it writes, if any: only the agent's sentence goes in the owner's
    /// history, one per effort per day, the later replacing the earlier.
    pub(super) fn journal_line(
        &self,
        key: &str,
        machine: &str,
        at_ms: i64,
        day: &str,
    ) -> Option<JournalLine> {
        self.journal_text.map(|j| JournalLine {
            id: line_id(key, 0),
            effort: if self.page_binding {
                format!("initiative:{}", self.number)
            } else {
                format!("{:04}", self.number.parse::<u32>().unwrap_or(0))
            },
            text: j.to_owned(),
            machine: machine.to_owned(),
            at_ms,
            day: day.to_owned(),
            replace: true,
        })
    }
}

/// Who saved and when, as a save stamps it.
pub(super) struct Saver<'a> {
    pub now: &'a str,
    pub machine: &'a str,
    pub provider: &'a str,
    pub session: &'a str,
}

/// The record's text and state edited as the request asks: Current state, plan, pieces,
/// ticks, then the change. Gives the status from before.
pub(super) fn edit_bound(
    file: &mut EffortFile,
    req: &SaveRequest,
    initiative: Option<&str>,
    day: &str,
) -> Result<String, String> {
    if let Some(current) = &req.current {
        file.current = current.clone();
    }
    if let Some(plan) = &req.plan {
        file.plan = Some(plan.trim().to_owned());
    }
    if let Some(p) = &req.pieces {
        keeps_ticked(file, p)?;
        // A piece that comes after the plan says so. A save that carries a plan is the plan,
        // so its new pieces aren't tagged, though a piece already tagged keeps its tag; a
        // first list is the one the work starts from.
        let p = if file.pieces().is_some() {
            file.with_added_tags(p, req.plan.is_none().then_some(day))
        } else {
            p.clone()
        };
        file.set_pieces(&p);
    } else if let Some(p) = req.plan_pieces.as_ref().filter(|_| file.pieces().is_none()) {
        file.set_pieces(p);
    }
    apply_ticks(file, &req.ticks, &req.unticks, day)?;
    super::lesson::add_to(file, &req.lessons);
    req.change(initiative).apply(file, day)
}

/// The plan without the checklist under its own Pieces heading, and that checklist when it
/// has one: a heading named Pieces at any level, and the checkbox lines in its own text,
/// before any deeper heading. Everything else in the section stays in the plan where it was,
/// under its heading; the heading goes only when nothing else is left under it.
fn lift_plan_pieces(plan: &str) -> (String, Option<String>) {
    let lines: Vec<&str> = plan.split('\n').collect();
    let fenced = crate::effort::fence_mask(lines.iter().copied());
    fn level(l: &str) -> Option<(usize, &str)> {
        let n = l.bytes().take_while(|b| *b == b'#').count();
        (n > 0 && l[n..].starts_with(' ')).then(|| (n, l[n..].trim()))
    }
    let heads: Vec<Option<(usize, &str)>> = lines
        .iter()
        .zip(&fenced)
        .map(|(l, f)| if *f { None } else { level(l) })
        .collect();
    let Some((at, n)) = heads.iter().enumerate().find_map(|(i, h)| {
        h.filter(|(_, t)| t.eq_ignore_ascii_case("pieces"))
            .map(|(n, _)| (i, n))
    }) else {
        return (plan.to_owned(), None);
    };
    let end = (at + 1..lines.len())
        .find(|i| heads[*i].is_some_and(|(m, _)| m <= n))
        .unwrap_or(lines.len());
    // The section's own text runs to its first deeper heading.
    let own_end = (at + 1..end).find(|i| heads[*i].is_some()).unwrap_or(end);
    let is_box = |i: usize| {
        let l = lines[i].trim_start();
        !fenced[i]
            && (l.starts_with("- [ ] ") || l.starts_with("- [x] ") || l.starts_with("- [X] "))
    };
    let lifted: Vec<&str> = (at + 1..own_end)
        .filter(|&i| is_box(i))
        .map(|i| lines[i])
        .collect();
    if lifted.is_empty() {
        return (plan.to_owned(), None);
    }
    let kept: Vec<&str> = (at + 1..end)
        .filter(|&i| !(i < own_end && is_box(i)))
        .map(|i| lines[i])
        .collect();
    let mut out: Vec<&str> = lines[..at].to_vec();
    if kept.iter().any(|l| !l.trim().is_empty()) {
        out.push(lines[at]);
        // Lines taken out of a run of text leave no doubled blank line behind.
        for line in kept {
            if line.trim().is_empty() && out.last().is_some_and(|l| l.trim().is_empty()) {
                continue;
            }
            out.push(line);
        }
    }
    out.extend(&lines[end..]);
    let lifted = lifted
        .iter()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");
    (
        out.join("\n").trim().to_owned(),
        Some(lifted.trim().to_owned()),
    )
}

fn without_pieces_heading(text: &str) -> String {
    let trimmed = text.trim_start();
    match trimmed.split_once('\n') {
        Some((first, rest)) if first.trim().eq_ignore_ascii_case("## pieces") => rest.to_owned(),
        None if trimmed.trim().eq_ignore_ascii_case("## pieces") => String::new(),
        _ => text.to_owned(),
    }
}

/// A new list replaces the old only with every ticked outcome still there and still ticked:
/// a met outcome is history, and only `--untick` reverses one.
fn keeps_ticked(file: &EffortFile, new: &str) -> Result<(), String> {
    let mut next = EffortFile::default();
    next.set_pieces(new);
    let kept: Vec<String> = next
        .ticked_labels()
        .iter()
        .map(|l| l.to_lowercase())
        .collect();
    let lost: Vec<String> = file
        .ticked_labels()
        .into_iter()
        .filter(|l| !kept.contains(&l.to_lowercase()))
        .map(|l| format!("- [x] {l}"))
        .collect();
    if lost.is_empty() {
        return Ok(());
    }
    Err(format!(
        "the new Pieces list loses ticked outcomes, which stay in the list as met; keep them ticked, or reverse one with --untick \"<piece>\" first. Nothing was written. Missing or unticked:\n{}",
        lost.join("\n")
    ))
}

/// Who saved it and when.
pub(super) fn stamp_saver(file: &mut EffortFile, saver: &Saver) {
    file.set("updated", saver.now);
    file.set(
        "saved_by",
        &format!("{} · {}", saver.machine, saver.provider),
    );
    file.set("session", saver.session);
}

/// An initiative page's stamp: the properties a page needs, filled only where missing, then
/// who saved it.
pub(super) fn stamp_initiative(file: &mut EffortFile, repo: &str, day: &str, saver: &Saver) {
    for (key, value) in [
        ("status", "ready"),
        ("repo", repo),
        ("involvement", "check in at plan"),
        ("authority", "edit"),
        ("created", day),
    ] {
        if file.prop(key).is_none() {
            file.set(key, value);
        }
    }
    stamp_saver(file, saver);
}

/// The save's commit subject: the new status when it changed, else a checkpoint.
pub(super) fn commit_message(
    page: bool,
    number: &str,
    status: Option<&str>,
    before: &str,
) -> String {
    let name = if page {
        format!("initiative {number}")
    } else {
        format!("{:04}", number.parse::<u32>().unwrap_or(0))
    };
    match status {
        Some(s) if s != before => format!("{name}: {s}"),
        _ => format!("{name}: checkpoint"),
    }
}

/// What a save's reply reports, gathered by the coordinator once the write is done.
pub(super) struct Reply<'a> {
    /// "effort N" or "initiative N".
    pub record: &'a str,
    pub status: &'a str,
    pub body_replaced: bool,
    pub journal_written: bool,
    pub pulled: &'a Pull,
    pub sync_started: bool,
    /// For a save by number, the session's own record, which it left alone.
    pub unchanged: Option<&'a str>,
    pub notes_lines: &'a [String],
    pub issue: Option<&'a str>,
    pub unfinished: &'a str,
}

/// The reply: status and what was written, the pull and sync, divergence, plan notes, the
/// issue, then unfinished pieces.
pub(super) fn saved_reply(r: &Reply) -> String {
    let saved = saved_text(r.pulled, r.sync_started);
    let mut head = format!(
        "Saved {}: status {}{}{}; {saved}.",
        r.record,
        r.status,
        if r.body_replaced {
            ", Current state replaced"
        } else {
            ""
        },
        if r.journal_written {
            ", journal line written"
        } else {
            ""
        }
    );
    if let Some(label) = r.unchanged {
        head += &format!(" This session's {label} is unchanged.");
    }
    if matches!(r.pulled, Pull::Diverged(_)) {
        head += " Notes diverged from origin; resolve in the notes checkout when you can.";
    }
    for l in r.notes_lines {
        head += &format!(" {l}");
    }
    if let Some(issue) = r.issue {
        head += &format!(" {issue}");
    }
    head += r.unfinished;
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(props: &[(&str, &str)]) -> EffortFile {
        let mut f = EffortFile::default();
        for (k, v) in props {
            f.set(k, v);
        }
        f
    }

    #[test]
    fn a_commit_names_a_status_change_or_a_checkpoint() {
        assert_eq!(
            commit_message(false, "7", Some("done"), "active"),
            "0007: done"
        );
        assert_eq!(
            commit_message(false, "7", Some("active"), "active"),
            "0007: checkpoint"
        );
        assert_eq!(
            commit_message(false, "128", None, "active"),
            "0128: checkpoint"
        );
        assert_eq!(
            commit_message(true, "simplify", Some("done"), "active"),
            "initiative simplify: done"
        );
        assert_eq!(
            commit_message(true, "simplify", None, ""),
            "initiative simplify: checkpoint"
        );
    }

    #[test]
    fn initiative_defaults_fill_only_missing_properties() {
        let saver = Saver {
            now: "2026-11-08T10:00-07:00",
            machine: "mac",
            provider: "agent",
            session: "s1",
        };
        let mut page = file(&[("status", "active"), ("authority", "push")]);
        stamp_initiative(&mut page, "demo", "2026-11-08", &saver);
        for (key, value) in [
            ("status", "active"),
            ("authority", "push"),
            ("repo", "demo"),
            ("involvement", "check in at plan"),
            ("created", "2026-11-08"),
            ("updated", "2026-11-08T10:00-07:00"),
            ("saved_by", "mac · agent"),
            ("session", "s1"),
        ] {
            assert_eq!(page.prop(key), Some(value), "{key}");
        }
    }

    #[test]
    fn a_save_clears_the_wait_unless_it_stays_blocked_or_in_review() {
        let save = |status: &str, req: SaveRequest| {
            let mut f = file(&[("status", status), ("waits_for", "owner: look")]);
            let before = edit_bound(&mut f, &req, None, "2026-11-08").unwrap();
            assert_eq!(before, status);
            f.prop("waits_for").unwrap_or("").to_owned()
        };
        assert_eq!(save("blocked", SaveRequest::default()), "owner: look");
        assert_eq!(save("review", SaveRequest::default()), "owner: look");
        assert_eq!(save("active", SaveRequest::default()), "");
        let moving_on = SaveRequest {
            status: Some("active".to_owned()),
            ..Default::default()
        };
        assert_eq!(save("blocked", moving_on), "");
        let new_wait = SaveRequest {
            waits_for: Some("CI"),
            ..Default::default()
        };
        assert_eq!(save("active", new_wait), "CI");
    }

    #[test]
    fn a_closed_record_takes_no_wait() {
        let save = |status: &str, req: SaveRequest| {
            let mut f = file(&[("status", status), ("waits_for", "")]);
            edit_bound(&mut f, &req, None, "2026-11-08").map(|_| {
                (
                    f.prop("status").unwrap_or("").to_owned(),
                    f.prop("waits_for").unwrap_or("").to_owned(),
                )
            })
        };
        let wait = |status: Option<&str>| SaveRequest {
            status: status.map(str::to_owned),
            waits_for: Some("owner: three picks"),
            ..Default::default()
        };
        // 106's save: already done, a wait added with no --status.
        let refused = save("done", wait(None)).unwrap_err();
        assert!(
            refused.contains("a done record waits on nothing"),
            "{refused}"
        );
        assert!(save("abandoned", wait(None)).is_err());
        // Closing and waiting in one save.
        assert!(save("active", wait(Some("done"))).is_err());
        // Reopening with a wait, or closing without one, goes through.
        assert_eq!(
            save("done", wait(Some("blocked"))).unwrap(),
            ("blocked".to_owned(), "owner: three picks".to_owned())
        );
        let closing = SaveRequest {
            status: Some("done".to_owned()),
            waits_for: Some(""),
            ..Default::default()
        };
        assert_eq!(
            save("review", closing).unwrap(),
            ("done".to_owned(), String::new())
        );
    }

    #[test]
    fn a_reply_reports_in_a_fixed_order() {
        let notes = ["The plan's headings were shifted.".to_owned()];
        let diverged = Pull::Diverged("x".to_owned());
        let reply = Reply {
            record: "effort 3",
            status: "done",
            body_replaced: true,
            journal_written: true,
            pulled: &diverged,
            sync_started: true,
            unchanged: None,
            notes_lines: &notes,
            issue: Some("This effort answers issue #5."),
            unfinished: "\nUnticked pieces:\n- [ ] a",
        };
        assert_eq!(
            saved_reply(&reply),
            "Saved effort 3: status done, Current state replaced, journal line written; committed, background sync started. Notes diverged from origin; resolve in the notes checkout when you can. The plan's headings were shifted. This effort answers issue #5.\nUnticked pieces:\n- [ ] a"
        );
        let offline = Pull::Offline("no remote".to_owned());
        let by_number = Reply {
            record: "effort 4",
            status: "active",
            body_replaced: false,
            journal_written: false,
            pulled: &offline,
            sync_started: false,
            unchanged: Some("effort 3"),
            notes_lines: &[],
            issue: None,
            unfinished: "",
        };
        assert_eq!(
            saved_reply(&by_number),
            "Saved effort 4: status active; saved locally; sync unavailable: no remote configured. This session's effort 3 is unchanged."
        );
        let quiet = Reply {
            pulled: &Pull::Pulled,
            ..by_number
        };
        assert_eq!(
            saved_reply(&quiet),
            "Saved effort 4: status active; committed; background sync could not start. This session's effort 3 is unchanged."
        );
    }
}
