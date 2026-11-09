//! Board and page markup from a typed snapshot; no Git operations.
use super::nav::tree_html;
use super::settings::{Look, Palette, Theme};
use crate::board::{BoardData, EffortRow, Lesson};
use chrono::{DateTime, FixedOffset};
use std::path::Path;

/// The same vector is used in the page, browser tab and project artwork.
pub(super) const ICON: &str = include_str!("../../../../assets/branding/astrolabe.svg");

/// Done and abandoned efforts stay on the board this long after their last save.
pub(super) const DONE_DAYS: i64 = 14;

/// How often the file reloads itself in a browser, and how often the served board does.
pub(super) const FILE_REFRESH_SECS: u32 = 60;

pub(super) const SERVE_REFRESH_SECS: u32 = 5;

pub(super) const SENTENCE_CHARS: usize = 160;

pub(super) struct Card<'a> {
    row: &'a EffortRow,
    updated_ms: Option<i64>,
}

impl Card<'_> {
    fn status(&self) -> &str {
        self.row.status.as_deref().unwrap_or_default()
    }
    fn priority_rank(&self) -> u8 {
        match self.row.priority.as_deref().unwrap_or_default() {
            "urgent" | "top" => 0,
            "high" => 1,
            "low" => 3,
            _ => 2,
        }
    }
}

/// Where a link points: the file on disk, or the same note through the server (a browser
/// refuses a file link from an http page).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Links {
    File,
    Served,
}

impl Links {
    pub(super) fn href(self, root: &Path, rel: &str) -> String {
        match self {
            Links::Served => format!("/{}", percent(rel)),
            Links::File => file_url(&root.join(rel)),
        }
    }
}

/// The board as a file. `now_ms` is passed in so a test can fix the clock.
pub fn render(rows: &BoardData, root: &Path, now_ms: i64, look: Look) -> String {
    page(rows, root, now_ms, look, FILE_REFRESH_SECS, Links::File)
}

pub(super) fn page(
    rows: &BoardData,
    root: &Path,
    now_ms: i64,
    look: Look,
    refresh_secs: u32,
    links: Links,
) -> String {
    let crumb = "<b>Board</b>";
    shell(
        look,
        "Astrolabe home",
        crumb,
        &board_html(rows, root, now_ms, links),
        &tree_html(root, rows, links, None),
        // The file reloads itself; the served page holds a stream and swaps in place.
        (links == Links::File).then_some(refresh_secs),
        links,
    )
}

/// What needs the owner and active work first, followed by review, waiting, ready and
/// proposed work, recent journal entries, initiatives and recently closed work. Every
/// section folds, and the browser remembers each viewer's folds.
pub(super) fn board_html(rows: &BoardData, root: &Path, now_ms: i64, links: Links) -> String {
    let at = At {
        root,
        now_ms,
        links,
    };
    let cards: Vec<Card> = rows.efforts.iter().map(Card::of).collect();
    let attention_pages: Vec<Card> = rows.initiative_attention.iter().map(Card::of).collect();
    let closed_pages: Vec<Card> = rows
        .initiatives
        .iter()
        .filter(|i| i.page.status.as_deref() == Some("done"))
        .map(|i| Card::of(&i.page))
        .filter(|c| at.fresh(c))
        .collect();
    let columns = columns(&cards, &attention_pages, &closed_pages, at);
    let below = format!(
        "<div class=\"below\">{}{}{}</div>\n",
        journal_section(rows, &cards, at),
        initiatives_section(rows, at),
        lessons_section(rows, at)
    );
    let board =
        view_heads() + &work_sections(&columns, at) + &below + &done_section(&columns[DONE], at);
    hero(rows, &columns, &chips(rows, &columns), &board, now_ms)
}

/// What a board section needs to draw a link or an age.
#[derive(Clone, Copy)]
struct At<'a> {
    root: &'a Path,
    now_ms: i64,
    links: Links,
}

impl At<'_> {
    /// Closed this fortnight, so still on the board.
    fn fresh(self, card: &Card) -> bool {
        card.updated_ms
            .is_some_and(|t| self.now_ms - t <= DONE_DAYS * 86_400_000)
    }
}

impl<'a> Card<'a> {
    fn of(row: &'a EffortRow) -> Self {
        Card {
            row,
            updated_ms: row.updated.as_deref().and_then(stamp_ms),
        }
    }
}

