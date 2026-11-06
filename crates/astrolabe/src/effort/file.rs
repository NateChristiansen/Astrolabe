//! Effort note sections, properties and checklists, including fenced examples.

/// What some Windows editors put before a file's text.
pub(super) const BOM: char = '\u{feff}';

/// The plain properties at the top of a note: `key: value`, quoted or not, and a list as
/// `[a, b]` or `- a` lines, joined with commas. Nested YAML isn't read.
pub(crate) fn frontmatter(text: &str) -> Vec<(String, String)> {
    // A byte-order mark, as some Windows editors save, isn't part of the first line.
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Vec::new();
    }
    let unquote = |v: &str| {
        v.trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_owned()
    };
    let mut props: Vec<(String, String)> = Vec::new();
    for line in lines.take_while(|line| line.trim_end() != "---") {
        if let (Some(item), Some((_, value))) =
            (line.trim_start().strip_prefix("- "), props.last_mut())
        {
            if !value.is_empty() {
                value.push_str(", ");
            }
            value.push_str(&unquote(item));
        } else if let Some((key, value)) = line.split_once(':').filter(|_| !line.starts_with(' ')) {
            let value = value.trim();
            let value = match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                Some(list) => list.split(',').map(unquote).collect::<Vec<_>>().join(", "),
                None => unquote(value),
            };
            props.push((key.trim().to_ascii_lowercase(), value));
        }
    }
    props
}

/// Which lines sit inside a Markdown code fence. A fence opens with three or more of one
/// character (backtick or tilde) and closes only with a line of at least as many of the same
/// character and nothing else, so a shorter run or the other character inside stays text.
pub(crate) fn fence_mask<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<bool> {
    let run = |t: &str, c: char| t.chars().take_while(|x| *x == c).count();
    let mut open: Option<(char, usize)> = None;
    lines
        .map(|l| {
            let t = l.trim();
            match open {
                Some((c, n)) => {
                    if run(t, c) >= n && t.chars().all(|x| x == c) {
                        open = None;
                    }
                    true
                }
                None => {
                    for c in ['`', '~'] {
                        let n = run(t, c);
                        if n >= 3 {
                            open = Some((c, n));
                            return true;
                        }
                    }
                    false
                }
            }
        })
        .collect()
}

/// An effort note as a whole: properties, title, the sections the program touches, and
/// everything else kept as it is. Only `## Current state` is replaced whole; `## Plan` is set
/// once a plan is approved and sits below Decisions; `## Decisions` only grows. The rest is the
/// owner's or history.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EffortFile {
    pub props: Vec<(String, String)>,
    pub title: String,
    pub current: String,
    pub plan: Option<String>,
    /// Kept verbatim; a save only appends a line, because a rebuilt list drops nested text.
    pub decisions: String,
    /// Other sections, heading and body, in their order.
    pub other: Vec<(String, String)>,
}

/// The order properties are written in; unknown ones follow, as read.
pub(super) const PROP_ORDER: [&str; 15] = [
    "status",
    "repo",
    "initiative",
    "priority",
    "involvement",
    "authority",
    "waits_for",
    "created",
    "updated",
    "saved_by",
    "session",
    "branch",
    "head",
    "unpushed",
    "dirty",
];

