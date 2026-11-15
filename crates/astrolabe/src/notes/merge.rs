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
//!   both are kept;
//! - only where both sides are program saves (each changed `updated`) and changed the same
//!   lines does the later save win. The record then carries an `unseen` property naming the
//!   other save, committed with it, so every machine reads the same warning until the record
//!   is next saved. Both texts stay in git.
//!
//! Anything else (a hand edit, or an autostash restoring one, changing the same lines as a
//! save) falls back to git's own line merge, conflict markers and all, as before.
use chrono::DateTime;
use std::path::Path;
use std::process::{Command, Stdio};

/// The property that marks a record whose two saves overlapped.
pub const UNSEEN: &str = "unseen";

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
    // A save rewrites the stamp; one made in the same minute as the base by the same session
    // doesn't change it, so the commit it came in is asked too.
    let restamped = |r: &Record| {
        r.value("updated")
            .is_some_and(|u| Some(u) != b.value("updated"))
    };
    // Only two program saves are ever decided between; a hand edit never loses.
    let decidable = (saves.ours || restamped(&o)) && (saves.theirs || restamped(&t));
    let later = later(&o, &t);
    let mut overlap = false;

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
        // A key one side changed is that side's; one both changed is the later save's,
        // which for the stamp is no overlap: it says who saved last.
        let chosen = if ov == tv || tv == bv {
            ov
        } else if ov == bv {
            tv
        } else if decidable && STAMP.contains(&key) {
            pick(later, ov, tv)
        } else if decidable {
            overlap = true;
            pick(later, ov, tv)
        } else {
            return None;
        };
        if let Some(lines) = chosen {
            header.extend(lines.iter().cloned());
        }
    }

    let (body, body_overlap) = merge_body(b.body, o.body, t.body, decidable.then_some(later))?;
    overlap |= body_overlap;

    // An earlier overlap's mark stays until a save drops it; a new one names this one.
    let unseen = if overlap {
        let lost = if later == Side::Ours { &t } else { &o };
        let machine = lost
            .value("saved_by")
            .and_then(|s| s.split(" · ").next().map(str::to_owned))
            .unwrap_or_else(|| "another machine".to_owned());
        let when = lost.value("updated").unwrap_or_default();
        Some(vec![format!(
            "{UNSEEN}: \"{} {}; both are in git\"",
            machine.replace('"', "'"),
            when.replace('"', "'")
        )])
    } else {
        let (bv, ov, tv) = (b.prop(UNSEEN), o.prop(UNSEEN), t.prop(UNSEEN));
        let kept = if ov == tv || tv == bv {
            ov
        } else if ov == bv {
            tv
        } else {
            pick(later, ov, tv)
        };
        kept.cloned()
    };
    if let Some(lines) = unseen {
        header.extend(lines);
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

/// The later of two saves by `updated` with its offset; within the same minute, the larger
/// `saved_by`, then the larger text, so every machine picks the same side.
fn later(o: &Record, t: &Record) -> Side {
    let at = |r: &Record| {
        r.value("updated")
            .and_then(|u| DateTime::parse_from_str(&u, "%Y-%m-%dT%H:%M%:z").ok())
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

/// The body merged three ways by `git merge-file`. A conflict where both sides only added
/// lines keeps both, ours first; one where both changed lines takes `later`'s, when given,
/// and says so. None when a conflict is left with no side to take.
fn merge_body(base: &str, ours: &str, theirs: &str, later: Option<Side>) -> Option<(String, bool)> {
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
        return Some((text, false));
    }
    resolve(&text, later)
}

/// Settles the conflict blocks `git merge-file --diff3` left in `text`.
fn resolve(text: &str, later: Option<Side>) -> Option<(String, bool)> {
    let open = format!("{} ours", "<".repeat(MARKER));
    let mid = format!("{} base", "|".repeat(MARKER));
    let split = "=".repeat(MARKER);
    let close = format!("{} theirs", ">".repeat(MARKER));
    let mut out = String::new();
    let mut overlap = false;
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
        if o == t {
            out.push_str(&o);
        } else if b.trim().is_empty() {
            // Both only added here: what they added alike once (a new section's heading
            // both wrote, say), then each one's own lines, ours first.
            out.push_str(&both_added(&o, &t));
        } else {
            overlap = true;
            match later? {
                Side::Ours => out.push_str(&o),
                Side::Theirs => out.push_str(&t),
            }
        }
    }
    Some((out, overlap))
}

/// Two additions at one place: their common first and last lines once, the rest of each
/// between, ours first.
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
        assert!(!merged.contains(UNSEEN), "{merged}");
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
    fn two_saves_of_one_section_keep_the_later_and_mark_it_unseen() {
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
        assert!(
            merged.contains("Mac view.") && !merged.contains("PC view."),
            "{merged}"
        );
        assert!(
            merged.contains("unseen: \"pc 2026-11-14T10:05-07:00; both are in git\""),
            "{merged}"
        );
        assert!(crate::effort::EffortFile::parse(&merged).is_ok());
    }

    #[test]
    fn a_tick_and_an_untick_of_one_piece_are_an_overlap() {
        let ticked = BODY.replace("- [ ] a", "- [x] a");
        let base = record("2026-11-14T10:00-07:00", "mac · claude", &ticked);
        let ours = record(
            "2026-11-14T10:05-07:00",
            "pc · claude",
            &ticked.replace("- [ ] b", "- [x] b"),
        );
        let theirs = record("2026-11-14T10:07-07:00", "mac · claude", BODY);
        let merged = merge_record(&base, &ours, &theirs, Saves::default()).unwrap();
        // The pieces lines sit together, so the later side's block wins whole.
        assert!(merged.contains("- [ ] a\n- [ ] b"), "{merged}");
        assert!(merged.contains(UNSEEN), "{merged}");
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