// The board's columns, by slot.
const NEEDS: usize = 0;
const ACTIVE: usize = 1;
const READY: usize = 2;
const BLOCKED: usize = 3;
const IDEAS: usize = 4;
const DONE: usize = 5;
const REVIEW: usize = 6;

type Columns<'c, 'r> = [Vec<&'c Card<'r>>; 7];

/// Each card in its column, highest priority and most recent first. Initiative pages that
/// wait on the owner join the attention column, and pages closed this fortnight join done.
fn columns<'c, 'r>(
    cards: &'c [Card<'r>],
    attention_pages: &'c [Card<'r>],
    closed_pages: &'c [Card<'r>],
    at: At,
) -> Columns<'c, 'r> {
    let mut columns: Columns = Default::default();
    for card in cards {
        let slot = if card.row.needs_owner {
            NEEDS
        } else {
            match status_slot(card.status()) {
                DONE if !at.fresh(card) => continue,
                slot => slot,
            }
        };
        columns[slot].push(card);
    }
    columns[NEEDS].extend(attention_pages.iter());
    columns[DONE].extend(closed_pages.iter());
    for column in &mut columns {
        column.sort_by_key(|c| (c.priority_rank(), -c.updated_ms.unwrap_or(0)));
    }
    columns
}

/// The column a status files a card in when it doesn't wait on the owner.
fn status_slot(status: &str) -> usize {
    match status {
        "active" => ACTIVE,
        "ready" => READY,
        "blocked" => BLOCKED,
        "review" => REVIEW,
        "done" | "abandoned" => DONE,
        _ => IDEAS,
    }
}

/// How many cards under Needs you have the status that column `slot` holds. A card
/// that waits on the owner sits there instead of in its status column, and an
/// initiative's header still counts it by status, so the board's counts and summary
/// count it too.
fn held_for_owner(columns: &Columns, slot: usize) -> usize {
    columns[NEEDS]
        .iter()
        .filter(|c| status_slot(c.status()) == slot)
        .count()
}

/// The titles of the views that show one list, headed the way the notes view is. Each
/// shows only in its own view.
fn view_heads() -> String {
    [
        ("initiatives", "Initiatives", "Plans that span efforts."),
        ("drafts", "Drafts", "Text waiting for you to post or decide on."),
        ("journal", "Journal", "What changed, day by day."),
        ("lessons", "Lessons", "Friction waiting for triage."),
    ]
    .iter()
    .map(|(view, title, line)| {
        format!("<header class=\"page-head view-head\" data-for=\"{view}\"><h1>{title}</h1><p>{line}</p></header>\n")
    })
    .collect()
}

fn heading(title: &str, n: usize) -> String {
    format!("<h2><span>{title}</span> <span class=\"n\">{n}</span></h2>")
}

/// How many journal lines show before the rest fold away.
pub(super) const RECENT_JOURNAL: usize = 5;

/// A board section that folds: its heading is the disclosure's summary, and the script
/// keeps each viewer's choice under the `data-persist` key. `extra` adds to the section's
/// class; `open` is the state before the viewer has chosen.
fn fold(key: &str, extra: &str, title: &str, n: usize, open: bool, body: &str) -> String {
    format!(
        "<section class=\"col {key}{extra}\"><details class=\"fold\" data-persist=\"{key}\"{}><summary>{}</summary>\n{body}</details></section>\n",
        if open { " open" } else { "" },
        heading(title, n)
    )
}

fn cards_of(column: &[&Card], small: bool, at: At) -> String {
    column
        .iter()
        .map(|c| card_html(c, at.root, at.now_ms, at.links, small))
        .collect()
}

