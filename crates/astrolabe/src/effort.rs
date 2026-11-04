//! `astrolabe effort new | save`: the command an agent types to record its work, and what
//! the hook reads out of it. The agent writes meaning (the title, the goal, a Current state
//! body); code writes everything else. The body rides in a heredoc, because prose in
//! command arguments breaks on quoting, PowerShell most of all, and because the hook sees the
//! whole command string and does the write itself, outside the sandbox the command would run
//! in: an agent in a product repo can't write the notes checkout with its own tools.
//!
//! The grammar is small on purpose and anything else is refused with the accepted form shown,
//! so a command that parses is one the program fully understands.

/// The one delimiter, quoted so the shell expands nothing in the body.
pub const DELIMITER: &str = "ASTROLABE";

/// What an `astrolabe effort …` command says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortCommand {
    /// `new` or `save`.
    pub verb: String,
    /// `--key value` pairs, in order, keys without the dashes.
    pub flags: Vec<(String, String)>,
    /// The heredoc or here-string body, trimmed, with `\n` line ends. None when none was given.
    pub body: Option<String>,
}

/// The flags each verb takes; any other is refused, so a mistyped one can't be ignored.
pub const NEW_FLAGS: [&str; 6] = ["title", "status", "involvement", "authority", "initiative", "priority"];
pub const SAVE_FLAGS: [&str; 5] = ["status", "decision", "waits-for", "reason", "plan-file"];

/// The accepted forms, for a refusal.
pub const FORMS: &str = "Accepted forms, each run alone as one command:\n\
POSIX shell:\n  astrolabe effort save --status active --decision \"chose X over Y: reason\" <<'ASTROLABE'\n  **Goal:** …\n  **Next:** …\n  ASTROLABE\n\
PowerShell (the here-string as the last argument, or piped in):\n  astrolabe effort save --status active @'\n  **Goal:** …\n  '@\n  or:  @'\n  **Goal:** …\n  '@ | astrolabe effort save --status active\n\
Flags take a bare word or a double-quoted value with no $, backtick or inner double quote. The body is prose; it needs no quoting.";

impl EffortCommand {
    /// Whether a command string is one of these at all, so other commands pass untouched.
    /// Any shape that names `astrolabe effort` is claimed, so one the grammar rejects is
    /// refused with the forms rather than run by whatever `astrolabe` is on PATH.
    pub fn is_effort_command(command: &str) -> bool {
        let words: Vec<&str> = command.split_whitespace().collect();
        words.windows(2).any(|w| {
            (w[0] == "astrolabe" || w[0].ends_with("/astrolabe") || w[0].ends_with("\\astrolabe"))
                && w[1] == "effort"
        })
    }

