//! Searching the text of the notes, for the notes view's search box: every note whose path
//! and text hold all the words asked for, each with a line of text around the first word.

use crate::notes;
use std::path::Path;

/// The longest query read; the search box takes no more, so every word typed is searched.
const MAX_QUERY: usize = 200;

/// The most notes one answer names; a query this loose is narrowed by typing more.
pub(super) const MAX_HITS: usize = 300;

/// About this many characters of text either side of the word that matched.
const BEFORE: usize = 50;
const AFTER: usize = 110;

/// The answer to `/search?q=…`: `{"q": …, "hits": {path: snippet, …}, "more": bool}`, the
/// query echoed so the page can drop an answer to one it has since moved past, and `more`
/// when the hits stopped at `MAX_HITS`.
pub(super) fn answer(root: &Path, query: &str) -> String {
    let query: String = query.chars().take(MAX_QUERY).collect();
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut hits = serde_json::Map::new();
    let mut more = false;
    if !words.is_empty() {
        for rel in note_files(root) {
            if hits.len() >= MAX_HITS {
                more = true;
                break;
            }
            let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
                continue;
            };
            if let Some(snippet) = matches(&rel, &text, &words) {
                hits.insert(rel, snippet.into());
            }
        }
    }
    serde_json::json!({ "q": query, "hits": hits, "more": more }).to_string()
}

/// Every note the tree lists: the ones at the top of the checkout, then each folder's.
fn note_files(root: &Path) -> Vec<String> {
    let mut top = Vec::new();
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            if name != ".git" && name != ".obsidian" {
                dirs.push(name);
            }
        } else if name.ends_with(".md") {
            top.push(name);
        }
    }
    top.sort();
    dirs.sort();
    for dir in dirs {
        top.extend(notes::md_files(root, &dir));
    }
    top
}

/// The snippet when every word is in the note's path or text, else None.
pub(super) fn matches(rel: &str, text: &str, words: &[String]) -> Option<String> {
    let body = without_front_matter(text);
    let lower = body.to_lowercase();
    let path = rel.to_lowercase();
    if !words
        .iter()
        .all(|w| lower.contains(w.as_str()) || path.contains(w.as_str()))
    {
        return None;
    }
    // Lowercasing can change a character's length, and offsets into the lowercase then miss
    // the original's characters; the snippet comes from the lowercase instead.
    let same = body
        .chars()
        .all(|c| c.to_lowercase().map(char::len_utf8).sum::<usize>() == c.len_utf8());
    let source = if same { body } else { &lower };
    let at = words.iter().filter_map(|w| lower.find(w.as_str())).min();
    Some(at.map_or_else(String::new, |at| snippet(source, at)))
}

/// The text after a `---` block at the very top, closed by a line of just `---`, if there is
/// one.
fn without_front_matter(text: &str) -> &str {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return text;
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        if line.trim_end() == "---" {
            return &rest[at..];
        }
    }
    text
}

