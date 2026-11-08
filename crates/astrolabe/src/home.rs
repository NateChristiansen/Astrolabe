//! The owner's home: the board, the notes tree beside it, and each note as a page. Served on
//! localhost by `astrolabe home --serve`, and written as `home.html` at the notes checkout
//! root whenever the notes change on this machine, from the same rows `astrolabe efforts
//! --json` returns, so it opens from a file on any machine too.

use crate::notes::{self, Notes};
use std::path::PathBuf;
mod markdown;
mod nav;
mod render;
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
                "<span class=\"need\">3 waiting on you</span>",
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
                "<span class=\"need\">1 waiting on you</span>",
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
            "drafts": 2,
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
            ideas.contains("class=\"card s-x&quot; onmouseover=&quot;alert(1) compact\"")
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
            page.contains("<details data-persist=\"closed-work\">"),
            "closed work remains available behind a disclosure"
        );
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
            page.contains("class=\"quiet\">Nothing ready, nothing blocked, nothing in review, no ideas parked.<"),
            "{page}"
        );
        assert!(!page.contains("<b>0</b>"), "a zero is no chip: {page}");
        assert!(page.contains("nothing closed lately"));
        assert!(
            !page.contains("to triage"),
            "no lessons, no section and no count"
        );
        assert!(!page.contains("open draft"), "no drafts, no count");
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
            assert!(idea_section.contains("class=\"cards ideas-grid\""));
            assert_eq!(idea_section.matches("class=\"card ").count(), 15);
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
                // Ideas follows the closed status row, rather than occupying one lane.
                assert!(page.contains("</section>\n</div><section class=\"col ideas\">"));
                assert!(page.find("class=\"col ready\"") < page.find("class=\"col blocked\""));
                assert!(page.find("class=\"col blocked\"") < page.find("class=\"col ideas\""));
            } else {
                assert!(page.contains("<section class=\"col active\" hidden></section>"));
                assert!(!page.contains("<span>In motion</span>"));
                assert!(!page.contains("class=\"stack\""));
            }
        }
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
                && auto.contains("prefers-color-scheme:dark"),
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
        assert!(page[journal..earlier].contains("Update 7 &lt;safe&gt;"));
        assert!(page[journal..earlier].contains("Update 5 &lt;safe&gt;"));
        assert!(!page[journal..earlier].contains("Update 4"));
        assert!(page[earlier..].contains("Update 1 &lt;safe&gt;"));
        assert!(page.contains("class=\"brand\" href=\"home.html#overview\" data-home"));
        assert!(page.contains("id=\"system-appearance\""));
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
            page.contains("<b>0001-a.md</b>"),
            "the crumb ends in the file: {page}"
        );
        assert!(
            page.contains(
                "<a href=\"/#notes\">Notes</a><span>›</span><a href=\"/#notes/efforts\">efforts</a>"
            ),
            "each crumb folder opens the notes view there: {page}"
        );
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
            ".agents/brief.md",
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
        assert!(hid.contains("href=\"/.agents/brief.md\""), "{tree}");
        assert!(
            !tree.contains("data-key=\"efforts/2026\""),
            "a year is a shelf of months, not a level: {tree}"
        );
        assert!(tree.contains(">November 2026</span>"), "{tree}");
        assert!(tree.contains("Show .agents · 1 note agents wrote for each other"));
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
        assert!(note_page_at("# Bare\n\ntext\n").contains("<h1 id=\"bare\">Bare</h1>"));
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