impl EffortFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.replace("\r\n", "\n");
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let props = frontmatter(text);
        let body = match text.strip_prefix("---\n") {
            Some(rest) => rest.split_once("\n---\n").map_or("", |(_, b)| b),
            None => text,
        };
        let mut file = Self {
            props,
            ..Default::default()
        };
        let mut heading: Option<String> = None;
        let mut buf = String::new();
        // A `## ` line inside a code fence (a plan's shell sample, say) is text, not a
        // section, or a fenced "## Current state" would replace the real one on reread.
        let fenced = fence_mask(body.lines());
        let flush = |heading: &Option<String>, buf: &mut String, file: &mut Self| {
            let body = buf.trim().to_owned();
            match heading.as_deref() {
                None => {}
                Some("Current state") => file.current = body,
                Some("Plan") => file.plan = Some(body),
                Some("Decisions") => file.decisions = body,
                Some(h) => file.other.push((h.to_owned(), body)),
            }
            buf.clear();
        };
        for (line, in_fence) in body.lines().zip(fenced) {
            if in_fence {
                // Before any section, a fenced line is dropped like any other text there.
                if heading.is_some() {
                    buf.push_str(line);
                    buf.push('\n');
                }
            } else if let Some(t) = line
                .strip_prefix("# ")
                .filter(|_| file.title.is_empty() && heading.is_none())
            {
                file.title = t.trim().to_owned();
            } else if let Some(h) = line.strip_prefix("## ") {
                flush(&heading, &mut buf, &mut file);
                heading = Some(h.trim().to_owned());
            } else if heading.is_some() {
                buf.push_str(line);
                buf.push('\n');
            }
        }
        flush(&heading, &mut buf, &mut file);
        Ok(file)
    }

    pub fn add_decision(&mut self, line: &str) {
        let mut text = self.decisions.trim_end().to_owned();
        if !text.is_empty() {
            text.push('\n');
        }
        text += &format!("- {line}");
        self.decisions = text;
    }

    pub fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some((_, v)) => *v = value.to_owned(),
            None => self.props.push((key.to_owned(), value.to_owned())),
        }
    }

    /// The number from the title, "0012 Smooth ink" → 12.
    pub fn number(&self) -> Option<u32> {
        self.title.split_whitespace().next()?.parse().ok()
    }

    /// The title without its number.
    pub fn name(&self) -> &str {
        let mut parts = self.title.splitn(2, ' ');
        match (parts.next(), parts.next()) {
            (Some(n), Some(rest)) if n.parse::<u32>().is_ok() => rest.trim(),
            _ => self.title.as_str(),
        }
    }

    /// The first paragraph of Current state: where the effort stands, for the home view
    /// and suggestions. An older labelled block gives its `**Goal:**` line.
    pub fn first_paragraph(&self) -> Option<&str> {
        self.field("Goal").or_else(|| self.paragraphs().next())
    }

    /// The last paragraph of Current state: what's next and what isn't checked. An older
    /// labelled block gives its `**Next:**` line.
    pub fn last_paragraph(&self) -> Option<&str> {
        self.field("Next").or_else(|| self.paragraphs().last())
    }

    fn paragraphs(&self) -> impl Iterator<Item = &str> {
        self.current
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
    }

    /// A `**Label:**` line's text, in the labelled form efforts had before 2026-11-04:
    /// only when the block opens with `**Goal:**`, so a bold word later in prose isn't one.
    fn field(&self, name: &str) -> Option<&str> {
        let legacy = self
            .current
            .lines()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|l| l.trim_start().starts_with("**Goal:**"));
        let prefix = format!("**{name}:**");
        legacy
            .then(|| {
                self.current
                    .lines()
                    .find_map(|l| l.trim_start().strip_prefix(prefix.as_str()))
            })
            .flatten()
            .map(str::trim)
    }

    /// A line that opens with a `**Label:**` in Current state. The block is plain
    /// paragraphs now; the old labelled form read as a form and pushed the text into
    /// fragments. Bold inside a sentence, or a bold phrase with no colon, is prose.
    pub fn labelled_line(&self) -> Option<&str> {
        self.current.lines().map(str::trim_start).find(|l| {
            l.strip_prefix("**")
                .and_then(|rest| rest.split_once("**"))
                .is_some_and(|(span, _)| span.ends_with(':'))
        })
    }

    /// The `## Pieces` checklist, when the effort has one.
    pub fn pieces(&self) -> Option<&str> {
        self.other
            .iter()
            .find(|(h, _)| h == "Pieces")
            .map(|(_, b)| b.as_str())
    }

    pub fn set_pieces(&mut self, text: &str) {
        let text = text.trim().to_owned();
        match self.other.iter_mut().find(|(h, _)| h == "Pieces") {
            Some((_, body)) => *body = text,
            None => self.other.insert(0, ("Pieces".to_owned(), text)),
        }
    }

    /// Ticked and total items of the checklist.
    pub fn pieces_count(&self) -> Option<(usize, usize)> {
        let text = self.pieces()?;
        let items = text.lines().filter(|l| piece_box(l).is_some());
        let (mut done, mut total) = (0, 0);
        for l in items {
            total += 1;
            if piece_box(l) == Some(true) {
                done += 1;
            }
        }
        Some((done, total))
    }

    /// The one item whose text starts with `label`; refused when none or more than one does,
    /// so a tick never lands on the wrong piece.
    pub fn piece_at(&self, label: &str) -> Result<usize, String> {
        let label = label.trim();
        let text = self
            .pieces()
            .ok_or("this effort has no `## Pieces` checklist; add one with --pieces-file")?;
        let wanted = label.to_lowercase();
        let matches: Vec<usize> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                piece_box(l).is_some() && piece_text(l).to_lowercase().starts_with(&wanted)
            })
            .map(|(i, _)| i)
            .collect();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(format!("no piece starts with {label:?}")),
            _ => Err(format!(
                "{label:?} names more than one piece; say more of it"
            )),
        }
    }

    /// Ticks or unticks one item by its label.
    pub fn tick(&mut self, label: &str, done: bool) -> Result<(), String> {
        let at = self.piece_at(label)?;
        let text = self.pieces().unwrap_or_default();
        let lines: Vec<String> = text
            .lines()
            .enumerate()
            .map(|(i, l)| {
                if i == at {
                    let mark = if done { "[x]" } else { "[ ]" };
                    format!("- {mark} {}", piece_text(l))
                } else {
                    l.to_owned()
                }
            })
            .collect();
        self.set_pieces(&lines.join("\n"));
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut out = String::from("---\n");
        let quoted = |v: &str| {
            if v.is_empty() {
                "\"\"".to_owned()
            } else if v.contains(':') || v.contains('#') || v.starts_with('[') || v.starts_with('"')
            {
                format!("\"{}\"", v.replace('"', "'"))
            } else {
                v.to_owned()
            }
        };
        let mut written = Vec::new();
        for key in PROP_ORDER {
            if let Some((k, v)) = self.props.iter().find(|(k, _)| k == key) {
                out += &format!("{k}: {}\n", quoted(v));
                written.push(k.clone());
            }
        }
        for (k, v) in &self.props {
            if !written.contains(k) {
                out += &format!("{k}: {}\n", quoted(v));
            }
        }
        out += "---\n";
        out += &format!(
            "# {}\n\n## Current state\n{}\n",
            self.title,
            self.current.trim()
        );
        if !self.decisions.trim().is_empty() {
            out += &format!("\n## Decisions\n{}\n", self.decisions.trim());
        }
        // Below Decisions: the plan is written for agents, the rest for the owner.
        if let Some(plan) = &self.plan {
            out += &format!("\n## Plan\n{}\n", plan.trim());
        }
        for (h, body) in &self.other {
            out += &format!("\n## {h}\n{}\n", body.trim());
        }
        out
    }
}

