//! Git's merge driver for records (effort notes and initiative pages), so two machines' saves
//! of one record don't strand a pull as diverged.
//!
//! Every save rewrites the stamp lines at the top of a record (`updated`, `saved_by`,
//! `session`…). They sit together, so git's line merge conflicted whenever two machines saved
//! one record between pulls, whatever else they changed. This merges the raw text, never a
//! parsed and re-rendered record, so nothing a parser drops can be lost:
//!
//! - properties merge one key at a time; the stamp keys move together from the later save;
//! - the body merges three ways by line, and where both sides only added lines at one place,
//!   both are kept, the earlier save's first;
//! - only where both sides are program saves (each changed `updated`) are both decided
//!   between. Decisions and Pieces both rewrote merge entry by entry, keeping every entry and
//!   every tick either side made. Elsewhere the later save wins where both changed the same
//!   lines, and what it replaced from the earlier save goes in the record's `## Unmerged
//!   state` section, committed with it, so every machine shows the same text until a save
//!   with a Current state replaces it.
//!
//! Anything else (a hand edit, or an autostash restoring one, changing the same lines as a
//! save) falls back to git's own line merge, conflict markers and all, as before.
use std::path::Path;
use std::process::{Command, Stdio};

/// The property older builds marked a record with when its two saves overlapped: still read,
/// carried through a merge and dropped by a save, but never written now.
pub const UNSEEN: &str = "unseen";

/// The section that keeps what the later of two saves replaced from the earlier one, so it's
/// in the note and not only in git. A save with a new Current state replaces it.
pub const UNMERGED: &str = "Unmerged state";

/// Who saved a record, when, and the work's facts then: where both saves changed one, the
/// later save's stands, and that's no overlap.
const STAMP: [&str; 8] = [
    "updated", "saved_by", "session", "branch", "head", "unpushed", "dirty", "runs_on",
];

/// Wide markers, so a record's own text never reads as one.
const MARKER: usize = 31;

/// The driver: `astrolabe notes merge-file <base> <ours> <theirs> <path>`. The result goes to
/// `ours`; the exit code is 0 when clean and 1 when git's conflict markers were left there.
pub fn run(base: &Path, ours: &Path, theirs: &Path, path: &str) -> i32 {
    let read = |p: &Path| std::fs::read_to_string(p).map(|t| t.replace("\r\n", "\n"));
    if let (Ok(b), Ok(o), Ok(t)) = (read(base), read(ours), read(theirs)) {
        let saves = Saves {
            ours: committed_by_program("HEAD", path, &o),
            theirs: replayed(path, &t),
        };
        if let Some(merged) = merge_record(&b, &o, &t, saves)
            && std::fs::write(ours, merged).is_ok()
        {
            return 0;
        }
    }
    // As git would have merged it without this driver.
    let status = Command::new("git")
        .args(["merge-file", "-L", "ours", "-L", "base", "-L", "theirs"])
        .arg(ours)
        .arg(base)
        .arg(theirs)
        .stdin(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => 0,
        _ => 1,
    }
}

/// Whether each side is known, from git, to be a version a program save committed.
#[derive(Clone, Copy, Default, Debug)]
pub struct Saves {
    pub ours: bool,
    pub theirs: bool,
}

/// Whether `text` is what the last commit to touch `path` from `rev` holds, and that commit
/// is one the program made (it carries an `Op-Key` trailer).
fn committed_by_program(rev: &str, path: &str, text: &str) -> bool {
    let Some(sha) = git_out(&["log", "-1", "--format=%H", rev, "--", path]) else {
        return false;
    };
    program_version(sha.trim(), path, text)
}

/// Whether `text` is the version of `path` in the commit a rebase is replaying, and the
/// program made that commit. During a replay git hands the driver that commit's own copy;
/// while it puts an autostash back, the copy is the hand-edited one, which isn't.
fn replayed(path: &str, text: &str) -> bool {
    let Some(done) = git_out(&["rev-parse", "--git-path", "rebase-merge/done"]) else {
        return false;
    };
    let Ok(done) = std::fs::read_to_string(done.trim()) else {
        return false;
    };
    let Some(sha) = done.lines().rev().find_map(|l| {
        l.strip_prefix("pick ")
            .and_then(|r| r.split_whitespace().next())
    }) else {
        return false;
    };
    program_version(sha, path, text)
}

fn program_version(sha: &str, path: &str, text: &str) -> bool {
    // The record the save was for, by its subject: a hand edit another save swept into its
    // commit is still a hand edit.
    let made = git_out(&[
        "log",
        "-1",
        "--format=%s%x1f%(trailers:key=Op-Key,valueonly)",
        sha,
    ])
    .and_then(|out| {
        let (subject, key) = out.split_once('\u{1f}')?;
        (!key.trim().is_empty()).then(|| saved_for(subject, path))
    })
    .unwrap_or(false);
    made && git_out(&["show", &format!("{sha}:{path}")])
        .is_some_and(|there| there.replace("\r\n", "\n") == text)
}

/// Whether a save's commit subject (`0356: active`, `initiative deck-frame-time:
/// checkpoint`) names the record at `path`.
fn saved_for(subject: &str, path: &str) -> bool {
    let Some((name, _)) = subject.split_once(':') else {
        return false;
    };
    let file = path.rsplit('/').next().unwrap_or_default();
    match name.strip_prefix("initiative ") {
        Some(slug) => path == format!("initiatives/{slug}.md"),
        None => {
            name.len() == 4
                && name.bytes().all(|b| b.is_ascii_digit())
                && path.starts_with("efforts/")
                && file.starts_with(&format!("{name}-"))
        }
    }
}

fn git_out(args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Which side a choice falls to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Ours,
    Theirs,
}

/// One property: its key and its lines, a list's `- item` lines included.
type Prop = (String, Vec<String>);

