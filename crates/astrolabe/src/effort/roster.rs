//! An initiative's members as the agent working near them should see them: which initiatives
//! are open in a repo, whether a new title names one, and a roll call of where each stands.
//! The rules already told agents to bind workers to members and mark them as work moved, and
//! they didn't (Deck frame time, 2026-11-08), so the replies show the members instead.

use super::{EffortFile, Initiative, initiative_catalog, initiative_members};
use std::path::Path;

/// An open initiative page and its members.
pub struct Open {
    pub page: Initiative,
    pub members: Vec<(String, EffortFile)>,
}

fn closed(status: Option<&str>) -> bool {
    matches!(status, Some("done" | "abandoned"))
}

/// Every initiative not done or abandoned whose page names this repo. Unreadable pages or
/// members leave it out: these are reminders, never a gate on other work.
pub fn open_in_repo(root: &Path, repo: &str) -> Vec<Open> {
    let Ok(pages) = initiative_catalog(root) else {
        return Vec::new();
    };
    pages
        .into_iter()
        .filter(|page| !closed(page.file.prop("status")))
        .filter(|page| {
            (page.file.prop("repo").unwrap_or(""))
                .split(',')
                .any(|r| r.trim() == repo)
        })
        .filter_map(|page| {
            let members = initiative_members(root, &page).ok()?;
            Some(Open { page, members })
        })
        .collect()
}

/// Lowercase words, so "Deck frame time A3" and "deck-frame-time" compare.
fn words(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a new effort's title names this initiative, by its title or its slug.
pub fn names(title: &str, page: &Initiative) -> bool {
    let title = format!(" {} ", words(title));
    [&page.file.title, &page.slug]
        .into_iter()
        .map(|name| words(name))
        .any(|name| !name.is_empty() && title.contains(&format!(" {name} ")))
}

/// Whether any member is being worked on: active, in review or blocked.
pub fn in_flight(members: &[(String, EffortFile)]) -> bool {
    members
        .iter()
        .any(|(_, f)| matches!(f.prop("status"), Some("active" | "review" | "blocked")))
}

/// One line per state, open members by title, closed ones only counted.
pub fn roll_call(members: &[(String, EffortFile)]) -> String {
    let mut lines = Vec::new();
    for (label, states) in [
        ("active", &["active"][..]),
        ("in review", &["review"]),
        ("blocked", &["blocked"]),
        ("to start", &["proposed", "ready"]),
    ] {
        let titles: Vec<&str> = members
            .iter()
            .filter(|(_, f)| f.prop("status").is_some_and(|s| states.contains(&s)))
            .map(|(_, f)| f.title.as_str())
            .collect();
        if !titles.is_empty() {
            lines.push(format!("- {label}: {}", titles.join("; ")));
        }
    }
    let count = |s: &str| {
        members
            .iter()
            .filter(|(_, f)| f.prop("status") == Some(s))
            .count()
    };
    let (done, abandoned) = (count("done"), count("abandoned"));
    if done + abandoned > 0 {
        lines.push(format!("- closed: {done} done, {abandoned} abandoned"));
    }
    if lines.is_empty() {
        return "- no members yet".to_owned();
    }
    lines.join("\n")
}

/// What a page save, or an effort started beside one, shows of an open initiative.
pub fn summary(open: &Open) -> String {
    let mut text = format!(
        "Initiative \"{}\" ({}, {}) members now:\n{}",
        open.page.file.title,
        open.page.slug,
        open.page.file.prop("status").unwrap_or("ready"),
        roll_call(&open.members)
    );
    if open.page.file.prop("status") == Some("active")
        && !in_flight(&open.members)
        && open.members.iter().any(|(_, f)| !closed(f.prop("status")))
    {
        text += "\nNo member is active, in review or blocked while the initiative is active: if work has started or landed, its member's note is behind.";
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(slug: &str, title: &str) -> Initiative {
        let text = format!(
            "---\nstatus: active\nrepo: demo\n---\n# {title}\n\n## Current state\nGoing.\n"
        );
        Initiative {
            slug: slug.to_owned(),
            path: format!("initiatives/{slug}.md"),
            file: EffortFile::parse(&text).unwrap(),
            text,
        }
    }

    fn member(title: &str, status: &str) -> (String, EffortFile) {
        let text = format!("---\nstatus: {status}\n---\n# {title}\n\n## Current state\nGoal.\n");
        (String::new(), EffortFile::parse(&text).unwrap())
    }

    #[test]
    fn a_title_names_an_initiative_by_whole_words_of_its_title_or_slug() {
        let deck = page("deck-frame-time", "Deck frame time");
        assert!(names("Deck frame time A3 play loop", &deck));
        assert!(names("deck-frame-time: A3", &deck));
        assert!(!names("Deck frame timer", &deck));
        assert!(!names("Frame time on the Deck", &deck));
    }

    #[test]
    fn the_roll_call_lists_open_members_and_flags_nothing_in_flight() {
        let members = vec![
            member("0108 A1 Bench", "proposed"),
            member("0109 A2 Drivers", "ready"),
            member("0122 G Hub", "abandoned"),
        ];
        let open = Open {
            page: page("deck-frame-time", "Deck frame time"),
            members,
        };
        let text = summary(&open);
        assert!(
            text.contains("- to start: 0108 A1 Bench; 0109 A2 Drivers"),
            "{text}"
        );
        assert!(text.contains("- closed: 0 done, 1 abandoned"), "{text}");
        assert!(text.contains("No member is active"), "{text}");
        let open = Open {
            members: vec![member("0108 A1 Bench", "active")],
            ..open
        };
        let text = summary(&open);
        assert!(
            text.contains("- active: 0108 A1 Bench") && !text.contains("No member"),
            "{text}"
        );
    }
}