/// A file name's slug from a title: lowercase words joined by dashes, at most six.
/// Whether a line is a checklist item, and whether it's ticked.
pub(super) fn piece_box(line: &str) -> Option<bool> {
    let rest = line.trim_start().strip_prefix("- ")?;
    if rest.starts_with("[x] ") || rest.starts_with("[X] ") {
        Some(true)
    } else if rest.starts_with("[ ] ") {
        Some(false)
    } else {
        None
    }
}

pub(super) fn piece_text(line: &str) -> &str {
    line.trim_start()
        .strip_prefix("- ")
        .and_then(|r| r.get(4..))
        .unwrap_or("")
        .trim()
}

pub fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
        .map(|w| w.to_lowercase())
        .collect();
    if words.is_empty() {
        "effort".to_owned()
    } else {
        words.join("-")
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn a_note_round_trips_and_only_its_own_sections_change() {
        let text = "---\nstatus: active\nrepo: demo\nnote: \"a: b\"\n---\n# 0012 Smooth ink\n\n## Current state\nActive, half done.\n\nNext: n.\n\n## History\nold stuff\n\n## Decisions\n- 2026-11-03: a over b: why.\n";
        let mut file = EffortFile::parse(text).unwrap();
        assert_eq!(file.number(), Some(12));
        assert_eq!(file.name(), "Smooth ink");
        assert_eq!(file.first_paragraph(), Some("Active, half done."));
        assert_eq!(file.last_paragraph(), Some("Next: n."));
        assert_eq!(file.labelled_line(), None);
        assert_eq!(file.prop("note"), Some("a: b"));
        assert_eq!(
            file.other,
            vec![("History".to_owned(), "old stuff".to_owned())]
        );
        file.current = "Done.".to_owned();
        file.set("status", "done");
        let out = file.render();
        assert!(out.contains("status: done\n"));
        assert!(out.contains("## Current state\nDone.\n"));
        assert!(out.contains("## History\nold stuff\n"));
        assert!(out.contains("- 2026-11-03: a over b: why.\n"));
        file.add_decision("2026-11-04: c over d: why.");
        assert!(
            file.render()
                .contains("- 2026-11-03: a over b: why.\n- 2026-11-04: c over d: why.\n")
        );
        file.plan = Some("### Steps\n1. a".to_owned());
        let out = file.render();
        assert!(
            out.find("## Decisions").unwrap() < out.find("## Plan").unwrap(),
            "{out}"
        );
        let back = EffortFile::parse(&out).unwrap();
        assert_eq!(back.plan.as_deref(), Some("### Steps\n1. a"));
        // A fenced section heading inside the plan stays in the plan.
        let mut fenced = back.clone();
        fenced.plan = Some("### Steps\n```md\n## Current state\nstolen\n```\n".to_owned());
        let out = fenced.render();
        let again = EffortFile::parse(&out).unwrap();
        assert_eq!(again.current, "Done.");
        assert_eq!(
            again.plan.as_deref(),
            Some("### Steps\n```md\n## Current state\nstolen\n```")
        );
        assert_eq!(
            fence_mask("a\n````\n```\nb\n````\nc".lines()),
            vec![false, true, true, true, true, false]
        );
        // A fenced heading before any section is neither a title nor a section.
        let odd = EffortFile::parse(
            "```\n# not a title\n## Not a section\n```\n# Real\n\n## Current state\nx\n",
        )
        .unwrap();
        assert_eq!(odd.title, "Real");
        assert_eq!(odd.current, "x");
        assert!(odd.other.is_empty());
        assert_eq!(back.decisions.lines().count(), 2);
        let mut labelled = back;
        labelled.current =
            "**Done.** Merged the **parser:** cleanup.\n  **Verified:** a.".to_owned();
        assert_eq!(labelled.labelled_line(), Some("**Verified:** a."));
        labelled.current = "**Goal:** g.\n**Done:** d.\n**Next:** n.".to_owned();
        assert_eq!(labelled.first_paragraph(), Some("g."));
        assert_eq!(labelled.last_paragraph(), Some("n."));
        labelled.current = "Prose first.\n\n**Goal:** example in prose.".to_owned();
        assert_eq!(labelled.first_paragraph(), Some("Prose first."));
        assert_eq!(
            labelled.last_paragraph(),
            Some("**Goal:** example in prose.")
        );
        assert_eq!(
            slug("Smooth ink: across frames, v2!"),
            "smooth-ink-across-frames-v2"
        );
    }
}