struct Record<'a> {
    props: Vec<Prop>,
    body: &'a str,
}

impl<'a> Record<'a> {
    /// A record with a closed header, or None: one that isn't is merged by git.
    fn read(text: &'a str) -> Option<Self> {
        let bounds = crate::effort::header_bounds(text)?;
        if !bounds.closed() {
            return None;
        }
        let mut props: Vec<Prop> = Vec::new();
        for line in text[bounds.start..bounds.end].lines() {
            let top = !line.starts_with([' ', '\t', '-']) && line.contains(':');
            match props.last_mut() {
                Some((_, lines)) if !top => lines.push(line.to_owned()),
                None if !top => return None,
                _ => {
                    let key = line.split(':').next().unwrap_or_default().trim();
                    props.push((key.to_ascii_lowercase(), vec![line.to_owned()]));
                }
            }
        }
        Some(Self {
            props,
            body: &text[bounds.body..],
        })
    }

    fn prop(&self, key: &str) -> Option<&Vec<String>> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// A property's plain value, unquoted.
    fn value(&self, key: &str) -> Option<String> {
        let line = self.prop(key)?.first()?;
        let value = line.split_once(':')?.1.trim();
        Some(value.trim_matches('"').trim_matches('\'').trim().to_owned())
    }
}

/// The merged record, or None to leave it to git's line merge.
pub fn merge_record(base: &str, ours: &str, theirs: &str, saves: Saves) -> Option<String> {
    let (b, o, t) = (
        Record::read(base)?,
        Record::read(ours)?,
        Record::read(theirs)?,
    );
    // A save rewrites the stamp; one made in the same second as the base by the same session
    // doesn't change it, so the commit it came in is asked too.
    let restamped = |r: &Record| {
        r.value("updated")
            .is_some_and(|u| Some(u) != b.value("updated"))
    };
    // Only two program saves are ever decided between; a hand edit never loses.
    let decidable = (saves.ours || restamped(&o)) && (saves.theirs || restamped(&t));
    let later = later(&o, &t);
    let earlier = match later {
        Side::Ours => Side::Theirs,
        Side::Theirs => Side::Ours,
    };
    // What the earlier save had where the later one won, kept in the note.
    let mut lost: Vec<String> = Vec::new();
    let mut lost_props: Vec<String> = Vec::new();

    let mut keys: Vec<&str> = o.props.iter().map(|(k, _)| k.as_str()).collect();
    for (k, _) in &t.props {
        if !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }
    let mut header: Vec<String> = Vec::new();
    for key in keys {
        if key == UNSEEN {
            continue;
        }
        let (bv, ov, tv) = (b.prop(key), o.prop(key), t.prop(key));
        // A key one side changed is that side's; one both changed is the later save's. The
        // stamp comes whole from the later save, even a key it left as the base had it, or a
        // save after the base by its own machine would read as the other machine's.
        let chosen = if decidable && STAMP.contains(&key) {
            pick(later, ov, tv).or(pick(earlier, ov, tv))
        } else if ov == tv || tv == bv {
            ov
        } else if ov == bv {
            tv
        } else if decidable {
            lost_props.extend(pick(earlier, ov, tv).into_iter().flatten().cloned());
            pick(later, ov, tv)
        } else {
            return None;
        };
        if let Some(lines) = chosen {
            header.extend(lines.iter().cloned());
        }
    }
    if !lost_props.is_empty() {
        lost.push(part("Its properties", &lost_props.join("\n")));
    }
    // An older build's overlap mark stays until a save drops it.
    let (bv, ov, tv) = (b.prop(UNSEEN), o.prop(UNSEEN), t.prop(UNSEEN));
    let kept = if ov == tv || tv == bv {
        ov
    } else if ov == bv {
        tv
    } else {
        pick(later, ov, tv)
    };
    header.extend(kept.into_iter().flatten().cloned());

    let (ob, tb) = match decidable {
        true => settle(b.body, o.body, t.body, later, &mut lost),
        false => (o.body.to_owned(), t.body.to_owned()),
    };
    let (mut body, blocks) = merge_body(b.body, &ob, &tb, decidable.then_some(later))?;
    for block in blocks {
        lost.push(part("Its text the later save replaced", &block));
    }
    if !lost.is_empty() {
        let who = |r: &Record| {
            format!(
                "{} at {}",
                r.value("saved_by")
                    .unwrap_or_else(|| "another save".to_owned()),
                r.value("updated").unwrap_or_default()
            )
        };
        let (e, l) = match later {
            Side::Ours => (&t, &o),
            Side::Theirs => (&o, &t),
        };
        let intro = format!(
            "{} and {} saved this note without seeing each other's save. The later save stayed; below is what the merge didn't keep, the earlier save's unless a part says otherwise. The next save with a Current state replaces this section, so fold in what still holds.",
            who(e),
            who(l)
        );
        body = with_unmerged(&body, &intro, &lost);
    }

    let mut out = String::from("---\n");
    for line in header {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&body);
    // A fence opened by the merge, which neither side had, would swallow the rest.
    let open = |text: &str| !fences(text).is_multiple_of(2);
    if open(&out) && !open(ours) && !open(theirs) {
        return None;
    }
    // Nor one the program can no longer read, such as a managed section written twice.
    let reads = |text: &str| crate::effort::EffortFile::parse(text).is_ok();
    if !reads(&out) && reads(ours) && reads(theirs) {
        return None;
    }
    Some(out)
}

fn pick<'a>(
    side: Side,
    ours: Option<&'a Vec<String>>,
    theirs: Option<&'a Vec<String>>,
) -> Option<&'a Vec<String>> {
    match side {
        Side::Ours => ours,
        Side::Theirs => theirs,
    }
}

