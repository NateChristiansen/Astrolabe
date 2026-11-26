//! What a refused effort command says: the real cause first, then the one line that would be
//! accepted in its place. Most refusals were a save run with other commands, piped or
//! redirected, and naming whatever the strict parser met first sent agents to fix the wrong
//! thing (audit, 2026-11-18). The full list of forms is printed only when help was asked for.

use super::command::{
    DELIMITER, EffortCommand, FORMS, Simple, astrolabe_args, is_assignment, is_astrolabe,
    split_commands,
};

/// The reply to a command the grammar refused, and whether it answered what was asked: only
/// a request for help does.
pub fn refusal(command: &str, powershell: bool, why: &str) -> (String, bool) {
    let cmds = split_commands(command);
    let claimed = |c: &Simple| {
        astrolabe_args(&c.words).is_some_and(|args| {
            matches!(
                args.first().map(String::as_str),
                Some("effort" | "journal" | "initiative")
            )
        })
    };
    let asks_help = (cmds.iter().filter_map(|c| astrolabe_args(&c.words)))
        .any(|args| args.iter().any(|w| w == "--help" || w == "-h"));
    if asks_help && cmds.iter().any(claimed) {
        return (
            format!("Effort commands have no --help; this is it.\n{FORMS}"),
            true,
        );
    }
    let refused =
        |cause: &str, line: String| (format!("Effort command refused: {cause}.\n{line}"), false);
    let Some(at) = cmds.iter().position(claimed) else {
        return (format!("Effort command refused: {why}."), false);
    };
    let me = &cmds[at];
    // A here-string piped in is this command's body, not another command before it.
    let piped_body = at > 0 && {
        let p = &cmds[at - 1];
        p.body && p.words.len() == 1 && p.ended_by.as_deref() == Some("|")
    };
    let start = if piped_body { at - 1 } else { at };
    let (kept, mut redirects) = without_redirects(&me.words);
    // `2>&1` reads as a word `2>` ended by `&`, then a command `1`.
    let mut tail = at + 1;
    if me.ended_by.as_deref() == Some("&")
        && me.words.last().is_some_and(|w| w.ends_with('>'))
        && let Some(next) = cmds.get(at + 1)
        && next.words.len() == 1
        && next.words[0]
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-')
    {
        if let Some(last) = redirects.last_mut() {
            *last = format!("{last}&{}", next.words[0]);
        }
        tail += 1;
    }
    let (before, after) = (&cmds[..start], cmds.get(tail..).unwrap_or_default());
    let cause = if let Some(first) = before.first() {
        format!(
            "it runs after {} in the same call, and the hook takes an effort command only when it runs alone: run the other commands in their own call first",
            program(first)
        )
    } else if cmds[tail - 1].ended_by.as_deref() == Some("|")
        && let Some(next) = after.first()
    {
        format!(
            "its output is piped into {}, and the hook takes an effort command only when it runs alone, with nothing after it",
            program(next)
        )
    } else if let Some(next) = after.first() {
        format!(
            "{} runs after it in the same call, and the hook takes an effort command only when it runs alone",
            program(next)
        )
    } else if let Some(r) = redirects.first() {
        format!(
            "it redirects its output (`{r}`), and the hook takes an effort command only when it runs alone, with no redirect"
        )
    } else if why.starts_with("a heredoc here must be") {
        format!(
            "its body must open with <<'{DELIMITER}', quoted, and end at a line holding only {DELIMITER}"
        )
    } else {
        why.to_owned()
    };
    let args = astrolabe_args(&kept).unwrap_or_default();
    let had_body = me.body || piped_body;
    // A read takes no body; a new effort or initiative needs one.
    let body = match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (_, Some("read")) => false,
        (Some("initiative"), _) | (_, Some("new")) => true,
        _ => had_body,
    };
    let vars: Vec<(&str, &str)> = (before.iter().flat_map(|c| &c.words))
        .filter(|w| is_assignment(w))
        .filter_map(|w| w.split_once('='))
        .collect();
    match corrected(args, &vars, body, powershell) {
        Some(line) => {
            let close = if powershell { "'@" } else { DELIMITER };
            let with = match (body, had_body) {
                (false, _) => String::new(),
                (true, true) => format!(", with the same body and its closing {close} line"),
                (true, false) => {
                    format!(", with the body on the lines after it and a closing {close} line")
                }
            };
            refused(&cause, format!("Run instead{with}: {line}"))
        }
        None => refused(&cause, format!("The form: {}", form(args))),
    }
}

