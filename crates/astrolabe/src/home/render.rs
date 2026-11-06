//! Board and page markup from a typed snapshot; no Git operations.
use super::nav::tree_html;
use super::settings::{Look, Palette, Theme};
use crate::board::{BoardData, EffortRow};
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
    fn closed(&self) -> bool {
        matches!(self.status(), "done" | "abandoned")
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

/// Studio's attention and active work first, followed by ready, waiting and proposed
/// work, recent journal entries, initiatives and a disclosure of recently closed work.
pub(super) fn board_html(rows: &BoardData, root: &Path, now_ms: i64, links: Links) -> String {
    let cards: Vec<Card> = rows
        .efforts
        .iter()
        .map(|row| Card {
            row,
            updated_ms: row.updated.as_deref().and_then(stamp_ms),
        })
        .collect();
    let mut columns: [Vec<&Card>; 6] = Default::default();
    for card in &cards {
        let slot = if card.row.needs_owner {
            0
        } else {
            match card.status() {
                "active" => 1,
                "ready" => 2,
                "blocked" => 3,
                "done" | "abandoned" => {
                    let fresh = card
                        .updated_ms
                        .is_some_and(|t| now_ms - t <= DONE_DAYS * 86_400_000);
                    if !fresh {
                        continue;
                    }
                    5
                }
                _ => 4,
            }
        };
        columns[slot].push(card);
    }
    for column in &mut columns {
        column.sort_by_key(|c| (c.priority_rank(), -c.updated_ms.unwrap_or(0)));
    }
    let heading = |title: &str, n: usize| {
        format!("<h2><span>{title}</span> <span class=\"n\">{n}</span></h2>")
    };
    let cards_of = |column: &[&Card], compact: bool| {
        column
            .iter()
            .map(|c| card_html(c, root, now_ms, links, compact))
            .collect::<String>()
    };
    let mut board = String::new();
    let attention = if !columns[0].is_empty() {
        format!(
            "<section class=\"col needsyou\">{}<div class=\"cards wide\">{}</div></section>\n",
            heading("Needs you", columns[0].len()),
            cards_of(&columns[0], false)
        )
    } else {
        // Kept in the page so a test and the live swap can find the column by name.
        "<section class=\"col needsyou\" hidden></section>\n".to_owned()
    };
    // Ready, blocked and ideas sit below the active grid; an empty one is a few words
    // under the work instead of a dashed box.
    let mut stack = String::new();
    let mut quiet = Vec::new();
    for (key, title, slot, none) in [
        ("ready", "Up next", 2, "nothing ready"),
        ("blocked", "Waiting", 3, "nothing blocked"),
        ("ideas", "Ideas", 4, "no ideas parked"),
    ] {
        let column = &columns[slot];
        if column.is_empty() {
            quiet.push(none);
        } else {
            stack += &format!(
                "<section class=\"col {key}\">{}<div class=\"cards\">{}</div></section>\n",
                heading(title, column.len()),
                cards_of(column, true)
            );
        }
    }
    let active = if columns[1].is_empty() {
        "<p class=\"empty\">nothing in flight</p>\n".to_owned()
    } else {
        cards_of(&columns[1], false)
    };
    board += &format!(
        "<div class=\"work{}\"><section class=\"col active\">{}<div class=\"cards grid\">{active}</div></section>\n{}</div>\n",
        if stack.is_empty() { " solo" } else { "" },
        heading("In motion", columns[1].len()),
        if stack.is_empty() {
            String::new()
        } else {
            format!("<div class=\"stack\">{stack}</div>")
        }
    );
    if !quiet.is_empty() {
        let mut line = quiet.join(", ");
        if let Some(first) = line.get(..1) {
            line = first.to_uppercase() + &line[1..];
        }
        board += &format!("<p class=\"quiet\">{line}.</p>\n");
    }

    // The notes' inbox: lessons waiting for triage. Only a non-empty one is shown, since
    // the count is the backlog and zero is nothing to say.
    let lessons = &rows.lessons;
    let mut lesson_lines = String::new();
    for lesson in lessons {
        let file = lesson.file.as_str();
        lesson_lines += &format!(
            "<li><a href=\"{}\">{}</a></li>\n",
            esc(&links.href(root, file)),
            esc(lesson.title.as_str())
        );
    }
    let lessons_html = if lesson_lines.is_empty() {
        String::new()
    } else {
        format!(
            "<section class=\"col lessons\">{}<ul>\n{lesson_lines}</ul></section>",
            heading("Lessons to triage", lessons.len())
        )
    };

    let by_number: std::collections::HashMap<u64, &EffortRow> = cards
        .iter()
        .filter_map(|c| c.row.number.map(|n| (n, c.row)))
        .collect();
    let mut journal = String::new();
    for (index, line) in rows.journal_lines.iter().rev().enumerate() {
        if index == 3 {
            journal += &format!(
                "</ul><details class=\"history\" data-persist=\"journal-history\"><summary>Earlier updates <span>{}</span></summary><ul>",
                rows.journal_lines.len() - 3
            );
        }
        let effort = line.effort.as_str();
        let num = match effort.parse::<u64>().ok().and_then(|n| by_number.get(&n)) {
            // The number says which effort the link opens; the repo says where it landed.
            Some(row) => {
                let n = row.number.map(|n| n.to_string()).unwrap_or_default();
                let label = match row.repo.as_deref().filter(|r| !r.is_empty()) {
                    Some(repo) => format!("{n} · {}", esc(repo)),
                    None => format!("Effort {n}"),
                };
                format!(
                    "<span class=\"num\"><a href=\"{}\" title=\"{}\">{label}</a></span>",
                    esc(&links.href(root, row.file.as_deref().unwrap_or_default())),
                    esc(row.title.as_deref().unwrap_or("Effort")),
                )
            }
            None => format!("<span class=\"num repo\">{}</span>", esc(effort)),
        };
        journal += &format!("<li>{num}<span>{}</span></li>\n", esc(line.text.as_str()));
    }
    if journal.is_empty() {
        journal = "<li class=\"empty\">nothing yet today</li>\n".to_owned();
    }
    let journal_end = if rows.journal_lines.len() > 3 {
        "</ul></details>"
    } else {
        "</ul>"
    };

    let mut initiatives: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for card in &cards {
        let name = tag(card.row.initiative.as_deref().unwrap_or_default());
        if name.is_empty() {
            continue;
        }
        let entry = initiatives.entry(name.to_owned()).or_default();
        if card.closed() {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }
    // An initiative is listed while an effort of its is open; one whose efforts have all
    // closed has had its say and leaves the board with them.
    let mut initiative_lines = String::new();
    for (name, (open, done)) in &initiatives {
        if *open == 0 {
            continue;
        }
        initiative_lines += &format!(
            "<li class=\"ini\"><span>{}</span><span class=\"num\">{open} open · {done} done</span><span class=\"track\"><i style=\"width:{}%\"></i></span></li>\n",
            esc(name),
            100 * done / (open + done)
        );
    }
    if initiative_lines.is_empty() {
        initiative_lines = "<li class=\"empty\">nothing open</li>\n".to_owned();
    }
    board += &format!(
        "<div class=\"below\"><section class=\"col today\">{}<ul id=\"journal\">\n{journal}{journal_end}</section><section class=\"col initiatives\">{}<ul>\n{initiative_lines}</ul></section>{lessons_html}</div>\n",
        heading("Recent changes", rows.journal_lines.len()),
        heading(
            "Initiatives",
            initiatives.values().filter(|(open, _)| *open > 0).count()
        )
    );

    let done = &columns[5];
    board += &format!(
        "<section class=\"col done\"><details data-persist=\"closed-work\"><summary>Done this fortnight <span class=\"n\">{}</span></summary>\n{}</details></section>\n",
        done.len(),
        if done.is_empty() {
            "<p class=\"empty\">nothing closed lately</p>\n".to_owned()
        } else {
            format!(
                "<ul class=\"closed\">{}</ul>\n",
                done.iter()
                    .map(|c| done_html(c, root, now_ms, links))
                    .collect::<String>()
            )
        }
    );

    // The chips: every count that isn't zero, then the notes' state.
    let active_n = columns[1].len() + columns[0].iter().filter(|c| c.status() == "active").count();
    let chip = |dot: &str, n: usize, label: &str| {
        format!("<span class=\"chip\"><span class=\"dot {dot}\"></span><b>{n}</b> {label}</span>")
    };
    let mut chips = String::new();
    if !columns[0].is_empty() {
        chips += &chip(
            "needs",
            columns[0].len(),
            if columns[0].len() == 1 {
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
        ("ready", 2, "ready"),
        ("blocked", 3, "blocked"),
        ("idea", 4, "ideas"),
        ("done", 5, "done"),
    ] {
        if !columns[slot].is_empty() {
            chips += &chip(dot, columns[slot].len(), label);
        }
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
    if rows.drafts > 0 {
        chips += &chip(
            "draft",
            rows.drafts,
            if rows.drafts == 1 {
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
    let date = chrono::DateTime::from_timestamp_millis(now_ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%A, %B %-d")
                .to_string()
        })
        .unwrap_or_else(|| today_of(rows));
    let attention_title = match columns[0].len() {
        0 => "Nothing needs you.".to_owned(),
        1 => "One thing needs you.".to_owned(),
        n => format!("{n} things need you."),
    };
    let motion_title = match columns[1].len() {
        0 => "Room for what’s next.".to_owned(),
        1 => "One thing moving.".to_owned(),
        n => format!("{n} things moving."),
    };
    format!(
        "<div class=\"board\" id=\"board\"><div class=\"hero{}\"><div class=\"bhead\"><p class=\"eyebrow\">{}</p><h1>{attention_title}<br><em>{motion_title}</em></h1><div class=\"chips\">{chips}</div></div>{attention}</div>\n{board}</div>\n",
        if columns[0].is_empty() {
            " quiet-hero"
        } else {
            ""
        },
        esc(&date)
    )
}

pub(super) fn today_of(rows: &BoardData) -> String {
    rows.today.clone()
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
        ("journal", "Journal", "M5 3h14v18H5z M8 7h8 M8 11h8 M8 15h5"),
        ("notes", "Notes", "M3 7V4h6l3 3h9v13H3z")]
        .iter().map(|(view, label, path)| format!(r#"<a class="nav-link" data-view="{view}" href="{home}#{view}" aria-label="{label}"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="{path}"/></svg><span>{label}</span></a>"#)).collect::<String>();
    let back = if crumb == "<b>Board</b>" {
        String::new()
    } else {
        format!(r#"<a class="back-link" href="{home}">← Back to overview</a>"#)
    };
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
        back = back,
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
    compact: bool,
) -> String {
    let mut bits = Vec::new();
    let priority = card.row.priority.as_deref().unwrap_or_default();
    if matches!(priority, "high" | "urgent" | "top") {
        bits.push(format!("<span class=\"pri {priority}\">{priority}</span>"));
    }
    let initiative = tag(card.row.initiative.as_deref().unwrap_or_default());
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
    let pieces = match (
        card.row.pieces.as_ref().map(|p| p.done),
        card.row.pieces.as_ref().map(|p| p.total),
    ) {
        (Some(done), Some(total)) if total > 0 => format!(
            "<div class=\"bar\"><span class=\"track\" role=\"progressbar\" aria-label=\"Completed pieces\" aria-valuemin=\"0\" aria-valuemax=\"{total}\" aria-valuenow=\"{}\"><i style=\"width:{}%\"></i></span><span>{done} of {total} pieces</span></div>\n",
            done.min(total),
            100_u128 * done.min(total) as u128 / total as u128
        ),
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
    let next = if compact || card.row.next.as_deref().unwrap_or_default().is_empty() {
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
    format!(
        "<article class=\"card s-{}{}\" data-n=\"{}\"><h3><a href=\"{}\"><span class=\"num\">{}</span>{}</a></h3>\n{waits}<p class=\"meta\">{}</p>\n{context}{next}{pieces}{review}</article>\n",
        esc(state),
        if compact { " compact" } else { "" },
        card.row.number.unwrap_or(0),
        esc(&href),
        card.row.number.unwrap_or(0),
        esc(card.row.title.as_deref().unwrap_or_default()),
        bits.join(""),
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
        card.row.number.unwrap_or(0),
        esc(&href),
        card.row.number.unwrap_or(0),
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