/// The later of two saves by `updated` with its offset, to the second (an older stamp to the
/// minute reads as its minute's start); for equal stamps, the larger `saved_by`, then the
/// larger text, so every machine picks the same side.
fn later(o: &Record, t: &Record) -> Side {
    let at = |r: &Record| {
        r.value("updated")
            .and_then(|u| crate::notes::stamp_time(&u))
    };
    let key = |r: &Record| (at(r), r.value("saved_by"), r.body.to_owned());
    if key(t) > key(o) {
        Side::Theirs
    } else {
        Side::Ours
    }
}

fn fences(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            let l = l.trim_start();
            l.starts_with("```") || l.starts_with("~~~")
        })
        .count()
}

/// The sections both saves rewrote, settled before the line merge and written into both sides
/// alike, so the line merge takes them as they are. Decisions and Pieces are merged entry by
/// entry; Current state is the later save's, and the earlier one's goes to `lost`.
fn settle(
    base: &str,
    ours: &str,
    theirs: &str,
    later: Side,
    lost: &mut Vec<String>,
) -> (String, String) {
    let (mut o, mut t) = (ours.to_owned(), theirs.to_owned());
    for name in ["Current state", "Decisions", "Pieces"] {
        let (Some(or), Some(tr)) = (section(&o, name), section(&t, name)) else {
            continue;
        };
        let bs = section(base, name).map_or("", |r| &base[r]);
        let (os, ts) = (o[or.clone()].to_owned(), t[tr.clone()].to_owned());
        if os == ts || os == bs || ts == bs {
            continue;
        }
        let (es, ls) = match later {
            Side::Ours => (&ts, &os),
            Side::Theirs => (&os, &ts),
        };
        let merged = if name == "Current state" {
            lost.push(part("Its Current state", es));
            ls.clone()
        } else {
            let Some((merged, dropped)) = merge_list(bs, es, ls, name == "Pieces") else {
                continue;
            };
            let lines = |later: bool| {
                let lines: Vec<&str> = dropped
                    .iter()
                    .filter(|(l, _)| *l == later)
                    .map(|(_, t)| t.as_str())
                    .collect();
                lines.join("\n")
            };
            for (later, title) in [
                (false, format!("Its {name} lines the later save replaced")),
                (
                    true,
                    format!("The later save's {name} lines a tick from the earlier save kept out"),
                ),
            ] {
                let text = lines(later);
                if !text.is_empty() {
                    lost.push(part(&title, &text));
                }
            }
            merged
        };
        o.replace_range(or, &merged);
        t.replace_range(tr, &merged);
    }
    (o, t)
}

/// Where a `## <name>` section's text lies in a body: after its heading, up to the next
/// heading outside a fence.
fn section(body: &str, name: &str) -> Option<std::ops::Range<usize>> {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let fenced = crate::effort::fence_mask(lines.iter().map(|l| l.trim_end_matches(['\r', '\n'])));
    let (mut at, mut start) = (0, None);
    for (line, fence) in lines.iter().zip(fenced) {
        if !fence && let Some(heading) = line.strip_prefix("## ") {
            if let Some(s) = start {
                return Some(s..at);
            }
            if heading.trim() == name {
                start = Some(at + line.len());
            }
        }
        at += line.len();
    }
    start.map(|s| s..body.len())
}

/// What one save did to an entry of the base list.
#[derive(Clone, PartialEq, Debug)]
enum Fate {
    Same,
    Changed(String),
    /// Gone, with a new entry in its place: the new one's key and text.
    Reworded(String, String),
    Gone,
}

/// Where an entry goes in the merged list: a base entry's place, or a new one's.
#[derive(Clone, PartialEq, Debug)]
enum Slot {
    Base(usize),
    Added(String),
}

