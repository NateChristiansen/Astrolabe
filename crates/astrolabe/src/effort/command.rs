//! Shell command grammar; parsing performs no filesystem writes or subprocess work.

/// The flags each verb takes; any other is refused, so a mistyped one can't be ignored.
pub const NEW_FLAGS: [&str; 7] = [
    "title",
    "repo",
    "status",
    "involvement",
    "authority",
    "initiative",
    "priority",
];
pub const SAVE_FLAGS: [&str; 9] = [
    "status",
    "decision",
    "waits-for",
    "reason",
    "plan-file",
    "journal",
    "tick",
    "untick",
    "pieces-file",
];

/// The one delimiter, quoted so the shell expands nothing in the body.
pub const DELIMITER: &str = "ASTROLABE";

/// What an `astrolabe effort …` or `astrolabe journal …` command says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortCommand {
    /// `new`, `save` or `journal`.
    pub verb: String,
    /// `--key value` pairs, in order, keys without the dashes.
    pub flags: Vec<(String, String)>,
    /// The heredoc or here-string body, trimmed, with `\n` line ends. None when none was given.
    pub body: Option<String>,
}

impl EffortCommand {
    /// Whether a command string is one of these at all, so other commands pass untouched.
    /// A command is claimed when `astrolabe` (bare or a path) starts one of its simple
    /// commands and `effort` or `journal` follows, wherever that command sits in a chain, so
    /// one the grammar rejects is refused with the forms rather than run by whatever
    /// `astrolabe` is on PATH. Text the shell wouldn't run (quoted words, heredoc and
    /// here-string bodies, comments) is never claimed, so a prompt or a commit message that
    /// names the command passes through.
    pub fn is_effort_command(command: &str) -> bool {
        simple_commands(command).iter().any(|words| {
            // Assignments and the words that only introduce a command come before the program.
            const LEAD: [&str; 10] = [
                "!", "{", "if", "elif", "then", "while", "until", "do", "else", "time",
            ];
            let mut words = words
                .iter()
                .skip_while(|w| LEAD.contains(&w.as_str()) || is_assignment(w));
            let program = words.next().map(String::as_str).unwrap_or_default();
            let named = program == "astrolabe"
                || program.ends_with("/astrolabe")
                || program.ends_with("\\astrolabe");
            named && matches!(words.next().map(String::as_str), Some("effort" | "journal"))
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
        let verb = match words.next_word()?.as_deref() {
            Some("effort") => match words.next_word()?.as_deref() {
                Some(v @ ("new" | "save")) => v.to_owned(),
                Some(other) => {
                    return Err(format!("`effort {other}` isn't a verb; use new or save"));
                }
                None => return Err("`effort` needs a verb: new or save".to_owned()),
            },
            Some("journal") => {
                // The sentence as one quoted word, or in the body; never both. From the
                // notes checkout, --repo <name> says which repo the finding is about.
                let mut body = body;
                let mut flags = Vec::new();
                while let Some(word) = words.next_word()? {
                    if let Some(key) = word.strip_prefix("--") {
                        if key != "repo" {
                            return Err(format!(
                                "--{key} isn't a flag of `journal`; it takes only --repo <name>, with the sentence quoted or in a body"
                            ));
                        }
                        let value = words
                            .next_word()?
                            .filter(|v| !v.starts_with("--"))
                            .ok_or("--repo needs a value")?;
                        flags.push(("repo".to_owned(), value));
                        continue;
                    }
                    if body.is_some() {
                        return Err(
                            "`journal` takes the sentence once, quoted whole, or in a body, not both"
                                .to_owned(),
                        );
                    }
                    body = Some(word);
                }
                let text = body.as_deref().map(str::trim).unwrap_or_default();
                if text.is_empty() {
                    return Err("`journal` needs the sentence".to_owned());
                }
                if text.lines().count() > 1 {
                    return Err("a journal line is one line".to_owned());
                }
                return Ok(Self {
                    verb: "journal".to_owned(),
                    flags,
                    body: Some(text.to_owned()),
                });
            }
            _ => return Err("the second word must be `effort` or `journal`".to_owned()),
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
            let allowed: &[&str] = if verb == "new" {
                &NEW_FLAGS
            } else {
                &SAVE_FLAGS
            };
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

    /// Every value of a flag given more than once, in order: `--tick a --tick b`.
    pub fn flags_all(&self, key: &str) -> Vec<&str> {
        self.flags
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
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

/// The accepted forms, for a refusal.
pub const FORMS: &str = "Accepted forms, each run alone as one command:\n\
POSIX shell:\n  astrolabe effort save --status active --decision \"chose X over Y: reason\" <<'ASTROLABE'\n  <where it stands>\n\n  <what was decided and why>\n\n  <what's next; what isn't checked>\n  ASTROLABE\n\
PowerShell (the here-string as the last argument, or piped in):\n  astrolabe effort save --status active @'\n  <the same three paragraphs>\n  '@\n  or:  @'\n  …\n  '@ | astrolabe effort save --status active\n\
The approved plan, on a save:  --plan-file <path> (the app's plan file), or in the body below the paragraphs after a line `## Plan`.\n\
A journal line with no effort:  astrolabe journal \"Found the Deck preset in the video settings: everything Low, render scale 0.75.\"\n\
Flags take a bare word or a double-quoted value with no $, backtick or inner double quote. For --tick, an unambiguous prefix of the piece is accepted, for example --tick \"First repo note\" for a piece containing inline code. The body is plain paragraphs, no **Label:** lines; it needs no quoting.";

/// `NAME=value` before a program, as in `FOO=1 astrolabe effort save`.
pub(super) fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The words of each simple command in a command string, read roughly as a POSIX shell or
/// PowerShell would: quotes group a word and hide what's inside them; a heredoc body, a
/// PowerShell here-string body and a `#` comment are skipped; `|`, `;`, `&`, newline, `(`,
/// `)` and backtick outside quotes end a command. It's only for deciding what to claim:
/// `parse` reads the claimed command strictly.
pub(super) fn simple_commands(text: &str) -> Vec<Vec<String>> {
    let text = text.replace("\r\n", "\n");
    let chars: Vec<char> = text.chars().collect();
    let mut commands = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    // A word that was quoted, even empty (`''`), is still a word.
    let mut started = false;
    // Heredoc delimiters opened on this line, skipped in order once it ends.
    let mut heredocs: Vec<(String, bool)> = Vec::new();
    let mut i = 0;

    fn end_word(words: &mut Vec<String>, word: &mut String, started: &mut bool) {
        if *started || !word.is_empty() {
            words.push(std::mem::take(word));
        }
        *started = false;
    }
    fn end_command(commands: &mut Vec<Vec<String>>, words: &mut Vec<String>) {
        if !words.is_empty() {
            commands.push(std::mem::take(words));
        }
    }
    // The index just past the line that starts at `from`, or the end.
    fn next_line(chars: &[char], from: usize) -> usize {
        chars[from..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(chars.len(), |p| from + p + 1)
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '"' => {
                started = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            }
            '\\' if chars.get(i + 1) == Some(&'\n') => i += 2,
            '\\' => {
                // Kept as written, so a Windows path keeps its separators.
                word.push(c);
                if let Some(&next) = chars.get(i + 1) {
                    word.push(next);
                }
                i += 2;
            }
            '#' if !started && word.is_empty() => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '@' if !started
                && word.is_empty()
                && matches!(chars.get(i + 1), Some('\'' | '"'))
                && chars[i + 2..]
                    .iter()
                    .take_while(|&&c| c != '\n')
                    .all(|c| c.is_whitespace()) =>
            {
                // A PowerShell here-string: its body runs to a line starting with the
                // closing quote and `@`, and it's one word.
                let close = [chars[i + 1], '@'];
                let mut at = next_line(&chars, i);
                loop {
                    if at >= chars.len() {
                        i = at;
                        break;
                    }
                    if chars[at..].starts_with(&close) {
                        i = at + 2;
                        break;
                    }
                    at = next_line(&chars, at);
                }
                started = true;
            }
            '<' if chars.get(i + 1) == Some(&'<') && chars.get(i + 2) != Some(&'<') => {
                end_word(&mut words, &mut word, &mut started);
                i += 2;
                let strip_tabs = chars.get(i) == Some(&'-');
                if strip_tabs {
                    i += 1;
                }
                while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
                    i += 1;
                }
                let mut delimiter = String::new();
                while i < chars.len() && !" \t\n;|&<>()".contains(chars[i]) {
                    if chars[i] != '\'' && chars[i] != '"' && chars[i] != '\\' {
                        delimiter.push(chars[i]);
                    }
                    i += 1;
                }
                if !delimiter.is_empty() {
                    heredocs.push((delimiter, strip_tabs));
                }
            }
            '\n' => {
                end_word(&mut words, &mut word, &mut started);
                end_command(&mut commands, &mut words);
                i += 1;
                for (delimiter, strip_tabs) in std::mem::take(&mut heredocs) {
                    while i < chars.len() {
                        let end = next_line(&chars, i);
                        let line: String = chars[i..end].iter().collect();
                        i = end;
                        let line = line.trim_end_matches('\n');
                        let line = if strip_tabs {
                            line.trim_start_matches('\t')
                        } else {
                            line
                        };
                        if line == delimiter {
                            break;
                        }
                    }
                }
            }
            '|' | ';' | '&' | '(' | ')' | '`' => {
                end_word(&mut words, &mut word, &mut started);
                end_command(&mut commands, &mut words);
                i += 1;
            }
            c if c.is_whitespace() => {
                end_word(&mut words, &mut word, &mut started);
                i += 1;
            }
            _ => {
                word.push(c);
                i += 1;
            }
        }
    }
    end_word(&mut words, &mut word, &mut started);
    end_command(&mut commands, &mut words);
    commands
}

/// The command line and its body apart: a POSIX heredoc `<<'ASTROLABE' … ASTROLABE`, a
/// PowerShell here-string `@' … '@`, or no body. The delimiter must be the one word, quoted
/// (either quote), alone on its closing line.
pub(super) fn split_body(text: &str) -> Result<(&str, Option<String>), String> {
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
pub(super) fn end_of_body<'a>(rest: &'a str, end: &str) -> Result<(String, &'a str), String> {
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
pub(super) struct Words<'a> {
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
        assert!(EffortCommand::is_effort_command(
            "@'\nx\n'@ | astrolabe effort new --title t"
        ));
        assert!(EffortCommand::is_effort_command(
            "echo hi | astrolabe effort save"
        ));
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
            (
                "astrolabe effort save --plan x",
                "isn't a flag of `effort save`",
            ),
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
    fn a_command_that_only_names_it_in_text_is_not_claimed() {
        for text in [
            "echo \"astrolabe effort save\"",
            "echo 'run astrolabe journal \"x\" alone'",
            "git commit -m \"Say astrolabe effort save in the guide\"",
            "codex exec 'When done, run astrolabe effort save --status done' < /dev/null",
            "cat > note.md <<'EOF'\nastrolabe effort save --status done\nEOF",
            "cat <<-EOF\n\tastrolabe journal \"x\"\n\tEOF\necho done",
            "grep -n \"astrolabe effort\" README.md # astrolabe effort save",
            "# astrolabe effort save\nls",
            "Write-Output @'\nastrolabe effort new --title t\n'@",
            "rg 'astrolabe journal' docs | head",
            "echo see \\\n  astrolabe effort save",
        ] {
            assert!(!EffortCommand::is_effort_command(text), "{text:?}");
        }
    }

    #[test]
    fn a_chained_or_nested_command_is_still_claimed() {
        for text in [
            "cd notes && astrolabe effort save",
            "ls; astrolabe journal \"x\"",
            "true || astrolabe effort new --title t",
            "echo $(astrolabe effort save)",
            "echo `astrolabe effort save`",
            "(astrolabe effort save)",
            "ls\nastrolabe effort save",
            "FOO=1 astrolabe effort save",
            "if astrolabe journal \"x\"; then echo ok; fi",
            "cat <<'EOF'\nbody\nEOF\nastrolabe effort save",
            "@'\nGoal.\n'@ | astrolabe effort new --title t",
            "& 'C:\\Users\\x\\.local\\bin\\astrolabe' effort save",
            "astrolabe effort save --title \"unclosed",
        ] {
            assert!(EffortCommand::is_effort_command(text), "{text:?}");
        }
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
