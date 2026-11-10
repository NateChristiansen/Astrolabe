//! Safe Markdown rendering and links to notes.
use super::nav::{label_of, tree_html};
use super::render::{Links, back_link, esc, percent, shell, stamp_ms, tag};
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
    // The board reads the same properties through the same reader, so the page and the
    // board's row show one value.
    let props = crate::effort::frontmatter(&text);
    let (_, body) = crate::effort::split_frontmatter(&text);
    let body = strip_comments(body);
    let prop = |key: &str| -> Option<String> {
        let value = props.iter().find(|(k, _)| k == key)?.1.as_str();
        (!value.is_empty() && value != "none").then(|| value.to_owned())
    };
    // An effort that's part of an initiative links to its page on a line of its own,
    // which stands in for the initiative's pill. The page is found the way the board
    // finds it, by slug or title.
    let part_of = prop("initiative").and_then(|name| {
        let pages = crate::effort::initiative_catalog(root).ok()?;
        let page = crate::effort::resolve_initiative(&pages, &name).ok()?;
        Some(format!(
            "<p class=\"part-of\">Part of <a href=\"/{}\">{} →</a></p>",
            esc(&percent(&format!("initiatives/{}.md", page.slug))),
            esc(&page.file.title)
        ))
    });
    // What the owner reads sits under the title; what the program keeps (the save's
    // branch, head and the like) is a small list of facts beside the outline.
    let mut pills = Vec::new();
    for key in [
        "status",
        "repo",
        "priority",
        "initiative",
        "issue",
        "involvement",
        "authority",
        "waits_for",
    ] {
        let Some(value) = prop(key) else { continue };
        if key == "initiative" && part_of.is_some() {
            continue;
        }
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
    let own = rows.efforts.iter().find(|e| e.file.as_deref() == Some(rel));
    let effort = own.is_some();
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
    let members = initiative_members(rel, rows, root);
    let events = with_effort_status(events.collect(), rows, members.as_deref());
    let mut rendered = String::new();
    html::push_html(&mut rendered, events.into_iter());
    // The rail beside the text holds the outline, when there are sections to jump
    // between, and always the facts, so every note keeps them in the same place.
    let outline = if outline.len() > 1 {
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
        format!(
            "<nav class=\"outline\" aria-label=\"On this page\"><h4>On this page</h4>{items}</nav>"
        )
    } else {
        String::new()
    };
    let props = if pills.is_empty() {
        String::new()
    } else {
        format!("<div class=\"props\">{}</div>", pills.join(""))
    };
    // What the record is up to, at a glance, from the rows the board draws: an
    // initiative's progress over its members, an effort's ticked pieces.
    let glance = if let Some(progress) = rows
        .initiatives
        .iter()
        .find(|i| i.page.file.as_deref() == Some(rel))
        .map(|i| &i.progress)
        .filter(|p| p.total + p.abandoned > 0)
    {
        let super::render::ProgressParts { counts, need, bar } =
            super::render::progress_parts(progress);
        format!(
            "<div class=\"glance\"><span class=\"num\">{counts}</span>{need}<span class=\"track\" aria-hidden=\"true\">{bar}</span></div>"
        )
    } else {
        match own.and_then(|e| e.pieces.as_ref()) {
            Some(p) if p.total > 0 => super::render::pieces_bar(p.done, p.total),
            _ => String::new(),
        }
    };
    let props = format!("{}{props}{glance}", part_of.unwrap_or_default());
    // The title and its properties head the page; a note that doesn't open on a title
    // starts with the properties.
    let (head, rendered) = match rendered.find("</h1>") {
        Some(i) if rendered.trim_start().starts_with("<h1") => (
            format!("{}{props}", &rendered[..i + 5]),
            rendered[i + 5..].to_owned(),
        ),
        _ => (props, rendered),
    };
    // There are no folder pages: each folder opens the notes view at that folder. The
    // crumb names folders and the note the way the tree does.
    let mut crumb = String::from("<a href=\"/#notes\">Notes</a>");
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts[..parts.len() - 1].iter().enumerate() {
        crumb += &format!(
            "<span>›</span><a href=\"/#notes/{}\">{}</a>",
            esc(&percent(&parts[..=i].join("/"))),
            esc(label_of(part))
        );
    }
    let label = if effort {
        title.clone()
    } else {
        name.trim_end_matches(".md").to_owned()
    };
    crumb += &format!("<span>›</span><b>{}</b>", esc(&label));
    let page = format!(
        "<article class=\"note frame\"><div class=\"note-back\">{}</div><header class=\"note-head\">{head}</header><div class=\"prose\">{rendered}</div><aside class=\"rail\">{outline}{facts}</aside></article>\n",
        back_link(Links::Served)
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

/// The members of the initiative whose page is at `rel`, in number order, or None for any
/// other note. Membership is each effort's own `initiative:` line, resolved the way the
/// board resolves it, so the page lists who the header counts.
fn initiative_members<'r>(
    rel: &str,
    rows: &'r BoardData,
    root: &Path,
) -> Option<Vec<&'r crate::board::EffortRow>> {
    rows.initiatives
        .iter()
        .find(|i| i.page.file.as_deref() == Some(rel))?;
    let pages = crate::effort::initiative_catalog(root).ok()?;
    let mut members: Vec<_> = rows
        .efforts
        .iter()
        .filter(|e| {
            e.initiative
                .as_deref()
                .filter(|name| !name.is_empty())
                .and_then(|name| crate::effort::resolve_initiative(&pages, name).ok())
                .is_some_and(|page| page.path == rel)
        })
        .collect();
    members.sort_by_key(|e| e.number);
    Some(members)
}