/// A Decisions or Pieces list merged entry by entry, so neither save's entries go with a
/// block the other won: every entry either side added is kept, earlier save's first; a piece
/// ticked on either side stays ticked; where both changed one entry differently the later
/// save's stands. What either side loses comes back, with whether it was the later save's (a
/// tick from the earlier save kept it out), to be kept in the note. Entries are `- `
/// lines with any lines indented under them, matched by their text, or a piece by its words.
/// None for a list holding anything else, or one entry twice: the line merge takes those.
fn merge_list(
    base: &str,
    earlier: &str,
    later: &str,
    pieces: bool,
) -> Option<(String, Vec<(bool, String)>)> {
    let key = |item: &str| match pieces {
        true => crate::effort::piece_key(item.lines().next()?),
        false => Some(item.to_owned()),
    };
    let read = |text: &str| -> Option<Vec<(String, String)>> {
        let mut items: Vec<String> = Vec::new();
        for line in text.trim().lines() {
            if line.starts_with("- ") {
                items.push(line.to_owned());
            } else if line.starts_with([' ', '\t']) && !line.trim().is_empty() {
                let item = items.last_mut()?;
                item.push('\n');
                item.push_str(line);
            } else {
                return None;
            }
        }
        let keyed: Vec<(String, String)> = items
            .into_iter()
            .map(|i| Some((key(&i)?, i)))
            .collect::<Option<_>>()?;
        let unique = keyed
            .iter()
            .enumerate()
            .all(|(n, (k, _))| keyed[..n].iter().all(|(x, _)| x != k));
        unique.then_some(keyed)
    };
    let (b, e, l) = (read(base)?, read(earlier)?, read(later)?);
    let in_base = |k: &str| b.iter().position(|(x, _)| x == k);
    // Each side's fate for each base entry, and the entries it added. A new entry in place
    // of a base one that's gone is a rewording, paired in order.
    let fates = |side: &[(String, String)]| {
        let mut new: std::collections::VecDeque<&(String, String)> =
            side.iter().filter(|(k, _)| in_base(k).is_none()).collect();
        let fates: Vec<Fate> = b
            .iter()
            .map(|(k, text)| match side.iter().find(|(x, _)| x == k) {
                Some((_, t)) if t == text => Fate::Same,
                Some((_, t)) => Fate::Changed(t.clone()),
                None => match new.pop_front() {
                    Some((nk, nt)) => Fate::Reworded(nk.clone(), nt.clone()),
                    None => Fate::Gone,
                },
            })
            .collect();
        (fates, new.into_iter().cloned().collect::<Vec<_>>())
    };
    let ((fe, added_e), (fl, added_l)) = (fates(&e), fates(&l));
    let ticked = |text: &str| pieces && crate::effort::piece_box(text) == Some(true);
    let mut dropped = Vec::new();
    let mut resolved: Vec<(Slot, Option<String>)> = Vec::new();
    for (i, (_, text)) in b.iter().enumerate() {
        let text_of = |f: &Fate| match f {
            Fate::Same => Some(text.clone()),
            Fate::Changed(t) | Fate::Reworded(_, t) => Some(t.clone()),
            Fate::Gone => None,
        };
        let (e, l) = (&fe[i], &fl[i]);
        let won = if e == l || *e == Fate::Same {
            l
        } else if *l == Fate::Same {
            e
        } else {
            // Both changed it, differently: a tick is never lost, and otherwise the later
            // save's stands.
            let tick = |f: &Fate| text_of(f).is_some_and(|t| ticked(&t));
            let (won, other, later) = match tick(e) && !tick(l) {
                true => (e, l, true),
                false => (l, e, false),
            };
            let lost = text_of(other).unwrap_or_else(|| format!("removed: {text}"));
            dropped.push((later, lost));
            won
        };
        resolved.push((Slot::Base(i), text_of(won)));
    }
    for (k, lt) in &added_l {
        let text = match added_e.iter().find(|(x, _)| x == k) {
            Some((_, et)) if et != lt && ticked(et) && !ticked(lt) => {
                dropped.push((true, lt.clone()));
                et
            }
            Some((_, et)) if et != lt => {
                dropped.push((false, et.clone()));
                lt
            }
            _ => lt,
        };
        resolved.push((Slot::Added(k.clone()), Some(text.clone())));
    }
    for (k, et) in &added_e {
        if !added_l.iter().any(|(x, _)| x == k) {
            resolved.push((Slot::Added(k.clone()), Some(et.clone())));
        }
    }
    let slot_of = |k: &str, fates: &[Fate]| match in_base(k) {
        Some(i) => Slot::Base(i),
        None => fates
            .iter()
            .position(|f| matches!(f, Fate::Reworded(nk, _) if nk == k))
            .map_or(Slot::Added(k.to_owned()), Slot::Base),
    };
    let text_at = |s: &Slot| {
        resolved
            .iter()
            .find(|(x, _)| x == s)
            .and_then(|(_, t)| t.clone())
    };
    // The later save's order, with the earlier save's entries placed after the entry they
    // followed there.
    let mut out: Vec<(Slot, String)> = Vec::new();
    for (k, _) in &l {
        let slot = slot_of(k, &fl);
        if let Some(text) = text_at(&slot)
            && !out.iter().any(|(s, _)| *s == slot)
        {
            out.push((slot, text));
        }
    }
    let mut at = 0;
    for (k, _) in &e {
        let slot = slot_of(k, &fe);
        if let Some(n) = out.iter().position(|(s, _)| *s == slot) {
            at = n + 1;
        } else if let Some(text) = text_at(&slot) {
            out.insert(at, (slot, text));
            at += 1;
        }
    }
    for (slot, text) in &resolved {
        if let Some(text) = text
            && !out.iter().any(|(s, _)| s == slot)
        {
            out.push((slot.clone(), text.clone()));
        }
    }
    // One entry per key, where a rewording on one side matches an addition on the other.
    let mut keys: Vec<String> = Vec::new();
    out.retain(|(_, text)| {
        let k = key(text).unwrap_or_else(|| text.clone());
        let new = !keys.contains(&k);
        keys.push(k);
        new
    });
    let lead = &later[..later.len() - later.trim_start().len()];
    let trail = &later[later.trim_end().len()..];
    let items: Vec<String> = out.into_iter().map(|(_, t)| t).collect();
    let trail = if trail.is_empty() && !items.is_empty() {
        "\n"
    } else {
        trail
    };
    Some((format!("{lead}{}{trail}", items.join("\n")), dropped))
}

/// A titled part of the earlier save's text, quoted so its headings and fences stay text.
fn part(title: &str, text: &str) -> String {
    let quoted: Vec<String> = text
        .trim()
        .lines()
        .map(|l| match l.trim().is_empty() {
            true => ">".to_owned(),
            false => format!("> {l}"),
        })
        .collect();
    format!("{title}:\n{}", quoted.join("\n"))
}

/// The body with this merge's entry added to its Unmerged state section, made at the end if
/// there's none yet.
fn with_unmerged(body: &str, intro: &str, parts: &[String]) -> String {
    let entry = format!("{intro}\n\n{}", parts.join("\n\n"));
    let mut out = body.to_owned();
    match section(&out, UNMERGED) {
        Some(r) => {
            let kept = out[r.clone()].trim_end().len();
            let gap = if kept == 0 { "" } else { "\n\n" };
            out.insert_str(r.start + kept, &format!("{gap}{entry}"));
        }
        None => {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("\n## {UNMERGED}\n{entry}\n"));
        }
    }
    out
}

