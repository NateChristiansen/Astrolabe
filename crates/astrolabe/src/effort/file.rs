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

/// Where `split_frontmatter` found a note's header, as byte offsets into the whole text, its
/// byte-order mark included, for a writer that changes the header and keeps every other byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeaderBounds {
    /// The header's first byte, after the opening line.
    pub start: usize,
    /// The end of the header's last line, where the closing line starts.
    pub end: usize,
    /// The body's first byte, after the closing line.
    pub body: usize,
}

impl HeaderBounds {
    /// Whether a closing line ends the header. One that never closes runs to the end of the
    /// text, so there's no line to write a new property before.
    pub(crate) fn closed(&self) -> bool {
        self.end < self.body
    }
}

/// The offsets of the header `split_frontmatter` returns, or None for a note with no opening
/// line. Every writer that splices by byte asks here, so none has its own idea of where the
/// header ends.
pub(crate) fn header_bounds(text: &str) -> Option<HeaderBounds> {
    let clean = text.strip_prefix(BOM).unwrap_or(text);
    let (header, body) = split_frontmatter(text);
    // With no opening line the body is the whole text; with one, it's at least a line shorter.
    if body.len() == clean.len() {
        return None;
    }
    let opener = clean.split_inclusive('\n').next().map_or(0, str::len);
    let start = text.len() - clean.len() + opener;
    Some(HeaderBounds {
        start,
        end: start + header.len(),
        body: text.len() - body.len(),
    })
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
    /// An effort's goal: the text between its title and its first section, given when it was
    /// filed and kept through every save, so the effort still says what it was for once
    /// Current state is rewritten. An initiative's text there is its purpose, written apart.
    pub lead: String,
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
    // Without a header the body starts after any byte-order mark.
    let bom = text.len() - text.strip_prefix(BOM).unwrap_or(text).len();
    let body = header_bounds(text).map_or(bom, |at| at.body);
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
pub(super) const PROP_ORDER: [&str; 17] = [
    "status",
    "repo",
    "initiative",
    "issue",
    "lessons",
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
        let text = text.strip_prefix(BOM).unwrap_or(&text);
        let props = frontmatter(text);
        let (_, body) = split_frontmatter(text);
        let mut file = Self {
            props,
            ..Default::default()
        };
        let mut seen = std::collections::HashSet::new();
        let mut heading: Option<String> = None;
        let mut buf = String::new();
        let mut lead = String::new();
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
                // Before any section, a fenced line is dropped like any other text there,
                // apart from the lead under the title.
                if heading.is_some() {
                    buf.push_str(line);
                    buf.push('\n');
                } else if !file.title.is_empty() {
                    lead.push_str(line);
                    lead.push('\n');
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
            } else if !file.title.is_empty() {
                lead.push_str(line);
                lead.push('\n');
            }
        }
        flush(&heading, &mut buf, &mut file);
        // Only a plain line of prose is a goal: text there with a heading, a fence or a list
        // is something else, kept as it is and never written back as a goal.
        file.lead = plain_goal(&lead)
            .filter(|_| !lead.trim().contains("\n\n"))
            .unwrap_or_default();
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

    /// The goal the effort was filed with, on one line, when its note keeps one.
    pub fn goal(&self) -> Option<String> {
        let goal = self.lead.split_whitespace().collect::<Vec<_>>().join(" ");
        (!goal.is_empty()).then_some(goal)
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
                (true, true, Some(false)) => {
                    format!("- [x] {}{} ({day})", piece_label(l), added_suffix(l))
                }
                (true, false, _) => format!("- [ ] {}{}", piece_label(l), added_suffix(l)),
                _ => l.to_owned(),
            })
            .collect();
        self.set_pieces(&lines.join("\n"));
    }

    /// A list that replaces this one, with each piece that came after the plan tagged
    /// ` (added <day>)`. Each line here is matched once, by its words, and a new line that
    /// matches one keeps that one's tag, or its lack of one, so a list copied back without
    /// its tags loses none; a line that repeats one already matched is new. A new line in
    /// place of an unticked one that's gone is a rewording, paired in order, and keeps the
    /// gone one's tag the same way; a dropped line, struck through, still names its piece.
    /// The new lines left over are tagged with `day`, or with nothing when there's none, as
    /// on a re-plan.
    pub fn with_added_tags(&self, new: &str, day: Option<&str>) -> String {
        let key_of = |l: &str| match piece_box(l) {
            Some(_) => Some(match_key(piece_label(l))),
            None => struck_label(l).map(match_key),
        };
        let old: Vec<(String, String, bool)> = self
            .pieces()
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let key = key_of(l)?;
                Some((key, added_suffix(l), piece_box(l) == Some(false)))
            })
            .collect();
        let new_lines: Vec<&str> = new.lines().collect();
        // First, every line that names a piece here takes the first such piece not taken yet.
        let mut used = vec![false; old.len()];
        let matched: Vec<Option<usize>> = new_lines
            .iter()
            .map(|l| {
                let key = key_of(l)?;
                let at = (0..old.len()).find(|&i| !used[i] && old[i].0 == key)?;
                used[at] = true;
                Some(at)
            })
            .collect();
        // Then the unticked pieces nobody took are gone, and the lines left over take their
        // places in order.
        let mut gone = (0..old.len()).filter(|&i| !used[i] && old[i].2);
        new_lines
            .iter()
            .zip(matched)
            .map(|(line, at)| {
                if key_of(line).is_none() {
                    return (*line).to_owned();
                }
                let at = at.or_else(|| gone.next());
                if piece_box(line).is_none() {
                    return (*line).to_owned();
                }
                let suffix = match at {
                    Some(i) => old[i].1.clone(),
                    None => day.map(|d| format!(" (added {d})")).unwrap_or_default(),
                };
                with_added_suffix(line, &suffix)
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        // The lead is written from the original's own bytes below, unless a save changed it.
        managed.lead.clear();
        let mut out = managed.render_fresh(initiative);
        let (body, spans) = body_spans(&original.text);
        let first = spans.first().map_or(original.text.len(), |s| s.0);
        let has_purpose = spans.iter().any(|(_, _, h)| h == "Purpose");
        let mut preamble = original.text[body..first].to_owned();
        if initiative || self.lead == original.parsed.lead {
            for (start, end, h) in &spans {
                if h.is_empty() {
                    preamble.push_str(&original.text[*start..*end]);
                }
            }
        } else {
            match plain_goal(&self.lead) {
                Some(goal) => preamble += &format!("\n{goal}\n\n"),
                None => preamble.push('\n'),
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
        out += &format!("# {}\n\n", self.title);
        // An initiative's lead is its purpose, which its own writer places.
        if let Some(goal) = plain_goal(&self.lead).filter(|_| !initiative) {
            out += &format!("{goal}\n\n");
        }
        out += &format!("## Current state\n{}\n", self.current.trim());
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

/// The goal a filing's body gives: its first paragraph on one line, when that paragraph is
/// plain prose. One with a heading, a fence, a list, a quote, a table or an indented block
/// isn't, because written under the title it would change how the note reads back.
pub fn plain_goal(text: &str) -> Option<String> {
    let first = text
        .split('\n')
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .collect::<Vec<_>>();
    let plain = |l: &&str| {
        let t = l.trim_start();
        let indent = l.len() - t.len();
        let marker = t.starts_with(['#', '>', '|', '<', '`', '~'])
            || ["- ", "* ", "+ "].iter().any(|m| t.starts_with(m))
            || t.trim_end()
                .chars()
                .all(|c| matches!(c, '-' | '=' | '*' | '_'))
            || t.split_once(['.', ')']).is_some_and(|(n, rest)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && rest.starts_with(' ')
            });
        indent < 4 && !marker
    };
    if first.is_empty() || !first.iter().all(plain) {
        return None;
    }
    Some(
        first
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
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
    let text = if piece_box(line) == Some(true) {
        text.strip_suffix(')')
            .and_then(|t| t.rsplit_once(" ("))
            .filter(|(_, d)| is_day(d))
            .map_or(text, |(label, _)| label.trim_end())
    } else {
        text
    };
    split_added(text).0
}

/// A piece's words apart from the ` (added YYYY-MM-DD)` a save writes on a piece that came
/// after the plan, and that day.
pub(crate) fn split_added(text: &str) -> (&str, Option<&str>) {
    text.strip_suffix(')')
        .and_then(|t| t.rsplit_once(" (added "))
        .filter(|(_, d)| is_day(d))
        .map_or((text, None), |(label, day)| (label.trim_end(), Some(day)))
}

/// The ` (added YYYY-MM-DD)` a piece line carries, or nothing.
fn added_suffix(line: &str) -> String {
    let text = piece_text(line);
    let text = match piece_box(line) {
        Some(true) => text
            .strip_suffix(')')
            .and_then(|t| t.rsplit_once(" ("))
            .filter(|(_, d)| is_day(d))
            .map_or(text, |(label, _)| label.trim_end()),
        _ => text,
    };
    split_added(text)
        .1
        .map(|day| format!(" (added {day})"))
        .unwrap_or_default()
}

/// A piece line carrying `suffix` (an added tag, or nothing) after its words and before any
/// tick stamp. A line that already carries a tag keeps its own.
fn with_added_suffix(line: &str, suffix: &str) -> String {
    if suffix.is_empty() || !added_suffix(line).is_empty() {
        return line.to_owned();
    }
    let indent = &line[..line.len() - line.trim_start().len()];
    let label = piece_label(line);
    match piece_box(line) {
        Some(true) => {
            let text = piece_text(line);
            let stamp = text
                .strip_suffix(')')
                .and_then(|t| t.rsplit_once(" ("))
                .filter(|(_, d)| is_day(d))
                .map(|(_, d)| format!(" ({d})"))
                .unwrap_or_default();
            format!("{indent}- [x] {label}{suffix}{stamp}")
        }
        _ => format!("{indent}- [ ] {label}{suffix}"),
    }
}

/// The outcome a dropped line names: `- ~~<outcome>~~ dropped <date>: <why>`.
fn struck_label(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("- ")?;
    let rest = rest
        .strip_prefix("[ ] ")
        .or_else(|| rest.strip_prefix("[x] "))
        .unwrap_or(rest);
    let (label, _) = rest.strip_prefix("~~")?.split_once("~~")?;
    Some(label.trim())
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
    fn a_goal_under_the_title_survives_a_rewritten_current_state() {
        let mut f = EffortFile {
            title: "0012 Smooth ink".to_owned(),
            lead: "Make the ink line smooth: no gaps, \"even\" at speed.".to_owned(),
            current: "Make the ink line smooth.".to_owned(),
            ..Default::default()
        };
        let text = f.render();
        assert!(
            text.contains("# 0012 Smooth ink\n\nMake the ink line smooth: no gaps, \"even\" at speed.\n\n## Current state\n"),
            "{text}"
        );
        let mut read = EffortFile::parse(&text).unwrap();
        assert_eq!(
            read.goal().as_deref(),
            Some("Make the ink line smooth: no gaps, \"even\" at speed.")
        );
        read.current = "Done and merged.".to_owned();
        let saved = read.render();
        assert_eq!(saved.matches("no gaps").count(), 1, "{saved}");
        let again = EffortFile::parse(&saved).unwrap();
        assert_eq!(again.goal(), read.goal());
        assert_eq!(again.current, "Done and merged.");
        // A note with nothing under its title has no goal, and a save adds none.
        f.lead.clear();
        let bare = EffortFile::parse(&f.render()).unwrap();
        assert_eq!(bare.goal(), None);
        assert!(
            bare.render()
                .contains("# 0012 Smooth ink\n\n## Current state\n")
        );
    }

    #[test]
    fn a_changed_goal_replaces_the_lead_and_keeps_the_rest() {
        let text = "---\nstatus: active\n---\n# 0012 Smooth ink\n\nOld goal.\n\n## Current state\nGoing.\n\n## Notes\n  kept  as is.  \n";
        let mut f = EffortFile::parse(text).unwrap();
        f.lead = "New goal.".to_owned();
        let out = f.render();
        assert!(
            out.contains("# 0012 Smooth ink\n\nNew goal.\n\n## Current state\nGoing.\n"),
            "{out}"
        );
        assert!(!out.contains("Old goal"), "{out}");
        assert!(out.contains("  kept  as is.  "), "{out}");
    }

    #[test]
    fn an_initiative_page_keeps_its_purpose_where_it_was() {
        let text = "---\nstatus: active\n---\n# Page\n\nA legacy purpose.\n\n## Current state\nGoing.\n\n## Decisions\n";
        let mut f = EffortFile::parse(text).unwrap();
        f.current = "Still going.".to_owned();
        let out = f.render_initiative();
        assert_eq!(out.matches("A legacy purpose.").count(), 1, "{out}");
    }

    fn with_pieces(list: &str) -> EffortFile {
        let mut f = EffortFile::default();
        f.set_pieces(list);
        f
    }

    #[test]
    fn a_piece_that_comes_after_the_list_is_tagged_with_the_day() {
        let f = with_pieces("- [x] one (2026-11-08)\n- [ ] two");
        let out = f.with_added_tags(
            "- [x] one (2026-11-08)\n- [ ] two\n- [ ] three",
            Some("2026-11-10"),
        );
        assert_eq!(
            out,
            "- [x] one (2026-11-08)\n- [ ] two\n- [ ] three (added 2026-11-10)"
        );
        // Copied back without its tag, a later piece keeps the day it came.
        let f = with_pieces(&out);
        let again = f.with_added_tags(
            "- [x] one (2026-11-08)\n- [ ] two\n- [ ] three\n- [ ] four",
            Some("2026-11-12"),
        );
        assert_eq!(
            again,
            "- [x] one (2026-11-08)\n- [ ] two\n- [ ] three (added 2026-11-10)\n- [ ] four (added 2026-11-12)"
        );
    }

    #[test]
    fn a_reworded_piece_keeps_its_tag_or_its_lack_of_one() {
        let f = with_pieces("- [ ] one\n- [ ] two (added 2026-11-09)\n- [ ] three");
        let out = f.with_added_tags(
            "- [ ] one, said better\n- [ ] two, said better\n- [ ] three\n- [ ] four",
            Some("2026-11-10"),
        );
        assert_eq!(
            out,
            "- [ ] one, said better\n- [ ] two, said better (added 2026-11-09)\n- [ ] three\n- [ ] four (added 2026-11-10)"
        );
        // A dropped piece, struck through, still names it, so a new piece beside it is added.
        let f = with_pieces("- [ ] one\n- [ ] two");
        let out = f.with_added_tags(
            "- [ ] one\n- ~~two~~ dropped 2026-11-10: not needed\n- [ ] five",
            Some("2026-11-10"),
        );
        assert_eq!(
            out,
            "- [ ] one\n- ~~two~~ dropped 2026-11-10: not needed\n- [ ] five (added 2026-11-10)"
        );
    }

    #[test]
    fn a_repeated_piece_is_added_and_a_replan_keeps_old_tags_without_adding_any() {
        let f = with_pieces("- [ ] a\n- [ ] b");
        assert_eq!(
            f.with_added_tags("- [ ] a\n- [ ] b\n- [ ] a", Some("2026-11-10")),
            "- [ ] a\n- [ ] b\n- [ ] a (added 2026-11-10)"
        );
        let f = with_pieces("- [ ] a\n- [ ] b (added 2026-11-09)");
        assert_eq!(
            f.with_added_tags("- [ ] a\n- [ ] b\n- [ ] c", None),
            "- [ ] a\n- [ ] b (added 2026-11-09)\n- [ ] c"
        );
    }

    #[test]
    fn a_goal_is_plain_prose_on_one_line() {
        assert_eq!(
            plain_goal("Smooth the\nink.\n\n## Plan\n1. x").as_deref(),
            Some("Smooth the ink.")
        );
        for not in [
            "## Plan\nx",
            "```\nSmooth.",
            "- a list",
            "1. a list",
            "> quote",
            "    code",
            "Title\n===",
            "",
        ] {
            assert_eq!(plain_goal(not), None, "{not:?}");
        }
        // Text under a title that isn't a plain paragraph is kept as it is, never read as a
        // goal, and never hides the sections after it once a save rewrites them.
        for lead in ["### Aside\nx", "Two\n\nparagraphs", "- a list"] {
            let text = format!("# 0012 Ink\n\n{lead}\n\n## Current state\nOld.\n");
            let mut f = EffortFile::parse(&text).unwrap();
            assert_eq!(f.goal(), None, "{lead:?}");
            f.current = "New.".to_owned();
            let out = f.render();
            assert!(
                out.contains(&format!("{lead}\n\n## Current state\nNew.\n")),
                "{out}"
            );
        }
        // A goal set to something that isn't plain is never written under the title.
        let f = EffortFile {
            title: "0012 Ink".to_owned(),
            lead: "```\nfenced".to_owned(),
            current: "Going.".to_owned(),
            ..Default::default()
        };
        assert!(
            f.render()
                .contains("# 0012 Ink\n\n## Current state\nGoing.")
        );
    }

    #[test]
    fn a_tagged_piece_ticks_and_matches_by_its_words() {
        let mut f = with_pieces("- [ ] one\n- [ ] two (added 2026-11-09)");
        let at = f.piece_at("two", true).unwrap();
        f.tick(at, true, "2026-11-10");
        assert_eq!(
            f.pieces(),
            Some("- [ ] one\n- [x] two (added 2026-11-09) (2026-11-10)")
        );
        assert_eq!(f.ticked_labels(), vec!["two"]);
        let at = f.piece_at("two", false).unwrap();
        f.tick(at, false, "2026-11-11");
        assert_eq!(f.pieces(), Some("- [ ] one\n- [ ] two (added 2026-11-09)"));
        // A ticked later piece copied back bare keeps both its days.
        let f = with_pieces("- [x] two (added 2026-11-09) (2026-11-10)");
        assert_eq!(
            f.with_added_tags("- [x] two (2026-11-10)", Some("2026-11-12")),
            "- [x] two (added 2026-11-09) (2026-11-10)"
        );
    }

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

    /// Notes whose header a hand or another editor left in an odd shape, each with the
    /// same body. `None` is a note with no header at all.
    fn odd_headers() -> Vec<(&'static str, String)> {
        let body = "# 0012 Smooth ink\n\nIntro  line.  \n\n## Current state\nOld.\n\n## History\n  keep  these spaces.  \n\n## Decisions\n- 2026-11-03: a over b: why.\n\n## Plan\n1. first\n\n## Order\nlast section\n";
        let props = "status: active\nrepo: demo\n";
        let plain = format!("---\n{props}---\n{body}");
        vec![
            ("plain", plain.clone()),
            (
                "space after the opener",
                format!("---  \n{props}---\n{body}"),
            ),
            (
                "space after the closer",
                format!("---\n{props}---  \n{body}"),
            ),
            ("tab after the closer", format!("---\n{props}---\t\n{body}")),
            ("space after both", format!("--- \n{props}--- \n{body}")),
            ("empty header", format!("---\n---\n{body}")),
            ("byte-order mark", format!("\u{feff}{plain}")),
            ("windows line endings", plain.replace('\n', "\r\n")),
            (
                "mark, windows endings and spaces",
                format!("\u{feff}---\n{props}---  \n{body}").replace('\n', "\r\n"),
            ),
            ("no final newline", plain.trim_end().to_owned()),
            (
                "blank line after the closer",
                format!("---\n{props}---\n\n{body}"),
            ),
            ("no header", body.to_owned()),
        ]
    }

    #[test]
    fn header_bounds_are_the_offsets_of_what_split_frontmatter_returns() {
        let mut texts: Vec<String> = odd_headers().into_iter().map(|(_, t)| t).collect();
        texts.extend(
            [
                "",
                "---",
                "---\n",
                "---\n---",
                "---\na: 1\n# T\n",
                "\u{feff}---\r\na: 1\r\n# T",
                "\u{feff}# T\n",
                "\n---\na: 1\n---\n",
            ]
            .map(str::to_owned),
        );
        for text in texts {
            let (header, body) = split_frontmatter(&text);
            match header_bounds(&text) {
                Some(at) => {
                    assert_eq!(&text[at.start..at.end], header, "{text:?}");
                    assert_eq!(&text[at.body..], body, "{text:?}");
                    let closer = &text[at.end..at.body];
                    assert_eq!(at.closed(), closer.trim_end() == "---", "{text:?}");
                    assert!(closer.is_empty() || at.closed(), "{text:?}");
                }
                None => {
                    assert_eq!(header, "", "{text:?}");
                    assert_eq!(body, text.strip_prefix(BOM).unwrap_or(&text), "{text:?}");
                }
            }
        }
        assert!(header_bounds("---\na: 1\n# T\n").is_some_and(|at| !at.closed()));
        assert!(header_bounds("# T\n---\n").is_none());
    }

    #[test]
    fn odd_headers_round_trip_and_a_save_keeps_everything_it_does_not_manage() {
        for (name, text) in odd_headers() {
            let mut file = EffortFile::parse(&text).unwrap();
            assert_eq!(file.title, "0012 Smooth ink", "{name}");
            assert_eq!(file.current, "Old.", "{name}");
            assert_eq!(file.decisions, "- 2026-11-03: a over b: why.", "{name}");
            assert_eq!(file.plan.as_deref(), Some("1. first"), "{name}");
            // Unchanged, it's written back as it was read.
            assert_eq!(file.render(), text, "{name}");
            let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let raw = |from: &str, to: &str| {
                let start = text.find(from).unwrap();
                let end = if to.is_empty() {
                    text.len()
                } else {
                    text.find(to).unwrap()
                };
                text[start..end].to_owned()
            };
            let intro = format!("{eol}Intro  line.  {eol}{eol}");
            let history = raw("## History", "## Decisions");
            let order = raw("## Order", "");
            for state in ["First save.", "Second save."] {
                file.current = state.into();
                file.set("status", "review");
                let out = file.render();
                let saved = EffortFile::parse(&out).unwrap();
                assert_eq!(saved.title, "0012 Smooth ink", "{name}: {out:?}");
                assert_eq!(saved.current, state, "{name}");
                assert_eq!(saved.decisions, file.decisions, "{name}");
                assert_eq!(saved.plan, file.plan, "{name}");
                assert_eq!(saved.prop("status"), Some("review"), "{name}");
                assert_eq!(saved.prop("repo"), file.prop("repo"), "{name}");
                // One header, one of each section, and the unmanaged text byte for byte.
                assert_eq!(out.matches("status:").count(), 1, "{name}: {out:?}");
                for once in [
                    "# 0012 Smooth ink",
                    "## Current state",
                    "## History",
                    "## Decisions",
                    "## Plan",
                    "## Order",
                    intro.as_str(),
                    history.as_str(),
                    order.as_str(),
                ] {
                    assert_eq!(out.matches(once).count(), 1, "{name}: {once:?} in {out:?}");
                }
                assert!(out.ends_with(&order), "{name}: {out:?}");
                file = saved;
            }
        }
    }

    #[test]
    fn a_well_formed_note_saves_to_the_same_bytes_as_before() {
        // What the strict reader wrote for these, kept as written so a change shows.
        let text = "---\nstatus: active\nrepo: demo\n---\n# 0012 Smooth ink\n\nIntro.\n\n## Current state\nOld.\n\n## History\nold stuff\n\n## Decisions\n- a\n";
        let mut file = EffortFile::parse(text).unwrap();
        file.current = "New.".into();
        assert_eq!(
            file.render(),
            "---\nstatus: active\nrepo: demo\n---\n# 0012 Smooth ink\n\nIntro.\n\n## Current state\nNew.\n\n## Decisions\n- a\n## History\nold stuff\n\n"
        );
        let crlf = text.replace('\n', "\r\n");
        let mut file = EffortFile::parse(&format!("\u{feff}{crlf}")).unwrap();
        file.current = "New.".into();
        assert_eq!(
            file.render(),
            "---\nstatus: active\nrepo: demo\n---\n# 0012 Smooth ink\n\r\nIntro.\r\n\r\n## Current state\nNew.\n\n## Decisions\n- a\n## History\r\nold stuff\r\n\r\n"
        );
    }

    #[test]
    fn an_unclosed_header_is_all_header_and_is_written_back_untouched() {
        let text = "---\nstatus: active\n# 0012 Smooth ink\n\n## Current state\nOld.\n";
        let file = EffortFile::parse(text).unwrap();
        assert_eq!(file.prop("status"), Some("active"));
        assert_eq!((file.title.as_str(), file.current.as_str()), ("", ""));
        assert_eq!(file.render(), text);
        assert_eq!(file.purpose(), None);
        let (body, spans) = body_spans(text);
        assert_eq!((body, spans.len()), (text.len(), 0));
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