/// The one form for a verb, for a refusal with no line to correct: shell-neutral, because
/// the replies that use it don't know the shell.
pub fn form(args: &[String]) -> String {
    let words: Vec<&str> = args.iter().take(2).map(String::as_str).collect();
    let body = |head: &str| {
        format!(
            "{head} <<'{DELIMITER}' (in PowerShell @'), then the body, then a line holding only {DELIMITER} ('@)"
        )
    };
    match words.as_slice() {
        ["effort", "new", ..] => body("astrolabe effort new --title \"<title>\""),
        ["effort", "read", ..] => "astrolabe effort read <n>".to_owned(),
        ["journal", ..] => "astrolabe journal \"<one sentence>\"".to_owned(),
        ["initiative", ..] => body("astrolabe initiative new --title \"<name>\""),
        _ => {
            body("astrolabe effort save [--status <state>] [--decision \"chose X over Y: reason\"]")
        }
    }
}

/// The words with redirections taken out (`> f`, `2>/dev/null`, `<`), and those apart.
fn without_redirects(words: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut kept, mut redirects) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        let op = word.trim_start_matches(|c: char| c.is_ascii_digit());
        if i > 0 && (op.starts_with('>') || op.starts_with('<')) {
            if op.trim_start_matches(['>', '<']).is_empty() && i + 1 < words.len() {
                redirects.push(format!("{word} {}", words[i + 1]));
                i += 2;
            } else {
                redirects.push(word.clone());
                i += 1;
            }
            continue;
        }
        // A here-string reads as one empty word: the body, not a value.
        if !word.is_empty() {
            kept.push(word.clone());
        }
        i += 1;
    }
    (kept, redirects)
}

/// How a refusal names another simple command: its program, in backticks.
fn program(command: &Simple) -> String {
    let words = &command.words;
    if astrolabe_args(words).is_some() {
        return "another astrolabe command".to_owned();
    }
    match words.iter().find(|w| !is_assignment(w)) {
        Some(w) if !is_astrolabe(w) => format!("`{w}`"),
        _ => format!(
            "setting `{}`",
            words[0].split_once('=').map_or("", |(name, _)| name)
        ),
    }
}

/// The command as it would be accepted: variables set earlier in the call filled in, the
/// characters the shell would read left out of values, a flag the verb lacks dropped, and the
/// body opened with the one delimiter. Only a line the grammar accepts is offered.
fn corrected(
    args: &[String],
    vars: &[(&str, &str)],
    body: bool,
    powershell: bool,
) -> Option<String> {
    let mut words: Vec<String> = args.iter().map(|a| expand(a, vars)).collect();
    for i in 0..words.len() {
        let after = if i > 0 {
            words[i - 1].clone()
        } else {
            String::new()
        };
        let value = &mut words[i];
        if value.starts_with("--") {
            continue;
        }
        let read: &[char] = &['$', '`', '"', '\\'];
        if matches!(after.as_str(), "--tick" | "--untick") {
            // A tick takes a prefix of its piece, so it stops before what the shell would
            // read; taking those characters out would match no piece.
            if let Some(at) = value.find(read) {
                value.truncate(at);
                value.truncate(value.trim_end_matches([' ', ':', ';', ',', '.', '-']).len());
            }
        } else if matches!(after.as_str(), "--plan-file" | "--pieces-file") {
            value.retain(|c| !read[..3].contains(&c));
        } else {
            value.retain(|c| !read.contains(&c));
        }
    }
    for _ in 0..4 {
        let line = render(&words, body, powershell);
        let whole = match (body, powershell) {
            (false, _) => line.clone(),
            (true, false) => format!("{line}\nx\n{DELIMITER}"),
            (true, true) => format!("{line}\nx\n'@"),
        };
        match EffortCommand::parse_in(&whole, powershell) {
            Ok(_) => return Some(line),
            Err(why) => {
                let flag = why.strip_prefix("--")?.split_once(" isn't a flag of")?.0;
                let at = words.iter().position(|w| *w == format!("--{flag}"))?;
                let with_value = words.get(at + 1).is_some_and(|v| !v.starts_with("--"));
                words.drain(at..at + 1 + usize::from(with_value));
            }
        }
    }
    None
}