/// The work: what needs the owner, then active cards, review, waiting, ready and ideas,
/// each a folding grid that fits the window, and a line naming what's empty. A status
/// whose only cards wait on the owner isn't empty: the line says they're under Needs you.
fn work_sections(columns: &Columns, at: At) -> String {
    let mut sections = String::new();
    let mut quiet = Vec::new();
    for (key, title, slot, none, held, small) in [
        ("needsyou", "Needs you", NEEDS, "", "", false),
        ("active", "In motion", ACTIVE, "", "", false),
        (
            "review",
            "In review",
            REVIEW,
            "nothing in review",
            "in review",
            true,
        ),
        (
            "blocked",
            "Waiting",
            BLOCKED,
            "nothing blocked",
            "blocked",
            true,
        ),
        ("ready", "Up next", READY, "nothing ready", "ready", true),
        ("ideas", "Ideas", IDEAS, "no ideas parked", "parked", true),
    ] {
        let column = &columns[slot];
        if column.is_empty() {
            if none.is_empty() {
                // The hero already says nothing needs you or moves. The section stays in
                // the page so a test and the live swap can find the column by name.
                sections += &format!("<section class=\"col {key}\" hidden></section>\n");
            } else {
                quiet.push(match held_for_owner(columns, slot) {
                    0 => none.to_owned(),
                    n => format!("{n} {held} under Needs you"),
                });
            }
            continue;
        }
        sections += &fold(
            key,
            "",
            title,
            column.len(),
            true,
            &format!("<div class=\"cards\">{}</div>", cards_of(column, small, at)),
        );
    }
    let mut work = format!("<div class=\"work\">{sections}</div>\n");
    if !quiet.is_empty() {
        let mut line = quiet.join(", ");
        if let Some(first) = line.get(..1) {
            line = first.to_uppercase() + &line[1..];
        }
        work += &format!("<p class=\"quiet\">{line}.</p>\n");
    }
    work
}

/// The notes' inboxes: lessons waiting for triage and drafts waiting on the owner. Each has
/// its own page; the overview shows lessons only when some wait, since zero is nothing to
/// say, and shows drafts only as a count.
fn lessons_section(rows: &BoardData, at: At) -> String {
    inbox_section(
        at,
        "lessons",
        "Lessons to triage",
        &rows.lessons,
        "nothing to triage",
    ) + &inbox_section(
        at,
        "drafts",
        "Open drafts",
        &rows.drafts,
        "nothing waiting on you",
    )
}

fn inbox_section(at: At, class: &str, title: &str, pages: &[Lesson], empty: &str) -> String {
    let mut lines = String::new();
    for page in pages {
        lines += &format!(
            "<li><a href=\"{}\">{}</a></li>\n",
            esc(&at.links.href(at.root, page.file.as_str())),
            esc(page.title.as_str())
        );
    }
    if lines.is_empty() {
        lines = format!("<li class=\"empty\">{empty}</li>\n");
    }
    fold(
        class,
        if pages.is_empty() { " none" } else { "" },
        title,
        pages.len(),
        true,
        &format!("<ul>\n{lines}</ul>"),
    )
}

/// Recent journal lines, newest first, each linked to its effort or initiative; past the
/// first few they fold into a disclosure.
fn journal_section(rows: &BoardData, cards: &[Card], at: At) -> String {
    let by_number: std::collections::HashMap<u64, &EffortRow> = cards
        .iter()
        .filter_map(|c| c.row.number.map(|n| (n, c.row)))
        .collect();
    let mut journal = String::new();
    for (index, line) in rows.journal_lines.iter().rev().enumerate() {
        if index == RECENT_JOURNAL {
            journal += &format!(
                "</ul><details class=\"history\" data-persist=\"journal-history\"><summary>Earlier updates <span>{}</span></summary><ul>",
                rows.journal_lines.len() - RECENT_JOURNAL
            );
        }
        let effort = line.effort.as_str();
        let num = if let Some(slug) = line.initiative_slug() {
            let rel = format!("initiatives/{slug}.md");
            let title = rows
                .initiative_titles
                .get(slug)
                .map(String::as_str)
                .unwrap_or(slug);
            format!(
                "<span class=\"num\"><a href=\"{}\">{}</a></span>",
                esc(&at.links.href(at.root, &rel)),
                esc(title)
            )
        } else {
            match effort.parse::<u64>().ok().and_then(|n| by_number.get(&n)) {
                // The number says which effort the link opens; the repo says where it landed.
                Some(row) => {
                    let n = row.number.map(|n| n.to_string()).unwrap_or_default();
                    let label = match row.repo.as_deref().filter(|r| !r.is_empty()) {
                        Some(repo) => format!("{n} · {}", esc(repo)),
                        None => format!("Effort {n}"),
                    };
                    format!(
                        "<span class=\"num\"><a href=\"{}\" title=\"{}\">{label}</a></span>",
                        esc(&at
                            .links
                            .href(at.root, row.file.as_deref().unwrap_or_default())),
                        esc(row.title.as_deref().unwrap_or("Effort")),
                    )
                }
                None => format!("<span class=\"num repo\">{}</span>", esc(effort)),
            }
        };
        // A line that names a page is a card: its link covers the whole line.
        let linked = if num.contains("<a ") {
            " class=\"linked\""
        } else {
            ""
        };
        journal += &format!(
            "<li{linked}>{num}<span>{}</span></li>\n",
            esc(line.text.as_str())
        );
    }
    if journal.is_empty() {
        journal = "<li class=\"empty\">nothing yet today</li>\n".to_owned();
    }
    let journal_end = if rows.journal_lines.len() > RECENT_JOURNAL {
        "</ul></details>"
    } else {
        "</ul>"
    };
    fold(
        "today",
        "",
        "Recent changes",
        rows.journal_lines.len(),
        true,
        &format!("<ul id=\"journal\">\n{journal}{journal_end}"),
    )
}

