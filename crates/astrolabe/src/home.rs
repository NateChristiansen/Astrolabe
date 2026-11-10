//! The owner's home: the board, the notes tree beside it, and each note as a page. Served on
//! localhost by `astrolabe home --serve`, and written as `home.html` at the notes checkout
//! root whenever the notes change on this machine, from the same rows `astrolabe efforts
//! --json` returns, so it opens from a file on any machine too.

use crate::notes::{self, Notes};
use std::path::PathBuf;
mod markdown;
mod nav;
mod render;
mod search;
mod server;
mod settings;

pub use render::render;
pub use server::{HANDOVER_FLAG, HANDOVER_WAIT, ensure_serving, hold, listen, preview, serve};
pub use settings::{Look, Palette, SERVE_PORT, Theme, look, port, set_look, set_port, url};

pub const FILE: &str = "home.html";

/// Renders and writes the board; the path written, or why not. Under the caller's lock.
pub fn write(notes: &Notes) -> Result<PathBuf, String> {
    // A git error is no licence to overwrite.
    if !notes.git(&["ls-files", "--", FILE])?.trim().is_empty() {
        return Err(format!(
            "{FILE} is tracked in the notes repo, so it isn't overwritten; run `git rm --cached {FILE}` there and the board is drawn at the next save"
        ));
    }
    let rows = crate::record::rows_in(notes, None, true);
    let path = notes.root.join(FILE);
    notes::write_text(
        &path,
        &render(&rows, &notes.root, notes::now_ms(), look(notes)),
    )?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::nav::*;
    use super::render::*;
    use super::server::*;
    use super::settings::*;
    use super::*;
    use crate::board::BoardData;
    use serde_json::Value;
    use serde_json::json;
    use std::path::Path;
    fn note_page(text: &str, rel: &str, rows: &Value, root: &Path, now: i64, look: Look) -> String {
        super::markdown::note_page(
            text,
            rel,
            &serde_json::from_value(rows.clone()).unwrap(),
            root,
            now,
            look,
        )
    }

    #[test]
    fn an_active_initiative_with_nothing_in_flight_says_its_members_may_be_behind() {
        let progress = |active, review, blocked| crate::board::InitiativeProgress {
            total: 3,
            done: 1,
            to_start: 2 - active - review - blocked,
            active,
            review,
            blocked,
            ..Default::default()
        };
        let flag = "nothing in flight: members' notes may be behind the work";
        assert!(initiative_html("Deck", None, &progress(0, 0, 0), Some("active")).contains(flag));
        for (p, status) in [
            (progress(1, 0, 0), Some("active")),
            (progress(0, 1, 0), Some("active")),
            (progress(0, 0, 1), Some("active")),
            (progress(0, 0, 0), Some("ready")),
            (progress(0, 0, 0), None),
        ] {
            assert!(
                !initiative_html("Deck", None, &p, status).contains(flag),
                "{p:?} {status:?}"
            );
        }
    }

    #[test]
    fn initiatives_show_pages_derived_progress_and_legacy_groups() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, true);
        // The renderer consumes a snapshot, never the notes on disk.
        std::fs::remove_dir_all(&notes.root).unwrap();
        for links in [Links::File, Links::Served] {
            let html = board_html(&rows, &notes.root, NOW, links);
            let column = html
                .split("<section class=\"col initiatives\">")
                .nth(1)
                .unwrap()
                .split("</section>")
                .next()
                .unwrap();
            for expected in [
                "Initiatives</span> <span class=\"n\">4</span>",
                "initiatives/shared-work.md",
                ">Shared work</a>",
                "Building together.",
                "<span class=\"k-done\">1 of 5 done</span> · <span class=\"k-review\">1 in review</span> · <span class=\"k-active\">1 active</span> · <span class=\"k-blocked\">1 blocked</span> · <span class=\"k-start\">1 to start</span> · <span class=\"k-abandoned\">1 abandoned</span>",
                "<span class=\"need\">3 need you</span>",
                "<i class=\"k-done\" style=\"width:20.00%\"></i><i class=\"k-review\" style=\"width:20.00%\"></i><i class=\"k-active\" style=\"width:20.00%\"></i><i class=\"k-blocked\" style=\"width:20.00%\"></i></span>",
                "initiatives/finished-members.md",
                "awaiting acceptance",
                "Accept the combined result.",
                "initiatives/empty-work.md",
                "A legacy purpose.",
                "no members yet",
                "all members abandoned",
                "Unfiled work <span class=\"pill\">no page</span>",
                "<span class=\"k-done\">1 of 3 done</span> · <span class=\"k-active\">1 active</span> · <span class=\"k-blocked\">1 blocked</span>",
                "<span class=\"need\">1 needs you</span>",
                "loose-tools <span class=\"pill\">no page</span>",
                "<span class=\"k-done\">0 of 1 done</span> · <span class=\"k-start\">1 to start</span>",
            ] {
                assert!(column.contains(expected), "{expected}: {column}");
            }
            for absent in [
                "Another sentence",
                "More purpose",
                "not the summary",
                "Accepted work",
                "Abandoned page",
                "Discarded experiment",
            ] {
                assert!(!column.contains(absent), "{absent}: {column}");
            }
            // A page's whole card opens it; a group without a page isn't a link.
            assert_eq!(column.matches("<li class=\"ini linked\">").count(), 4);
            assert_eq!(column.matches("<li class=\"ini\">").count(), 2);
            assert!(html.contains("<span class=\"pill tag\">Shared work</span>"));
            assert!(!html.contains("<span class=\"pill tag\">shared-work</span>"));
            let done = html.split("Done this fortnight").nth(1).unwrap();
            assert!(
                done.contains("initiatives/accepted-work.md") && done.contains("Accepted work")
            );
            assert!(done.contains("efforts/0001-member.md"));
            assert!(!done.contains("Old work"));
        }
    }

    #[test]
    fn an_effort_page_links_to_its_initiative_under_the_title() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, true);
        let note = |text: &str| {
            super::markdown::note_page(
                text,
                "efforts/0042-member.md",
                &rows,
                &notes.root,
                NOW,
                Look::default(),
            )
        };
        let page = |initiative: &str| {
            note(&format!(
                "---\nstatus: active\ninitiative: {initiative}\n---\n# 0042 Member\n"
            ))
        };
        let linked = page("shared-work");
        assert!(
            linked.contains(
                "</h1><p class=\"part-of\">Part of <a href=\"/initiatives/shared-work.md\">Shared work →</a></p><div class=\"props\">"
            ),
            "{linked}"
        );
        assert!(!linked.contains("<i>initiative</i>"), "{linked}");
        assert!(
            page("\"[[shared-work]]\"").contains("<a href=\"/initiatives/shared-work.md\">"),
            "a wikilink names the same page"
        );
        assert!(
            page("Shared work").contains("<a href=\"/initiatives/shared-work.md\">"),
            "so does its title"
        );
        // Single quotes, a byte-order mark and a capitalised key read as the board reads them.
        for text in [
            "---\nstatus: active\ninitiative: 'shared-work'\n---\n# 0042 Member\n".to_owned(),
            "\u{feff}---\nstatus: active\ninitiative: shared-work\n---\n# 0042 Member\n".to_owned(),
            "---\nStatus: active\nInitiative: shared-work\n---\n# 0042 Member\n".to_owned(),
        ] {
            let linked = note(&text);
            assert!(
                linked.contains("<a href=\"/initiatives/shared-work.md\">Shared work →</a>"),
                "{text:?}: {linked}"
            );
            assert!(
                linked.contains("<span class=\"pill status-active\"><i>status</i>active</span>"),
                "{text:?}: {linked}"
            );
            assert!(
                !linked.contains("---"),
                "the header isn't body text: {linked}"
            );
        }
        // A list of repos is one pill, without brackets or quotes.
        let listed = note("---\nrepo: [demo, 'other']\n---\n# 0042 Member\n");
        assert!(listed.contains("<i>repo</i>demo, other</span>"), "{listed}");
        // A legacy tag with no page keeps its pill and gets no link.
        let legacy = page("loose-tools");
        assert!(!legacy.contains("class=\"part-of\""), "{legacy}");
        assert!(legacy.contains("<i>initiative</i>loose-tools"), "{legacy}");
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn the_board_a_note_page_search_and_a_save_agree_on_where_the_body_starts() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, true);
        let page = |text: &str| {
            super::markdown::note_page(
                text,
                "efforts/0042-member.md",
                &rows,
                &notes.root,
                NOW,
                Look::default(),
            )
        };
        let found = |text: &str, word: &str| {
            super::search::matches("efforts/0042-member.md", text, &[word.to_owned()]).is_some()
        };
        let body = "# 0042 Member\n\n## Current state\nBodyword here.\n";
        let props = "status: active\nrepo: headerword\n";
        for text in [
            format!("---\n{props}---\n{body}"),
            format!("---  \n{props}---\n{body}"),
            format!("---\n{props}---  \n{body}"),
            format!("\u{feff}---\n{props}---\n{body}"),
            format!("\u{feff}--- \n{props}---\t\n{body}").replace('\n', "\r\n"),
            format!("---\n{props}---\n{body}").replace('\n', "\r\n"),
            format!("---\n{props}---\n{}", body.trim_end()),
        ] {
            // The board's row and a save read the note through one parse.
            let mut file = crate::effort::EffortFile::parse(&text).unwrap();
            assert_eq!(file.title, "0042 Member", "{text:?}");
            assert_eq!(file.current, "Bodyword here.", "{text:?}");
            assert_eq!(file.prop("repo"), Some("headerword"), "{text:?}");
            file.set("status", "review");
            let saved = file.render();
            assert_eq!(saved.matches("# 0042 Member").count(), 1, "{saved:?}");
            assert_eq!(saved.matches("Bodyword here.").count(), 1, "{saved:?}");
            assert_eq!(saved.matches("headerword").count(), 1, "{saved:?}");
            let html = page(&text);
            assert!(html.contains("Bodyword here."), "{text:?}: {html}");
            assert!(html.contains("<i>repo</i>headerword</span>"), "{text:?}");
            assert_eq!(html.matches("headerword").count(), 1, "{text:?}: {html}");
            assert!(found(&text, "bodyword"), "{text:?}");
            assert!(!found(&text, "headerword"), "{text:?}");
        }
        // A header that never closes is all header to each of them: no title, no body.
        let unclosed = format!("---\n{props}{body}");
        let file = crate::effort::EffortFile::parse(&unclosed).unwrap();
        assert_eq!((file.title.as_str(), file.current.as_str()), ("", ""));
        assert_eq!(file.prop("repo"), Some("headerword"));
        assert!(!page(&unclosed).contains("Bodyword"));
        assert!(!found(&unclosed, "bodyword") && !found(&unclosed, "headerword"));
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn an_initiative_page_shows_the_boards_progress_under_its_title() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, true);
        let page = |rel: &str| {
            let text = std::fs::read_to_string(notes.root.join(rel)).unwrap();
            super::markdown::note_page(&text, rel, &rows, &notes.root, NOW, Look::default())
        };
        let shared = page("initiatives/shared-work.md");
        let start = shared.find("<header class=\"note-head\">").unwrap();
        let head = &shared[start..start + shared[start..].find("</header>").unwrap()];
        let glance = &head[head.find("<div class=\"glance\">").unwrap()..];
        // The counts, the badge and the bar are the board's own, from one function.
        let counts = "<span class=\"num\"><span class=\"k-done\">1 of 5 done</span> · <span class=\"k-review\">1 in review</span> · <span class=\"k-active\">1 active</span> · <span class=\"k-blocked\">1 blocked</span> · <span class=\"k-start\">1 to start</span> · <span class=\"k-abandoned\">1 abandoned</span></span>";
        assert!(glance.contains(counts), "{glance}");
        assert!(
            glance.contains("<span class=\"need\">3 need you</span>"),
            "members and the page itself: {glance}"
        );
        assert!(
            glance.contains("<span class=\"track\" aria-hidden=\"true\"><i class=\"k-done\" style=\"width:20.00%\"></i><i class=\"k-review\" style=\"width:20.00%\"></i>"),
            "{glance}"
        );
        let board = board_html(&rows, &notes.root, NOW, Links::Served);
        assert!(
            board.contains(counts),
            "the board draws the same counts: {board}"
        );
        // Each page gets its own members' counts; one with no members has nothing to glance at.
        let finished = page("initiatives/finished-members.md");
        assert!(
            finished.contains("<span class=\"k-done\">1 of 1 done</span>")
                && !finished.contains("1 of 5 done"),
            "{finished}"
        );
        assert!(!page("initiatives/empty-work.md").contains("class=\"glance\""));
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn a_link_to_an_effort_carries_its_status() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, true);
        let rel = "initiatives/shared-work.md";
        let text = "# Shared work\n\n## Efforts\n\n1. [First](../efforts/0004-member.md). Does the first thing.\n2. [Second](../efforts/0005-member.md). Does the second.\n3. See [Sixth](../efforts/0006-member.md) for the review. Then [Seventh](../efforts/0007-member.md).\n4. [Ninth](../efforts/0009-member.md).\n5. [Owner](../knowledge/owner.md). Not an effort.\n\n[Fourth](../efforts/0004-member.md). Opens this paragraph.\n\nAnd [Fourth](../efforts/0004-member.md) inline. Done.\n";
        std::fs::create_dir_all(notes.root.join("knowledge")).unwrap();
        std::fs::write(notes.root.join("knowledge/owner.md"), "# Owner\n").unwrap();
        let page = super::markdown::note_page(text, rel, &rows, &notes.root, NOW, Look::default());
        let start = page.find("<div class=\"prose\">").unwrap();
        let prose = &page[start..start + page[start..].find("<aside").unwrap()];
        // A link that opens its item and ends in a full stop is a title line: it leads
        // with the effort's number, the pill ends it and the stop goes.
        assert!(
            prose.contains("<li><span class=\"etitle\"><a href=\"/efforts/0004-member.md\">4 First</a><span class=\"stat s-active\">active</span></span>Does the first thing.</li>"),
            "{prose}"
        );
        assert!(
            prose.contains("5 Second</a><span class=\"stat s-blocked\">blocked</span><span class=\"stat need\">needs you</span></span>Does the second.</li>"),
            "{prose}"
        );
        // Mid-sentence, the pill is inline and the sentence stays as written.
        assert!(
            prose.contains("See <a href=\"/efforts/0006-member.md\">Sixth</a><span class=\"stat s-review\">in review</span><span class=\"stat need\">needs you</span> for the review. Then <a href=\"/efforts/0007-member.md\">Seventh</a><span class=\"stat s-ready\">to start</span>.</li>"),
            "{prose}"
        );
        // A title with nothing after its stop, and a link to a note that isn't an effort.
        assert!(
            prose.contains(">9 Ninth</a><span class=\"stat s-done\">done</span></span></li>"),
            "{prose}"
        );
        assert!(prose.contains("Owner</a>. Not an effort.</li>"), "{prose}");
        // A paragraph opened by an effort link and a stop is a title line too; the same
        // link inside a sentence is not, and keeps its text without a number.
        assert!(
            prose.contains("<p><span class=\"etitle\"><a href=\"/efforts/0004-member.md\">4 Fourth</a><span class=\"stat s-active\">active</span></span>Opens this paragraph.</p>"),
            "{prose}"
        );
        assert!(
            prose.contains("<p>And <a href=\"/efforts/0004-member.md\">Fourth</a><span class=\"stat s-active\">active</span> inline. Done.</p>"),
            "{prose}"
        );
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    /// Gives fixture effort `n` the day its note was created.
    fn created_on(notes: &Notes, n: u32, day: &str) {
        let path = notes.root.join(format!("efforts/{n:04}-member.md"));
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replacen("updated:", &format!("created: {day}\nupdated:"), 1),
        )
        .unwrap();
    }

    /// The shared-work page of the fixture with `body` as its prose, written to disk the
    /// way a stored page is, and that page rendered.
    fn shared_work_page(notes: &Notes, body: &str) -> (String, String) {
        let rel = "initiatives/shared-work.md";
        let text = format!(
            "---\nstatus: active\nrepo: demo\nwaits_for: \nupdated: 2026-11-05T12:00-07:00\n---\n# Shared work\n\n{body}"
        );
        std::fs::write(notes.root.join(rel), &text).unwrap();
        let rows = crate::board::rows_in(notes, None, true);
        let page = super::markdown::note_page(&text, rel, &rows, &notes.root, NOW, Look::default());
        let start = page.find("<div class=\"prose\">").unwrap();
        let prose = page[start..start + page[start..].find("<aside").unwrap()].to_owned();
        (text, prose)
    }

    #[test]
    fn an_initiative_page_lists_every_member_with_its_status() {
        let notes = crate::board::initiative_fixture();
        // The prose names four efforts out of number order: two members, an abandoned
        // member, and one that belongs to another initiative now. Members 5, 6 and 9 joined
        // through their own front matter and the prose never heard of them.
        // A fifth is an effort with no status line that was never a member.
        std::fs::write(
            notes.root.join("efforts/0030-plain.md"),
            "---\nrepo: demo\n---\n# 0030 Plain\n\n## Current state\nParked.\n",
        )
        .unwrap();
        // Members that joined later say when; one whose note has no created day says
        // nothing, and the prose's own entries never do.
        for n in [5, 7, 9] {
            created_on(&notes, n, &format!("2026-11-{n:02}"));
        }
        let (text, prose) = shared_work_page(
            &notes,
            "## Efforts and order\n\n1. [Seventh](../efforts/0007-member.md). Starts it.\n2. [Fourth](../efforts/0004-member.md). Follows.\n3. [Dropped](../efforts/0008-member.md). Was turned down.\n4. [Gone](../efforts/0010-member.md). Moved out.\n5. [Old](../efforts/0030-plain.md). Moved elsewhere.\n\n## What completion means\n\nAll of it works.\n",
        );
        // The prose keeps its order and its text.
        let at = |needle: &str| {
            prose
                .find(needle)
                .unwrap_or_else(|| panic!("{needle}: {prose}"))
        };
        // Every title line leads with its effort's number, inside the link.
        assert!(
            at(">7 Seventh</a><span class=\"stat s-ready\">to start</span></span>Starts it.</li>")
                < at(
                    ">4 Fourth</a><span class=\"stat s-active\">active</span></span>Follows.</li>"
                )
        );
        // An abandoned member the prose links says so, and so does one that has left.
        at(
            ">8 Dropped</a><span class=\"stat s-abandoned\">abandoned</span></span>Was turned down.</li>",
        );
        at(
            ">10 Gone</a><span class=\"stat s-done\">done</span><span class=\"stat left\">not a member</span></span>Moved out.</li>",
        );
        // One with no status has no status pill and still says it isn't a member.
        at(
            "<span class=\"etitle\"><a href=\"/efforts/0030-plain.md\">30 Old</a><span class=\"stat left\">not a member</span></span>Moved elsewhere.</li>",
        );
        // The members the prose doesn't mention continue its numbered list, in number
        // order and in the same shape: number and title, status, then the first sentence
        // of Current state, with a quiet tag saying when each joined. No separate list
        // follows.
        let listed = "Moved elsewhere.</li>\n\
            <li><span class=\"etitle\"><a href=\"/efforts/0005-member.md\">5 Member</a><span class=\"stat s-blocked\">blocked</span><span class=\"stat need\">needs you</span><span class=\"stat added\">added Nov 5</span></span>Working.</li>\n\
            <li><span class=\"etitle\"><a href=\"/efforts/0006-member.md\">6 Member</a><span class=\"stat s-review\">in review</span><span class=\"stat need\">needs you</span></span>Working.</li>\n\
            <li><span class=\"etitle\"><a href=\"/efforts/0009-member.md\">9 Member</a><span class=\"stat s-done\">done</span><span class=\"stat added\">added Nov 9</span></span>Working.</li>\n</ol>";
        assert!(at(listed) < at("What completion means</h2>"));
        assert!(!prose.contains("members-label"), "{prose}");
        assert_eq!(prose.matches("stat added").count(), 2, "{prose}");
        // No member is listed twice, and no other initiative's member is listed at all.
        for n in [4, 5, 6, 7, 8, 9, 10] {
            assert_eq!(
                prose
                    .matches(&format!("href=\"/efforts/{n:04}-member.md\""))
                    .count(),
                1,
                "{n}: {prose}"
            );
        }
        assert!(!prose.contains("0011-member.md"), "{prose}");
        // Rendering writes nothing into the stored page.
        assert_eq!(
            std::fs::read_to_string(notes.root.join("initiatives/shared-work.md")).unwrap(),
            text
        );
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn a_page_whose_prose_names_no_member_lists_them_all_at_its_end() {
        let notes = crate::board::initiative_fixture();
        created_on(&notes, 5, "2026-11-05");
        let (_, prose) = shared_work_page(&notes, "## Purpose\n\nWork together.\n");
        // With no up-front list, no member joined it later, so none is tagged as added.
        assert!(!prose.contains("stat added"), "{prose}");
        let list = prose
            .split("Work together.</p>")
            .nth(1)
            .unwrap_or_else(|| panic!("{prose}"));
        assert!(
            list.trim_start()
                .starts_with("<p class=\"members-label\">Members</p>\n<ul class=\"members\">"),
            "{prose}"
        );
        let order: Vec<usize> = [4, 5, 6, 7, 8, 9]
            .iter()
            .map(|n| {
                list.find(&format!("href=\"/efforts/{n:04}-member.md\""))
                    .unwrap()
            })
            .collect();
        assert!(order.is_sorted(), "{list}");
        assert!(
            list.contains("<li><span class=\"etitle\"><a href=\"/efforts/0008-member.md\">8 Member</a><span class=\"stat s-abandoned\">abandoned</span></span>Working.</li>"),
            "{list}"
        );
        // A page with no members, and a note that isn't an initiative, get no list; a
        // link from another note to an effort is never called a non-member.
        let rows = crate::board::rows_in(&notes, None, true);
        for (rel, text) in [
            (
                "initiatives/empty-work.md",
                "# Empty work\n\nNothing yet.\n",
            ),
            (
                "efforts/0004-member.md",
                "# 0004 Member\n\n[Gone](0010-member.md). Elsewhere.\n",
            ),
        ] {
            let page =
                super::markdown::note_page(text, rel, &rows, &notes.root, NOW, Look::default());
            assert!(
                !page.contains("class=\"members") && !page.contains("not a member"),
                "{rel}: {page}"
            );
        }
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn later_members_continue_the_list_that_holds_the_last_member_title_line() {
        let notes = crate::board::initiative_fixture();
        created_on(&notes, 5, "2026-11-05");
        // A list split across two sections: the members it doesn't name go at the end of
        // the second, which holds the last member title line, and that list's numbering
        // carries on. A title that already leads with its number keeps it once.
        let (_, prose) = shared_work_page(
            &notes,
            "## First part\n\n1. [Fourth](../efforts/0004-member.md). Starts.\n\n## Second part\n\n2. [0007 Seventh](../efforts/0007-member.md). Then this.\n   - [Ninth](../efforts/0009-member.md) nested inline.\n\n## After\n\nDone.\n",
        );
        let second = prose
            .split("Second part</h2>")
            .nth(1)
            .and_then(|s| s.split("<h2").next())
            .unwrap_or_else(|| panic!("{prose}"));
        assert!(second.contains("<ol start=\"2\">"), "{second}");
        assert!(second.contains(">0007 Seventh</a>"), "{second}");
        let tail = second
            .rsplit_once("</ul>\n</li>\n")
            .map(|(_, t)| t)
            .unwrap_or_else(|| panic!("{second}"));
        let order: Vec<usize> = [5, 6, 8]
            .iter()
            .map(|n| {
                tail.find(&format!(
                    "<li><span class=\"etitle\"><a href=\"/efforts/{n:04}-member.md\">{n} Member</a>"
                ))
                .unwrap_or_else(|| panic!("{n}: {second}"))
            })
            .collect();
        assert!(order.is_sorted(), "{tail}");
        assert!(tail.trim_end().ends_with("</ol>"), "{tail}");
        assert!(!prose.contains("members-label"), "{prose}");
        // A list whose items are paragraphs gets paragraph items.
        let (_, prose) = shared_work_page(
            &notes,
            "1. [Fourth](../efforts/0004-member.md). Starts.\n\n2. [Seventh](../efforts/0007-member.md). Then this.\n",
        );
        assert!(
            prose.contains(
                "<li>\n<p><span class=\"etitle\"><a href=\"/efforts/0005-member.md\">5 Member</a>"
            ),
            "{prose}"
        );
        // An Efforts list of bare links, with no stop or description, is a list of title
        // lines on an initiative's page, and the rest continue it.
        let (_, prose) = shared_work_page(
            &notes,
            "## Efforts\n\n1. [Fourth](../efforts/0004-member.md)\n2. [Seventh](../efforts/0007-member.md)\n",
        );
        assert!(
            prose.contains("<li><span class=\"etitle\"><a href=\"/efforts/0007-member.md\">7 Seventh</a><span class=\"stat s-ready\">to start</span></span></li>\n<li><span class=\"etitle\"><a href=\"/efforts/0005-member.md\">5 Member</a>"),
            "{prose}"
        );
        assert!(!prose.contains("members-label"), "{prose}");
        // Prose that links members only inline has no list to continue, so the rest are
        // listed after its section, labelled, in the same shape.
        let (_, prose) = shared_work_page(
            &notes,
            "## Plan\n\nStart with [Fourth](../efforts/0004-member.md), then the rest.\n\n## After\n\nDone.\n",
        );
        assert!(prose.contains("5 Member</a><span class=\"stat s-blocked\">blocked</span><span class=\"stat need\">needs you</span><span class=\"stat added\">added Nov 5</span></span>"), "{prose}");
        let label = prose
            .find("<p class=\"members-label\">Other members</p>\n<ul class=\"members\">\n<li><span class=\"etitle\"><a href=\"/efforts/0005-member.md\">5 Member</a>")
            .unwrap_or_else(|| panic!("{prose}"));
        assert!(label < prose.find("After</h2>").unwrap(), "{prose}");
        assert!(prose.contains(">Fourth</a><span"), "{prose}");
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn the_board_and_an_initiative_header_say_owner_blocked_the_same_way() {
        let notes = crate::board::initiative_fixture();
        let rows = crate::board::rows_in(&notes, None, false);
        let board = board_html(&rows, &notes.root, NOW, Links::Served);
        // Shared work has one member blocked on the owner and one in review on the owner,
        // and the page itself waits on the owner. The header counts them by status and
        // again as needing the owner, in the board's own words.
        let column = section(&board, "initiatives");
        assert!(
            column.contains("<span class=\"k-blocked\">1 blocked</span>")
                && column.contains("<span class=\"need\">3 need you</span>")
                && column.contains("<span class=\"need\">1 needs you</span>")
                && !column.contains("waiting on you"),
            "{column}"
        );
        // Their cards sit under Needs you, so the status columns are empty. The summary
        // says where they went instead of saying there are none, and the chips count them
        // by status as the header does.
        let needs = section(&board, "needsyou");
        assert!(
            needs.contains("data-n=\"3\"") && needs.contains("data-n=\"5\""),
            "{needs}"
        );
        assert!(
            board.contains(
                "class=\"quiet\">1 in review under Needs you, 2 blocked under Needs you, no ideas parked.<"
            ),
            "{board}"
        );
        assert!(
            !board.contains("othing blocked") && !board.contains("othing in review"),
            "{board}"
        );
        assert!(
            board.contains("<b>2</b> blocked</span>")
                && board.contains("<b>1</b> in review</span>")
                && board.contains("<b>4</b> need you</span>"),
            "{board}"
        );
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn a_later_member_is_described_by_its_goal_when_its_note_keeps_one() {
        let notes = crate::board::initiative_fixture();
        // Member 9 is done, so its Current state reads like an acceptance, not a purpose.
        std::fs::write(
            notes.root.join("efforts/0009-member.md"),
            "---\nstatus: done\nrepo: demo\ninitiative: shared-work\ncreated: 2026-11-09\n---\n# 0009 Member\n\nMake the ink <smooth> at speed.\n\n## Current state\nDone: I looked at it in game and accepted it.\n",
        )
        .unwrap();
        let (_, prose) = shared_work_page(
            &notes,
            "## Efforts\n\n1. [Seventh](../efforts/0007-member.md). Starts it.\n",
        );
        assert!(
            prose.contains("<span class=\"stat added\">added Nov 9</span></span>Make the ink &lt;smooth&gt; at speed.</li>"),
            "{prose}"
        );
        assert!(!prose.contains("accepted it"), "{prose}");
        // One whose note keeps no goal still reads its Current state's first sentence.
        assert!(prose.contains(">5 Member</a>"), "{prose}");
        assert!(
            prose.matches("</span>Working.</li>").count() >= 1,
            "{prose}"
        );
    }

    #[test]
    fn an_effort_page_folds_its_plan_and_shows_one_checklist() {
        let notes = crate::board::initiative_fixture();
        let rel = "efforts/0021-planned.md";
        let text = "---\nstatus: active\nrepo: demo\n---\n# 0021 Planned\n\nCap the tether.\n\n## Current state\nGoing.\n\n## Decisions\n- 2026-11-08: a over b: why.\n\n## Plan\n1. Cap it.\n2. Land it.\n\n### Pieces\n- [ ] Cap\n\n### Risks\nNone.\n\n## Pieces\n- [x] Cap (2026-11-09)\n- [ ] Land it (added 2026-11-10)\n- [x] Later one (added 2026-11-10) (2026-11-11)\n";
        std::fs::write(notes.root.join(rel), text).unwrap();
        let rows = crate::board::rows_in(&notes, None, true);
        let page = super::markdown::note_page(text, rel, &rows, &notes.root, NOW, Look::default());
        let start = page.find("<div class=\"prose\">").unwrap();
        let prose = &page[start..start + page[start..].find("<aside").unwrap()];
        // The goal reads under the title, before Current state.
        assert!(
            prose.find("Cap the tether.").unwrap() < prose.find("Current state</h2>").unwrap(),
            "{prose}"
        );
        // The plan is closed, steps and its own list inside it; the live list follows.
        let fold = prose
            .find("<details class=\"plan-record\" id=\"plan\"><summary>Plan, as approved</summary>")
            .unwrap_or_else(|| panic!("{prose}"));
        let closed = prose.find("</details>").unwrap();
        assert!(
            fold < prose.find("Cap it.").unwrap() && prose.find("Risks").unwrap() < closed,
            "{prose}"
        );
        assert!(!prose.contains("<h2 id=\"plan\">"), "{prose}");
        assert!(prose.contains("<h3 id=\"plan-pieces\">"), "{prose}");
        assert_eq!(prose.matches("id=\"pieces\"").count(), 1, "{prose}");
        let pieces = prose.find("<h2 id=\"pieces\">").unwrap();
        assert!(closed < pieces, "{prose}");
        // A later piece carries the quiet tag, before its tick day.
        assert!(
            prose.contains("Land it<span class=\"stat added\">added Nov 10</span>"),
            "{prose}"
        );
        assert!(
            prose.contains("Later one<span class=\"stat added\">added Nov 10</span> (2026-11-11)"),
            "{prose}"
        );
        // The same words elsewhere are text: in a code sample, in Decisions, or mid-line.
        let other = text.replace(
            "## Decisions\n- 2026-11-08: a over b: why.\n",
            "## Decisions\n- 2026-11-08: a over b (added 2026-11-10)\n\n```\n- [ ] x (added 2026-11-10)\n```\n",
        ).replace("- [ ] Land it (added 2026-11-10)", "- [ ] Land it (added 2026-11-10) later");
        std::fs::write(notes.root.join(rel), &other).unwrap();
        let rows2 = crate::board::rows_in(&notes, None, true);
        let page2 =
            super::markdown::note_page(&other, rel, &rows2, &notes.root, NOW, Look::default());
        assert_eq!(page2.matches("stat added").count(), 1, "{page2}");
        assert!(
            page2.contains("a over b (added 2026-11-10)") && page2.contains("x (added 2026-11-10)")
        );
        // The outline keeps the plan's own entry and none of its inner headings.
        let rail = &page[page.find("<nav class=\"outline\"").unwrap()..];
        let rail = &rail[..rail.find("</nav>").unwrap()];
        assert!(
            rail.contains("href=\"#plan\"") && !rail.contains("Risks"),
            "{rail}"
        );
        assert!(rail.contains("href=\"#pieces\""), "{rail}");
    }

    #[test]
    fn an_effort_page_shows_its_pieces_under_its_title() {
        let notes = crate::board::initiative_fixture();
        let rel = "efforts/0020-pieces.md";
        let text = "---\nstatus: active\nrepo: demo\n---\n# 0020 Pieces\n\n## Current state\nGoing.\n\n## Pieces\n- [x] one\n- [ ] two\n- [ ] three\n";
        std::fs::write(notes.root.join(rel), text).unwrap();
        let rows = crate::board::rows_in(&notes, None, true);
        let page = super::markdown::note_page(text, rel, &rows, &notes.root, NOW, Look::default());
        let start = page.find("<header class=\"note-head\">").unwrap();
        let head = &page[start..start + page[start..].find("</header>").unwrap()];
        assert!(
            head.contains("<div class=\"bar\"><span class=\"track\" role=\"progressbar\" aria-label=\"Completed pieces\" aria-valuemin=\"0\" aria-valuemax=\"3\" aria-valuenow=\"1\"><i style=\"width:33%\"></i></span><span>1 of 3 pieces</span></div>"),
            "{head}"
        );
        // An effort without a checklist gets no bar.
        let plain = "efforts/0004-member.md";
        let page = super::markdown::note_page(
            &std::fs::read_to_string(notes.root.join(plain)).unwrap(),
            plain,
            &rows,
            &notes.root,
            NOW,
            Look::default(),
        );
        let start = page.find("<header class=\"note-head\">").unwrap();
        let head = &page[start..start + page[start..].find("</header>").unwrap()];
        assert!(!head.contains("class=\"bar\""), "{head}");
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn the_board_and_a_note_page_read_the_same_properties() {
        let notes = crate::board::initiative_fixture();
        let rel = "efforts/0099-quoted.md";
        let text = "---\nstatus: 'active'\nrepo: [demo, other]\ninitiative: 'shared-work'\n---\n# 0099 Quoted properties\n\n## Current state\nWorking.\n";
        std::fs::write(notes.root.join(rel), text).unwrap();
        let rows = crate::board::rows_in(&notes, None, true);
        let row = rows.efforts.iter().find(|r| r.number == Some(99)).unwrap();
        assert_eq!(row.status.as_deref(), Some("active"));
        assert_eq!(row.repo.as_deref(), Some("demo, other"));
        assert_eq!(row.initiative_title.as_deref(), Some("Shared work"));
        let page = super::markdown::note_page(text, rel, &rows, &notes.root, NOW, Look::default());
        assert!(
            page.contains(&format!(
                "<p class=\"part-of\">Part of <a href=\"/initiatives/shared-work.md\">{} →</a></p>",
                row.initiative_title.as_deref().unwrap()
            )),
            "{page}"
        );
        for (key, value) in [
            ("status", row.status.as_deref().unwrap()),
            ("repo", row.repo.as_deref().unwrap()),
        ] {
            assert!(
                page.contains(&format!("<i>{key}</i>{value}</span>")),
                "{key}: {page}"
            );
        }
        let props = &page[page.find("<div class=\"props\">").unwrap()..];
        let props = &props[..props.find("</div>").unwrap()];
        // Values are escaped, so a quote left in one would show as an entity.
        for raw in ["&#39;", "&quot;", "[", "]"] {
            assert!(!props.contains(raw), "{raw}: {props}");
        }
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    #[test]
    fn done_pages_keep_priority_order_and_distinct_persistent_keys() {
        let notes = crate::board::initiative_fixture();
        let mut rows = crate::board::rows_in(&notes, None, true);
        let high = rows
            .efforts
            .iter_mut()
            .find(|r| r.number == Some(1))
            .unwrap();
        high.priority = Some("high".into());
        high.updated = Some("2026-11-01T12:00-07:00".into());
        let second = rows
            .initiatives
            .iter_mut()
            .find(|i| i.page.title.as_deref() == Some("Old work"))
            .unwrap();
        second.page.updated = Some("2026-11-04T12:00-07:00".into());
        let html = board_html(&rows, &notes.root, NOW, Links::Served);
        let done = html.split("Done this fortnight").nth(1).unwrap();
        assert!(
            done.find("data-n=\"1\"").unwrap()
                < done
                    .find("data-n=\"initiatives/accepted-work.md\"")
                    .unwrap()
        );
        for path in ["initiatives/accepted-work.md", "initiatives/old-work.md"] {
            assert_eq!(done.matches(&format!("data-n=\"{path}\"")).count(), 1);
        }
        assert!(!done.contains("data-n=\"0\""));
        std::fs::remove_dir_all(notes.root).unwrap();
    }

    const NOW: i64 = 1_793_916_000_000; // 2026-11-05T15:00-07:00

    fn row(n: u64, status: &str, extra: Value) -> Value {
        let mut v = json!({
            "number": n, "title": format!("Effort {n}"), "file": format!("efforts/{n:04}-e.md"),
            "status": status, "priority": "none", "initiative": "", "repo": "demo",
            "waits_for": "", "needs_owner": false, "updated": "2026-11-05T12:00-07:00",
            "next": "Next thing. Then more.", "pieces": null,
        });
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    }

    fn section<'a>(page: &'a str, key: &str) -> &'a str {
        let start = page.find(&format!("class=\"col {key}\"")).unwrap();
        let end = page[start + 1..]
            .find("<section")
            .map_or(page.len(), |i| start + 1 + i);
        &page[start..end]
    }

    #[test]
    fn an_uncommitted_lesson_or_draft_is_marked_and_a_committed_one_is_not() {
        let rows = json!({
            "today": "2026-11-05",
            "lessons": [
                {"file": "lessons/2026-11-01-gates.md", "title": "Gates"},
                {"file": "lessons/2026-11-09-new.md", "title": "Just written", "uncommitted": true},
            ],
            "drafts": [{"file": "drafts/2026-11-09-reply.md", "title": "Reply", "uncommitted": true}],
        });
        let page = render(
            &serde_json::from_value(rows).unwrap(),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        let mark = " <span class=\"pill uncommitted\" title=\"Only on this machine until the next save here commits it\">not committed</span>";
        assert!(
            page.contains(&format!(">Just written</a>{mark}</li>"))
                && page.contains(&format!(">Reply</a>{mark}</li>")),
            "{page}"
        );
        assert!(page.contains(">Gates</a></li>"), "{page}");
        assert_eq!(page.matches("not committed</span>").count(), 2);
    }

    #[test]
    fn cards_land_in_their_columns() {
        let rows = json!({
            "today": "2026-11-05",
            "efforts": [
                row(1, "active", json!({"title": "A <b>&</b> \"q\""})),
                row(2, "active", json!({"waits_for": "owner: pick one", "needs_owner": true})),
                row(3, "blocked", json!({"waits_for": "the merge driver"})),
                row(4, "ready", json!({})),
                row(5, "proposed", json!({})),
                row(6, "", json!({"updated": null})),
                row(7, "done", json!({"updated": "2026-11-02T09:00-07:00"})),
                row(8, "done", json!({"updated": "2026-10-10T09:00-07:00", "initiative": "[[finished]]"})),
                row(9, "active", json!({"priority": "high", "pieces": {"done": 2, "total": 5}, "initiative": "[[agent-tooling]]", "next": "**Next:** Measure it. Then decide."})),
                row(10, "x\" onmouseover=\"alert(1)", json!({})),
            ],
            "journal_lines": [{"effort": "0001", "text": "Finished <it>."}],
            "notes": [],
            "lessons": [{"file": "lessons/2026-11-01-gates.md", "title": "Gates <did> not stop"}],
            "drafts": [
                {"file": "drafts/2026-11-06-release-note.md", "title": "Release <note>"},
                {"file": "drafts/2026-11-06-reply.md", "title": "Reply"},
            ],
        });
        let page = render(
            &serde_json::from_value(rows.clone()).unwrap(),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        assert!(page.contains("A &lt;b&gt;&amp;&lt;/b&gt; &quot;q&quot;"));
        assert!(!page.contains("<b>&</b>"));
        let needs = section(&page, "needsyou");
        assert!(needs.contains("Effort 2") && needs.contains("class=\"ask\">pick one<"));
        let active = section(&page, "active");
        assert!(active.contains("&quot;q&quot;") && active.contains("Effort 9"));
        assert!(!active.contains("Effort 2"));
        // Priority first, then the newest save.
        assert!(active.find("Effort 9").unwrap() < active.find("&quot;q&quot;").unwrap());
        assert!(active.contains("class=\"pri high\">high<"));
        assert!(active.contains(">2 of 5 pieces<"));
        assert!(active.contains("class=\"pill tag\">agent-tooling<"));
        assert!(active.contains("class=\"next\">Measure it.<"));
        assert!(active.contains(">3 h</span>"));
        assert!(active.contains("href=\"file:///n/efforts/0001-e.md\""));
        let blocked = section(&page, "blocked");
        assert!(blocked.contains("Effort 3") && blocked.contains(">waits for the merge driver<"));
        assert!(section(&page, "ready").contains("Effort 4"));
        let ideas = section(&page, "ideas");
        assert!(ideas.contains("Effort 5") && ideas.contains("Effort 6"));
        assert!(ideas.contains("no status"));
        assert!(
            ideas.contains("class=\"card s-x&quot; onmouseover=&quot;alert(1) small\"")
                && !page.contains("onmouseover=\""),
            "a status stays inside its class: {ideas}"
        );
        let done = section(&page, "done");
        assert!(done.contains("Effort 7"));
        assert!(
            !page.contains("Effort 8"),
            "an old done effort is off the board"
        );
        assert!(page.contains("<b>Board</b>"), "{page}");
        assert!(
            page.contains("<b>1</b> needs you</span>")
                && page.contains("<b>3</b> active</span>")
                && page.contains("<b>1</b> blocked</span>")
                && page.contains("<b>1</b> done</span>"),
            "{page}"
        );
        assert!(page.contains("<b>1</b> lesson to triage</span>"), "{page}");
        assert!(
            page.contains("<h2><span>Lessons to triage</span> <span class=\"n\">1</span></h2>")
        );
        assert!(page.contains("<b>2</b> open drafts</span>"), "{page}");
        assert!(page.contains("<section class=\"col lessons\">"), "{page}");
        assert!(
            page.contains("<section class=\"col drafts\"><details class=\"fold\" data-persist=\"drafts\" open><summary><h2><span>Open drafts</span> <span class=\"n\">2</span></h2></summary>")
                && page.contains("href=\"file:///n/drafts/2026-11-06-release-note.md\">Release &lt;note&gt;</a>"),
            "{page}"
        );
        for view in ["drafts", "lessons"] {
            assert!(
                page.contains(&format!("data-view=\"{view}\" href=\"home.html#{view}\"")),
                "the sidebar links the {view} page: {page}"
            );
        }
        let nav = |view: &str| page.find(&format!("data-view=\"{view}\"")).unwrap();
        assert!(
            nav("initiatives") < nav("drafts")
                && nav("drafts") < nav("journal")
                && nav("journal") < nav("lessons")
                && nav("lessons") < nav("notes"),
            "drafts sit above journal, lessons between journal and notes"
        );
        assert!(page.contains(
            "href=\"file:///n/lessons/2026-11-01-gates.md\">Gates &lt;did&gt; not stop</a>"
        ));
        let served = board_html(
            &serde_json::from_value(rows.clone()).unwrap(),
            Path::new("/n"),
            NOW,
            Links::Served,
        );
        assert!(
            served.contains("href=\"/lessons/2026-11-01-gates.md\">Gates &lt;did&gt; not stop</a>")
        );
        assert!(page.contains("notes in sync"));
        assert!(page.contains("title=\"A &lt;b&gt;&amp;&lt;/b&gt; &quot;q&quot;\">1 · demo</a></span><span>Finished &lt;it&gt;.</span></li>"));

        assert!(
            done.contains("<details class=\"fold\" data-persist=\"done\"><summary>"),
            "closed work stays folded until the viewer opens it: {done}"
        );
        // Every section folds at its heading and the browser remembers it by name.
        for key in [
            "needsyou",
            "active",
            "blocked",
            "ready",
            "ideas",
            "today",
            "initiatives",
            "lessons",
            "drafts",
        ] {
            assert!(
                page.contains(&format!(
                    "<section class=\"col {key}\"><details class=\"fold\" data-persist=\"{key}\" open><summary><h2>"
                )),
                "{key} folds and starts open: {page}"
            );
        }
        // What needs you comes first, then what moves, waits, is ready and is parked.
        let at = |key: &str| page.find(&format!("class=\"col {key}\"")).unwrap();
        assert!(
            at("needsyou") < at("active")
                && at("active") < at("blocked")
                && at("blocked") < at("ready")
                && at("ready") < at("ideas")
                && at("ideas") < at("today")
        );
        assert!(!page.contains("class=\"stack\"") && !page.contains("quiet-hero"));
    }

    #[test]
    fn empty_board_and_warnings() {
        let rows = json!({"today": "2026-11-05", "efforts": [], "journal_lines": [], "notes": ["2 notes commit(s) not yet pushed."]});
        let page = render(
            &serde_json::from_value(rows.clone()).unwrap(),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        assert!(page.contains("class=\"sync warn\">2 notes commit(s) not yet pushed.<"));
        assert!(page.contains("nothing yet today"));
        assert!(page.contains("nothing open"));
        assert!(
            page.contains("<section class=\"col active\" hidden></section>")
                && !page.contains("<span>In motion</span>")
                && page.contains("Room for what’s next."),
            "{page}"
        );
        assert!(
            page.contains("class=\"quiet\">Nothing in review, nothing blocked, nothing ready, no ideas parked.<"),
            "{page}"
        );
        assert!(
            page.contains("<section class=\"col needsyou\" hidden></section>"),
            "the hero already says nothing needs you: {page}"
        );
        assert!(!page.contains("<b>0</b>"), "a zero is no chip: {page}");
        assert!(page.contains("nothing closed lately"));
        assert!(
            !page.contains("lessons to triage")
                && page.contains("<section class=\"col lessons none\">")
                && page.contains("nothing to triage"),
            "no lessons: no count, and the section only shows on its own page: {page}"
        );
        assert!(!page.contains("open draft"), "no drafts, no count");
        assert!(
            page.contains("<section class=\"col drafts none\">")
                && page.contains("nothing waiting on you")
        );
    }

    #[test]
    fn ideas_keep_their_order_and_space_with_or_without_active_work() {
        let ideas: Vec<_> = (1..=15)
            .map(|n| {
                row(
                    n,
                    "proposed",
                    json!({
                        "priority": if n == 1 { "high" } else { "none" },
                        "updated": format!("2026-11-05T12:{n:02}-07:00"),
                    }),
                )
            })
            .collect();
        for busy in [false, true] {
            let mut efforts = ideas.clone();
            if busy {
                efforts.extend([
                    row(20, "active", json!({})),
                    row(21, "ready", json!({})),
                    row(22, "blocked", json!({"waits_for": "a build"})),
                    row(
                        23,
                        "active",
                        json!({"needs_owner": true, "waits_for": "owner: review"}),
                    ),
                ]);
            }
            let page = board_html(
                &serde_json::from_value(json!({"efforts": efforts})).unwrap(),
                Path::new("/n"),
                NOW,
                Links::Served,
            );
            let idea_section = section(&page, "ideas");
            assert!(idea_section.contains("<div class=\"cards\">"));
            assert_eq!(idea_section.matches("class=\"card ").count(), 15);
            assert_eq!(
                idea_section.matches(" small\"").count(),
                15,
                "{idea_section}"
            );
            assert!(
                !idea_section.contains("class=\"next\"") && !idea_section.contains("pieces"),
                "a small card is a title, its meta and context: {idea_section}"
            );
            let mut previous = 0;
            for n in std::iter::once(1).chain((2..=15).rev()) {
                let position = idea_section
                    .find(&format!("href=\"/efforts/{n:04}-e.md\""))
                    .unwrap();
                assert!(
                    position > previous,
                    "priority and recency order stays row-major"
                );
                previous = position;
            }
            if busy {
                assert!(section(&page, "active").contains("Effort 20"));
                assert!(section(&page, "needsyou").contains("Effort 23"));
                // Each status is its own full-width grid: waiting, then ready, then ideas.
                assert!(page.find("class=\"col blocked\"") < page.find("class=\"col ready\""));
                assert!(page.find("class=\"col ready\"") < page.find("class=\"col ideas\""));
            } else {
                assert!(page.contains("<section class=\"col active\" hidden></section>"));
                assert!(!page.contains("<span>In motion</span>"));
            }
        }
    }

    /// A busy repo's attention is one grid of full cards, not a column beside the headline.
    #[test]
    fn needs_you_is_one_grid_of_full_cards() {
        let efforts: Vec<_> = (1..=15)
            .map(|n| {
                row(
                    n,
                    "active",
                    json!({
                        "needs_owner": true,
                        "waits_for": format!("owner: look at {n}"),
                        "pieces": {"done": 1, "total": 2},
                    }),
                )
            })
            .collect();
        let page = board_html(
            &serde_json::from_value(json!({"efforts": efforts})).unwrap(),
            Path::new("/n"),
            NOW,
            Links::Served,
        );
        let needs = section(&page, "needsyou");
        assert_eq!(needs.matches("<div class=\"cards\">").count(), 1);
        assert_eq!(
            needs.matches("class=\"card s-needs\"").count(),
            15,
            "full cards, none small: {needs}"
        );
        assert_eq!(needs.matches("class=\"review-link\"").count(), 15);
        assert!(needs.contains(">1 of 2 pieces<") && needs.contains("class=\"next\">Next thing.<"));
        assert!(page.contains("15 things need you."));
        assert!(
            page.contains("<div class=\"hero\"><p class=\"eyebrow\">"),
            "the headline stands alone above the work: {page}"
        );
    }

    #[test]
    fn sentences_and_ages() {
        assert_eq!(first_sentence("One. Two."), "One.");
        assert_eq!(first_sentence("**Next:** go\nnow. x"), "go now.");
        assert_eq!(first_sentence("No stop"), "No stop");
        let long = "x".repeat(200);
        let cut = first_sentence(&long);
        assert_eq!(cut.chars().count(), SENTENCE_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(age(5 * 60_000), "5 min");
        assert_eq!(age(3 * 3_600_000 + 1), "3 h");
        assert_eq!(age(50 * 3_600_000), "2 d");
        assert_eq!(stamp_ms("2026-11-05T12:00-07:00"), Some(1_793_905_200_000));
        assert_eq!(stamp_ms("nope"), None);
    }

    #[test]
    fn themes_and_palettes() {
        let rows = json!({"today": "2026-11-05", "efforts": [], "journal_lines": [], "notes": []});
        let auto = render(
            &serde_json::from_value(rows.clone()).unwrap(),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        assert!(
            auto.contains("<html lang=\"en\" data-palette=\"iris\"><head>")
                && auto.contains("content=\"light dark\"")
                && auto
                    .split_whitespace()
                    .collect::<String>()
                    .contains("prefers-color-scheme:dark"),
            "{auto}"
        );
        let dark = render(
            &serde_json::from_value(rows.clone()).unwrap(),
            Path::new("/n"),
            NOW,
            Look {
                theme: Theme::Dark,
                palette: Palette::Iris,
            },
        );
        assert!(dark.contains("data-palette=\"iris\" data-theme=\"dark\""));
        assert!(dark.contains("content=\"dark\""));
        assert!(dark.contains(":root[data-palette=\"iris\"]"));
        let at = |palette| auto.find(&format!("data-p=\"{palette}\"")).unwrap();
        assert!(at("iris") < at("ember") && at("ember") < at("slate"));
        assert_eq!(Palette::default(), Palette::Iris);
        assert_eq!(Theme::parse("system"), Some(Theme::Auto));
        assert_eq!(Theme::parse("blue"), None);
        assert_eq!(Palette::parse("slate"), Some(Palette::Slate));
        assert_eq!(Palette::parse("mauve"), None);
        for palette in [
            Palette::Iris,
            Palette::Ember,
            Palette::Slate,
            Palette::Moss,
            Palette::Graphite,
        ] {
            assert_eq!(Palette::parse(palette.name()), Some(palette));
            for theme in [Theme::Auto, Theme::Light, Theme::Dark] {
                let page = render(
                    &BoardData::default(),
                    Path::new("/n"),
                    NOW,
                    Look { theme, palette },
                );
                assert!(page.contains(&format!("data-palette=\"{}\"", palette.name())));
                assert!(page.contains(&format!("data-p=\"{}\"", palette.name())));
            }
        }
    }

    #[test]
    fn studio_keeps_attention_and_journal_context_with_real_counts() {
        let rows = json!({"efforts": [
            row(1, "active", json!({"needs_owner":true,"waits_for":"owner: Choose <one>","state":"Context & detail."})),
            row(2, "blocked", json!({"needs_owner":true,"waits_for":"owner: Read the result"})),
            row(3, "active", json!({"pieces":{"done":9,"total":3}})),
            row(4, "ready", json!({"pieces":{"done":0,"total":0}}))
        ], "journal_lines": (1..=7).map(|n| json!({"effort":"0001","text":format!("Update {n} <safe>")})).collect::<Vec<_>>()});
        let page = render(
            &serde_json::from_value(rows).unwrap(),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        assert!(page.contains("2 things need you."));
        assert!(page.contains("One thing moving."));
        assert_eq!(page.matches("class=\"review-link\"").count(), 2);
        assert!(page.contains("Context &amp; detail."));
        assert!(page.contains("Choose &lt;one&gt;"));
        assert!(page.contains("aria-valuemax=\"3\" aria-valuenow=\"3\""));
        assert!(!page.contains("width:300%"));
        assert!(!page.contains("0 of 0 pieces"));
        let journal = page.find("id=\"journal\"").unwrap();
        let earlier = page.find("data-persist=\"journal-history\"").unwrap();
        // The newest five show; the rest wait behind the disclosure.
        assert!(page[journal..earlier].contains("Update 7 &lt;safe&gt;"));
        assert!(page[journal..earlier].contains("Update 3 &lt;safe&gt;"));
        assert!(!page[journal..earlier].contains("Update 2"));
        assert!(page[earlier..].contains("Earlier updates <span>2</span>"));
        assert!(page[earlier..].contains("Update 1 &lt;safe&gt;"));
        assert!(page.contains("class=\"brand\" href=\"home.html#overview\" data-home"));
        assert!(page.contains("id=\"system-appearance\""));
        for view in ["initiatives", "drafts", "journal", "lessons"] {
            assert!(
                page.contains(&format!(
                    "class=\"page-head view-head\" data-for=\"{view}\"><h1>"
                )),
                "each list view has its own title: {view}"
            );
        }
        assert!(
            !page.contains("class=\"back-link\""),
            "the board has no way back to itself"
        );
    }

    #[test]
    fn the_icon_is_self_contained_on_boards_and_notes() {
        let rows = json!({"efforts": [], "notes": []});
        for page in [
            render(
                &serde_json::from_value(rows.clone()).unwrap(),
                Path::new("/n"),
                NOW,
                Look::default(),
            ),
            page(
                &serde_json::from_value(rows.clone()).unwrap(),
                Path::new("/n"),
                NOW,
                Look::default(),
                SERVE_REFRESH_SECS,
                Links::Served,
            ),
            note_page_at("# A note\n"),
        ] {
            assert!(page.contains("class=\"astrolabe-mark brand-icon\""));
            let marker = "href=\"data:image/svg+xml,";
            let encoded = page
                .split_once(marker)
                .unwrap()
                .1
                .split('"')
                .next()
                .unwrap();
            assert_eq!(decode(encoded).as_deref(), Some(ICON));
            assert!(page.contains("id=\"favicon\" rel=\"icon\" type=\"image/svg+xml\""));
        }
    }

    fn note_page_at(text: &str) -> String {
        note_page(
            text,
            "efforts/0001-x.md",
            &json!({"efforts": []}),
            Path::new("/n"),
            NOW,
            Look::default(),
        )
    }

    #[test]
    fn links_resolve_to_notes_and_headings_get_ids() {
        let dir = std::env::temp_dir().join(format!("astrolabe-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("efforts/2026/11-November")).unwrap();
        std::fs::create_dir_all(dir.join("knowledge")).unwrap();
        std::fs::write(dir.join("knowledge/owner.md"), "# Owner\n").unwrap();
        std::fs::write(dir.join("efforts/2026/11-November/0002-b.md"), "# b\n").unwrap();
        let rel = "efforts/2026/11-November/0001-a.md";
        let text = "# 0001 A\n\n## Current state\n\nSee [b](0002-b.md), [owner](../../../knowledge/owner.md#Boundaries), [root](knowledge/owner.md), [gone](nope.md), [[Rough Sketch]].\n\n### Pieces & bits\n\n- [ ] one <!-- hidden -->\n";
        let page = note_page(
            text,
            rel,
            &json!({"efforts": []}),
            &dir,
            NOW,
            Look::default(),
        );
        assert!(
            page.contains("href=\"/efforts/2026/11-November/0002-b.md\""),
            "{page}"
        );
        assert!(
            page.contains("href=\"/knowledge/owner.md#boundaries\""),
            "{page}"
        );
        assert_eq!(
            page.matches("href=\"/knowledge/owner.md").count(),
            3,
            "{page}"
        );
        assert!(
            page.contains("href=\"nope.md\""),
            "a link to nothing is left as written"
        );
        assert!(page.contains("href=\"#\">Rough Sketch</a>"), "{page}");
        assert!(page.contains("<h2 id=\"current-state\">"), "{page}");
        assert!(page.contains("<h3 id=\"pieces-bits\">"), "{page}");
        assert!(
            page.contains("class=\"l2\" href=\"#current-state\""),
            "an outline: {page}"
        );
        assert!(
            !page.contains("hidden -->"),
            "a comment is not shown: {page}"
        );
        assert!(
            page.contains("<b>0001-a</b>"),
            "the crumb ends in the note's name as the tree shows it: {page}"
        );
        assert!(
            page.contains(
                "<a href=\"/#notes\">Notes</a><span>›</span><a href=\"/#notes/efforts\">efforts</a>"
            ),
            "each crumb folder opens the notes view there: {page}"
        );
        assert!(
            page.contains("<a href=\"/#notes/efforts/2026/11-November\">November</a>"),
            "a month folder reads as its name, as in the tree: {page}"
        );
        // The rail holds the outline above the facts; the title heads the column.
        let rail = &page[page.find("<aside class=\"rail\">").unwrap()..];
        assert!(
            rail.find("class=\"outline\"").unwrap() < rail.find("class=\"facts\"").unwrap(),
            "{page}"
        );
        assert!(
            page.contains("<header class=\"note-head\"><h1 id=\"0001-a\">"),
            "{page}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crumb_folder_link_is_encoded_and_decodes_to_the_folder() {
        let page = note_page(
            "# n\n",
            "50% done/a#b/what?/n.md",
            &json!({"efforts": []}),
            Path::new("/n"),
            NOW,
            Look::default(),
        );
        for (link, folder, label) in [
            ("50%25%20done", "50% done", "50% done"),
            ("50%25%20done/a%23b", "50% done/a#b", "a#b"),
            ("50%25%20done/a%23b/what%3F", "50% done/a#b/what?", "what?"),
        ] {
            assert!(
                page.contains(&format!("<a href=\"/#notes/{link}\">{label}</a>")),
                "{link}: {page}"
            );
            // What the page's script decodes it to is the folder's key in the tree.
            assert_eq!(decode(link).as_deref(), Some(folder));
        }
    }

    #[test]
    fn a_link_written_encoded_resolves_to_the_note_it_names() {
        let dir = std::env::temp_dir().join(format!("astrolabe-encoded-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes/knowledge")).unwrap();
        for name in ["my note.md", "a#b.md", "100%.md", "a%20b.md"] {
            std::fs::write(dir.join("notes/knowledge").join(name), "# n\n").unwrap();
        }
        std::fs::write(dir.join("secret.md"), "# s\n").unwrap();
        let root = dir.join("notes");
        let to = |dest: &str| markdown::resolve_link(&root, "knowledge/from.md", dest);
        assert_eq!(
            to("my%20note.md").as_deref(),
            Some("/knowledge/my%20note.md")
        );
        assert_eq!(
            to("knowledge/my%20note.md").as_deref(),
            Some("/knowledge/my%20note.md"),
            "from the root too"
        );
        assert_eq!(
            to("a%23b.md#Top").as_deref(),
            Some("/knowledge/a%23b.md#top")
        );
        // A name is tried as written first, so a real `%` in one still resolves.
        assert_eq!(to("100%.md").as_deref(), Some("/knowledge/100%25.md"));
        assert_eq!(to("100%25.md").as_deref(), Some("/knowledge/100%25.md"));
        assert_eq!(to("a%20b.md").as_deref(), Some("/knowledge/a%2520b.md"));
        // Decoding opens no way out of the checkout.
        assert_eq!(to("%2E%2E/%2E%2E/secret.md"), None);
        assert_eq!(to("..%2F..%2Fsecret.md"), None);
        assert_eq!(to("..%5C..%5Csecret.md"), None);
        // Nor does a decoded `:`, which the check on the link as written refuses: a drive
        // or a scheme can't be spelled with an escape instead.
        std::fs::write(dir.join("notes/knowledge/a:b.md"), "# n\n").unwrap_or_default();
        assert_eq!(to("a%3Ab.md"), None);
        let outside = dir.join("secret.md");
        let drive = outside
            .to_string_lossy()
            .replace('\\', "/")
            .replacen(':', "%3A", 1);
        assert_eq!(to(&drive), None);
        assert_eq!(to("C%3A/Users/me/x.txt"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_is_served_only_from_inside_the_checkout() {
        let dir = std::env::temp_dir().join(format!("astrolabe-inside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes/knowledge")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::fs::write(dir.join("notes/knowledge/owner.md"), "# o\n").unwrap();
        std::fs::write(dir.join("elsewhere/secret.md"), "# s\n").unwrap();
        let root = dir.join("notes");
        assert!(inside(&root, "knowledge/owner.md"));
        assert!(!inside(&root, "knowledge/nope.md"), "nothing there");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("elsewhere"), root.join("out")).unwrap();
            std::os::unix::fs::symlink(
                dir.join("elsewhere/secret.md"),
                root.join("knowledge/alias.md"),
            )
            .unwrap();
            assert!(!inside(&root, "out/secret.md"), "a folder link out");
            assert!(!inside(&root, "knowledge/alias.md"), "a file link out");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_look_keeps_the_port_and_the_port_keeps_the_look() {
        let dir = std::env::temp_dir().join(format!("astrolabe-look-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let notes = Notes::new(dir.join("notes"), dir.join("state"));
        assert_eq!(look(&notes).palette, Palette::Iris);
        set_look(&notes, None, Some(Palette::Ember)).unwrap();
        assert_eq!(
            look(&notes).palette,
            Palette::Ember,
            "keep an explicit choice"
        );
        assert_eq!(port(&notes), SERVE_PORT);
        set_port(&notes, 5050).unwrap();
        set_look(&notes, Some(Theme::Dark), Some(Palette::Slate)).unwrap();
        assert_eq!(port(&notes), 5050);
        set_port(&notes, 5051).unwrap();
        assert_eq!(
            look(&notes),
            Look {
                theme: Theme::Dark,
                palette: Palette::Slate
            }
        );
        assert_eq!(saved_port(&notes), Some(5051));
        set_port(&notes, 0).unwrap();
        assert_eq!(port(&notes), SERVE_PORT, "0 means the default");
        assert_eq!(saved_port(&notes), Some(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_tree_lists_shelves_newest_first_and_hides_dot_folders() {
        let dir = std::env::temp_dir().join(format!("astrolabe-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for p in [
            "efforts/2026/10-October/0001-a.md",
            "efforts/2026/11-November/0002-b.md",
            "journal/2026/11-November/05-Thursday.md",
            "knowledge/owner.md",
            "lessons/2026-11-06-trial-binary-killed.md",
            ".handoffs/brief.md",
            "README.md",
        ] {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "# x\n").unwrap();
        }
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/config"), "").unwrap();
        let rows = json!({"efforts": [
            {"number": 2, "title": "B", "file": "efforts/2026/11-November/0002-b.md", "status": "active", "needs_owner": false},
        ]});
        let tree = tree_html(
            &dir,
            &serde_json::from_value(rows.clone()).unwrap(),
            Links::Served,
            Some("efforts/2026/11-November/0002-b.md"),
        );
        let at = |s: &str| tree.find(s).unwrap_or_else(|| panic!("{s} in {tree}"));
        assert!(at("data-key=\"efforts\"") < at("data-key=\"journal\""));
        assert!(at("data-key=\"journal\"") < at("data-key=\"knowledge\""));
        assert!(at("November") < at("October"), "newest month first: {tree}");
        assert!(
            tree.contains("<details open data-key=\"efforts/2026/11-November\">"),
            "{tree}"
        );
        assert!(
            tree.contains("<details data-key=\"efforts/2026/10-October\">"),
            "{tree}"
        );
        assert!(
            tree.contains(
                "class=\"f on\" style=\"--d:2\" href=\"/efforts/2026/11-November/0002-b.md\""
            ),
            "{tree}"
        );
        assert!(tree.contains("class=\"st s-active\""));
        assert!(
            !tree.contains(">Board<"),
            "the sidebar leads home, not the tree: {tree}"
        );
        assert!(tree.contains("<span class=\"num\">2</span><span class=\"t\">B</span>"));
        assert!(tree.contains(
            "<span class=\"num date\">Nov 6</span><span class=\"t\">trial-binary-killed</span>"
        ));
        assert!(tree.contains("<span class=\"num\">05</span><span class=\"t\">Thursday</span>"));
        assert!(tree.contains("href=\"/README.md\""));
        assert!(!tree.contains(".git/"), "{tree}");
        let hid = &tree[at("id=\"hid\" hidden")..];
        assert!(hid.contains("href=\"/.handoffs/brief.md\""), "{tree}");
        assert!(
            !tree.contains("data-key=\"efforts/2026\""),
            "a year is a shelf of months, not a level: {tree}"
        );
        assert!(tree.contains(">November 2026</span>"), "{tree}");
        assert!(tree.contains("Show handoffs · 1 note agents wrote for each other"));
        assert!(
            tree.contains("data-key=\".handoffs\"><summary style=\"--d:0\"><span class=\"chev\">›</span><span class=\"t\">handoffs</span>"),
            "the dot stays on disk, not on the page: {tree}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_note_is_a_page_with_its_properties_and_no_raw_html() {
        let text = "---\nstatus: active\nrepo: demo\nwaits_for: \"owner: pick\"\nupdated: \"2026-11-05T12:00-07:00\"\n---\n# 0001 Smooth <ink>\n\n## Current state\n\nDone & dusted. <script>alert(1)</script>\n\n- [x] one\n";
        let page = note_page_at(text);
        assert!(
            page.contains("<title>0001 Smooth &lt;ink&gt;</title>"),
            "{page}"
        );
        assert!(
            page.contains("<h1 id=\"0001-smooth\">0001 Smooth &lt;ink&gt;</h1>"),
            "{page}"
        );
        assert!(
            page.contains("class=\"pill status-active\"><i>status</i>active</span>")
                && page.contains("<i>waits for</i>owner: pick</span>"),
            "{page}"
        );
        assert!(page.contains("<h2 id=\"current-state\">Current state</h2>"));
        assert!(page.contains("Done &amp; dusted."), "{page}");
        assert!(
            !page.contains("<script>alert") && page.contains("&lt;script&gt;alert(1)"),
            "raw html is text: {page}"
        );
        assert!(page.contains("checked"), "task lists render: {page}");
        assert!(page.contains("href=\"/\""), "a way back to the board");
        assert!(
            !page.contains("http-equiv=\"refresh\""),
            "a note doesn't reload under the reader"
        );
        // No frontmatter: the whole text is the body.
        let bare = note_page_at("# Bare\n\ntext\n");
        assert!(bare.contains("<h1 id=\"bare\">Bare</h1>"));
        assert!(
            bare.contains("<aside class=\"rail\"><div class=\"facts\">"),
            "a note without sections keeps its facts in the rail: {bare}"
        );
        assert!(
            bare.contains("<div class=\"note-back\"><a class=\"back-link\" href=\"/\">"),
            "{bare}"
        );
        // Links keep to pages and mail; a script or file link loses its destination.
        let links = note_page_at(
            "[a](javascript:alert(1)) [b](JAVASCRIPT:x) [c](&#106;avascript:y) [d](vbscript:z) [e](file:///etc/passwd) [f](data:text/html,x) ![g](javascript:w) [h](https://example.com/x?y#z) [i](mailto:o@example.com) [j](/efforts/0002-x.md) [k](other.md) [l](a/b:c)\n",
        );
        // The note's own part of the page: the shell's style and script have their own
        // `href="#"` rule and the tree its own links.
        assert!(links.contains("Content-Security-Policy"), "{links}");
        let links = &links[links.find("class=\"prose\"").unwrap()..links.find("</main>").unwrap()];
        assert!(
            !links.to_ascii_lowercase().contains("javascript:"),
            "{links}"
        );
        for bad in ["vbscript:", "file:", "data:text"] {
            assert!(!links.contains(bad), "{bad}: {links}");
        }
        assert_eq!(links.matches("href=\"#\"").count(), 6, "{links}");
        assert!(links.contains("src=\"#\""), "{links}");
        for good in [
            "href=\"https://example.com/x?y#z\"",
            "href=\"mailto:o@example.com\"",
            "href=\"/efforts/0002-x.md\"",
            "href=\"other.md\"",
            "href=\"a/b:c\"",
        ] {
            assert!(links.contains(good), "{good}: {links}");
        }
    }

    #[test]
    fn served_names_round_trip() {
        assert_eq!(
            decode(&percent("0001-caf\u{e9} b.md")).as_deref(),
            Some("0001-caf\u{e9} b.md")
        );
        assert_eq!(decode("a%2"), None);
        assert_eq!(decode("%ff"), None);
        assert_eq!(decode("%+A"), None);
    }

    #[test]
    fn a_search_reads_its_words_from_the_query_string() {
        assert_eq!(
            query_param("/search?q=plain+words%2B%C3%A9", "q").as_deref(),
            Some("plain words+\u{e9}")
        );
        assert_eq!(query_param("/search?x=1&q=a", "q").as_deref(), Some("a"));
        assert_eq!(query_param("/search?q", "q").as_deref(), Some(""));
        assert_eq!(query_param("/search", "q"), None);
        assert_eq!(query_param("/search?q=%zz", "q"), None);
    }

    #[test]
    fn file_links() {
        assert_eq!(
            file_url(Path::new("/Users/n/My Notes/efforts/1-a#b.md")),
            "file:///Users/n/My%20Notes/efforts/1-a%23b.md"
        );
        assert_eq!(
            file_url(Path::new("/n/caf\u{e9}.md")),
            "file:///n/caf%C3%A9.md"
        );
        assert_eq!(
            file_url(Path::new("C:\\Users\\n\\notes\\x.md")),
            "file:///C:/Users/n/notes/x.md"
        );
        assert_eq!(
            file_url(Path::new("\\\\?\\C:\\n\\x.md")),
            "file:///C:/n/x.md"
        );
    }
}
