//! Collect board data once. Rendering consumes this snapshot and performs no Git calls.
use crate::effort::EffortFile;
use crate::notes::{self, JournalEntry, Notes};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BoardData {
    pub efforts: Vec<EffortRow>,
    pub initiative_attention: Vec<EffortRow>,
    pub initiatives: Vec<InitiativeRow>,
    pub legacy_initiatives: std::collections::BTreeMap<String, LegacyInitiative>,
    pub initiative_titles: std::collections::BTreeMap<String, String>,
    pub today: String,
    pub journal: String,
    pub journal_lines: Vec<JournalEntry>,
    pub notes: Vec<String>,
    pub lessons: Vec<Lesson>,
    pub drafts: Vec<Lesson>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EffortRow {
    pub number: Option<u64>,
    pub title: Option<String>,
    pub file: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub initiative: Option<String>,
    pub initiative_title: Option<String>,
    pub repo: Option<String>,
    pub waits_for: Option<String>,
    pub updated: Option<String>,
    pub saved_by: Option<String>,
    pub branch: Option<String>,
    pub unpushed: Option<String>,
    pub dirty: Option<String>,
    pub state: Option<String>,
    pub next: Option<String>,
    pub needs_owner: bool,
    pub pieces: Option<Pieces>,
    /// The lessons this effort answers, which marking it done closes. Left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lessons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pieces {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lesson {
    pub file: String,
    pub title: String,
    /// Git has no commit of the file as it stands, so no sync can carry it yet. Left out of
    /// the JSON when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uncommitted: bool,
    /// The effort that names this lesson, so the owner tells answered work from a lesson
    /// nobody's on. Left out of the JSON when none does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<Answer>,
}

/// An effort that answers a lesson, as the board shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Answer {
    pub number: u64,
    pub title: String,
    pub status: String,
    pub file: String,
}

