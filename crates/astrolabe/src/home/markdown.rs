//! Safe Markdown rendering and links to notes.
use super::nav::tree_html;
use super::render::{Links, esc, percent, shell, stamp_ms, tag};
use super::settings::Look;
use crate::board::BoardData;
use std::path::Path;

/// A note as a page: its properties as pills, the Markdown rendered in the board's shell,
/// an outline beside it when it has sections. Raw HTML in the note is shown as text, since
/// notes are written by agents and the page runs in the owner's browser. A relative link
/// to another note becomes a link to that note's page.
pub(super) fn note_page(
    text: &str,
    rel: &str,
    rows: &BoardData,
    root: &Path,
    now_ms: i64,
    look: Look,
) -> String {
    use pulldown_cmark::{Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd, html};
    let text = text.replace("\r\n", "\n");
    let (front, body) = match text.strip_prefix("---\n").and_then(|rest| {
        rest.find("\n---\n")
            .map(|end| (&rest[..end], &rest[end + 5..]))
    }) {
        Some((front, body)) => (front, body),
        None => ("", text.as_str()),
    };
    let body = strip_comments(body);
    let prop = |key: &str| -> Option<String> {
        let value = front
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?
            .trim()
            .trim_matches('"');
        (!value.is_empty() && value != "none").then(|| value.to_owned())
    };
    // What the owner reads sits under the title; what the program keeps (the save's
    // branch, head and the like) is a small list of facts beside the outline.
    let mut pills = Vec::new();
    for key in [
        "status",
        "repo",
        "priority",
        "initiative",
        "involvement",
        "authority",
        "waits_for",
    ] {
        let Some(value) = prop(key) else { continue };
        let class = match key {
            "status" => format!(" status-{}", esc(&value)),
            _ => String::new(),
        };
        pills.push(format!(
            "<span class=\"pill{class}\"><i>{}</i>{}</span>",
            esc(&key.replace('_', " ")),
            esc(tag(&value))
        ));
    }
    let mut facts = Vec::new();
    for key in ["updated", "saved_by", "branch", "head", "unpushed", "dirty"] {
        let Some(value) = prop(key) else { continue };
        let shown = match key {
            "updated" => esc(&when(&value, now_ms)),
            "branch" | "head" => format!("<code>{}</code>", esc(tag(&value))),
            _ => esc(tag(&value)),
        };
        facts.push(format!(
            "<dt>{}</dt><dd>{shown}</dd>",
            esc(&key.replace('_', " "))
        ));
    }
    facts.push(format!("<dt>file</dt><dd><code>{}</code></dd>", esc(rel)));
    let facts = format!("<div class=\"facts\"><dl>{}</dl></div>", facts.join(""));
    let name = rel.rsplit('/').next().unwrap_or(rel);
    // An effort's file keeps its padded number ("0043 …"); its page reads "43 …".
    let effort = rows.efforts.iter().any(|e| e.file.as_deref() == Some(rel));
    let raw_title = body
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(name.trim_end_matches(".md"))
        .trim();
    let title = if effort {
        unpadded(raw_title)
    } else {
        raw_title.to_owned()
    };
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_WIKILINKS);
    let mut events: Vec<Event> = Parser::new_ext(&body, options).collect();
    // Headings get ids, and the second and third levels make the outline.
    let mut outline = Vec::new();
    let mut trimmed = false;
    let mut i = 0;
    while i < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[i] {
            let level = *level;
            let mut heading = String::new();
            let mut j = i + 1;
            while j < events.len() {
                match &events[j] {
                    Event::End(TagEnd::Heading(_)) => break,
                    Event::Text(t) | Event::Code(t) => heading.push_str(t),
                    _ => {}
                }
                j += 1;
            }
            let id = heading_id(&heading);
            if let Event::Start(Tag::Heading { id: slot, .. }) = &mut events[i] {
                *slot = Some(id.clone().into());
            }
            match level {
                HeadingLevel::H2 => outline.push((2, heading, id)),
                HeadingLevel::H3 => outline.push((3, heading, id)),
                // The id above keeps the padded number, so links to it still land.
                HeadingLevel::H1 if effort && !trimmed => {
                    trimmed = true;
                    if let Some(Event::Text(t)) = events.get_mut(i + 1) {
                        *t = unpadded(t).into();
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    let events = events.into_iter().map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        // A link may name a note, a page or a mail address, never a script or a file.
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            // A wikilink names a note by title, which nothing here resolves yet.
            let wiki = matches!(link_type, LinkType::WikiLink { .. });
            let dest_url = if wiki || !plain_url(&dest_url) {
                "#".into()
            } else if let Some(page) = resolve_link(root, rel, &dest_url) {
                page.into()
            } else {
                dest_url
            };
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            })
        }
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if plain_url(&dest_url) {
                dest_url
            } else {
                "#".into()
            },
            title,
            id,
        }),
        other => other,
    });
    let mut rendered = String::new();
    html::push_html(&mut rendered, events);
    let aside = if outline.len() > 1 {
        let items: String = outline
            .iter()
            .map(|(level, text, id)| {
                format!(
                    "<a class=\"l{level}\" href=\"#{}\">{}</a>",
                    esc(id),
                    esc(text)
                )
            })
            .collect();
        format!("<aside class=\"outline\"><h4>On this page</h4>{items}{facts}</aside>")
    } else {
        String::new()
    };
    let props = if pills.is_empty() {
        String::new()
    } else {
        format!("<div class=\"props\">{}</div>", pills.join(""))
    };
    // The properties follow the title; a note without one starts with them.
    let rendered = match rendered.find("</h1>") {
        Some(i) => format!("{}{props}{}", &rendered[..i + 5], &rendered[i + 5..]),
        None => format!("{props}{rendered}"),
    };
    let rendered = if aside.is_empty() {
        format!("{rendered}{facts}")
    } else {
        rendered
    };
    // There are no folder pages: each folder opens the notes view at that folder.
    let mut crumb = String::from("<a href=\"/#notes\">Notes</a>");
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts[..parts.len() - 1].iter().enumerate() {
        crumb += &format!(
            "<span>›</span><a href=\"/#notes/{}\">{}</a>",
            esc(&parts[..=i].join("/")),
            esc(part)
        );
    }
    crumb += &format!("<span>›</span><b>{}</b>", esc(name));
    let page = format!(
        "<div class=\"note{}\"><div class=\"prose\">{rendered}</div>{aside}</div>\n",
        if aside.is_empty() { " wide" } else { "" }
    );
    shell(
        look,
        &title,
        &crumb,
        &page,
        &tree_html(root, rows, Links::Served, Some(rel)),
        None,
        Links::Served,
    )
}