    /// Reads a command, or says why it isn't one the program accepts.
    pub fn parse(command: &str) -> Result<Self, String> {
        let text = command.replace("\r\n", "\n");
        let text = text.trim();
        let (head, body) = split_body(text)?;
        let mut words = Words::new(head);
        let program = words.next_word()?.ok_or("empty command")?;
        if program != "astrolabe" && !program.ends_with("/astrolabe") {
            return Err(format!("{program:?} isn't the astrolabe program"));
        }
        if words.next_word()?.as_deref() != Some("effort") {
            return Err("the second word must be `effort`".to_owned());
        }
        let verb = match words.next_word()?.as_deref() {
            Some(v @ ("new" | "save")) => v.to_owned(),
            Some(other) => return Err(format!("`effort {other}` isn't a verb; use new or save")),
            None => return Err("`effort` needs a verb: new or save".to_owned()),
        };
        let mut flags = Vec::new();
        while let Some(word) = words.next_word()? {
            let Some(key) = word.strip_prefix("--") else {
                return Err(format!("{word:?} isn't a flag; flags start with --"));
            };
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                return Err(format!("{word:?} isn't a flag name"));
            }
            let value = words
                .next_word()?
                .ok_or_else(|| format!("--{key} needs a value"))?;
            if value.starts_with("--") {
                return Err(format!("--{key} needs a value, not another flag"));
            }
            let allowed: &[&str] = if verb == "new" { &NEW_FLAGS } else { &SAVE_FLAGS };
            if !allowed.contains(&key) {
                let named: Vec<String> = allowed.iter().map(|f| format!("--{f}")).collect();
                return Err(format!(
                    "--{key} isn't a flag of `effort {verb}`; its flags are {}",
                    named.join(", ")
                ));
            }
            flags.push((key.to_owned(), value));
        }
        Ok(Self { verb, flags, body })
    }

    pub fn flag(&self, key: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A stable name for this exact command: what a retry would type again.
    pub fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(self.verb.as_bytes());
        for (k, v) in &self.flags {
            hash.update([0]);
            hash.update(k.as_bytes());
            hash.update([1]);
            hash.update(v.as_bytes());
        }
        hash.update([2]);
        hash.update(self.body.as_deref().unwrap_or_default().as_bytes());
        hash.finalize()
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The command line and its body apart: a POSIX heredoc `<<'ASTROLABE' … ASTROLABE`, a
/// PowerShell here-string `@' … '@`, or no body. The delimiter must be the one word, quoted
/// (either quote), alone on its closing line.
fn split_body(text: &str) -> Result<(&str, Option<String>), String> {
    let heredoc = [
        "<<'ASTROLABE'",
        "<<\"ASTROLABE\"",
        "<< 'ASTROLABE'",
        "<< \"ASTROLABE\"",
    ]
    .iter()
    .find_map(|mark| text.find(mark).map(|at| (at, mark.len())));
    if let Some((at, len)) = heredoc {
        let head = &text[..at];
        let rest = &text[at + len..];
        let (first, rest) = rest
            .split_once('\n')
            .ok_or("the heredoc has no body lines")?;
        if !first.trim().is_empty() {
            return Err(format!(
                "nothing may follow <<'{DELIMITER}' on its line, found {:?}",
                first.trim()
            ));
        }
        let (body, after) = end_of_body(rest, DELIMITER)?;
        if !after.trim().is_empty() {
            return Err(format!(
                "nothing may follow the closing {DELIMITER} line, found {:?}",
                after.trim()
            ));
        }
        return Ok((head, Some(body)));
    }
    if let Some(at) = text.find("@'") {
        let head = &text[..at];
        let rest = &text[at + 2..];
        let (first, rest) = rest
            .split_once('\n')
            .ok_or("the here-string has no body lines")?;
        if !first.trim().is_empty() {
            return Err("nothing may follow @' on its line".to_owned());
        }
        let (body, after) = end_of_body(rest, "'@")?;
        // Either the here-string is the last argument, or it's piped into the command.
        if head.trim().is_empty() {
            let piped = after.trim_start().strip_prefix('|').ok_or(
                "a here-string first must be piped into the command: '@ | astrolabe effort …",
            )?;
            return Ok((piped.trim(), Some(body)));
        }
        if !after.trim().is_empty() {
            return Err("nothing may follow the closing '@ line".to_owned());
        }
        return Ok((head, Some(body)));
    }
    if text.contains("<<") {
        return Err(format!("a heredoc here must be <<'{DELIMITER}'"));
    }
    Ok((text, None))
}

/// The lines up to the one that is exactly `end`, and what follows it.
fn end_of_body<'a>(rest: &'a str, end: &str) -> Result<(String, &'a str), String> {
    let mut lines = Vec::new();
    let mut remaining = rest;
    loop {
        let (line, after) = match remaining.split_once('\n') {
            Some(pair) => pair,
            None => (remaining, ""),
        };
        if line.trim() == end {
            return Ok((lines.join("\n").trim().to_owned(), after));
        }
        // A here-string may close and pipe on one line: `'@ | astrolabe effort …`.
        if end == "'@"
            && let Some(tail) = line.trim_start().strip_prefix("'@")
            && tail.trim_start().starts_with('|')
        {
            let body = lines.join("\n").trim().to_owned();
            let at = rest.len() - remaining.len() + (line.len() - line.trim_start().len()) + 2;
            return Ok((body, &rest[at..]));
        }
        if after.is_empty() && !remaining.contains('\n') {
            return Err(format!("the body never ends: no line with only {end}"));
        }
        lines.push(line.trim_end());
        remaining = after;
    }
}

/// Words of a command line: bare words, or double-quoted values that may hold spaces,
/// apostrophes and any letters, but no `$`, backtick, backslash or inner double quote, because
/// the shell would read those and the program must read the command the way the shell would.
struct Words<'a> {
    rest: &'a str,
}