/// `$NAME` and `${NAME}` in a word, from assignments earlier in the same call.
fn expand(word: &str, vars: &[(&str, &str)]) -> String {
    let mut out = word.to_owned();
    // Longest names first, so `$PATH2` isn't read as `$PATH` and a 2.
    let mut vars = vars.to_vec();
    vars.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, value) in vars {
        out = out.replace(&format!("${{{name}}}"), value);
        out = out.replace(&format!("${name}"), value);
    }
    out
}

fn render(words: &[String], body: bool, powershell: bool) -> String {
    let bare = |w: &str| {
        !w.is_empty()
            && w.chars()
                .all(|c| c.is_alphanumeric() || "-_./:@%+=,~".contains(c))
    };
    let mut out = vec!["astrolabe".to_owned()];
    for (i, w) in words.iter().enumerate() {
        let file = i > 0 && matches!(words[i - 1].as_str(), "--plan-file" | "--pieces-file");
        out.push(if bare(w) {
            w.clone()
        } else if file && !w.contains('\'') {
            format!("'{w}'")
        } else {
            format!("\"{w}\"")
        });
    }
    let mut line = out.join(" ");
    if body {
        line += if powershell { " @'" } else { " <<'ASTROLABE'" };
    }
    line
}

/// The refusal families from the audit's ledger (2026-11-18), with neutral paths and text.
/// The record tests keep one of them through the hook, to check it reaches this reply.
#[cfg(test)]
mod tests {
    use super::*;

    /// The reply's two lines for a command the grammar refuses, after checking it isn't
    /// answered as help and doesn't print the full list of forms.
    fn refused(command: &str) -> (String, String) {
        let why = EffortCommand::parse_in(command, false).expect_err(command);
        let (out, ok) = refusal(command, false, &why);
        assert!(!ok, "{out}");
        assert!(!out.contains("Accepted forms"), "{out}");
        let mut lines = out.lines();
        let first = lines.next().unwrap_or_default().to_owned();
        let second = lines.next().unwrap_or_default().to_owned();
        assert!(lines.all(|l| l.trim().is_empty()), "two lines: {out}");
        (first, second)
    }

    /// Catches: a save chained after `cd` was refused as "\"cd\" isn't the astrolabe program"
    /// with the forms, which read as a typo in the program's name.
    #[test]
    fn a_save_chained_after_cd_names_the_chaining() {
        let (first, second) = refused(
            "cd /work/repo && astrolabe effort save --status active <<'ASTROLABE'\nWhere it stands.\nASTROLABE\n",
        );
        assert_eq!(
            first,
            "Effort command refused: it runs after `cd` in the same call, and the hook takes an effort command only when it runs alone: run the other commands in their own call first."
        );
        assert_eq!(
            second,
            "Run instead, with the same body and its closing ASTROLABE line: astrolabe effort save --status active <<'ASTROLABE'"
        );
    }