/// A link to an effort note carries the effort's status after it, as the board shows it,
/// and "needs you" when the effort waits on the owner. A link that opens its paragraph or
/// list item and is followed by a full stop is the effort's title line, as an
/// initiative's Efforts list writes them: the pill ends the line and the stop goes, so
/// the text after it reads as the description below.
///
/// On an initiative's page, `members` are its members. Every title line there leads with
/// its effort's number, so an entry matches the work named after it. A title line for an
/// effort that isn't a member says so, since the prose is written once and an effort can
/// leave. The members the prose never links continue the list that holds its last member
/// title line, in number order and in the same shape; when there's no such list, they're
/// listed after the section that holds its last member link, or at the end when it links
/// none. The page shows every member, and the stored prose keeps its own order and text.
fn with_effort_status<'a>(
    events: Vec<pulldown_cmark::Event<'a>>,
    rows: &BoardData,
    members: Option<&[&crate::board::EffortRow]>,
) -> Vec<pulldown_cmark::Event<'a>> {
    use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};
    let is_member = |effort: &crate::board::EffortRow| {
        members.is_none_or(|all| all.iter().any(|m| m.file == effort.file))
    };
    let mut out = Vec::with_capacity(events.len());
    let mut linked: Vec<&str> = Vec::new();
    let mut after_last_member = None;
    // Where the last member title line that is a list item went, and whether its list
    // wraps each item in a paragraph.
    let mut last_title_item = None;
    let mut i = 0;
    while i < events.len() {
        let effort = match &events[i] {
            Event::Start(Tag::Link { dest_url, .. }) => linked_effort(dest_url, rows),
            _ => None,
        };
        let pill = effort.and_then(status_pill);
        // An effort with no status has no pill, and can still be one the list names
        // that isn't a member.
        let left = effort.is_some_and(|e| !is_member(e));
        let end = (pill.is_some() || left)
            .then(|| {
                events[i..]
                    .iter()
                    .position(|e| matches!(e, Event::End(TagEnd::Link)))
                    .map(|k| i + k)
            })
            .flatten();
        let member = effort.filter(|e| members.is_some() && is_member(e));
        if let Some(effort) = member {
            linked.extend(effort.file.as_deref());
            after_last_member = Some(out.len());
        }
        let Some(end) = end else {
            out.push(events[i].clone());
            i += 1;
            continue;
        };
        let pill = pill.unwrap_or_default();
        let before = |back: usize| i.checked_sub(back).and_then(|k| events.get(k));
        let in_item = matches!(before(1), Some(Event::Start(Tag::Item)));
        let in_loose_item = matches!(before(1), Some(Event::Start(Tag::Paragraph)))
            && matches!(before(2), Some(Event::Start(Tag::Item)));
        let opens = i == 0 || in_item || matches!(before(1), Some(Event::Start(Tag::Paragraph)));
        // The title line's description, and how many events after the link it takes. On
        // an initiative's page a link that fills its list item is a title line too, the
        // way an Efforts list without descriptions writes them.
        let rest = match events.get(end + 1) {
            Some(Event::Text(t)) if opens => t
                .strip_prefix('.')
                .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                .map(|r| (r.trim_start().to_owned(), 1)),
            Some(Event::End(TagEnd::Item)) if members.is_some() && in_item => {
                Some((String::new(), 0))
            }
            Some(Event::End(TagEnd::Paragraph))
                if members.is_some()
                    && in_loose_item
                    && matches!(events.get(end + 2), Some(Event::End(TagEnd::Item))) =>
            {
                Some((String::new(), 0))
            }
            _ => None,
        };
        if let Some((rest, taken)) = rest {
            let left = if left {
                "<span class=\"stat left\">not a member</span>"
            } else {
                ""
            };
            if member.is_some() && (in_item || in_loose_item) {
                last_title_item = Some((out.len(), in_loose_item));
            }
            out.push(Event::Html("<span class=\"etitle\">".into()));
            out.push(events[i].clone());
            // On an initiative's page the title leads with its number, unless the prose
            // already wrote it there.
            let number = effort.and_then(|e| e.number).filter(|n| {
                members.is_some()
                    && !matches!(events.get(i + 1), Some(Event::Text(t))
                        if t.split_whitespace().next().and_then(|w| w.parse::<u64>().ok()) == Some(*n))
            });
            if let Some(n) = number {
                out.push(Event::Text(format!("{n} ").into()));
            }
            out.extend(events[i + 1..=end].iter().cloned());
            out.push(Event::Html(format!("{pill}{left}</span>").into()));
            if !rest.is_empty() {
                out.push(Event::Text(rest.into()));
            }
            i = end + 1 + taken;
        } else {
            out.extend(events[i..=end].iter().cloned());
            if !pill.is_empty() {
                out.push(Event::Html(pill.into()));
            }
            i = end + 1;
        }
    }
    let unlisted: Vec<_> = members
        .unwrap_or_default()
        .iter()
        .filter(|m| !m.file.as_deref().is_some_and(|f| linked.contains(&f)))
        .collect();
    if unlisted.is_empty() {
        return out;
    }
    let items = |loose: bool| -> String {
        unlisted
            .iter()
            .map(|m| {
                let line = member_line(m);
                if loose {
                    format!("<li>\n<p>{line}</p>\n</li>\n")
                } else {
                    format!("<li>{line}</li>\n")
                }
            })
            .collect()
    };
    // The end of the list that holds the last member title line: its own end, past any
    // list nested inside it.
    let list_end = last_title_item.and_then(|(from, loose)| {
        let mut depth = 0usize;
        out[from..]
            .iter()
            .position(|e| match e {
                Event::Start(Tag::List(_)) => {
                    depth += 1;
                    false
                }
                Event::End(TagEnd::List(_)) if depth == 0 => true,
                Event::End(TagEnd::List(_)) => {
                    depth -= 1;
                    false
                }
                _ => false,
            })
            .map(|k| (from + k, loose))
    });
    if let Some((at, loose)) = list_end {
        out.insert(at, Event::Html(items(loose).into()));
        return out;
    }
    let label = if linked.is_empty() {
        "Members"
    } else {
        "Other members"
    };
    let list = Event::Html(
        format!(
            "<p class=\"members-label\">{label}</p>\n<ul class=\"members\">\n{}</ul>\n",
            items(false)
        )
        .into(),
    );
    // The end of the section that holds the last member link: the next first or
    // second level heading after it.
    let at = after_last_member
        .and_then(|from| {
            out[from..]
                .iter()
                .position(|e| {
                    matches!(
                        e,
                        Event::Start(Tag::Heading {
                            level: HeadingLevel::H1 | HeadingLevel::H2,
                            ..
                        })
                    )
                })
                .map(|k| from + k)
        })
        .unwrap_or(out.len());
    out.insert(at, list);
    out
}

