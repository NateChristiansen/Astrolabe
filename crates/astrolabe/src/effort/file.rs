//! Effort note sections, properties and checklists, including fenced examples.

/// What some Windows editors put before a file's text.
pub(super) const BOM: char = '\u{feff}';

/// A note's header and body. The header is the lines between a first `---` line and the next
/// one, trailing whitespace and a carriage return allowed on either; a byte-order mark before
/// it isn't part of it. A note with no opening line is all body, and one whose header never
/// closes is all header, the way saves have always read it.
pub(crate) fn split_frontmatter(text: &str) -> (&str, &str) {
    // A byte-order mark, as some Windows editors save, isn't part of the first line.
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next().filter(|l| l.trim_end() == "---") else {
        return ("", text);
    };
    let start = first.len();
    let mut end = start;
    for line in lines {
        if line.trim_end() == "---" {
            return (&text[start..end], &text[end + line.len()..]);
        }
        end += line.len();
    }
    (&text[start..], "")
}

/// The plain properties at the top of a note: `key: value`, quoted or not, and a list as
/// `[a, b]` or `- a` lines, joined with commas. Keys are read in lowercase. Nested YAML isn't
/// read. The board and each note's page both read properties here, so they never disagree.
pub(crate) fn frontmatter(text: &str) -> Vec<(String, String)> {
    let (header, _) = split_frontmatter(text);
    let unquote = |v: &str| {
        v.trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_owned()
    };
    let mut props: Vec<(String, String)> = Vec::new();
    for line in header.lines() {
        if let (Some(item), Some((_, value))) =
            (line.trim_start().strip_prefix("- "), props.last_mut())
        {
            if !value.is_empty() {
                value.push_str(", ");
            }
            value.push_str(&unquote(item));
        } else if let Some((key, value)) = line.split_once(':').filter(|_| !line.starts_with(' ')) {
            let value = value.trim();
            let value = if value.starts_with("[[") && value.ends_with("]]") {
                // A legacy wikilink is a name, not a nested YAML list.
                unquote(value)
            } else {
                match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    Some(list) => list.split(',').map(unquote).collect::<Vec<_>>().join(", "),
                    None => unquote(value),
                }
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
    /// Original bytes and parsed values, so unmanaged text survives edits.
    pub original: Option<Box<OriginalFile>>,
    pub title: String,
    pub current: String,
    pub plan: Option<String>,
    /// Kept verbatim; a save only appends a line, because a rebuilt list drops nested text.
    pub decisions: String,
    /// Other sections, heading and body, in their order.
    pub other: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalFile {
    text: String,
    parsed: EffortFile,
}

/// Byte spans of the body, respecting fenced headings and retaining line endings.
fn body_spans(text: &str) -> (usize, Vec<(usize, usize, String)>) {
    let clean = text.strip_prefix(BOM).unwrap_or(text);
    let bom = text.len() - clean.len();
    let mut body = bom;
    if clean.lines().next() == Some("---") {
        let mut offset = bom;
        for (i, line) in clean.split_inclusive('\n').enumerate() {
            offset += line.len();
            if i > 0 && line.trim_end() == "---" {
                body = offset;
                break;
            }
        }
    }
    let lines: Vec<_> = text[body..].split_inclusive('\n').collect();
    let fenced = fence_mask(lines.iter().map(|l| l.trim_end_matches(['\r', '\n'])));
    let mut starts = Vec::new();
    let mut offset = body;
    let mut title = false;
    for (line, fence) in lines.into_iter().zip(fenced) {
        if !fence {
            if let Some(h) = line.strip_prefix("## ") {
                starts.push((offset, h.trim().to_owned()));
            } else if !title && starts.is_empty() && line.starts_with("# ") {
                starts.push((offset, "#".to_owned()));
                starts.push((offset + line.len(), String::new()));
                title = true;
            }
        }
        offset += line.len();
    }
    let spans = starts
        .iter()
        .enumerate()
        .map(|(i, (start, h))| {
            (
                *start,
                starts.get(i + 1).map_or(text.len(), |s| s.0),
                h.clone(),
            )
        })
        .collect();
    (body, spans)
}

/// The order properties are written in; unknown ones follow, as read.
pub(super) const PROP_ORDER: [&str; 16] = [
    "status",
    "repo",
    "initiative",
    "issue",
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

/// Initiative managed properties have a separate, single ordering definition.
const INITIATIVE_PROP_ORDER: [&str; 9] = [
    "status",
    "repo",
    "involvement",
    "authority",
    "waits_for",
    "created",
    "updated",
    "saved_by",
    "session",
];

impl EffortFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        let original_text = text.to_owned();
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
        let mut seen = std::collections::HashSet::new();
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
                let h = h.trim();
                if matches!(h, "Current state" | "Decisions" | "Plan") && !seen.insert(h.to_owned())
                {
                    return Err(format!("duplicate managed section: {h}"));
                }
                flush(&heading, &mut buf, &mut file);
                heading = Some(h.to_owned());
            } else if heading.is_some() {
                buf.push_str(line);
                buf.push('\n');
            }
        }
        flush(&heading, &mut buf, &mut file);
        file.original = Some(Box::new(OriginalFile {
            text: original_text,
            parsed: file.clone(),
        }));
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

    /// Initiative purpose from its parsed section or original, fence-aware preamble.
    pub fn purpose(&self) -> Option<String> {
        if let Some((_, body)) = self.other.iter().find(|(heading, _)| heading == "Purpose") {
            return (!body.trim().is_empty()).then(|| body.trim().to_owned());
        }
        let original = self.original.as_ref()?;
        let (body, spans) = body_spans(&original.text);
        let first = spans.first().map_or(original.text.len(), |span| span.0);
        let mut preamble = original.text[body..first].to_owned();
        for (start, end, heading) in spans {
            if heading.is_empty() {
                preamble.push_str(&original.text[start..end]);
            }
        }
        (!preamble.trim().is_empty()).then(|| preamble.trim().to_owned())
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

    /// Unchecked checklist lines, recognized exactly as the piece counter recognizes them.
    pub fn unticked_pieces(&self) -> impl Iterator<Item = &str> {
        self.pieces()
            .unwrap_or_default()
            .lines()
            .filter(|line| piece_box(line) == Some(false))
    }

    /// When ticking, an exact label wins; otherwise a unique unfinished prefix selects the
    /// item. Unticking keeps its existing unique-prefix match across all pieces.
    pub fn piece_at(&self, label: &str, done: bool) -> Result<usize, String> {
        let label = label.trim();
        let text = self
            .pieces()
            .ok_or("this effort has no `## Pieces` checklist; add one with --pieces-file")?;
        let wanted = match_key(label);
        let items: Vec<(usize, &str)> = text
            .lines()
            .enumerate()
            .filter(|(_, l)| piece_box(l).is_some())
            .collect();
        let exact: Vec<_> = items
            .iter()
            .filter(|(_, l)| match_key(piece_label(l)) == wanted)
            .collect();
        let matches: Vec<_> = if !done || exact.is_empty() {
            items
                .iter()
                .filter(|(_, l)| {
                    (!done || piece_box(l) == Some(false))
                        && match_key(piece_label(l)).starts_with(&wanted)
                })
                .collect()
        } else {
            exact
        };
        let names = |items: &[&(usize, &str)]| {
            items
                .iter()
                .map(|(_, l)| format!("- {}", piece_label(l)))
                .collect::<Vec<_>>()
                .join("\n")
        };
        match matches.as_slice() {
            [one] => Ok(one.0),
            [] => Err(format!(
                "no piece starts with {label:?}{}; available pieces:\n{}",
                if done { " among unfinished pieces" } else { "" },
                names(&items.iter().collect::<Vec<_>>())
            )),
            _ => Err(format!(
                "{label:?} names more than one piece; say more of it:\n{}",
                names(&matches)
            )),
        }
    }

    /// Ticks or unticks an item resolved by `piece_at` before any checklist changes. A tick
    /// stamps the day its outcome was met; an untick drops the stamp; an item already ticked
    /// keeps its first day.
    pub fn tick(&mut self, at: usize, done: bool, day: &str) {
        let text = self.pieces().unwrap_or_default();
        let lines: Vec<String> = text
            .lines()
            .enumerate()
            .map(|(i, l)| match (i == at, done, piece_box(l)) {
                (true, true, Some(false)) => format!("- [x] {} ({day})", piece_label(l)),
                (true, false, _) => format!("- [ ] {}", piece_label(l)),
                _ => l.to_owned(),
            })
            .collect();
        self.set_pieces(&lines.join("\n"));
    }

    /// The labels of ticked items, without their day stamps.
    pub fn ticked_labels(&self) -> Vec<&str> {
        self.pieces()
            .unwrap_or_default()
            .lines()
            .filter(|l| piece_box(l) == Some(true))
            .map(piece_label)
            .collect()
    }

    pub fn render(&self) -> String {
        if let Some(original) = &self.original {
            let mut plain = self.clone();
            plain.original = None;
            if plain == original.parsed {
                return original.text.clone();
            }
            return self.render_preserved(original, false);
        }
        self.render_fresh(false)
    }

    /// Initiative records always carry both managed sections, even before a decision.
    pub fn render_initiative(&self) -> String {
        if let Some(original) = &self.original {
            self.render_preserved(original, true)
        } else {
            self.render_fresh(true)
        }
    }

    fn render_preserved(&self, original: &OriginalFile, initiative: bool) -> String {
        let mut managed = self.clone();
        managed.original = None;
        managed.other.clear();
        let mut out = managed.render_fresh(initiative);
        let (body, spans) = body_spans(&original.text);
        let first = spans.first().map_or(original.text.len(), |s| s.0);
        let has_purpose = spans.iter().any(|(_, _, h)| h == "Purpose");
        let mut preamble = original.text[body..first].to_owned();
        for (start, end, h) in &spans {
            if h.is_empty() {
                preamble.push_str(&original.text[*start..*end]);
            }
        }
        if !initiative {
            let at = out
                .find("\n## Current state\n")
                .expect("fresh managed section");
            out.replace_range(at..at + 1, &preamble);
            preamble.clear();
        } else if !has_purpose && !preamble.trim().is_empty() {
            out.push_str("\n## Purpose\n");
            out.push_str(&preamble);
            preamble.clear();
        }
        let mut written = Vec::new();
        for (start, end, heading) in spans {
            if matches!(
                heading.as_str(),
                "" | "#" | "Current state" | "Decisions" | "Plan"
            ) {
                continue;
            }
            if heading == "Purpose" && !preamble.trim().is_empty() {
                let raw = &original.text[start..end];
                let at = raw.find('\n').map_or(raw.len(), |n| n + 1);
                out.push_str(&raw[..at]);
                out.push_str(&preamble);
                out.push_str(&raw[at..]);
                written.push(heading);
                continue;
            }
            let before = original.parsed.other.iter().find(|(h, _)| h == &heading);
            let after = self.other.iter().find(|(h, _)| h == &heading);
            if before == after {
                out.push_str(&original.text[start..end]);
            } else if let Some((h, b)) = after {
                out.push_str(&format!("\n## {h}\n{}\n", b.trim()));
            }
            written.push(heading);
        }
        for (h, b) in &self.other {
            if !written.contains(h) {
                out.push_str(&format!("\n## {h}\n{}\n", b.trim()));
            }
        }
        out
    }

    fn render_fresh(&self, initiative: bool) -> String {
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
        let order: &[&str] = if initiative {
            &INITIATIVE_PROP_ORDER
        } else {
            &PROP_ORDER
        };
        for &key in order {
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
        if initiative || !self.decisions.trim().is_empty() {
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

/// An item's text without the ` (YYYY-MM-DD)` a tick stamps at its end, so ticking and
/// matching by the outcome's words keep working once it's stamped. Only a ticked item has a
/// stamp: an unticked item's trailing date is its own, such as a deadline.
pub(super) fn piece_label(line: &str) -> &str {
    let text = piece_text(line);
    if piece_box(line) != Some(true) {
        return text;
    }
    text.strip_suffix(')')
        .and_then(|t| t.rsplit_once(" ("))
        .filter(|(_, d)| is_day(d))
        .map_or(text, |(label, _)| label.trim_end())
}

/// How a tick's value is compared with a label: case and inline-code backticks ignored,
/// because a flag value can't carry a backtick and pieces often name a command in code.
fn match_key(text: &str) -> String {
    text.replace('`', "").trim().to_lowercase()
}

fn is_day(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

/// A file name's slug from a title: lowercase words joined by dashes, at most six.
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
    fn a_piece_label_drops_only_a_trailing_day_stamp() {
        for (line, label) in [
            ("- [x] Tether cap (2026-11-08)", "Tether cap"),
            ("- [x] Tether cap", "Tether cap"),
            (
                "- [ ] Check (slow) links (soon)",
                "Check (slow) links (soon)",
            ),
            ("- [x] Release (2026-11)", "Release (2026-11)"),
            ("- [ ] Demo build (2026-12-01)", "Demo build (2026-12-01)"),
        ] {
            assert_eq!(piece_label(line), label, "{line}");
        }
    }

    #[test]
    fn a_tick_matches_a_piece_whose_label_starts_with_inline_code() {
        let mut f = EffortFile::default();
        f.set_pieces("- [ ] `tally --csv` prints a header\n- [ ] `tally --out` writes a file");
        assert_eq!(f.piece_at("tally --csv", true), Ok(0));
        assert_eq!(f.piece_at("tally --out writes a file", true), Ok(1));
    }

    #[test]
    fn ticking_keeps_a_date_that_belongs_to_the_outcome() {
        let mut f = EffortFile::default();
        f.set_pieces("- [ ] Demo build (2026-12-01)");
        let at = f.piece_at("Demo build (2026-12-01)", true).unwrap();
        f.tick(at, true, "2026-11-08");
        assert_eq!(
            f.pieces(),
            Some("- [x] Demo build (2026-12-01) (2026-11-08)")
        );
        let at = f.piece_at("demo build (2026-12-01)", false).unwrap();
        f.tick(at, false, "2026-11-08");
        assert_eq!(f.pieces(), Some("- [ ] Demo build (2026-12-01)"));
    }

    #[test]
    fn frontmatter_reads_quotes_lists_casing_and_byte_order_marks() {
        let one = |text: &str, key: &str| {
            frontmatter(text)
                .into_iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v)
        };
        for (text, key, value) in [
            ("---\nstatus: 'active'\n---\n", "status", "active"),
            ("---\nstatus: \"active\"\n---\n", "status", "active"),
            ("---\nnote: \"a: b\"\n---\n", "note", "a: b"),
            ("---\nrepo: [demo, 'other']\n---\n", "repo", "demo, other"),
            (
                "---\nrepo:\n  - demo\n  - \"other\"\n---\n",
                "repo",
                "demo, other",
            ),
            ("---\nStatus: active\n---\n", "status", "active"),
            ("\u{feff}---\nstatus: active\n---\n", "status", "active"),
            ("---\r\nstatus: active\r\n---\r\n", "status", "active"),
            ("--- \nstatus: active\n---  \n", "status", "active"),
            (
                "---\ninitiative: \"[[shared-work]]\"\n---\n",
                "initiative",
                "[[shared-work]]",
            ),
            (
                "---\ninitiative: [[shared-work]]\n---\n",
                "initiative",
                "[[shared-work]]",
            ),
        ] {
            assert_eq!(one(text, key).as_deref(), Some(value), "{text:?}");
        }
        // No header, or one that only appears later, is no properties.
        assert!(frontmatter("# Title\nstatus: active\n").is_empty());
        assert!(frontmatter("\n---\nstatus: active\n---\n").is_empty());
        // An unterminated header reads to the end, as saves always have.
        assert_eq!(
            one("---\nstatus: active\n# Title\n", "status").as_deref(),
            Some("active")
        );
    }

    #[test]
    fn split_frontmatter_separates_header_and_body() {
        for (text, header, body) in [
            ("---\na: 1\n---\n# T\n", "a: 1\n", "# T\n"),
            ("\u{feff}---\na: 1\n---\n# T\n", "a: 1\n", "# T\n"),
            ("---\r\na: 1\r\n---\r\n# T\r\n", "a: 1\r\n", "# T\r\n"),
            ("---\n---\n# T\n", "", "# T\n"),
            ("---\na: 1\n---", "a: 1\n", ""),
            ("# T\n---\na: 1\n---\n", "", "# T\n---\na: 1\n---\n"),
            ("\u{feff}# T\n", "", "# T\n"),
            ("---\na: 1\n# T\n", "a: 1\n# T\n", ""),
        ] {
            assert_eq!(split_frontmatter(text), (header, body), "{text:?}");
        }
    }

    #[test]
    fn purpose_uses_fence_aware_sections_and_preambles() {
        let purpose =
            "Purpose first.\n\n```md\n## Current state\n# False title\n```\n\nStill purpose.";
        for text in [
            format!("# Page\n\n{purpose}\n\n## Decisions\n"),
            format!("# Page\n\n## Purpose\n{purpose}\n\n## Decisions\n"),
            format!("{purpose}\n\n# Page\n\n## Decisions\n"),
        ] {
            let file = EffortFile::parse(&text.replace('\n', "\r\n")).unwrap();
            assert_eq!(file.purpose().unwrap().replace("\r\n", "\n"), purpose);
            assert_eq!(file.first_paragraph(), None);
        }
    }

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
