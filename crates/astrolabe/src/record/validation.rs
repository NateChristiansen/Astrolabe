//! Pure validation of saved prose, domain values and plan sections.
use crate::effort::EffortFile;

pub(super) const STATUSES: [&str; 7] = [
    "proposed",
    "ready",
    "active",
    "review",
    "blocked",
    "done",
    "abandoned",
];

pub(super) use crate::effort::{AUTHORITIES, INVOLVEMENTS};

/// Current state is three short paragraphs the owner reads; past this it's a report.
pub(super) const CURRENT_WORDS: usize = 150;

/// A journal line is one or two sentences, aimed at 25 words; past this it rambles.
pub(super) const JOURNAL_WORDS: usize = 40;
pub(super) fn one_of(key: &str, value: &str, allowed: &[&str]) -> Result<String, String> {
    allowed
        .contains(&value)
        .then(|| value.to_owned())
        .ok_or_else(|| format!("--{key} {value:?} isn't one of {}", allowed.join(", ")))
}

/// The body as a Current state: plain paragraphs, trimmed. A `**Label:**` line is refused,
/// because the labelled form read as a form and pushed the text into fragments; a long one
/// is refused because the owner reads it.
pub(super) fn as_current(body: &str) -> Result<String, String> {
    let current = body.trim().to_owned();
    let probe = EffortFile {
        current: current.clone(),
        ..Default::default()
    };
    if let Some(line) = probe.labelled_line() {
        return Err(format!(
            "Current state is plain paragraphs now, no **Label:** lines ({line}): where it stands, then what was decided and why, then what's next and what isn't checked. Nothing was written"
        ));
    }
    let words = current.split_whitespace().count();
    if words > CURRENT_WORDS {
        return Err(format!(
            "Current state is {words} words; the owner reads it, so keep it under {CURRENT_WORDS} (three short paragraphs). Nothing was written"
        ));
    }
    Ok(current)
}

/// Closing words that say where the work stands: the board shows that.
pub(super) const STATUS_WORDS: [&str; 9] = [
    "merged",
    "landed",
    "done",
    "pushed",
    "shipped",
    "blocked",
    "abandoned",
    "merging",
    "unmerged",
];

/// A journal line the owner would write: one or two sentences on the change itself. The
/// refusals are the shapes sessions wrote with the rule alone (2026-11-05: nine of seventeen
/// lines); each says what to write instead, as the label-line refusal on Current state does.
pub(super) fn check_journal(text: &str) -> Result<(), String> {
    if text.lines().count() > 1 {
        return Err("a journal line is one line".to_owned());
    }
    let words = text.split_whitespace().count();
    if words > JOURNAL_WORDS {
        return Err(format!(
            "the journal line is {words} words; aim for 25, at most {JOURNAL_WORDS}: what changed, as the owner would say it. Nothing was written"
        ));
    }
    let sentences = journal_sentences(text);
    if sentences.len() > 2 {
        return Err(format!(
            "the journal line is {} sentences; two at most, the change and its why. Nothing was written",
            sentences.len()
        ));
    }
    if let Some(last) = sentences.last() {
        let closing: Vec<String> = last
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_ascii_lowercase()
            })
            .filter(|w| !w.is_empty())
            .collect();
        if !closing.is_empty()
            && closing.len() <= 3
            && closing
                .iter()
                .all(|w| w == "and" || STATUS_WORDS.contains(&w.as_str()))
        {
            return Err(format!(
                "the line ends with \"{}\", which says where the work stands; that's the effort's status, already on the board, so drop it and keep the change. Nothing was written",
                last.trim()
            ));
        }
    }
    if let Some(file) = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && !"/.~".contains(c)))
        .find(|w| looks_like_a_file(w))
    {
        return Err(format!(
            "the line names a file ({file}); the owner reads things, not paths, so say what it is. Nothing was written"
        ));
    }
    if text.to_ascii_lowercase().contains("the owner") {
        return Err(
            "the line says \"the owner\"; the journal is in their words, so say I or my. Nothing was written"
                .to_owned(),
        );
    }
    Ok(())
}