    /// Catches the friction log's 22:06 case and the ledger's largest family: a pieces file
    /// written by a heredoc, then the save, in one call, refused as "a heredoc here must be
    /// <<'ASTROLABE'" when the cause was the chaining.
    #[test]
    fn a_save_after_writing_its_pieces_file_names_the_chaining_not_the_heredoc() {
        let (first, second) = refused(
            "cat > /scratch/pieces.md <<'EOF'\n- [ ] One outcome\nEOF\nastrolabe effort save --pieces-file /scratch/pieces.md <<'ASTROLABE'\nWhere it stands.\nASTROLABE\n",
        );
        assert!(
            first.starts_with("Effort command refused: it runs after `cat` in the same call"),
            "{first}"
        );
        assert!(!first.contains("heredoc"), "{first}");
        assert_eq!(
            second,
            "Run instead, with the same body and its closing ASTROLABE line: astrolabe effort save --pieces-file /scratch/pieces.md <<'ASTROLABE'"
        );
    }

    /// Catches: a path held in a shell variable set earlier in the call was refused for its
    /// `;` or `$`; the line offered has the path filled in, since the hook never sees it.
    #[test]
    fn a_variable_set_earlier_in_the_call_is_filled_in_on_the_line_offered() {
        let (first, second) = refused(
            "S=/scratch/run; cat > $S/pieces.md <<'EOF'\n- [ ] One outcome\nEOF\nastrolabe effort save --pieces-file $S/pieces.md",
        );
        assert!(
            first.starts_with("Effort command refused: it runs after setting `S` in the same call"),
            "{first}"
        );
        assert_eq!(
            second,
            "Run instead: astrolabe effort save --pieces-file /scratch/run/pieces.md"
        );
    }

    /// Catches: `| head` after a read was refused as "'|' in a bare word", as if a value
    /// needed quoting.
    #[test]
    fn a_read_piped_into_another_program_names_the_pipe() {
        let (first, second) = refused("astrolabe effort read 1 | head -30");
        assert_eq!(
            first,
            "Effort command refused: its output is piped into `head`, and the hook takes an effort command only when it runs alone, with nothing after it."
        );
        assert_eq!(second, "Run instead: astrolabe effort read 1");
    }

    /// Catches: `2>&1` and `> /dev/null` refused as "'>' in a bare word".
    #[test]
    fn a_redirect_is_named_as_a_redirect() {
        for (command, named) in [
            ("astrolabe effort read 1 2>&1", "`2>&1`"),
            ("astrolabe effort read 1 > /dev/null", "`> /dev/null`"),
            (
                "astrolabe journal \"Fixed the dead link in the guide.\" 2>&1",
                "`2>&1`",
            ),
        ] {
            let (first, _) = refused(command);
            assert_eq!(
                first,
                format!(
                    "Effort command refused: it redirects its output ({named}), and the hook takes an effort command only when it runs alone, with no redirect."
                ),
                "{command}"
            );
        }
        let (_, second) = refused("astrolabe journal \"Fixed the dead link in the guide.\" 2>&1");
        assert_eq!(
            second,
            "Run instead: astrolabe journal \"Fixed the dead link in the guide.\""
        );
    }

    /// Catches: a redirect and pipe on the heredoc's own line were refused as "nothing may
    /// follow <<'ASTROLABE' on its line".
    #[test]
    fn a_pipe_after_the_heredoc_marker_names_the_pipe() {
        let (first, second) = refused(
            "astrolabe effort save --status review <<'ASTROLABE' 2>&1 | tail -3\nWhere it stands.\nASTROLABE\n",
        );
        assert!(
            first.starts_with("Effort command refused: its output is piped into `tail`"),
            "{first}"
        );
        assert_eq!(
            second,
            "Run instead, with the same body and its closing ASTROLABE line: astrolabe effort save --status review <<'ASTROLABE'"
        );
    }