/// A line of text around byte `at`, cut on character boundaries, with markdown marks and
/// runs of space taken out, and an ellipsis where it was cut.
fn snippet(text: &str, at: usize) -> String {
    let start = back(text, at, BEFORE);
    let end = forward(text, at, AFTER);
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    let mut space = false;
    for c in text[start..end].chars() {
        if c.is_whitespace() {
            space = !out.is_empty() && !out.ends_with('…');
            continue;
        }
        if matches!(c, '#' | '*' | '`' | '>' | '|') {
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    if end < text.len() {
        out.push('…');
    }
    unlink(&out)
}

/// A markdown link as its text: `[text](target)` reads `text`, `[[note]]` reads `note`.
fn unlink(text: &str) -> String {
    let text = text.replace("[[", "").replace("]]", "");
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("](") {
        out += &rest[..i];
        match rest[i + 2..].find(')') {
            Some(j) => rest = &rest[i + 2 + j + 1..],
            // Cut inside the target: the rest is the target.
            None => {
                if rest.ends_with('…') {
                    out.push('…');
                }
                rest = "";
            }
        }
    }
    out += rest;
    out.replace('[', "")
}

/// The byte `chars` characters before `at`, moved on to the start of a word when it lands
/// inside one.
fn back(text: &str, at: usize, chars: usize) -> usize {
    let head = &text[..at];
    let Some((start, _)) = head.char_indices().rev().nth(chars) else {
        return 0;
    };
    match head[start..].find(char::is_whitespace) {
        Some(gap) if start + gap < at => start + gap,
        _ => start,
    }
}

/// The byte `chars` characters after `at`, moved back to the end of a word when it lands
/// inside one.
fn forward(text: &str, at: usize, chars: usize) -> usize {
    let Some((end, _)) = text[at..].char_indices().nth(chars) else {
        return text.len();
    };
    let end = at + end;
    match text[at..end].rfind(char::is_whitespace) {
        Some(gap) if gap > 0 => at + gap,
        _ => end,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(q: &str) -> Vec<String> {
        q.split_whitespace().map(str::to_lowercase).collect()
    }

    #[test]
    fn every_word_must_be_in_the_text_or_the_path() {
        let text = "# Board\n\nThe refresh failed quietly when the server was down.\n";
        let rel = "efforts/2026/11-November/0129-board.md";
        assert!(matches(rel, text, &words("refresh SERVER")).is_some());
        assert!(
            matches(rel, text, &words("0129 quietly")).is_some(),
            "path and text"
        );
        assert!(matches(rel, text, &words("refresh missing")).is_none());
    }

    #[test]
    fn front_matter_is_not_searched() {
        let text = "---\nstatus: active\nowner: x\n---\n\n# Title\n\nBody words.\n";
        assert!(matches("a.md", text, &words("active")).is_none());
        assert_eq!(
            matches("a.md", text, &words("body")).as_deref(),
            Some("Title Body words.")
        );
        // An empty block, and a `---foo` line that doesn't close one.
        assert_eq!(without_front_matter("---\n---\nafter\n"), "after\n");
        assert_eq!(without_front_matter("---\na: 1\n---foo\n---\nbody"), "body");
        assert_eq!(
            without_front_matter("---\nnever closed\n"),
            "---\nnever closed\n"
        );
    }

    #[test]
    fn a_snippet_is_cut_on_words_with_marks_and_space_taken_out() {
        let long = format!(
            "{} the **needle** sits `here` {}",
            "lead ".repeat(30),
            "tail ".repeat(40)
        );
        let s = matches("a.md", &long, &words("needle")).unwrap();
        assert!(s.starts_with("…lead"), "{s}");
        assert!(s.ends_with("tail…"), "{s}");
        assert!(s.contains("the needle sits here"), "{s}");
        assert!(!s.contains("  "), "{s}");
        assert_eq!(
            matches(
                "a.md",
                "See [the board](../efforts/0129-board.md) and [[owner]] for more.",
                &words("board")
            )
            .as_deref(),
            Some("See the board and owner for more.")
        );
        assert_eq!(unlink("cut [in a link](../far/away…"), "cut in a link…");
        // One character that grows when lowercased and one that shrinks keep the total
        // length; the offsets still mustn't land inside a character.
        let even = format!(
            "\u{130} {} needle \u{212b} {}",
            "x ".repeat(60),
            "y ".repeat(80)
        );
        assert!(matches("a.md", &even, &words("needle")).is_some());
        // Characters past one byte don't split.
        let wide = format!("{}ünïcödé needle {}", "é".repeat(80), "ß".repeat(200));
        assert!(matches("a.md", &wide, &words("needle")).is_some());
    }

    #[test]
    fn the_answer_names_notes_under_the_checkout_and_echoes_the_query() {
        let dir = std::env::temp_dir().join(format!("astrolabe-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, text) in [
            ("knowledge/owner.md", "# Owner\n\nLikes plain words.\n"),
            ("efforts/2026/11-November/0001-a.md", "# A\n\nNothing.\n"),
            (".agents/brief.md", "# Brief\n\nPlain words too.\n"),
            (".git/notes.md", "plain words in git\n"),
            ("README.md", "# Readme\n"),
        ] {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        let out: serde_json::Value = serde_json::from_str(&answer(&dir, "plain  WORDS")).unwrap();
        assert_eq!(out["q"], "plain  WORDS");
        let hits = out["hits"].as_object().unwrap();
        let mut names: Vec<&str> = hits.keys().map(String::as_str).collect();
        names.sort();
        assert_eq!(names, [".agents/brief.md", "knowledge/owner.md"]);
        assert_eq!(hits["knowledge/owner.md"], "Owner Likes plain words.");
        let none: serde_json::Value = serde_json::from_str(&answer(&dir, "   ")).unwrap();
        assert!(none["hits"].as_object().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