/// A page and progress derived from its members, independent of board filters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InitiativeRow {
    pub page: EffortRow,
    pub progress: InitiativeProgress,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyInitiative {
    pub name: String,
    pub progress: InitiativeProgress,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InitiativeProgress {
    pub total: usize,
    pub done: usize,
    pub review: usize,
    pub active: usize,
    pub blocked: usize,
    pub to_start: usize,
    pub abandoned: usize,
    pub need_you: usize,
}

impl InitiativeProgress {
    pub fn add(&mut self, status: &str, needs_owner: bool) {
        if status == "abandoned" {
            self.abandoned += 1;
        } else {
            self.total += 1;
        }
        match status {
            "done" => self.done += 1,
            "review" => self.review += 1,
            "active" => self.active += 1,
            "blocked" => self.blocked += 1,
            "proposed" | "ready" => self.to_start += 1,
            _ => {}
        }
        self.need_you += usize::from(needs_owner);
    }
}

/// Whether a record waits on the owner. A closed one never does: saves refuse a wait on one
/// now, but a note written before that, or by an older build on another machine, can carry
/// one, and it put a finished effort in Needs you.
fn waits_on_owner(file: &EffortFile) -> bool {
    !matches!(file.prop("status"), Some("done" | "abandoned"))
        && file
            .prop("waits_for")
            .is_some_and(|w| w.starts_with("owner:"))
}

/// The same rows from a checkout already in hand: what the board is drawn from.
pub fn rows_in(notes: &Notes, repo: Option<&str>, all: bool) -> BoardData {
    let mut rows = Vec::new();
    let mut warnings = notes.warnings();
    let pages = crate::effort::initiative_catalog(&notes.root).unwrap_or_else(|error| {
        warnings.push(error);
        Vec::new()
    });
    let mut legacy_initiatives: std::collections::BTreeMap<String, LegacyInitiative> =
        Default::default();
    let mut page_progress: std::collections::BTreeMap<String, InitiativeProgress> =
        Default::default();
    // Every effort that names a lesson, whatever filter the rows have: lessons aren't filtered.
    let mut answers: Vec<(Vec<String>, Answer)> = Vec::new();
    for (_, rel) in notes.numbers() {
        let path = notes.root.join(&rel);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                warnings.push(format!("{rel}: {error}"));
                continue;
            }
        };
        let file = match EffortFile::parse(&text) {
            Ok(file) => file,
            Err(error) => {
                warnings.push(format!("{rel}: {error}"));
                continue;
            }
        };
        let status = file.prop("status").unwrap_or("");
        let needs_owner = waits_on_owner(&file);
        let lessons = crate::record::lesson::of(&file);
        if let (false, Some(number)) = (lessons.is_empty(), file.number()) {
            answers.push((
                lessons.clone(),
                Answer {
                    number: u64::from(number),
                    title: file.name().to_owned(),
                    status: status.to_owned(),
                    file: rel.clone(),
                },
            ));
        }
        let membership = file.prop("initiative").filter(|name| !name.is_empty());
        let page = membership.and_then(|name| crate::effort::resolve_initiative(&pages, name).ok());
        if let Some(page) = page {
            page_progress
                .entry(page.slug.clone())
                .or_default()
                .add(status, needs_owner);
        }
        if let Some(r) = repo
            && !file
                .prop("repo")
                .is_some_and(|rs| rs.split(',').any(|x| x.trim() == r))
        {
            continue;
        }
        if let Some(name) = membership
            && page.is_none()
        {
            legacy_initiatives
                .entry(crate::effort::initiative_name(name))
                .or_insert_with(|| LegacyInitiative {
                    name: name.to_owned(),
                    ..Default::default()
                })
                .progress
                .add(status, needs_owner);
        }
        if !all && (status == "done" || status == "abandoned") {
            continue;
        }
        rows.push(EffortRow {
            number: file.number().map(u64::from),
            title: Some(file.name().to_owned()),
            file: Some(rel),
            status: Some(status.to_owned()),
            needs_owner,
            state: file.first_paragraph().map(str::to_owned),
            next: file.last_paragraph().map(str::to_owned),
            pieces: file
                .pieces_count()
                .map(|(done, total)| Pieces { done, total }),
            priority: file.prop("priority").map(str::to_owned),
            initiative: file.prop("initiative").map(str::to_owned),
            initiative_title: page.map(|page| page.file.title.clone()),
            repo: file.prop("repo").map(str::to_owned),
            waits_for: file.prop("waits_for").map(str::to_owned),
            updated: file.prop("updated").map(str::to_owned),
            saved_by: file.prop("saved_by").map(str::to_owned),
            branch: file.prop("branch").map(str::to_owned),
            unpushed: file.prop("unpushed").map(str::to_owned),
            dirty: file.prop("dirty").map(str::to_owned),
            lessons,
        });
    }
    let mut initiative_attention = Vec::new();
    let mut initiative_titles = std::collections::BTreeMap::new();
    let mut initiatives = Vec::new();
    legacy_initiatives.retain(|_, group| group.progress.total > group.progress.done);
    for page in &pages {
        let file = &page.file;
        initiative_titles.insert(page.slug.clone(), file.title.clone());
        if let Some(repo) = repo
            && !file
                .prop("repo")
                .is_some_and(|rs| rs.split(',').any(|r| r.trim() == repo))
        {
            continue;
        }
        let needs_owner = waits_on_owner(file);
        let mut progress = page_progress.remove(&page.slug).unwrap_or_default();
        progress.need_you += usize::from(needs_owner);
        let row = EffortRow {
            title: Some(file.title.clone()),
            file: Some(page.path.clone()),
            status: file.prop("status").map(str::to_owned),
            repo: file.prop("repo").map(str::to_owned),
            waits_for: file.prop("waits_for").map(str::to_owned),
            needs_owner,
            state: file
                .first_paragraph()
                .map(str::to_owned)
                .or_else(|| file.purpose()),
            next: file.last_paragraph().map(str::to_owned),
            updated: file.prop("updated").map(str::to_owned),
            ..Default::default()
        };
        if needs_owner {
            initiative_attention.push(row.clone());
        }
        initiatives.push(InitiativeRow {
            page: row,
            progress,
        });
    }
    let uncommitted = uncommitted_pages(notes);
    let (day, _) = notes::local_day_time();
    let mut journal =
        std::fs::read_to_string(notes.root.join(notes::journal_file(&day))).unwrap_or_default();
    // A day file at the journal root, from before the shelves or from a machine still
    // writing there, is today's too.
    if let Ok(flat) = std::fs::read_to_string(notes.root.join("journal").join(format!("{day}.md")))
    {
        if !journal.is_empty() && !journal.ends_with('\n') {
            journal.push('\n');
        }
        journal.push_str(&flat);
        // One line per effort across both, the later one.
        journal = notes::tidy_journal(&journal);
    }
    BoardData {
        efforts: rows,
        initiative_attention,
        initiative_titles,
        initiatives,
        legacy_initiatives,
        today: day,
        journal_lines: notes::parse_journal(&journal),
        journal,
        notes: warnings,
        lessons: answered(
            mark_uncommitted(lessons_in(&notes.root), &uncommitted),
            &answers,
        ),
        drafts: mark_uncommitted(drafts_in(&notes.root), &uncommitted),
    }
}

