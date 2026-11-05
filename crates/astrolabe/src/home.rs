//! The owner's board: `home.html` at the notes checkout root, rewritten whenever the notes
//! change on this machine, from the same rows `astrolabe efforts --json` returns. One
//! self-contained file, no script, no network, so it opens in any browser on any machine.

use crate::notes::{self, Notes};
use chrono::{DateTime, FixedOffset};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const FILE: &str = "home.html";
/// Done and abandoned efforts stay on the board this long after their last save.
const DONE_DAYS: i64 = 14;
const SENTENCE_CHARS: usize = 160;

/// Renders and writes the board; the path written, or why not.
pub fn write(notes: &Notes) -> Result<PathBuf, String> {
    let rows = crate::record::rows_in(notes, None, true);
    let path = notes.root.join(FILE);
    notes::write_text(&path, &render(&rows, &notes.root, notes::now_ms()))?;
    Ok(path)
}

struct Card<'a> {
    row: &'a Value,
    updated_ms: Option<i64>,
}

impl Card<'_> {
    fn text(&self, key: &str) -> &str {
        self.row[key].as_str().unwrap_or_default()
    }
    fn status(&self) -> &str {
        self.text("status")
    }
    fn closed(&self) -> bool {
        matches!(self.status(), "done" | "abandoned")
    }
    fn priority_rank(&self) -> u8 {
        match self.text("priority") {
            "urgent" => 0,
            "high" => 1,
            "low" => 3,
            _ => 2,
        }
    }
}