/// Sentences end at a full stop, question or exclamation mark before a space and a capital,
/// so "Astra. viral-studio" and "about 1.5 s" stay one sentence.
pub(super) fn journal_sentences(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..b.len() {
        if matches!(b[i], b'.' | b'!' | b'?')
            && b.get(i + 1) == Some(&b' ')
            && b.get(i + 2).is_some_and(u8::is_ascii_uppercase)
        {
            out.push(&text[start..=i]);
            start = i + 2;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// A word that is a path or a file name: two slashes, a slash with a dot or a root beside
/// it, or a code or document extension. "and/or" and "Mac/PC" are words.
pub(super) fn looks_like_a_file(word: &str) -> bool {
    const EXTENSIONS: [&str; 22] = [
        "md", "rs", "cs", "gd", "js", "mjs", "ts", "json", "toml", "yaml", "yml", "py", "sh",
        "ps1", "txt", "html", "css", "tscn", "tres", "cfg", "ini", "lock",
    ];
    if word.matches('/').count() > 1
        || (word.contains('/') && (word.contains('.') || word.starts_with(['/', '~', '.'])))
    {
        return true;
    }
    word.rsplit_once('.').is_some_and(|(name, ext)| {
        name.chars().last().is_some_and(char::is_alphanumeric) && EXTENSIONS.contains(&ext)
    })
}

/// Plan headings shifted together so the top level lands at `###`: the effort file's
/// sections are read by `## `, and a plan file's own `# ` title would read as a second
/// document. Lines inside a code fence, line endings and indentation stay as they are.
pub(super) fn shift_plan_headings(plan: &str) -> (String, bool) {
    let level = |l: &str| {
        let n = l.bytes().take_while(|b| *b == b'#').count();
        let rest = l[n..].trim_end_matches('\r');
        (n > 0 && (rest.is_empty() || rest.starts_with(' '))).then_some(n)
    };
    // Each line with its heading level, or none inside a code fence.
    let fenced = crate::effort::fence_mask(plan.split('\n'));
    let lines: Vec<(&str, Option<usize>)> = plan
        .split('\n')
        .zip(fenced)
        .map(|(l, in_fence)| (l, (!in_fence).then(|| level(l)).flatten()))
        .collect();
    let Some(min) = lines.iter().filter_map(|(_, n)| *n).min() else {
        return (plan.to_owned(), false);
    };
    if min == 3 {
        return (plan.to_owned(), false);
    }
    let out = lines
        .iter()
        .map(|(l, n)| match n {
            Some(n) => format!("{}{}", "#".repeat(n + 3 - min), &l[*n..]),
            None => (*l).to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    (out, true)
}

/// The body apart from a plan it carries after a `## Plan` line: Codex keeps no plan file,
/// so the plan rides in the one heredoc, below the Current state fields.
pub(super) fn split_plan(body: Option<&str>) -> (Option<String>, Option<String>, bool) {
    let Some(body) = body else {
        return (None, None, false);
    };
    let mut lines = body.lines();
    let current: Vec<&str> = lines
        .by_ref()
        .take_while(|l| l.trim() != "## Plan")
        .collect();
    let plan: Vec<&str> = lines.collect();
    let part = |lines: &[&str]| {
        let text = lines.join("\n");
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    };
    let marked = body.lines().any(|l| l.trim() == "## Plan");
    let plan = marked.then(|| part(&plan)).flatten();
    (part(&current), plan, marked)
}

#[cfg(test)]
mod plan_heading_tests {
    use super::shift_plan_headings;

    #[test]
    fn headings_shift_together_and_fences_and_blank_ones_are_handled() {
        assert_eq!(
            shift_plan_headings("# T\n\n## A\n### B\n#not\n"),
            ("### T\n\n#### A\n##### B\n#not\n".to_owned(), true)
        );
        assert_eq!(
            shift_plan_headings("#### Only\n##### deep"),
            ("### Only\n#### deep".to_owned(), true)
        );
        assert_eq!(
            shift_plan_headings("### fine\n```sh\n# comment\n```\n#### sub"),
            (
                "### fine\n```sh\n# comment\n```\n#### sub".to_owned(),
                false
            )
        );
        assert_eq!(
            shift_plan_headings("## A\n~~~\n# in fence\n~~~\n"),
            ("### A\n~~~\n# in fence\n~~~\n".to_owned(), true)
        );
        // A longer fence isn't closed by a shorter one, nor by the other character.
        assert_eq!(
            shift_plan_headings("## A\n````md\n```\n# still in\n~~~\n# still in\n````\n# out"),
            (
                "#### A\n````md\n```\n# still in\n~~~\n# still in\n````\n### out".to_owned(),
                true
            )
        );
        assert_eq!(
            shift_plan_headings("#\r\n#### Steps\r\n"),
            ("###\r\n###### Steps\r\n".to_owned(), true)
        );
        assert_eq!(
            shift_plan_headings("no headings"),
            ("no headings".to_owned(), false)
        );
        assert_eq!(shift_plan_headings(""), (String::new(), false));
    }
}

/// `--issue 5` or `--issue "5, 7"`: the GitHub issues this effort answers, kept as `5, 7`.
pub(super) fn issues(value: &str) -> Result<String, String> {
    let mut numbers = Vec::new();
    for part in value.split(',') {
        let n = part.trim().trim_start_matches('#');
        if n.is_empty()
            || !n.bytes().all(|b| b.is_ascii_digit())
            || n.trim_start_matches('0').is_empty()
        {
            return Err(format!(
                "--issue {value:?} isn't a list of issue numbers, as in `--issue 5` or `--issue \"5, 7\"`"
            ));
        }
        numbers.push(n.trim_start_matches('0').to_owned());
    }
    Ok(numbers.join(", "))
}

/// GitHub's closing keywords: one of these, an optional colon, then `#N`, closes issue N
/// when the commit reaches the default branch.
const CLOSING: [&str; 9] = [
    "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved",
];

/// Whether a commit message closes issue `n` in its own repo.
pub(crate) fn closes(message: &str, n: &str) -> bool {
    let text = message.to_ascii_lowercase();
    let mark = format!("#{n}");
    let mut from = 0;
    while let Some(at) = text[from..].find(&mark).map(|i| i + from) {
        from = at + mark.len();
        if text[from..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
            continue;
        }
        let before = text[..at].trim_end();
        let before = before.strip_suffix(':').unwrap_or(before).trim_end();
        let hit = CLOSING.iter().any(|word| {
            before.strip_suffix(word).is_some_and(|rest| {
                rest.is_empty() || !rest.ends_with(|c: char| c.is_alphanumeric() || c == '_')
            })
        });
        if hit {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> String {
        vec!["word"; n].join(" ")
    }

    fn refusal(result: Result<impl std::fmt::Debug, String>) -> String {
        result.expect_err("refused")
    }

    #[test]
    fn current_state_is_short_plain_paragraphs() {
        let why = refusal(as_current(&words(151)));
        assert!(
            why.contains("151 words") && why.contains("under 150"),
            "{why}"
        );
        assert!(why.contains("Nothing was written"), "{why}");
        assert_eq!(as_current(&words(150)).unwrap(), words(150));
        assert_eq!(
            as_current("  where it stands\n\n").unwrap(),
            "where it stands"
        );
        let why = refusal(as_current("**Status:** active"));
        assert!(why.contains("no **Label:** lines"), "{why}");
        assert!(why.contains("Nothing was written"), "{why}");
    }

    #[test]
    fn journal_lines_refuse_the_shapes_sessions_wrote() {
        let why = refusal(check_journal(&words(41)));
        assert!(
            why.contains("41 words") && why.contains("aim for 25"),
            "{why}"
        );
        assert!(check_journal(&words(40)).is_ok());
        assert!(refusal(check_journal("two\nlines")).contains("one line"));
        for (line, why) in [
            (
                "Landed the ink smoothing helper. Merged.",
                "where the work stands",
            ),
            (
                "Strokes stop splitting across frames. Dropped the stand-in. Merged and pushed.",
                "3 sentences",
            ),
            (
                "Chat replies drift without voice.md at start.",
                "names a file (voice.md)",
            ),
            (
                "Replies lose the owner's voice without the voice note.",
                "the owner",
            ),
            (
                "Moved the hook script out of src/hooks/old and it fires again.",
                "names a file",
            ),
        ] {
            let out = refusal(check_journal(line));
            assert!(
                out.contains(why) && out.contains("Nothing was written"),
                "{line}: {out}"
            );
        }
        // A name after a full stop, a decimal, a slash between words and a status word inside
        // a sentence are not those shapes.
        for line in [
            "Audited the notes with Astra: five wrong facts gone. viral-studio is cloned and registered.",
            "Cut the boot to about 1.5 s. Dropped the stand-in and send-ahead.",
            "Deleted 22 landed branches on the Mac/PC pair, keeping the old Windows branch with an unmerged commit.",
        ] {
            assert_eq!(check_journal(line), Ok(()), "{line}");
        }
    }

    #[test]
    fn sentences_end_before_a_capital_and_files_need_a_path_or_extension() {
        assert_eq!(
            journal_sentences("Cut it to 1.5 s. Then more! Why? done"),
            vec!["Cut it to 1.5 s.", "Then more!", "Why? done"]
        );
        assert_eq!(
            journal_sentences("Astra. viral-studio stays"),
            vec!["Astra. viral-studio stays"]
        );
        assert_eq!(journal_sentences("Ends. "), vec!["Ends. "]);
        assert!(journal_sentences("").is_empty());
        for word in [
            "voice.md",
            "src/hooks/old",
            "/etc",
            "~/x",
            "./run",
            "a/b.c",
            "Cargo.lock",
        ] {
            assert!(looks_like_a_file(word), "{word}");
        }
        for word in ["and/or", "Mac/PC", "1.5", "e.g", "md", ".md", "done."] {
            assert!(!looks_like_a_file(word), "{word}");
        }
    }

    #[test]
    fn issue_lists_are_numbers() {
        assert_eq!(issues("5").unwrap(), "5");
        assert_eq!(issues("#3, 07").unwrap(), "3, 7");
        for bad in ["", "five", "5,", "0", "5 7", "-1"] {
            assert!(issues(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn closing_keywords_match_github_forms_only() {
        for yes in [
            "Fixes #5",
            "fix #5",
            "CLOSES: #5",
            "body\n\nResolved #5.",
            "Fixes #5\nFixes #6",
        ] {
            assert!(closes(yes, "5"), "{yes}");
        }
        for no in [
            "See #5",
            "Fixes #55",
            "prefixes #5",
            "Fixes #5a",
            "Fixes 5",
            "Refs #5, fixes #6",
        ] {
            assert!(!closes(no, "5"), "{no}");
        }
    }
}

#[cfg(test)]
mod example_tests {
    use super::*;

    #[test]
    fn the_session_texts_example_is_a_current_state_the_save_accepts() {
        let example = crate::activate::SHAPE
            .split("Like this, at done:\n")
            .nth(1)
            .unwrap();
        let body: Vec<&str> = example
            .lines()
            .map(|l| l.strip_prefix("  ").unwrap_or(l))
            .collect();
        let body = body.join("\n");
        assert_eq!(body.split("\n\n").count(), 3, "{body}");
        as_current(&body).unwrap();
    }
}