    /// Catches: a command after the closing line was refused as "nothing may follow the
    /// closing ASTROLABE line", which hid that it was a second command.
    #[test]
    fn a_command_after_the_body_names_that_command() {
        let (first, _) = refused(
            "astrolabe effort save --status active <<'ASTROLABE'\nWhere it stands.\nASTROLABE\ngit -C /work/repo branch --show-current",
        );
        assert_eq!(
            first,
            "Effort command refused: `git` runs after it in the same call, and the hook takes an effort command only when it runs alone."
        );
    }

    /// The heredoc message stays for the one case it is true: the save's own body opened
    /// with another delimiter.
    #[test]
    fn a_body_with_another_delimiter_is_told_the_one_delimiter() {
        let (first, second) =
            refused("astrolabe effort save --status active <<EOF\nWhere it stands.\nEOF\n");
        assert_eq!(
            first,
            "Effort command refused: its body must open with <<'ASTROLABE', quoted, and end at a line holding only ASTROLABE."
        );
        assert_eq!(
            second,
            "Run instead, with the same body and its closing ASTROLABE line: astrolabe effort save --status active <<'ASTROLABE'"
        );
    }

    /// Catches: `$`, a backtick or a backslash in a quoted value; the line offered leaves it
    /// out.
    #[test]
    fn a_value_the_shell_would_read_is_offered_without_it() {
        let (first, second) = refused(
            "astrolabe effort save --decision \"chose a \\$95 stop line over none: it bounds the run\"",
        );
        assert!(
            first.starts_with("Effort command refused: '\\\\' inside a quoted value"),
            "{first}"
        );
        assert_eq!(
            second,
            "Run instead: astrolabe effort save --decision \"chose a 95 stop line over none: it bounds the run\""
        );
    }

    /// Catches: a flag the verb lacks; the line offered drops it, and a new effort's goal
    /// goes in its body.
    #[test]
    fn a_flag_the_verb_lacks_is_dropped_from_the_line_offered() {
        let (first, second) =
            refused("astrolabe effort new --title \"Faster ink\" --goal \"strokes keep up\"");
        assert!(
            first.starts_with("Effort command refused: --goal isn't a flag of `effort new`"),
            "{first}"
        );
        assert_eq!(
            second,
            "Run instead, with the body on the lines after it and a closing ASTROLABE line: astrolabe effort new --title \"Faster ink\" <<'ASTROLABE'"
        );
    }

    /// Where no line can be offered, the one form for the verb is, not the list.
    #[test]
    fn with_no_line_to_offer_the_verbs_one_form_is_shown() {
        let (first, second) = refused("astrolabe effort save --tick \"--once flag added\"");
        assert_eq!(
            first,
            "Effort command refused: --tick needs a value, not another flag."
        );
        assert!(
            second.starts_with("The form: astrolabe effort save [--status <state>]"),
            "{second}"
        );
    }

    /// Catches: `--help` was refused as a malformed command (F-2). It's answered with the
    /// forms, as an answer.
    #[test]
    fn help_is_answered_with_the_forms() {
        for command in [
            "astrolabe effort new --help 2>&1 | head -30",
            "astrolabe effort --help",
            "astrolabe effort save --help",
        ] {
            let why = EffortCommand::parse_in(command, false).expect_err(command);
            let (out, ok) = refusal(command, false, &why);
            assert!(ok, "{command}: {out}");
            assert!(
                out.starts_with("Effort commands have no --help; this is it.\nAccepted forms"),
                "{out}"
            );
        }
    }

    /// PowerShell is offered its own body marker and closing line.
    #[test]
    fn powershell_is_offered_a_here_string() {
        let command =
            "cd C:\\work\\repo; astrolabe effort save --status active @'\nWhere it stands.\n'@";
        let why = EffortCommand::parse_in(command, true).expect_err(command);
        let (out, ok) = refusal(command, true, &why);
        assert!(!ok, "{out}");
        let mut lines = out.lines();
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("Effort command refused: it runs after `cd` in the same call"),
            "{out}"
        );
        assert_eq!(
            lines.next().unwrap(),
            "Run instead, with the same body and its closing '@ line: astrolabe effort save --status active @'"
        );
    }
}