/// Open initiative pages with their progress, then legacy tags that have no page.
fn initiatives_section(rows: &BoardData, at: At) -> String {
    let mut initiative_lines = String::new();
    let mut open_pages = 0;
    for initiative in &rows.initiatives {
        let row = &initiative.page;
        if matches!(row.status.as_deref(), Some("done" | "abandoned")) {
            continue;
        }
        open_pages += 1;
        let title = format!(
            "<a href=\"{}\">{}</a>",
            esc(&at
                .links
                .href(at.root, row.file.as_deref().unwrap_or_default())),
            esc(row.title.as_deref().unwrap_or_default())
        );
        initiative_lines += &initiative_html(
            &title,
            row.state.as_deref(),
            &initiative.progress,
            Some(row.status.as_deref().unwrap_or("ready")),
        );
    }
    for group in rows.legacy_initiatives.values() {
        initiative_lines += &initiative_html(
            &format!(
                "{} <span class=\"pill\">no page</span>",
                esc(tag(&group.name))
            ),
            None,
            &group.progress,
            None,
        );
    }
    if initiative_lines.is_empty() {
        initiative_lines = "<li class=\"empty\">nothing open</li>\n".to_owned();
    }
    fold(
        "initiatives",
        "",
        "Initiatives",
        open_pages,
        true,
        &format!("<ul>\n{initiative_lines}</ul>"),
    )
}

/// Work closed this fortnight, folded until the viewer opens it.
fn done_section(done: &[&Card], at: At) -> String {
    let body = if done.is_empty() {
        "<p class=\"empty\">nothing closed lately</p>\n".to_owned()
    } else {
        format!(
            "<ul class=\"closed\">{}</ul>\n",
            done.iter()
                .map(|c| done_html(c, at.root, at.now_ms, at.links))
                .collect::<String>()
        )
    };
    fold("done", "", "Done this fortnight", done.len(), false, &body)
}

/// The chips: every count that isn't zero, then the notes' state.
fn chips(rows: &BoardData, columns: &Columns) -> String {
    let lessons = &rows.lessons;
    // Counted by status, as an initiative's header counts its members: a card that waits
    // on the owner is under Needs you and is still active, blocked or in review.
    let with_held = |slot: usize| columns[slot].len() + held_for_owner(columns, slot);
    let active_n = with_held(ACTIVE);
    let chip = |dot: &str, n: usize, label: &str| {
        format!("<span class=\"chip\"><span class=\"dot {dot}\"></span><b>{n}</b> {label}</span>")
    };
    let mut chips = String::new();
    if !columns[NEEDS].is_empty() {
        chips += &chip(
            "needs",
            columns[NEEDS].len(),
            if columns[NEEDS].len() == 1 {
                "needs you"
            } else {
                "need you"
            },
        );
    }
    if active_n > 0 {
        chips += &chip("active", active_n, "active");
    }
    for (dot, slot, label) in [
        ("ready", READY, "ready"),
        ("blocked", BLOCKED, "blocked"),
        ("review", REVIEW, "in review"),
        ("idea", IDEAS, "ideas"),
    ] {
        if with_held(slot) > 0 {
            chips += &chip(dot, with_held(slot), label);
        }
    }
    if !columns[DONE].is_empty() {
        chips += &chip("done", columns[DONE].len(), "done");
    }
    let warnings: Vec<&str> = rows.notes.iter().map(String::as_str).collect();
    if !lessons.is_empty() {
        chips += &chip(
            "lesson",
            lessons.len(),
            if lessons.len() == 1 {
                "lesson to triage"
            } else {
                "lessons to triage"
            },
        );
    }
    // Open drafts wait on the owner the way lessons do, and leave the folder the same way.
    if !rows.drafts.is_empty() {
        chips += &chip(
            "draft",
            rows.drafts.len(),
            if rows.drafts.len() == 1 {
                "open draft"
            } else {
                "open drafts"
            },
        );
    }
    chips += &if warnings.is_empty() {
        "<span class=\"sync\">notes in sync</span>".to_owned()
    } else {
        format!(
            "<span class=\"sync warn\">{}</span>",
            esc(&warnings.join("; "))
        )
    };
    chips
}