/// The lessons and drafts git reports as untracked, changed or staged: written as plain
/// files, they wait for the next notes write to commit them, and until then no other machine
/// can have them. Read here, once, because rendering runs no git. A folder that isn't a
/// checkout marks nothing.
fn uncommitted_pages(notes: &Notes) -> std::collections::BTreeSet<String> {
    let status = notes
        .git(&[
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
            "--",
            "lessons",
            "drafts",
        ])
        .unwrap_or_default();
    let mut paths = std::collections::BTreeSet::new();
    let mut entries = status.split('\0');
    while let Some(entry) = entries.next() {
        let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else {
            continue;
        };
        // A rename or copy is followed by the path it came from, which isn't a page here.
        if code.contains(['R', 'C']) {
            entries.next();
        }
        paths.insert(path.to_owned());
    }
    paths
}

fn mark_uncommitted(
    mut pages: Vec<Lesson>,
    uncommitted: &std::collections::BTreeSet<String>,
) -> Vec<Lesson> {
    for page in &mut pages {
        page.uncommitted = uncommitted.contains(&page.file);
    }
    pages
}

/// The notes' inbox: every `lessons/*.md`, by name, since a triaged lesson is deleted and
/// the folder holds only what waits. A lesson's title is its first heading, else its file name.
/// Lessons reach the board and nothing else: never a session.
pub fn lessons_in(root: &Path) -> Vec<Lesson> {
    pages_in(root, "lessons")
}

/// The open drafts: every `drafts/*.md`, since a posted or decided draft is deleted and the
/// folder holds only what waits on the owner. The board lists them and nothing else:
/// never a session, and the program never removes one.
pub fn drafts_in(root: &Path) -> Vec<Lesson> {
    pages_in(root, "drafts")
}

/// Every `<folder>/*.md`, by name, titled by its first heading, else its file name.
/// Each lesson with the effort that answers it: one still open over a finished one, since an
/// open one is the work in flight, then the newest.
fn answered(mut lessons: Vec<Lesson>, answers: &[(Vec<String>, Answer)]) -> Vec<Lesson> {
    for lesson in &mut lessons {
        lesson.answered_by = (answers.iter())
            .filter(|(names, _)| names.iter().any(|n| n.eq_ignore_ascii_case(&lesson.file)))
            .map(|(_, answer)| answer)
            .max_by_key(|a| (!matches!(a.status.as_str(), "done" | "abandoned"), a.number))
            .cloned();
    }
    lessons
}

