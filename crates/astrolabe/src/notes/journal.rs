//! Journal formats, replacement rules and legacy parsing.
use serde::{Deserialize, Serialize};

/// One journal line: the agent's sentence, with a hidden tail the owner never reads. The
/// tail names the effort (or the repo, for a finding with no effort) and the machine, so one
/// line per effort per day can be replaced by its own writer and a renumber can retag it,
/// and an id so a retry can tell it's already there. Nothing visible is code-written: the
/// owner found code-built lines mechanical and unreadable (2026-11-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalLine {
    #[serde(default)]
    pub id: String,
    /// `0016`, or a repo name for a line with no effort.
    pub effort: String,
    pub text: String,
    pub machine: String,
    /// When it was written, so after a union merge of two machines' replacements the latest
    /// line per effort is the one kept; hidden in the tail, never shown.
    #[serde(default)]
    pub at_ms: i64,
    /// The day file `YYYY-MM-DD`, fixed when the op was planned.
    pub day: String,
    /// One line per effort per day: this one replaces an earlier line for the effort, so
    /// "started" gives way to "finished" when both happen in a day.
    #[serde(default)]
    pub replace: bool,
}

impl JournalLine {
    pub fn rendered(&self) -> String {
        format!("- {} <!-- {} -->", self.text, self.tail())
    }

    /// `0016 mac 1793790492505 e:abc`: the tail inside the comment.
    pub fn tail(&self) -> String {
        format!(
            "{} {} {} e:{}",
            self.effort, self.machine, self.at_ms, self.id
        )
    }
}

/// A day file with one line per effort: where a union merge or two writers left more, the
/// latest by its stamp stays and the rest go. Lines for findings (repo tags) and lines in the
/// first shape are left as they are.
pub fn tidy_journal(text: &str) -> String {
    let entries = parse_journal(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = vec![true; lines.len()];
    let mut latest: Vec<(String, usize, i64)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(entry) = entries.iter().find(|e| e.line == *line) else {
            continue;
        };
        // An unstamped line (one early build wrote them) counts as older than any stamped.
        if !(entry.is_effort() || entry.initiative_slug().is_some()) || entry.legacy {
            continue;
        }
        match latest.iter_mut().find(|(tag, _, _)| *tag == entry.effort) {
            Some((_, at, stamp)) => {
                if entry.at_ms >= *stamp {
                    keep[*at] = false;
                    *at = i;
                    *stamp = entry.at_ms;
                } else {
                    keep[i] = false;
                }
            }
            None => latest.push((entry.effort.clone(), i, entry.at_ms)),
        }
    }
    let mut out = String::new();
    for (line, k) in lines.iter().zip(keep) {
        if k {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// A journal line as read back: the sentence and its tail, in the current shape or the
/// first one (`time kind number title: text <!-- machine provider e:id -->`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub effort: String,
    pub text: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub at_ms: i64,
    /// The line as it is in the file.
    #[serde(skip)]
    pub line: String,
    /// In the first shape, which held history and is left alone.
    #[serde(skip)]
    pub legacy: bool,
}

impl JournalEntry {
    /// The distinct namespace keeps page journals separate from repository findings.
    pub fn initiative_slug(&self) -> Option<&str> {
        self.effort
            .strip_prefix("initiative:")
            .filter(|slug| !slug.is_empty())
    }
    /// Tagged with an effort number, not a repo.
    pub fn is_effort(&self) -> bool {
        self.effort.len() == 4 && self.effort.chars().all(|c| c.is_ascii_digit())
    }
}

pub fn parse_journal(text: &str) -> Vec<JournalEntry> {
    text.lines()
        .filter_map(|raw| {
            let line = raw.strip_prefix("- ")?;
            let (body, tail) = line.rsplit_once(" <!-- ")?;
            let tail = tail.strip_suffix(" -->")?;
            let words: Vec<&str> = tail.split_whitespace().collect();
            let id = words.last()?.strip_prefix("e:")?.to_owned();
            // The first shape's tail was `machine provider e:id`, the provider one of three
            // words, and its body began with the time and kind; the current tail is
            // `tag machine stamp e:id`.
            let legacy =
                words.len() == 3 && matches!(words[1], "claude" | "codex" | "astrolabe") && {
                    let mut parts = body.splitn(3, ' ');
                    let (time, kind) = (
                        parts.next().unwrap_or_default(),
                        parts.next().unwrap_or_default(),
                    );
                    time.len() == 5
                        && time.as_bytes()[2] == b':'
                        && time.chars().filter(|c| c.is_ascii_digit()).count() == 4
                        && matches!(
                            kind,
                            "started"
                                | "decision"
                                | "blocked"
                                | "done"
                                | "abandoned"
                                | "renumbered"
                        )
                };
            if !legacy {
                return Some(JournalEntry {
                    effort: words.first()?.to_string(),
                    text: body.to_owned(),
                    id,
                    at_ms: words.get(2).and_then(|w| w.parse().ok()).unwrap_or(0),
                    line: raw.to_owned(),
                    legacy: false,
                });
            }
            let mut parts = body.splitn(4, ' ');
            let (_time, _kind, number) = (parts.next()?, parts.next()?, parts.next()?);
            let rest = parts.next().unwrap_or_default();
            let text = rest.split_once(": ").map_or(rest, |(_, t)| t);
            Some(JournalEntry {
                effort: number.to_owned(),
                text: text.to_owned(),
                id,
                at_ms: 0,
                line: raw.to_owned(),
                legacy: true,
            })
        })
        .collect()
}

#[cfg(test)]
mod initiative_tests {
    use super::*;

    #[test]
    fn merged_page_replacements_do_not_replace_a_repository_with_the_same_slug() {
        let text = "- Old page. <!-- initiative:curves mac 10 e:a -->\n- Repo finding. <!-- curves mac 11 e:b -->\n- New page. <!-- initiative:curves pc 20 e:c -->\n- Older page arriving later. <!-- initiative:curves mac 15 e:d -->\n- Effort change. <!-- 0001 mac 12 e:e -->\n";
        let tidy = tidy_journal(text);
        assert!(!tidy.contains("Old page.") && !tidy.contains("Older page"));
        assert!(
            tidy.contains("Repo finding.")
                && tidy.contains("New page.")
                && tidy.contains("Effort change.")
        );
        let entries = parse_journal(&tidy);
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.initiative_slug() == Some("curves"))
                .count(),
            1
        );
    }
}