/// The date, how much needs the owner and how much is moving, and the chips, above the
/// rest of the board.
fn hero(rows: &BoardData, columns: &Columns, chips: &str, board: &str, now_ms: i64) -> String {
    let date = chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%A, %B %-d")
                .to_string()
        })
        .unwrap_or_else(|| today_of(rows));
    let attention_title = match columns[NEEDS].len() {
        0 => "Nothing needs you.".to_owned(),
        1 => "One thing needs you.".to_owned(),
        n => format!("{n} things need you."),
    };
    let motion_title = match columns[ACTIVE].len() {
        0 => "Room for what’s next.".to_owned(),
        1 => "One thing moving.".to_owned(),
        n => format!("{n} things moving."),
    };
    format!(
        "<div class=\"board\" id=\"board\"><div class=\"hero\"><p class=\"eyebrow\">{}</p><h1>{attention_title}<br><em>{motion_title}</em></h1><div class=\"chips\">{chips}</div></div>\n{board}</div>\n",
        esc(&date)
    )
}

pub(super) fn today_of(rows: &BoardData) -> String {
    rows.today.clone()
}

/// A note's way back to the board. The script points it at the view that opened the
/// note when there was one.
pub(super) fn back_link(links: Links) -> String {
    let home = if links == Links::Served {
        "/"
    } else {
        "home.html"
    };
    format!(r#"<a class="back-link" href="{home}">← Back to overview</a>"#)
}

/// The shared Studio shell: vertical navigation, sticky controls, the searchable notes
/// view and the small script that preserves navigation and appearance choices.
pub(super) fn shell(
    look: Look,
    title: &str,
    crumb: &str,
    body: &str,
    tree: &str,
    refresh_secs: Option<u32>,
    links: Links,
) -> String {
    let theme_attr = match look.theme {
        Theme::Auto => String::new(),
        t => format!(" data-theme=\"{}\"", t.name()),
    };
    let scheme = match look.theme {
        Theme::Auto => "light dark",
        Theme::Light => "light",
        Theme::Dark => "dark",
    };
    let refresh = refresh_secs
        .map(|s| format!("<meta http-equiv=\"refresh\" content=\"{s}\">"))
        .unwrap_or_default();
    let csp = match links {
        Links::Served => {
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src http: https: data:; connect-src 'self'; form-action 'none'\">"
        }
        Links::File => "",
    };
    let favicon = percent(ICON);
    let icon = ICON
        .replace(
            "class=\"astrolabe-mark\"",
            "class=\"astrolabe-mark brand-icon\"",
        )
        .replace(
            "role=\"img\" aria-labelledby=\"astrolabe-title\"",
            "aria-hidden=\"true\"",
        );
    let swatches = [Palette::Iris, Palette::Ember, Palette::Slate, Palette::Moss, Palette::Graphite]
        .iter().map(|p| {
            let (name, description) = match p {
                Palette::Iris => ("Iris", "Soft violet"),
                Palette::Ember => ("Ember", "Warm clay"),
                Palette::Slate => ("Slate", "Clear blue"),
                Palette::Moss => ("Moss", "Quiet green"),
                Palette::Graphite => ("Graphite", "Pure neutral"),
            };
            format!(r#"<button type="button" class="sw" data-p="{}" aria-pressed="false"><span class="palette-preview" aria-hidden="true"></span><span><strong>{name}</strong><small>{description}</small></span><span class="palette-check" aria-hidden="true">✓</span></button>"#, p.name())
        }).collect::<String>();
    let home = if links == Links::Served {
        "/"
    } else {
        "home.html"
    };
    let navigation = [("overview", "Overview", "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z"),
        ("efforts", "Efforts", "m3 7 9-4 9 4-9 4z M3 12l9 4 9-4 M3 17l9 4 9-4"),
        ("initiatives", "Initiatives", "M5 21V3 M5 4h12l-2.5 4.5L17 13H5"),
        ("drafts", "Drafts", "M4 20h4L19 9l-4-4L4 16z M13 7l4 4"),
        ("journal", "Journal", "M5 3h14v18H5z M8 7h8 M8 11h8 M8 15h5"),
        ("lessons", "Lessons", "M9 18h6 M10 21h4 M12 3a6 6 0 0 0-4 10.5c.7.7 1 1.5 1 2.5h6c0-1 .3-1.8 1-2.5A6 6 0 0 0 12 3z"),
        ("notes", "Notes", "M3 7V4h6l3 3h9v13H3z")]
        .iter().map(|(view, label, path)| format!(r#"<a class="nav-link" data-view="{view}" href="{home}#{view}" aria-label="{label}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="{path}"/></svg><span>{label}</span></a>"#)).collect::<String>();
    format!(
        include_str!("shell.html"),
        palette = look.palette.name(),
        theme_attr = theme_attr,
        refresh = refresh,
        scheme = scheme,
        csp = csp,
        title = esc(title),
        favicon = favicon,
        palettes = PALETTES,
        style = STYLE,
        icon = icon,
        navigation = navigation,
        home = home,
        crumb = crumb,
        swatches = swatches,
        body = body,
        tree = tree,
        script = SCRIPT
    )
}

pub(super) fn card_html(
    card: &Card,
    root: &Path,
    now_ms: i64,
    links: Links,
    small: bool,
) -> String {
    let mut bits = Vec::new();
    let priority = card.row.priority.as_deref().unwrap_or_default();
    if matches!(priority, "high" | "urgent" | "top") {
        bits.push(format!("<span class=\"pri {priority}\">{priority}</span>"));
    }
    let initiative = card
        .row
        .initiative_title
        .as_deref()
        .unwrap_or_else(|| tag(card.row.initiative.as_deref().unwrap_or_default()));
    if !initiative.is_empty() {
        bits.push(format!(
            "<span class=\"pill tag\">{}</span>",
            esc(initiative)
        ));
    }
    if !card.row.repo.as_deref().unwrap_or_default().is_empty() {
        bits.push(format!(
            "<span class=\"pill repo\">{}</span>",
            esc(card.row.repo.as_deref().unwrap_or_default())
        ));
    }
    if card.status().is_empty() {
        bits.push("<span class=\"pill nostatus\">no status</span>".to_owned());
    }
    if let Some(t) = card.updated_ms {
        bits.push(format!(
            "<span class=\"age\" data-t=\"{t}\" title=\"{}\">{}</span>",
            esc(card.row.updated.as_deref().unwrap_or_default()),
            age(now_ms - t)
        ));
    }
    // A small card is a title, its meta and one line of context: the pieces and the next
    // step wait in the note.
    let pieces = match card.row.pieces.as_ref() {
        Some(p) if p.total > 0 && !small => pieces_bar(p.done, p.total),
        _ => String::new(),
    };
    let waits = card.row.waits_for.as_deref().unwrap_or_default();
    let waits = if waits.is_empty() {
        String::new()
    } else if let Some(ask) = waits.strip_prefix("owner:") {
        format!("<p class=\"ask\">{}</p>\n", esc(ask.trim()))
    } else {
        format!("<p class=\"waits\">waits for {}</p>\n", esc(waits))
    };
    let href = match card.row.file.as_deref() {
        Some(file) => links.href(root, file),
        None => "#".to_owned(),
    };
    let next = if small || card.row.next.as_deref().unwrap_or_default().is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"next\">{}</p>\n",
            esc(&first_sentence(
                card.row.next.as_deref().unwrap_or_default()
            ))
        )
    };
    let context = card
        .row
        .state
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!("<p class=\"context\">{}</p>", esc(&first_sentence(s))))
        .unwrap_or_default();
    // The card's left rail says its state without the column heading.
    let state = if card.row.needs_owner {
        "needs"
    } else {
        match card.status() {
            "" => "idea",
            s => s,
        }
    };
    let review = if card.row.needs_owner {
        format!(
            "<a class=\"review-link\" href=\"{}\">Read context →</a>",
            esc(&href)
        )
    } else {
        String::new()
    };
    let is_initiative = card
        .row
        .file
        .as_deref()
        .is_some_and(|f| f.starts_with("initiatives/"));
    let number = if is_initiative {
        "Initiative".to_owned()
    } else {
        card.row.number.unwrap_or(0).to_string()
    };
    let key = if is_initiative {
        card.row.file.clone().unwrap_or_default()
    } else {
        number.clone()
    };
    format!(
        "<article class=\"card s-{}{}\" data-n=\"{}\"><h3><a href=\"{}\"><span class=\"num\">{}</span> {}</a></h3>\n{waits}<p class=\"meta\">{}</p>\n{context}{next}{pieces}{review}</article>\n",
        esc(state),
        if small { " small" } else { "" },
        esc(&key),
        esc(&href),
        number,
        esc(card.row.title.as_deref().unwrap_or_default()),
        bits.join(""),
    )
}

/// "x of y pieces" with its track: on a card, and under an effort's title.
pub(super) fn pieces_bar(done: usize, total: usize) -> String {
    format!(
        "<div class=\"bar\"><span class=\"track\" role=\"progressbar\" aria-label=\"Completed pieces\" aria-valuemin=\"0\" aria-valuemax=\"{total}\" aria-valuenow=\"{}\"><i style=\"width:{}%\"></i></span><span>{done} of {total} pieces</span></div>\n",
        done.min(total),
        100_u128 * done.min(total) as u128 / total as u128
    )
}

/// The counts, the owner badge and the bar segments of a progress row, drawn the same on
/// the board and under an initiative's own title.
pub(super) struct ProgressParts {
    pub counts: String,
    pub need: String,
    pub bar: String,
}

pub(super) fn progress_parts(p: &crate::board::InitiativeProgress) -> ProgressParts {
    let mut counts = format!(
        "<span class=\"k-done\">{} of {} done</span>",
        p.done, p.total
    );
    let mut bar = String::new();
    let segment = |count: usize| 100.0 * count as f64 / p.total.max(1) as f64;
    if p.done > 0 {
        bar += &format!(
            "<i class=\"k-done\" style=\"width:{:.2}%\"></i>",
            segment(p.done)
        );
    }
    for (count, key, label) in [
        (p.review, "review", "in review"),
        (p.active, "active", "active"),
        (p.blocked, "blocked", "blocked"),
        (p.to_start, "start", "to start"),
        (p.abandoned, "abandoned", "abandoned"),
    ] {
        if count == 0 {
            continue;
        }
        counts += &format!(" · <span class=\"k-{key}\">{count} {label}</span>");
        // Abandoned members sit outside the total, and to start is the empty track.
        if !matches!(key, "abandoned" | "start") {
            bar += &format!(
                "<i class=\"k-{key}\" style=\"width:{:.2}%\"></i>",
                segment(count)
            );
        }
    }
    // The board's own words for the same efforts: its chip reads "1 needs you".
    let need = match p.need_you {
        0 => String::new(),
        1 => "<span class=\"need\">1 needs you</span>".to_owned(),
        n => format!("<span class=\"need\">{n} need you</span>"),
    };
    ProgressParts { counts, need, bar }
}

/// Status counts add up to the total and colour the bar's segments; what waits on the
/// owner overlaps them (and can include the page itself), so it's a badge of its own.
pub(super) fn initiative_html(
    title: &str,
    state: Option<&str>,
    p: &crate::board::InitiativeProgress,
    page_status: Option<&str>,
) -> String {
    let has_page = page_status.is_some();
    let ProgressParts { counts, need, bar } = progress_parts(p);
    let edge = if p.total == 0 && p.abandoned == 0 {
        "no members yet"
    } else if p.total == 0 {
        "all members abandoned"
    } else if has_page && p.done == p.total {
        "awaiting acceptance"
    } else if page_status == Some("active") && p.active + p.review + p.blocked == 0 {
        // Counted from member notes, so a page saved about work its members never heard of
        // reads as nothing started (Deck frame time, 2026-11-08).
        "nothing in flight: members' notes may be behind the work"
    } else {
        ""
    };
    let state = state
        .map(|s| format!("<p>{}</p>", esc(&first_sentence(s))))
        .unwrap_or_default();
    let edge = if edge.is_empty() {
        String::new()
    } else {
        format!("<p>{edge}</p>")
    };
    format!(
        "<li class=\"ini{}\"><span>{title}</span>{need}<span class=\"num\">{counts}</span>{state}{edge}<span class=\"track\" aria-hidden=\"true\">{bar}</span></li>\n",
        // A page's whole card opens it; a group without one has nowhere to go.
        if has_page { " linked" } else { "" }
    )
}

/// A closed effort as one line of the fortnight's list.
pub(super) fn done_html(card: &Card, root: &Path, now_ms: i64, links: Links) -> String {
    let href = match card.row.file.as_deref() {
        Some(file) => links.href(root, file),
        None => "#".to_owned(),
    };
    let age = card
        .updated_ms
        .map(|t| {
            format!(
                "<span class=\"age\" data-t=\"{t}\" title=\"{}\">{}</span>",
                esc(card.row.updated.as_deref().unwrap_or_default()),
                age(now_ms - t)
            )
        })
        .unwrap_or_default();
    let repo = if card.row.repo.as_deref().unwrap_or_default().is_empty() {
        String::new()
    } else {
        format!(
            "<span class=\"pill repo\">{}</span>",
            esc(card.row.repo.as_deref().unwrap_or_default())
        )
    };
    format!(
        "<li class=\"{}\" data-n=\"{}\"><svg class=\"tick\" viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M5 12l5 5L20 7\"/></svg><a href=\"{}\"><span class=\"num\">{}</span><span class=\"t\">{}</span></a>{repo}{age}</li>\n",
        esc(card.status()),
        esc(&card
            .row
            .number
            .map(|n| n.to_string())
            .unwrap_or_else(|| card.row.file.clone().unwrap_or_default())),
        esc(&href),
        card.row
            .number
            .map(|n| n.to_string())
            .unwrap_or_else(|| "Initiative".to_owned()),
        esc(card.row.title.as_deref().unwrap_or_default()),
    )
}

/// A `file:///` link a browser follows: forward slashes, a Windows drive after the root,
/// the `\\?\` of a canonical Windows path dropped, everything but unreserved characters
/// percent-encoded.
pub(super) fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix("\\\\?\\UNC\\")
        .map(|rest| format!("\\\\{rest}"))
        .unwrap_or_else(|| text.trim_start_matches("\\\\?\\").to_owned());
    let text = text.replace('\\', "/");
    let mut url = String::from("file://");
    if !text.starts_with('/') {
        url.push('/');
    }
    url + &percent(&text)
}