/// The text with its HTML comments taken out: a journal line's hidden tail is for the
/// program, not the page.
pub(super) fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// A title whose first word is a number, with that number's leading zeros dropped:
/// "0043 Studio fixes" reads "43 Studio fixes". Any other title is returned as it is.
pub(super) fn unpadded(title: &str) -> String {
    let digits = title.len() - title.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let rest = &title[digits..];
    if digits == 0 || !(rest.is_empty() || rest.starts_with(' ')) {
        return title.to_owned();
    }
    let number = title[..digits].trim_start_matches('0');
    format!("{}{rest}", if number.is_empty() { "0" } else { number })
}

/// A heading's anchor: lowercase words joined by hyphens.
pub(super) fn heading_id(text: &str) -> String {
    let mut id = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            id.extend(c.to_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    if id.is_empty() {
        "section".to_owned()
    } else {
        id
    }
}

/// A relative link from the note at `from` to another note on disk, as that note's page;
/// None for a link elsewhere or to nothing.
pub(super) fn resolve_link(root: &Path, from: &str, dest: &str) -> Option<String> {
    if dest.starts_with('#') || dest.contains(':') {
        return None;
    }
    let (path, fragment) = dest.split_once('#').unwrap_or((dest, ""));
    let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
    let mut candidates = vec![normalize(&format!("{dir}/{path}")), normalize(path)];
    candidates.dedup();
    for c in candidates {
        if c.is_empty()
            || c.split('/')
                .any(|p| p.is_empty() || p == ".." || p == ".git")
            || !root.join(&c).is_file()
        {
            continue;
        }
        let fragment = if fragment.is_empty() {
            String::new()
        } else {
            format!("#{}", heading_id(fragment))
        };
        return Some(format!("/{}{fragment}", percent(&c)));
    }
    None
}

/// `a/./b/../c` as `a/c`; a path climbing past the root keeps its `..` and so is refused.
pub(super) fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// A save stamp as the owner reads it beside the clock: the time if today, "yesterday"
/// and the time, or the date.
pub(super) fn when(stamp: &str, now_ms: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(ms) = stamp_ms(stamp).or_else(|| {
        chrono::NaiveDate::parse_from_str(stamp, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .and_then(|t| Local.from_local_datetime(&t).single())
            .map(|t| t.timestamp_millis())
    }) else {
        return stamp.to_owned();
    };
    let dated = stamp.len() > 10;
    let (Some(then), Some(now)) = (
        Local.timestamp_millis_opt(ms).single(),
        Local.timestamp_millis_opt(now_ms).single(),
    ) else {
        return stamp.to_owned();
    };
    let days = (now.date_naive() - then.date_naive()).num_days();
    let time = if dated {
        then.format("%H:%M").to_string()
    } else {
        String::new()
    };
    match days {
        0 if dated => time,
        0 => "today".to_owned(),
        1 => format!("yesterday {time}").trim_end().to_owned(),
        _ => then.format("%b %-d").to_string(),
    }
}

/// A link destination the note page may carry: relative, or http, https or mailto. Anything
/// with another scheme (javascript, data, file, vbscript) is not.
pub(super) fn plain_url(url: &str) -> bool {
    let url = url.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    let scheme_end = url.find(':');
    let first_break = url.find(['/', '?', '#']).unwrap_or(url.len());
    match scheme_end {
        Some(end) if end < first_break => {
            let scheme = url[..end].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto")
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::unpadded;

    #[test]
    fn an_effort_title_loses_its_padding_and_nothing_else() {
        assert_eq!(
            unpadded("0043 Studio board feedback fixes"),
            "43 Studio board feedback fixes"
        );
        assert_eq!(unpadded("0000 Zero"), "0 Zero");
        assert_eq!(unpadded("0050"), "50");
        assert_eq!(unpadded("43 Already short"), "43 Already short");
        assert_eq!(unpadded("2026-11-01-a"), "2026-11-01-a");
        assert_eq!(unpadded("007bond"), "007bond");
        assert_eq!(unpadded("Notes"), "Notes");
    }
}