/// The body merged three ways by `git merge-file`. A conflict where both sides only added
/// lines keeps both; one where both changed lines takes `later`'s, when given, and returns
/// the earlier side's text there. None when a conflict is left with no side to take.
fn merge_body(
    base: &str,
    ours: &str,
    theirs: &str,
    later: Option<Side>,
) -> Option<(String, Vec<String>)> {
    let dir = scratch()?;
    let write = |name: &str, text: &str| {
        let path = dir.join(name);
        std::fs::write(&path, text).ok().map(|()| path)
    };
    let (b, o, t) = (
        write("base", base)?,
        write("ours", ours)?,
        write("theirs", theirs)?,
    );
    let out = Command::new("git")
        .args([
            "merge-file",
            "-p",
            "--diff3",
            &format!("--marker-size={MARKER}"),
        ])
        .args(["-L", "ours", "-L", "base", "-L", "theirs"])
        .arg(&o)
        .arg(&b)
        .arg(&t)
        .stdin(Stdio::null())
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let out = out.ok()?;
    // 0 is clean, a positive count is conflicts, and anything else is an error.
    let code = out.status.code()?;
    if !(0..=127).contains(&code) {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    if code == 0 {
        return Some((text, Vec::new()));
    }
    resolve(&text, later)
}

/// Settles the conflict blocks `git merge-file --diff3` left in `text`.
fn resolve(text: &str, later: Option<Side>) -> Option<(String, Vec<String>)> {
    let open = format!("{} ours", "<".repeat(MARKER));
    let mid = format!("{} base", "|".repeat(MARKER));
    let split = "=".repeat(MARKER);
    let close = format!("{} theirs", ">".repeat(MARKER));
    let mut out = String::new();
    let mut lost = Vec::new();
    let mut lines = text.split_inclusive('\n');
    while let Some(line) = lines.next() {
        if line.trim_end_matches(['\n', '\r']) != open {
            out.push_str(line);
            continue;
        }
        let (mut o, mut b, mut t) = (String::new(), String::new(), String::new());
        let mut part = 0;
        loop {
            let line = lines.next()?;
            let bare = line.trim_end_matches(['\n', '\r']);
            if bare == mid {
                part = 1;
            } else if bare == split {
                part = 2;
            } else if bare == close {
                break;
            } else {
                match part {
                    0 => o.push_str(line),
                    1 => b.push_str(line),
                    _ => t.push_str(line),
                }
            }
        }
        // The earlier save's side, then the later's; ours first when neither is known.
        let (first, second) = match later {
            Some(Side::Ours) => (&t, &o),
            _ => (&o, &t),
        };
        if o == t {
            out.push_str(&o);
        } else if b.trim().is_empty() {
            // Both only added here: what they added alike once (a new section's heading
            // both wrote, say), then each one's own lines.
            out.push_str(&both_added(first, second));
        } else {
            later?;
            out.push_str(second);
            if !first.trim().is_empty() {
                lost.push(first.clone());
            }
        }
    }
    Some((out, lost))
}

/// Two additions at one place: their common first and last lines once, the rest of each
/// between, `o`'s first.
fn both_added(o: &str, t: &str) -> String {
    let (o, t): (Vec<&str>, Vec<&str>) = (
        o.split_inclusive('\n').collect(),
        t.split_inclusive('\n').collect(),
    );
    let line = |l: &str| l.trim_end_matches(['\n', '\r']).to_owned();
    let mut head = 0;
    while head < o.len() && head < t.len() && line(o[head]) == line(t[head]) {
        head += 1;
    }
    let mut tail = 0;
    while tail < o.len() - head
        && tail < t.len() - head
        && line(o[o.len() - 1 - tail]) == line(t[t.len() - 1 - tail])
    {
        tail += 1;
    }
    let mut out = String::new();
    let mut push = |l: &str| {
        out.push_str(l);
        if !l.ends_with('\n') {
            out.push('\n');
        }
    };
    o[..head].iter().for_each(|l| push(l));
    o[head..o.len() - tail].iter().for_each(|l| push(l));
    t[head..t.len() - tail].iter().for_each(|l| push(l));
    o[o.len() - tail..].iter().for_each(|l| push(l));
    out
}

/// A fresh folder for the three bodies.
fn scratch() -> Option<std::path::PathBuf> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "astrolabe-merge-{}-{}-{}",
        std::process::id(),
        super::now_ms(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The driver command git runs, for `merge.astrolabe.driver`: this program, quoted for the
/// shell git runs drivers through, with forward slashes so Git for Windows' shell reads it.
/// None when this process isn't the program (a test harness), so git merges as before.
pub fn driver_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let stem = exe.file_stem()?.to_string_lossy().to_ascii_lowercase();
    if stem != "astrolabe" {
        return None;
    }
    Some(driver_for(&exe.to_string_lossy()))
}

fn driver_for(exe: &str) -> String {
    let path = exe.replace('\\', "/").replace('\'', "'\\''");
    format!("'{path}' notes merge-file %O %A %B %P")
}

/// The attribute lines that send records to this driver.
pub const ATTRIBUTES: [&str; 2] = [
    "efforts/**/*.md merge=astrolabe",
    "initiatives/**/*.md merge=astrolabe",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn record(updated: &str, saved_by: &str, body: &str) -> String {
        format!(
            "---\nstatus: active\nrepo: r\nupdated: \"{updated}\"\nsaved_by: {saved_by}\nsession: s-{saved_by}\n---\n{body}"
        )
    }

    const BODY: &str = "# 0001 Goal\n\nThe goal.\n\n## Current state\nBase view.\n\nBase next.\n\n## Decisions\n- first\n\n## Pieces\n- [ ] a\n- [ ] b\n";

    #[test]
    fn two_saves_of_different_sections_merge_with_the_later_stamp() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("- [ ] a", "- [x] a"),
        );
        let theirs = record(
            "2026-11-14T10:07-07:00",
            "mac · codex",
            &BODY.replace("Base view.", "Mac view."),
        );
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("- [x] a") && merged.contains("Mac view."),
            "{merged}"
        );
        assert!(
            merged.contains("saved_by: mac · codex") && merged.contains("session: s-mac · codex"),
            "{merged}"
        );
        assert!(
            !merged.contains(UNSEEN) && !merged.contains(UNMERGED),
            "{merged}"
        );
    }

    #[test]
    fn lines_both_sides_added_at_one_place_are_both_kept() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("- first\n", "- first\n- pc decided\n"),
        );
        let theirs = record(
            "2026-11-14T10:07-07:00",
            "mac · claude",
            &BODY.replace("- first\n", "- first\n- mac decided\n"),
        );
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("- first\n- pc decided\n- mac decided\n"),
            "{merged}"
        );
        assert!(!merged.contains(UNSEEN), "{merged}");
    }

    #[test]
    fn a_section_both_sides_start_is_written_once() {
        let body = "# 0001 Goal\n\n## Current state\nBase.\n";
        let base = record("2026-11-14T10:00-07:00", "mac · claude", body);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &format!("{body}\n## Decisions\n- pc chose\n"),
        );
        let theirs = record(
            "2026-11-14T10:07-07:00",
            "mac · claude",
            &format!("{body}\n## Decisions\n- mac chose\n"),
        );
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("## Decisions\n- pc chose\n- mac chose\n"),
            "{merged}"
        );
        assert_eq!(merged.matches("## Decisions").count(), 1, "{merged}");
        assert!(!merged.contains(UNSEEN), "{merged}");
    }

    #[test]
    fn two_rewrites_of_current_state_keep_the_later_and_the_earlier_in_the_note() {
        // Defect: the earlier save's state text was only in git, behind an `unseen` mark.
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("Base view.", "PC view."),
        );
        let theirs = record(
            "2026-11-14T09:07-08:00",
            "mac · claude",
            &BODY.replace("Base view.", "Mac view."),
        );
        // 09:07 at -08:00 is 10:07 at -07:00: the Mac's is later.
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        let file = crate::effort::EffortFile::parse(&merged).unwrap();
        assert!(file.current.contains("Mac view."), "{merged}");
        let unmerged = file.unmerged().unwrap();
        assert!(
            unmerged.contains("pc · claude at 2026-11-14T10:05-07:00")
                && unmerged.contains("Its Current state:\n> PC view.\n>\n> Base next."),
            "{merged}"
        );
        assert!(!merged.contains(UNSEEN), "{merged}");
    }

    #[test]
    fn a_tick_and_an_untick_of_different_pieces_both_stand() {
        // Defect: Pieces was taken whole from the later save, losing the earlier's tick.
        let ticked = BODY.replace("- [ ] a", "- [x] a");
        let base = record("2026-11-14T10:00-07:00", "mac · claude", &ticked);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &ticked.replace("- [ ] b", "- [x] b (2026-11-14)"),
        );
        let theirs = record("2026-11-14T10:07-07:00", "mac · claude", BODY);
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("- [ ] a\n- [x] b (2026-11-14)\n"),
            "{merged}"
        );
        assert!(!merged.contains(UNMERGED), "{merged}");
    }

    /// Case 1 of the two-machine sync test, in neutral text: both saves change state, add a
    /// decision and tick a piece; the later one is seconds later on the machine whose name
    /// sorts first.
    fn case_one() -> (String, String, String) {
        let base = record("2026-11-14T10:05:02-07:00", "mac · claude", BODY);
        let earlier = record(
            "2026-11-14T10:05:10-07:00",
            "pc · claude",
            &BODY
                .replace("Base view.", "PC view.")
                .replace("- first\n", "- first\n- 2026-11-14: pc chose\n")
                .replace("- [ ] a", "- [x] a (2026-11-14)"),
        );
        let later = record(
            "2026-11-14T10:05:28-07:00",
            "mac · claude",
            &BODY
                .replace("Base view.", "Mac view.")
                .replace("- first\n", "- first\n- 2026-11-14: mac chose\n")
                .replace("- [ ] b", "- [x] b (2026-11-14)"),
        );
        (base, earlier, later)
    }

    #[test]
    fn decisions_from_both_saves_are_kept_with_the_state_both_rewrote() {
        // Defect: the decisions sat in one changed block with Current state and went with it.
        let (base, earlier, later) = case_one();
        let merged = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        assert!(
            merged.contains(
                "## Decisions\n- first\n- 2026-11-14: pc chose\n- 2026-11-14: mac chose\n"
            ),
            "{merged}"
        );
    }

    #[test]
    fn ticks_from_both_saves_are_kept_with_the_state_both_rewrote() {
        // Defect: Pieces was taken whole with the later save's block.
        let (base, earlier, later) = case_one();
        let merged = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        assert!(
            merged.contains("- [x] a (2026-11-14)\n- [x] b (2026-11-14)\n"),
            "{merged}"
        );
    }

    #[test]
    fn the_later_save_by_seconds_wins_whatever_the_machine_names() {
        // Defect: stamps to the minute left two saves seconds apart to the larger machine
        // name, so the PC's earlier state won over the Mac's.
        let (base, earlier, later) = case_one();
        let merged = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        let file = crate::effort::EffortFile::parse(&merged).unwrap();
        assert!(file.current.contains("Mac view."), "{merged}");
        assert!(file.unmerged().unwrap().contains("> PC view."), "{merged}");
        assert!(
            merged.contains("updated: \"2026-11-14T10:05:28-07:00\"\nsaved_by: mac · claude\nsession: s-mac · claude\n"),
            "the whole stamp is the later save's, though its machine made the base: {merged}"
        );
        // An older stamp to the minute reads as its minute's start.
        let old = earlier.replace("10:05:10-07:00", "10:05-07:00");
        let merged = merge_record(&base, &old, &later, Saves::default()).unwrap();
        assert!(merged.contains("saved_by: mac · claude"), "{merged}");
    }

    #[test]
    fn either_merge_order_gives_the_same_bytes() {
        // Defect: both sides' additions came out ours first, so the machine that merged
        // decided the order.
        let (base, earlier, later) = case_one();
        let one = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        let two = merge_record(&base, &later, &earlier, Saves::default()).unwrap();
        assert_eq!(one, two);
        let added = |n: &str| record("2026-11-14T10:05-07:00", n, &format!("{BODY}{n} note\n"));
        let (a, b) = (added("pc · claude"), added("mac · claude"));
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        assert_eq!(
            merge_record(&base, &a, &b, Saves::default()),
            merge_record(&base, &b, &a, Saves::default())
        );
    }

    #[test]
    fn one_piece_reworded_two_ways_keeps_the_later_wording_and_the_other_in_the_note() {
        // Defect: the earlier wording was only in git.
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let earlier = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("- [ ] b\n", "- [ ] b two ways\n- [ ] c from pc\n"),
        );
        let later = record(
            "2026-11-14T10:07-07:00",
            "mac · claude",
            &BODY.replace("- [ ] b\n", "- [ ] b the mac way\n- [ ] d from mac\n"),
        );
        let merged = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        let file = crate::effort::EffortFile::parse(&merged).unwrap();
        let pieces = file.pieces().unwrap();
        assert!(
            pieces.contains("- [ ] b the mac way")
                && !pieces.contains("b two ways")
                && pieces.contains("- [ ] c from pc")
                && pieces.contains("- [ ] d from mac"),
            "{merged}"
        );
        assert!(
            file.unmerged()
                .unwrap()
                .contains("Its Pieces lines the later save replaced:\n> - [ ] b two ways"),
            "{merged}"
        );
    }

    /// The merged note, checked to be the same bytes in either merge order, and its
    /// Unmerged state section.
    fn both_ways(base: &str, earlier: &str, later: &str) -> (String, String) {
        let one = merge_record(base, earlier, later, Saves::default()).unwrap();
        let two = merge_record(base, later, earlier, Saves::default()).unwrap();
        assert_eq!(one, two);
        let file = crate::effort::EffortFile::parse(&one).unwrap();
        let unmerged = file.unmerged().unwrap_or_default().to_owned();
        (one, unmerged)
    }

    const KEPT_OUT: &str = "The later save's Pieces lines a tick from the earlier save kept out:";

    #[test]
    fn a_later_replace_a_tick_kept_out_is_quoted_as_the_later_saves() {
        // Defect: the later save's line was quoted as the earlier save's wording.
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let earlier = record(
            "2026-11-14T10:05:10-07:00",
            "pc · claude",
            &BODY.replace("- [ ] b", "- [x] b (2026-11-14)"),
        );
        // A `--pieces-file` replace: b is gone, c is new in its place.
        let later = record(
            "2026-11-14T10:05:28-07:00",
            "mac · claude",
            &BODY.replace("- [ ] b", "- [ ] c new"),
        );
        let (merged, unmerged) = both_ways(&base, &earlier, &later);
        let file = crate::effort::EffortFile::parse(&merged).unwrap();
        let pieces = file.pieces().unwrap();
        assert!(
            pieces.contains("- [x] b (2026-11-14)") && !pieces.contains("c new"),
            "{merged}"
        );
        assert!(
            unmerged.contains(&format!("{KEPT_OUT}\n> - [ ] c new")),
            "{merged}"
        );
        assert!(!unmerged.contains("Its Pieces lines"), "{merged}");
    }

    #[test]
    fn a_piece_both_added_and_only_the_earlier_ticked_quotes_the_later_line() {
        // Defect: the later save's unticked line was quoted as the earlier save's.
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let earlier = record(
            "2026-11-14T10:05:10-07:00",
            "pc · claude",
            &format!("{BODY}- [x] z (2026-11-14)\n"),
        );
        let later = record(
            "2026-11-14T10:05:28-07:00",
            "mac · claude",
            &format!("{BODY}- [ ] z\n"),
        );
        let (merged, unmerged) = both_ways(&base, &earlier, &later);
        assert!(merged.contains("- [x] z (2026-11-14)\n"), "{merged}");
        assert!(
            unmerged.contains(&format!("{KEPT_OUT}\n> - [ ] z")),
            "{merged}"
        );
        assert!(!unmerged.contains("Its Pieces lines"), "{merged}");
    }

    #[test]
    fn a_property_both_saves_changed_keeps_the_earlier_value_in_the_note() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let earlier = record("2026-11-14T10:05-07:00", "pc · claude", BODY)
            .replace("status: active", "status: blocked");
        let later = record("2026-11-14T10:07-07:00", "mac · claude", BODY)
            .replace("status: active", "status: review");
        let merged = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        assert!(merged.contains("status: review\n"), "{merged}");
        assert!(
            merged.contains("Its properties:\n> status: blocked"),
            "{merged}"
        );
    }

    #[test]
    fn a_second_overlap_adds_to_the_unmerged_section_already_there() {
        let (base, earlier, later) = case_one();
        let first = merge_record(&base, &earlier, &later, Saves::default()).unwrap();
        let again = |when: &str, by: &str, view: &str| {
            first
                .replace("2026-11-14T10:05:28-07:00", when)
                .replace("saved_by: mac · claude", &format!("saved_by: {by}"))
                .replace("Mac view.", view)
        };
        let merged = merge_record(
            &first,
            &again("2026-11-14T11:00:00-07:00", "pc · claude", "PC again."),
            &again("2026-11-14T11:00:05-07:00", "mac · claude", "Mac again."),
            Saves::default(),
        )
        .unwrap();
        let file = crate::effort::EffortFile::parse(&merged).unwrap();
        let unmerged = file.unmerged().unwrap();
        assert!(
            unmerged.contains("> PC view.") && unmerged.contains("> PC again."),
            "{merged}"
        );
        assert_eq!(
            merged.matches(&format!("## {UNMERGED}")).count(),
            1,
            "{merged}"
        );
    }

    #[test]
    fn a_hand_edit_is_never_decided_against() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("Base view.", "PC view."),
        );
        // The stamp is the base's: a hand edit, or an autostash restoring one.
        let theirs = record(
            "2026-11-14T10:00-07:00",
            "mac · claude",
            &BODY.replace("Base view.", "Hand view."),
        );
        assert_eq!(merge_record(&base, &ours, &theirs, Saves::default()), None);
        // But a hand edit beside a save, or adding beside one, merges.
        let theirs = record(
            "2026-11-14T10:00-07:00",
            "mac · claude",
            &format!("{BODY}\nA notice.\n"),
        );
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("PC view.") && merged.contains("A notice."),
            "{merged}"
        );
        assert!(merged.contains("saved_by: pc · claude"), "{merged}");
    }

    #[test]
    fn a_stamp_property_a_hand_edit_changed_is_never_decided_against() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record("2026-11-14T10:05-07:00", "pc · claude", BODY)
            .replace("session: s-pc · claude", "session: p9");
        let theirs = base.replace("session: s-mac · claude", "session: by hand");
        assert_eq!(merge_record(&base, &ours, &theirs, Saves::default()), None);
    }

    #[test]
    fn only_the_record_a_save_was_for_counts_as_saved() {
        let shelf = "efforts/2026/11-November/0356-planned-cases.md";
        assert!(saved_for("0356: active (worker bound)", shelf));
        assert!(saved_for("0356: checkpoint", shelf));
        assert!(!saved_for("0333: checkpoint", shelf));
        assert!(saved_for(
            "initiative deck-frame-time: done",
            "initiatives/deck-frame-time.md"
        ));
        assert!(!saved_for(
            "initiative deck-frame-time: done",
            "initiatives/deck-frame.md"
        ));
        assert!(!saved_for("0356: checkpoint", "initiatives/0356-x.md"));
        assert!(!saved_for("journal tidied after a merge", shelf));
    }

    #[test]
    fn a_same_minute_tie_goes_the_same_way_from_either_side() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let a = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &BODY.replace("Base view.", "PC view."),
        );
        let b = record(
            "2026-11-14T10:05-07:00",
            "mac · claude",
            &BODY.replace("Base view.", "Mac view."),
        );
        let one = merge_record(&base, &a, &b, Saves::default()).unwrap();
        let two = merge_record(&base, &b, &a, Saves::default()).unwrap();
        assert!(
            one.contains("PC view.") && two.contains("PC view."),
            "{one}\n{two}"
        );
    }

    #[test]
    fn unclosed_headers_and_opened_fences_are_left_to_git() {
        let base = record("2026-11-14T10:00-07:00", "mac · claude", BODY);
        let ours = record("2026-11-14T10:05-07:00", "pc · claude", BODY);
        assert_eq!(
            merge_record(&base, &ours, "---\nstatus: active\n", Saves::default()),
            None
        );
        // Each side adds one half of a fence at the same place: both kept would open one.
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &format!("{BODY}```sh\nls\n"),
        );
        let theirs = record(
            "2026-11-14T10:07-07:00",
            "mac · claude",
            &format!("{BODY}```\n"),
        );
        assert!(!fences(&ours).is_multiple_of(2));
        let merged = merge_record(&base, &ours, &theirs, Saves::default());
        assert!(
            merged.as_ref().is_none_or(|m| fences(m).is_multiple_of(2)),
            "{merged:?}"
        );
    }

    #[test]
    fn a_preamble_and_repeated_headings_survive_as_text() {
        let body = "# Page\n\n> a quoted preamble\n\n## Notes\none\n\n## Notes\ntwo\n\n## Current state\nBase.\n";
        let base = record("2026-11-14T10:00-07:00", "mac · claude", body);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &body.replace("> a quoted", "> an edited"),
        );
        let theirs = record(
            "2026-11-14T10:07-07:00",
            "mac · claude",
            &body.replace("two\n", "two, changed\n"),
        );
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        assert!(
            merged.contains("> an edited preamble")
                && merged.contains("## Notes\none\n\n## Notes\ntwo, changed\n"),
            "{merged}"
        );
    }

    #[test]
    fn an_overlap_mark_is_dropped_by_a_later_save_and_kept_beside_another() {
        let marked = format!(
            "{}unseen: \"pc 2026-11-14T10:05-07:00; both are in git\"\n---\n{BODY}",
            record("2026-11-14T10:07-07:00", "mac · claude", "").trim_end_matches("---\n")
        );
        // One side saved it again (dropping the mark); the other saved something else.
        let ours = record(
            "2026-11-14T10:20-07:00",
            "pc · claude",
            &BODY.replace("Base view.", "Reread view."),
        );
        let theirs = marked
            .replace("2026-11-14T10:07", "2026-11-14T10:21")
            .replace("- [ ] b", "- [x] b");
        let merged = merge_record(&marked, &ours, &theirs, Saves::default()).unwrap();
        assert!(!merged.contains(UNSEEN), "{merged}");
    }

    #[test]
    fn the_driver_command_quotes_a_path_with_a_space() {
        // The test harness isn't the program, so there's no driver here.
        assert_eq!(driver_command(), None);
        assert_eq!(
            driver_for(r"C:\Users\A B\it's\astrolabe.exe"),
            r"'C:/Users/A B/it'\''s/astrolabe.exe' notes merge-file %O %A %B %P"
        );
        // And the shell git runs it through reads it back as one word.
        let out = Command::new("sh")
            .args([
                "-c",
                &driver_for(r"C:\Users\A B\it's\x")
                    .replace(" notes merge-file %O %A %B %P", "")
                    .replacen("'", "printf %s '", 1),
            ])
            .output();
        if let Ok(out) = out {
            assert_eq!(String::from_utf8_lossy(&out.stdout), "C:/Users/A B/it's/x");
        }
    }
}