fn pages_in(root: &Path, folder: &str) -> Vec<Lesson> {
    let dir = root.join(folder);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".md"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
            let title = text
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map_or_else(|| name.trim_end_matches(".md").to_owned(), str::to_owned);
            Lesson {
                file: format!("{folder}/{name}"),
                title,
                uncommitted: false,
                answered_by: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod lessons_tests {
    use super::{drafts_in, lessons_in};

    #[test]
    fn lessons_are_listed_by_name_with_their_headings() {
        let root = std::env::temp_dir().join(format!("astrolabe-lessons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(lessons_in(&root).is_empty(), "no folder is no lessons");
        let dir = root.join("lessons");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitkeep"), "").unwrap();
        std::fs::write(
            dir.join("2026-11-05-b.md"),
            "# Rule text reaches a session once\n\nbody\n",
        )
        .unwrap();
        std::fs::write(dir.join("2026-11-01-a.md"), "no heading here\n").unwrap();
        let rows = lessons_in(&root);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].file, "lessons/2026-11-01-a.md");
        assert_eq!(rows[0].title, "2026-11-01-a");
        assert_eq!(rows[1].title, "Rule text reaches a session once");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn drafts_are_listed_without_the_gitkeep() {
        let root = std::env::temp_dir().join(format!("astrolabe-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(drafts_in(&root).is_empty(), "no folder is no drafts");
        let dir = root.join("drafts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitkeep"), "").unwrap();
        assert!(drafts_in(&root).is_empty(), "an empty folder is no drafts");
        std::fs::write(dir.join("2026-11-06-release-note.md"), "# Release note\n").unwrap();
        std::fs::write(dir.join("2026-11-06-reply.md"), "no heading\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a draft\n").unwrap();
        let rows = drafts_in(&root);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].file, "drafts/2026-11-06-release-note.md");
        assert_eq!(rows[0].title, "Release note");
        assert_eq!(rows[1].title, "2026-11-06-reply");
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
pub(crate) fn initiative_fixture() -> Notes {
    let root = std::env::temp_dir().join(format!(
        "astrolabe-board-initiatives-{}",
        crate::activate::random_hex().unwrap()
    ));
    std::fs::create_dir_all(root.join("efforts")).unwrap();
    std::fs::create_dir_all(root.join("initiatives")).unwrap();
    for (slug, title, status, wait, body) in [
        (
            "shared-work",
            "Shared work",
            "active",
            "owner: accept the plan",
            "## Current state\nBuilding together. Another sentence.\n\n## Purpose\nThis is not the summary.\n",
        ),
        (
            "finished-members",
            "Finished members",
            "ready",
            "",
            "## Purpose\nAccept the combined result. More purpose.\n",
        ),
        (
            "accepted-work",
            "Accepted work",
            "done",
            "",
            "## Current state\nAccepted together.\n",
        ),
        (
            "empty-work",
            "Empty work",
            "proposed",
            "",
            "A legacy purpose. Another sentence.\n",
        ),
        (
            "dropped-work",
            "Dropped work",
            "active",
            "",
            "## Current state\nThe direction was dropped.\n",
        ),
        (
            "old-work",
            "Old work",
            "done",
            "",
            "## Current state\nAccepted long ago.\n",
        ),
        ("abandoned-page", "Abandoned page", "abandoned", "", ""),
    ] {
        let updated = if slug == "old-work" {
            "2026-09-01T12:00-07:00"
        } else {
            "2026-11-05T12:00-07:00"
        };
        std::fs::write(root.join(format!("initiatives/{slug}.md")), format!(
            "---\nstatus: {status}\nrepo: demo\nwaits_for: {wait}\nupdated: {updated}\n---\n# {title}\n\n{body}"
        )).unwrap();
    }
    // Membership comes from frontmatter, even for cross-repository members and
    // orchestrate-style efforts. No page maintains a second list of its members.
    for (i, (initiative, status, wait, repo)) in [
        ("Unfiled work", "done", "", "demo"),
        ("Unfiled work", "active", "", "demo"),
        ("Unfiled work", "blocked", "owner: choose", "demo"),
        ("shared-work", "active", "", "demo"),
        ("Shared work", "blocked", "owner: review", "other"),
        ("[[shared-work]]", "review", "owner: accept", "demo"),
        ("shared-work", "ready", "", "demo"),
        ("shared-work", "abandoned", "", "demo"),
        ("shared-work", "done", "", "demo"),
        ("finished-members", "done", "", "demo"),
        ("accepted-work", "done", "", "demo"),
        ("[[loose-tools]]", "ready", "", "demo"),
        ("Discarded experiment", "abandoned", "", "demo"),
        ("dropped-work", "abandoned", "", "demo"),
    ]
    .into_iter()
    .enumerate()
    {
        std::fs::write(root.join(format!("efforts/{:04}-member.md", i + 1)), format!(
            "---\nstatus: {status}\nrepo: {repo}\ninitiative: {initiative}\nwaits_for: {wait}\nupdated: 2026-11-05T12:00-07:00\n---\n# {:04} Member\n\n## Current state\nWorking.\n", i + 1
        )).unwrap();
    }
    Notes::new(root.clone(), root.join("state"))
}

#[cfg(test)]
mod initiative_tests {
    use super::*;

    #[test]
    fn legacy_groups_keep_first_spelling_and_only_survive_with_open_members() {
        let notes = initiative_fixture();
        for (n, name, status) in [
            (20, "[[Mixed Case]]", "done"),
            (21, "mixed case", "abandoned"),
            (22, "MIXED CASE", "active"),
            (23, "Closed group", "done"),
            (24, "closed group", "abandoned"),
        ] {
            std::fs::write(
                notes.root.join(format!("efforts/{n:04}-legacy.md")),
                format!(
                    "---\ninitiative: {name}\nstatus: {status}\nrepo: demo\n---\n# {n:04} Legacy\n"
                ),
            )
            .unwrap();
        }
        let board = rows_in(&notes, None, false);
        let group = &board.legacy_initiatives["mixed case"];
        assert_eq!(group.name, "[[Mixed Case]]");
        assert_eq!(
            (
                group.progress.done,
                group.progress.total,
                group.progress.abandoned
            ),
            (1, 2, 1)
        );
        assert!(!board.legacy_initiatives.contains_key("closed group"));
        assert!(
            !board
                .legacy_initiatives
                .contains_key("discarded experiment")
        );
        let html = crate::home::render(
            &board,
            &notes.root,
            notes::now_ms(),
            crate::home::Look::default(),
        );
        assert!(html.contains("Mixed Case <span class=\"pill\">no page</span>"));
        assert!(!html.contains("Closed group") && !html.contains("Discarded experiment"));
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn a_closed_record_never_waits_on_the_owner() {
        let notes = initiative_fixture();
        for (n, status) in [(25, "done"), (26, "abandoned")] {
            std::fs::write(
                notes.root.join(format!("efforts/{n:04}-closed.md")),
                format!(
                    "---\nstatus: {status}\nrepo: demo\nwaits_for: \"owner: stale pick\"\nupdated: 2026-11-05T12:00-07:00\n---\n# {n:04} Closed ask {status}\n\n## Current state\nFinished.\n"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            notes.root.join("initiatives/closed-ask.md"),
            "---\nstatus: done\nrepo: demo\nwaits_for: \"owner: stale page pick\"\nupdated: 2026-11-05T12:00-07:00\n---\n# Closed ask page\n\n## Current state\nAccepted.\n",
        )
        .unwrap();
        let board = rows_in(&notes, None, true);
        for n in [25, 26] {
            let row = board.efforts.iter().find(|r| r.number == Some(n)).unwrap();
            assert!(!row.needs_owner, "effort {n}");
        }
        let page = board
            .initiatives
            .iter()
            .find(|i| i.page.title.as_deref() == Some("Closed ask page"))
            .unwrap();
        assert!(!page.page.needs_owner && page.progress.need_you == 0);
        assert!(
            !board
                .initiative_attention
                .iter()
                .any(|r| r.title.as_deref() == Some("Closed ask page"))
        );
        let html = crate::home::render(
            &board,
            &notes.root,
            notes::now_ms(),
            crate::home::Look::default(),
        );
        let needs = html
            .split("col needsyou")
            .nth(1)
            .and_then(|rest| rest.split("</section>").next())
            .unwrap_or_default();
        assert!(!needs.contains("Closed ask"), "{needs}");
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn bad_notes_do_not_erase_other_members_progress() {
        let notes = initiative_fixture();
        std::fs::write(notes.root.join("efforts/0020-unreadable.md"), [0xff]).unwrap();
        std::fs::write(
            notes.root.join("efforts/0021-malformed.md"),
            "# Bad\n## Current state\na\n## Current state\nb\n",
        )
        .unwrap();
        let board = rows_in(&notes, None, true);
        let shared = board
            .initiatives
            .iter()
            .find(|i| i.page.title.as_deref() == Some("Shared work"))
            .unwrap();
        assert_eq!(
            (
                shared.progress.done,
                shared.progress.total,
                shared.progress.need_you
            ),
            (1, 5, 3)
        );
        for filename in ["0020-unreadable.md", "0021-malformed.md"] {
            assert!(board.notes.iter().any(|warning| warning.contains(filename)));
        }
        let html = crate::home::render(
            &board,
            &notes.root,
            notes::now_ms(),
            crate::home::Look::default(),
        );
        assert!(!html.contains("member counts unavailable"));
        assert!(html.contains("1 of 5 done"));
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn pages_and_member_counts_survive_effort_filters() {
        let notes = initiative_fixture();
        let board = rows_in(&notes, Some("demo"), false);
        let shared = board
            .initiatives
            .iter()
            .find(|i| i.page.title.as_deref() == Some("Shared work"))
            .unwrap();
        let p = &shared.progress;
        assert_eq!(
            (
                p.done,
                p.total,
                p.review,
                p.active,
                p.blocked,
                p.to_start,
                p.abandoned,
                p.need_you
            ),
            (1, 5, 1, 1, 1, 1, 1, 3)
        );
        assert_eq!(board.initiatives.len(), 7);
        assert_eq!(board.legacy_initiatives["unfiled work"].progress.done, 1);
        assert!(
            !board
                .legacy_initiatives
                .contains_key("discarded experiment")
        );
        let member = board
            .efforts
            .iter()
            .find(|r| r.initiative.as_deref() == Some("[[shared-work]]"))
            .unwrap();
        assert_eq!(member.initiative_title.as_deref(), Some("Shared work"));
        assert_eq!(member.initiative.as_deref(), Some("[[shared-work]]"));
        std::fs::remove_dir_all(notes.root).unwrap();
    }
}