impl<'a> Words<'a> {
    fn new(text: &'a str) -> Self {
        let text = text.trim();
        let text = text.strip_prefix("& ").unwrap_or(text);
        Self { rest: text }
    }

    fn next_word(&mut self) -> Result<Option<String>, String> {
        self.rest = self.rest.trim_start();
        if self.rest.is_empty() {
            return Ok(None);
        }
        if let Some(quoted) = self.rest.strip_prefix('"') {
            let end = quoted.find('"').ok_or("an unclosed double quote")?;
            let value = &quoted[..end];
            if let Some(c) = value.chars().find(|c| "$`\\".contains(*c)) {
                return Err(format!(
                    "{c:?} inside a quoted value: the shell would read it; leave it out"
                ));
            }
            let after = &quoted[end + 1..];
            if !after.is_empty() && !after.starts_with(char::is_whitespace) {
                return Err("a closing quote must end its word".to_owned());
            }
            self.rest = after;
            return Ok(Some(value.to_owned()));
        }
        let end = self
            .rest
            .find(char::is_whitespace)
            .unwrap_or(self.rest.len());
        let word = &self.rest[..end];
        if let Some(c) = word.chars().find(|c| "\"'$`\\;|&<>()".contains(*c)) {
            return Err(format!(
                "{c:?} in a bare word {word:?}: quote the value with double quotes, or leave the character out"
            ));
        }
        self.rest = &self.rest[end..];
        Ok(Some(word.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posix_heredoc_save_parses_whole() {
        let text = "astrolabe effort save --status active --decision \"chose X over Y: it's cheaper\" <<'ASTROLABE'\n**Goal:** one line.\n**Next:** the next step, with café and 日本.\nASTROLABE\n";
        let cmd = EffortCommand::parse(text).unwrap();
        assert_eq!(cmd.verb, "save");
        assert_eq!(cmd.flag("status"), Some("active"));
        assert_eq!(cmd.flag("decision"), Some("chose X over Y: it's cheaper"));
        assert_eq!(
            cmd.body.as_deref(),
            Some("**Goal:** one line.\n**Next:** the next step, with café and 日本.")
        );
    }

    #[test]
    fn a_powershell_here_string_and_crlf_parse_the_same() {
        let text = "& astrolabe effort new --title \"Smooth ink\" @'\r\nGoal line.\r\n'@\r\n";
        let cmd = EffortCommand::parse(text).unwrap();
        assert_eq!(cmd.verb, "new");
        assert_eq!(cmd.flag("title"), Some("Smooth ink"));
        assert_eq!(cmd.body.as_deref(), Some("Goal line."));
        let full = "/Users/x/.local/bin/astrolabe effort save";
        assert_eq!(EffortCommand::parse(full).unwrap().body, None);
        let piped = "@'\r\nFind the thing; research only.\r\n'@ | astrolabe effort new --title \"Trace input\" --authority read";
        let cmd = EffortCommand::parse(piped).unwrap();
        assert_eq!(cmd.verb, "new");
        assert_eq!(cmd.flag("authority"), Some("read"));
        assert_eq!(cmd.body.as_deref(), Some("Find the thing; research only."));
        assert!(EffortCommand::is_effort_command("@'\nx\n'@ | astrolabe effort new --title t"));
        assert!(EffortCommand::is_effort_command("echo hi | astrolabe effort save"));
    }

    #[test]
    fn what_the_shell_would_read_is_refused_with_a_reason() {
        for (text, why) in [
            ("astrolabe effort save --title \"a $b\"", "'$'"),
            ("astrolabe effort save --title it's", "'\\''"),
            ("astrolabe effort save --title \"unclosed", "unclosed"),
            ("astrolabe effort save <<EOF\nx\nEOF", "<<'ASTROLABE'"),
            ("astrolabe effort save <<'ASTROLABE'\nx\n", "never ends"),
            (
                "astrolabe effort save <<'ASTROLABE'\nx\nASTROLABE\necho hi",
                "closing",
            ),
            ("astrolabe effort drop", "verb"),
            ("astrolabe effort save --status", "needs a value"),
            (
                "astrolabe effort save --status --title x",
                "not another flag",
            ),
            ("astrolabe effort save title", "isn't a flag"),
            ("astrolabe effort save --plan x", "isn't a flag of `effort save`"),
        ] {
            let err = EffortCommand::parse(text).unwrap_err();
            assert!(err.contains(why), "{text:?}: {err}");
        }
    }

    #[test]
    fn only_effort_commands_are_claimed() {
        assert!(EffortCommand::is_effort_command("astrolabe effort save"));
        assert!(EffortCommand::is_effort_command(
            "& ~/.local/bin/astrolabe effort new"
        ));
        assert!(!EffortCommand::is_effort_command(
            "astrolabe activate --status"
        ));
        assert!(!EffortCommand::is_effort_command("git effort"));
    }

    #[test]
    fn the_digest_names_the_exact_command() {
        let a =
            EffortCommand::parse("astrolabe effort save --status done <<'ASTROLABE'\nx\nASTROLABE")
                .unwrap();
        let b = EffortCommand::parse(
            "astrolabe effort save --status done <<'ASTROLABE'\nx\nASTROLABE\n",
        )
        .unwrap();
        let c =
            EffortCommand::parse("astrolabe effort save --status done <<'ASTROLABE'\ny\nASTROLABE")
                .unwrap();
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), c.digest());
    }
}

/// An effort note as a whole: properties, title, the sections the program touches, and
/// everything else kept as it is. Only `## Current state` is replaced whole; `## Plan` is set
/// once a plan is approved; `## Decisions` only grows. The rest is the owner's or history.
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
const PROP_ORDER: [&str; 15] = [
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
        let props = crate::activate::frontmatter(text);
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
        for line in body.lines() {
            if let Some(t) = line
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

    /// The `**Next:**` line of Current state, for suggestions and the home view.
    pub fn next_line(&self) -> Option<&str> {
        self.current
            .lines()
            .find_map(|l| l.strip_prefix("**Next:**"))
            .map(str::trim)
    }

    /// Fields the checkpoint shape asks for and this Current state lacks.
    pub fn missing_fields(&self) -> Vec<&'static str> {
        ["**Goal:**", "**Next:**"]
            .into_iter()
            .filter(|f| !self.current.lines().any(|l| l.starts_with(f)))
            .map(|f| f.trim_matches('*').trim_end_matches(':'))
            .collect()
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
        if let Some(plan) = &self.plan {
            out += &format!("\n## Plan\n{}\n", plan.trim());
        }
        if !self.decisions.trim().is_empty() {
            out += &format!("\n## Decisions\n{}\n", self.decisions.trim());
        }
        for (h, body) in &self.other {
            out += &format!("\n## {h}\n{}\n", body.trim());
        }
        out
    }
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
    fn a_note_round_trips_and_only_its_own_sections_change() {
        let text = "---\nstatus: active\nrepo: demo\nnote: \"a: b\"\n---\n# 0012 Smooth ink\n\n## Current state\n**Goal:** g.\n**Next:** n.\n\n## History\nold stuff\n\n## Decisions\n- 2026-11-03: a over b: why.\n";
        let mut file = EffortFile::parse(text).unwrap();
        assert_eq!(file.number(), Some(12));
        assert_eq!(file.name(), "Smooth ink");
        assert_eq!(file.next_line(), Some("n."));
        assert_eq!(file.prop("note"), Some("a: b"));
        assert_eq!(
            file.other,
            vec![("History".to_owned(), "old stuff".to_owned())]
        );
        file.current = "**Goal:** g2.".to_owned();
        file.set("status", "done");
        let out = file.render();
        assert!(out.contains("status: done\n"));
        assert!(out.contains("## Current state\n**Goal:** g2.\n"));
        assert!(out.contains("## History\nold stuff\n"));
        assert!(out.contains("- 2026-11-03: a over b: why.\n"));
        file.add_decision("2026-11-04: c over d: why.");
        assert!(file.render().contains("- 2026-11-03: a over b: why.\n- 2026-11-04: c over d: why.\n"));
        assert_eq!(
            EffortFile::parse(&out).unwrap().missing_fields(),
            vec!["Next"]
        );
        assert_eq!(
            slug("Smooth ink: across frames, v2!"),
            "smooth-ink-across-frames-v2"
        );
    }
}