/// A member the prose doesn't link, drawn the way a title line in an initiative's list
/// is: its number and title as the link, its status, then the first sentence of its
/// Current state, or its goal where an older note keeps one, as the description.
fn member_line(member: &crate::board::EffortRow) -> String {
    let number = member.number.map(|n| format!("{n} ")).unwrap_or_default();
    let description = member
        .state
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| esc(&super::render::first_sentence(s)))
        .unwrap_or_default();
    format!(
        "<span class=\"etitle\"><a href=\"/{}\">{}{}</a>{}</span>{description}",
        esc(&percent(member.file.as_deref().unwrap_or_default())),
        esc(&number),
        esc(member.title.as_deref().unwrap_or_default()),
        status_pill(member).unwrap_or_default()
    )
}

/// The effort a link to an effort's page opens, or None for any other link.
fn linked_effort<'r>(dest: &str, rows: &'r BoardData) -> Option<&'r crate::board::EffortRow> {
    let path = dest.strip_prefix('/')?.split('#').next()?;
    rows.efforts
        .iter()
        .find(|e| e.file.as_deref().is_some_and(|f| percent(f) == path))
}

/// An effort's status as a pill, with "needs you" when it waits on the owner; None for
/// an effort with no status.
fn status_pill(effort: &crate::board::EffortRow) -> Option<String> {
    let status = effort.status.as_deref().unwrap_or_default();
    let word = match status {
        "" => return None,
        "proposed" | "ready" => "to start",
        "review" => "in review",
        s => s,
    };
    let mut pill = format!(
        "<span class=\"stat s-{}\">{}</span>",
        esc(status),
        esc(word)
    );
    if effort.needs_owner {
        pill += "<span class=\"stat need\">needs you</span>";
    }
    Some(pill)
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
    // The name as written first, then with its escapes read: a name holding a space or a
    // `#` can only be linked encoded. A decoded `\` is refused, since Windows reads it as
    // a separator the checks below don't, and a decoded `:`, since the check above refuses
    // one as written: an escape must not spell a drive or a scheme.
    let decoded = super::server::decode(path).filter(|d| d != path && !d.contains(['\\', ':']));
    let mut candidates = Vec::new();
    for path in std::iter::once(path).chain(decoded.as_deref()) {
        candidates.push(normalize(&format!("{dir}/{path}")));
        candidates.push(normalize(path));
    }
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