/// The page, as text. `now_ms` is passed in so a test can fix the clock.
pub fn render(rows: &Value, root: &Path, now_ms: i64) -> String {
    let cards: Vec<Card> = rows["efforts"]
        .as_array()
        .map(|a| a.iter())
        .into_iter()
        .flatten()
        .map(|row| Card {
            row,
            updated_ms: row["updated"].as_str().and_then(stamp_ms),
        })
        .collect();
    let mut columns: [Vec<&Card>; 6] = Default::default();
    for card in &cards {
        let slot = if card.row["needs_owner"] == true {
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
    let titles = [
        "Needs you",
        "Active",
        "Ready",
        "Blocked",
        "Ideas",
        "Done, last 14 days",
    ];
    let keys = ["needs", "active", "ready", "blocked", "ideas", "done"];
    let mut board = String::new();
    for (i, column) in columns.iter().enumerate() {
        let body = if column.is_empty() {
            "<p class=\"empty\">nothing</p>\n".to_owned()
        } else {
            column
                .iter()
                .map(|c| card_html(c, root, now_ms))
                .collect::<String>()
        };
        let heading = format!(
            "<h2>{} <span class=\"count\">{}</span></h2>",
            titles[i],
            column.len()
        );
        if keys[i] == "done" {
            board += &format!(
                "<section class=\"col done\"><details><summary>{heading}</summary>\n{body}</details></section>\n"
            );
        } else {
            board += &format!(
                "<section class=\"col {}\">{heading}\n{body}</section>\n",
                keys[i]
            );
        }
    }

    let active = columns[1].len() + columns[0].iter().filter(|c| c.status() == "active").count();
    let counts = format!(
        "{active} active · {} need you · {} blocked",
        columns[0].len(),
        columns[3].len()
    );
    let sync = rows["notes"]
        .as_array()
        .map(|w| {
            w.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "notes in sync".to_owned());

    let mut journal = String::new();
    for line in rows["journal_lines"].as_array().into_iter().flatten() {
        journal += &format!(
            "<li>{} <span class=\"num\">{}</span></li>\n",
            esc(line["text"].as_str().unwrap_or_default()),
            esc(line["effort"].as_str().unwrap_or_default())
        );
    }
    if journal.is_empty() {
        journal = "<li class=\"empty\">nothing yet today</li>\n".to_owned();
    }

    let mut initiatives: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for card in &cards {
        let name = tag(card.text("initiative"));
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
    let mut initiative_lines = String::new();
    for (name, (open, done)) in &initiatives {
        initiative_lines += &format!(
            "<li><span class=\"tag\">{}</span> {open} open, {done} done</li>\n",
            esc(name)
        );
    }
    if initiative_lines.is_empty() {
        initiative_lines = "<li class=\"empty\">no initiatives named</li>\n".to_owned();
    }

    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"60\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>Astrolabe home</title>\n<style>{STYLE}</style></head><body>\n<header><h1>{}</h1><span class=\"counts\">{counts}</span><span class=\"sync\">{}</span></header>\n<div class=\"board\">\n{board}</div>\n<footer><section><h2>Today</h2><ul>\n{journal}</ul></section><section><h2>Initiatives</h2><ul>\n{initiative_lines}</ul></section></footer>\n</body></html>\n",
        esc(rows["today"].as_str().unwrap_or_default()),
        esc(&sync),
    )
}

fn card_html(card: &Card, root: &Path, now_ms: i64) -> String {
    let mut bits = Vec::new();
    let initiative = tag(card.text("initiative"));
    if !initiative.is_empty() {
        bits.push(format!("<span class=\"tag\">{}</span>", esc(initiative)));
    }
    if !card.text("repo").is_empty() {
        bits.push(format!(
            "<span class=\"repo\">{}</span>",
            esc(card.text("repo"))
        ));
    }
    let priority = card.text("priority");
    if matches!(priority, "high" | "urgent") {
        bits.push(format!("<span class=\"pri {priority}\">{priority}</span>"));
    }
    if let (Some(done), Some(total)) = (
        card.row["pieces"]["done"].as_u64(),
        card.row["pieces"]["total"].as_u64(),
    ) {
        bits.push(format!("<span class=\"pieces\">{done}/{total}</span>"));
    }
    if let Some(t) = card.updated_ms {
        bits.push(format!("<span class=\"age\">{}</span>", age(now_ms - t)));
    }
    if card.status().is_empty() {
        bits.push("<span class=\"nostatus\">no status</span>".to_owned());
    }
    let waits = card.text("waits_for");
    let waits = if waits.is_empty() {
        String::new()
    } else {
        let shown = waits.strip_prefix("owner:").unwrap_or(waits).trim();
        format!("<p class=\"waits\">{}</p>\n", esc(shown))
    };
    let href = match card.row["file"].as_str() {
        Some(file) => format!("file://{}", root.join(file).display()),
        None => "#".to_owned(),
    };
    format!(
        "<article class=\"card{}\"><h3><a href=\"{}\"><span class=\"num\">{}</span> {}</a></h3>\n<p class=\"meta\">{}</p>\n{waits}<p class=\"next\">{}</p></article>\n",
        if card.closed() { " done" } else { "" },
        esc(&href),
        card.row["number"].as_u64().unwrap_or(0),
        esc(card.text("title")),
        bits.join(" "),
        esc(&first_sentence(card.text("next"))),
    )
}

/// `2026-11-04T16:30-07:00`, the stamp saves write, as epoch milliseconds.
fn stamp_ms(stamp: &str) -> Option<i64> {
    DateTime::<FixedOffset>::parse_from_str(stamp, "%Y-%m-%dT%H:%M%:z")
        .ok()
        .map(|t| t.timestamp_millis())
}

fn age(ms: i64) -> String {
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
fn first_sentence(paragraph: &str) -> String {
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
fn tag(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .trim()
}

fn esc(text: &str) -> String {
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

const STYLE: &str = r#"
:root{--bg:#f6f5f2;--card:#fff;--ink:#1d1d1b;--mute:#6b6a66;--line:#e4e2dc;--accent:#b5562a;--needs:#fff3e8;--tag:#eef0f5}
@media(prefers-color-scheme:dark){:root{--bg:#1b1b19;--card:#262623;--ink:#ece9e2;--mute:#9b988f;--line:#36352f;--accent:#e08a5a;--needs:#33271f;--tag:#2f3138}}
*{box-sizing:border-box}body{margin:0;padding:24px 28px;background:var(--bg);color:var(--ink);font:15px/1.45 -apple-system,"Segoe UI",system-ui,sans-serif}
header{display:flex;flex-wrap:wrap;align-items:baseline;gap:18px;margin-bottom:20px}header h1{font-size:20px;margin:0;font-weight:600}header .counts,header .sync{color:var(--mute)}
.board{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:16px;align-items:start}
.col h2{font-size:13px;font-weight:600;text-transform:uppercase;letter-spacing:.04em;color:var(--mute);margin:0 0 10px}.count{font-weight:400;opacity:.7}
.col.needs h2{color:var(--accent)}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px 14px;margin-bottom:10px}.col.needs .card{background:var(--needs);border-color:transparent}
.card.done{opacity:.65}.card h3{font-size:15px;margin:0 0 6px;font-weight:600}.card h3 a{color:inherit;text-decoration:none}.card h3 a:hover{text-decoration:underline}
.num{color:var(--mute);font-weight:400;font-variant-numeric:tabular-nums}
.meta{margin:0 0 6px;display:flex;flex-wrap:wrap;gap:6px;font-size:12px;color:var(--mute)}.tag,.repo,.pri,.pieces,.nostatus{padding:1px 7px;border-radius:999px;background:var(--tag)}
.pri.high{background:#f7d9c5;color:#7a3a16}.pri.urgent{background:#f2b8b8;color:#7a1616}.age{margin-left:auto}
.next{margin:0}.waits{margin:0 0 6px;color:var(--accent)}.empty{color:var(--mute);font-style:italic}
details summary{list-style:none;cursor:pointer}details summary h2{display:inline}
footer{display:grid;grid-template-columns:2fr 1fr;gap:24px;margin-top:28px;padding-top:18px;border-top:1px solid var(--line)}footer h2{font-size:13px;text-transform:uppercase;letter-spacing:.04em;color:var(--mute);margin:0 0 8px}
footer ul{margin:0;padding-left:18px}footer li{margin-bottom:6px}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
                row(8, "done", json!({"updated": "2026-10-10T09:00-07:00"})),
                row(9, "active", json!({"priority": "high", "pieces": {"done": 2, "total": 5}, "initiative": "[[agent-tooling]]", "next": "**Next:** Measure it. Then decide."})),
            ],
            "journal_lines": [{"effort": "0001", "text": "Finished <it>."}],
            "notes": [],
        });
        let page = render(&rows, Path::new("/n"), NOW);
        assert!(page.contains("A &lt;b&gt;&amp;&lt;/b&gt; &quot;q&quot;"));
        assert!(!page.contains("<b>&</b>"));
        let needs = section(&page, "needs");
        assert!(needs.contains("Effort 2") && needs.contains("class=\"waits\">pick one<"));
        let active = section(&page, "active");
        assert!(active.contains("&quot;q&quot;") && active.contains("Effort 9"));
        assert!(!active.contains("Effort 2"));
        // Priority first, then the newest save.
        assert!(active.find("Effort 9").unwrap() < active.find("&quot;q&quot;").unwrap());
        assert!(active.contains("class=\"pri high\">high<"));
        assert!(active.contains("class=\"pieces\">2/5<"));
        assert!(active.contains("class=\"tag\">agent-tooling<"));
        assert!(active.contains("class=\"next\">Measure it.<"));
        assert!(active.contains("class=\"age\">3 h<"));
        assert!(active.contains("href=\"file:///n/efforts/0001-e.md\""));
        let blocked = section(&page, "blocked");
        assert!(blocked.contains("Effort 3") && blocked.contains(">the merge driver<"));
        assert!(section(&page, "ready").contains("Effort 4"));
        let ideas = section(&page, "ideas");
        assert!(ideas.contains("Effort 5") && ideas.contains("Effort 6"));
        assert!(ideas.contains("no status"));
        let done = section(&page, "done");
        assert!(done.contains("Effort 7"));
        assert!(
            !page.contains("Effort 8"),
            "an old done effort is off the board"
        );
        assert!(page.contains("<h1>2026-11-05</h1>"));
        assert!(page.contains("3 active · 1 need you · 1 blocked"));
        assert!(page.contains("notes in sync"));
        assert!(page.contains("<li>Finished &lt;it&gt;. <span class=\"num\">0001</span></li>"));
        assert!(page.contains(">agent-tooling</span> 1 open, 0 done<"));
    }

    #[test]
    fn empty_board_and_warnings() {
        let rows = json!({"today": "2026-11-05", "efforts": [], "journal_lines": [], "notes": ["2 notes commit(s) not yet pushed."]});
        let page = render(&rows, Path::new("/n"), NOW);
        assert!(page.contains("class=\"sync\">2 notes commit(s) not yet pushed.<"));
        assert!(page.contains("nothing yet today"));
        assert!(page.contains("no initiatives named"));
        assert_eq!(page.matches("class=\"empty\">nothing<").count(), 6);
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
}
