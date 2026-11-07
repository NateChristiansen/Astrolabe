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
    pub today: String,
    pub journal: String,
    pub journal_lines: Vec<JournalEntry>,
    pub notes: Vec<String>,
    pub lessons: Vec<Lesson>,
    pub drafts: usize,
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
}

/// The same rows from a checkout already in hand: what the board is drawn from.
pub fn rows_in(notes: &Notes, repo: Option<&str>, all: bool) -> BoardData {
    let mut rows = Vec::new();
    for (_, rel) in notes.numbers() {
        let path = notes.root.join(&rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = EffortFile::parse(&text) else {
            continue;
        };
        let status = file.prop("status").unwrap_or("");
        if !all && (status == "done" || status == "abandoned") {
            continue;
        }
        if let Some(r) = repo
            && !file
                .prop("repo")
                .is_some_and(|rs| rs.split(',').any(|x| x.trim() == r))
        {
            continue;
        }
        let needs_owner = file
            .prop("waits_for")
            .is_some_and(|w| w.starts_with("owner:"));
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
            repo: file.prop("repo").map(str::to_owned),
            waits_for: file.prop("waits_for").map(str::to_owned),
            updated: file.prop("updated").map(str::to_owned),
            saved_by: file.prop("saved_by").map(str::to_owned),
            branch: file.prop("branch").map(str::to_owned),
            unpushed: file.prop("unpushed").map(str::to_owned),
            dirty: file.prop("dirty").map(str::to_owned),
        });
    }
    let mut initiative_attention = Vec::new();
    if let Ok(pages) = crate::effort::initiative_catalog(&notes.root) {
        for page in pages {
            let file = &page.file;
            if file
                .prop("waits_for")
                .is_none_or(|w| !w.starts_with("owner:"))
            {
                continue;
            }
            if let Some(repo) = repo
                && !file
                    .prop("repo")
                    .is_some_and(|rs| rs.split(',').any(|r| r.trim() == repo))
            {
                continue;
            }
            initiative_attention.push(EffortRow {
                title: Some(file.title.clone()),
                file: Some(page.path),
                status: file.prop("status").map(str::to_owned),
                repo: file.prop("repo").map(str::to_owned),
                waits_for: file.prop("waits_for").map(str::to_owned),
                needs_owner: true,
                state: file.first_paragraph().map(str::to_owned),
                next: file.last_paragraph().map(str::to_owned),
                updated: file.prop("updated").map(str::to_owned),
                ..Default::default()
            });
        }
    }
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
        today: day,
        journal_lines: notes::parse_journal(&journal),
        journal,
        notes: notes.warnings(),
        lessons: lessons_in(&notes.root),
        drafts: drafts_in(&notes.root),
    }
}

/// The notes' inbox: every `lessons/*.md`, by name, since a triaged lesson is deleted and
/// the folder holds only what waits. A lesson's title is its first heading, else its file name.
/// Lessons reach the board and nothing else: never a session.
pub fn lessons_in(root: &Path) -> Vec<Lesson> {
    let dir = root.join("lessons");
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
                file: format!("lessons/{name}"),
                title,
            }
        })
        .collect()
}

/// The open drafts: every `drafts/*.md`, since a posted or decided draft is deleted and the
/// folder holds only what waits on the owner. The board counts them and nothing else:
/// never a session, and the program never removes one.
pub fn drafts_in(root: &Path) -> usize {
    std::fs::read_dir(root.join("drafts"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
        .count()
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
    fn drafts_are_counted_without_the_gitkeep() {
        let root = std::env::temp_dir().join(format!("astrolabe-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(drafts_in(&root), 0, "no folder is no drafts");
        let dir = root.join("drafts");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".gitkeep"), "").unwrap();
        assert_eq!(drafts_in(&root), 0, "an empty folder is no drafts");
        std::fs::write(dir.join("2026-11-06-release-note.md"), "# Release note\n").unwrap();
        std::fs::write(dir.join("2026-11-06-reply.md"), "no heading\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a draft\n").unwrap();
        assert_eq!(drafts_in(&root), 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