/// Percent-encodes everything but the unreserved characters, `/` and `:`.
pub(super) fn percent(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `2026-11-04T16:30-07:00`, the stamp saves write, as epoch milliseconds.
pub(super) fn stamp_ms(stamp: &str) -> Option<i64> {
    DateTime::<FixedOffset>::parse_from_str(stamp, "%Y-%m-%dT%H:%M%:z")
        .ok()
        .map(|t| t.timestamp_millis())
}

pub(super) fn age(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{} h", s / 3600)
    } else {
        format!("{} d", s / 86_400)
    }
}

/// The paragraph's first sentence, with a legacy `**Next:**` label dropped, cut short.
pub(super) fn first_sentence(paragraph: &str) -> String {
    let mut text = paragraph.trim();
    while let Some(rest) = text.strip_prefix("**")
        && let Some((label, after)) = rest.split_once("**")
        && label.ends_with(':')
    {
        text = after.trim_start();
    }
    let text = text.replace('\n', " ");
    let end = text
        .char_indices()
        .find(|&(i, c)| c == '.' && text[i + 1..].starts_with(' '))
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());
    let sentence = &text[..end];
    if sentence.chars().count() <= SENTENCE_CHARS {
        sentence.to_owned()
    } else {
        let cut: String = sentence.chars().take(SENTENCE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// `[[agent-tooling]]`, as Obsidian writes a link, reads as the name.
pub(super) fn tag(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .trim()
}

pub(super) fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The colour tokens of each palette, light and dark, chosen by the `data-palette` and
/// `data-theme` attributes on the document, so a viewer's override needs no new page.
pub(super) const PALETTES: &str = include_str!("palettes.css");

/* Layout: a fixed notes tree on the left, one scrolling pane on the right that is the board
or a note. The same sheet draws the file copy. Type: 11 px labels, 12 and 13 meta, 14 body,
15.5 card titles, 28 the date; numbers tabular throughout. */

pub(super) const STYLE: &str = include_str!("style.css");

/// What the page keeps for the viewer: which folders are open, the look they chose, the
/// scroll position across the file copy's reload; and the search box, the hidden folders'
/// toggle, the ages ticking, the drawer on a narrow screen.
pub(super) const SCRIPT: &str = include_str!("script.js");
