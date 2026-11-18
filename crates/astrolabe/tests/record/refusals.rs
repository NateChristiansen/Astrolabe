//! A refused effort command names its real cause first and offers the one line to run
//! instead. Each case is a family from the audit's refusal ledger (2026-11-18), rewritten
//! with neutral paths and text; before, every one of them printed the full list of forms,
//! and the chained ones named whatever the strict parser met first.
use super::*;

/// The reply's two lines, after checking nothing was written and the full list wasn't shown.
fn refused(mac: &Machine, command: &str) -> (String, String) {
    let before = mac.log();
    let (out, ok) = mac.bash("s1", command);
    assert!(!ok, "{out}");
    assert_eq!(mac.log(), before, "a refusal writes nothing");
    assert!(!out.contains("Accepted forms"), "{out}");
    let mut lines = out.lines();
    let first = lines.next().unwrap_or_default().to_owned();
    let second = lines.next().unwrap_or_default().to_owned();
    assert!(lines.all(|l| l.trim().is_empty()), "two lines: {out}");
    (first, second)
}

/// A machine with effort 1 bound to session `s1`; the world goes when the test ends.
fn bound() -> (World, Machine) {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    (world, mac)
}

/// Catches: a save chained after `cd` was refused as "\"cd\" isn't the astrolabe program"
/// with the forms, which read as a typo in the program's name.
#[test]
fn a_save_chained_after_cd_names_the_chaining() {
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
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
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
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
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
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

/// Catches: `| head` after a read was refused as "'|' in a bare word", as if a value needed
/// quoting.
#[test]
fn a_read_piped_into_another_program_names_the_pipe() {
    let (_world, mac) = bound();
    let (first, second) = refused(&mac, "astrolabe effort read 1 | head -30");
    assert_eq!(
        first,
        "Effort command refused: its output is piped into `head`, and the hook takes an effort command only when it runs alone, with nothing after it."
    );
    assert_eq!(second, "Run instead: astrolabe effort read 1");
}

/// Catches: `2>&1` and `> /dev/null` refused as "'>' in a bare word".
#[test]
fn a_redirect_is_named_as_a_redirect() {
    let (_world, mac) = bound();
    for (command, named) in [
        ("astrolabe effort read 1 2>&1", "`2>&1`"),
        ("astrolabe effort read 1 > /dev/null", "`> /dev/null`"),
        (
            "astrolabe journal \"Fixed the dead link in the guide.\" 2>&1",
            "`2>&1`",
        ),
    ] {
        let (first, _) = refused(&mac, command);
        assert_eq!(
            first,
            format!(
                "Effort command refused: it redirects its output ({named}), and the hook takes an effort command only when it runs alone, with no redirect."
            ),
            "{command}"
        );
    }
    let (_, second) = refused(
        &mac,
        "astrolabe journal \"Fixed the dead link in the guide.\" 2>&1",
    );
    assert_eq!(
        second,
        "Run instead: astrolabe journal \"Fixed the dead link in the guide.\""
    );
}

/// Catches: a redirect and pipe on the heredoc's own line were refused as "nothing may
/// follow <<'ASTROLABE' on its line".
#[test]
fn a_pipe_after_the_heredoc_marker_names_the_pipe() {
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
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
    let (_world, mac) = bound();
    let (first, _) = refused(
        &mac,
        "astrolabe effort save --status active <<'ASTROLABE'\nWhere it stands.\nASTROLABE\ngit -C /work/repo branch --show-current",
    );
    assert_eq!(
        first,
        "Effort command refused: `git` runs after it in the same call, and the hook takes an effort command only when it runs alone."
    );
}

/// The heredoc message stays for the one case it is true: the save's own body opened with
/// another delimiter.
#[test]
fn a_body_with_another_delimiter_is_told_the_one_delimiter() {
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
        "astrolabe effort save --status active <<EOF\nWhere it stands.\nEOF\n",
    );
    assert_eq!(
        first,
        "Effort command refused: its body must open with <<'ASTROLABE', quoted, and end at a line holding only ASTROLABE."
    );
    assert_eq!(
        second,
        "Run instead, with the same body and its closing ASTROLABE line: astrolabe effort save --status active <<'ASTROLABE'"
    );
}

/// Catches: `$`, a backtick or a backslash in a quoted value; the line offered leaves it out.
#[test]
fn a_value_the_shell_would_read_is_offered_without_it() {
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
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

/// Catches: a flag the verb lacks; the line offered drops it, and a new effort's goal goes
/// in its body.
#[test]
fn a_flag_the_verb_lacks_is_dropped_from_the_line_offered() {
    let (_world, mac) = bound();
    let (first, second) = refused(
        &mac,
        "astrolabe effort new --title \"Faster ink\" --goal \"strokes keep up\"",
    );
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
    let (_world, mac) = bound();
    let (first, second) = refused(&mac, "astrolabe effort save --tick \"--once flag added\"");
    assert_eq!(
        first,
        "Effort command refused: --tick needs a value, not another flag."
    );
    assert!(
        second.starts_with("The form: astrolabe effort save [--status <state>]"),
        "{second}"
    );
}

/// Catches: `--help` was refused as a malformed command (F-2). It's answered with the forms,
/// and the shell sees it worked.
#[test]
fn help_is_answered_with_the_forms() {
    let (_world, mac) = bound();
    for command in [
        "astrolabe effort new --help 2>&1 | head -30",
        "astrolabe effort --help",
        "astrolabe effort save --help",
    ] {
        let (out, ok) = mac.bash("s1", command);
        assert!(ok, "{command}: {out}");
        assert!(
            out.starts_with("Effort commands have no --help; this is it.\nAccepted forms"),
            "{out}"
        );
    }
}

/// PowerShell gets its own body marker and closing line, and its read fails with `exit 1`.
#[test]
fn powershell_is_offered_a_here_string() {
    let (_world, mac) = bound();
    let command =
        "cd C:\\work\\repo; astrolabe effort save --status active @'\nWhere it stands.\n'@";
    let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": "PowerShell", "tool_use_id": "call-ps", "tool_input": {"command": command}});
    let (out, ok) = mac.shell(input, &mac.demo, command);
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

/// Catches effort 401's exit-status defect: the app ran `cat` of a refusal, so the shell said
/// a refused save had worked. The read the hook hands back now fails when the save did.
#[cfg(unix)]
#[test]
fn a_refused_save_fails_in_the_shell_and_an_accepted_one_succeeds() {
    let (_world, mac) = bound();
    let run = |command: &str| {
        let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": "Bash", "tool_use_id": format!("call-{}", command.len()), "tool_input": {"command": command}});
        let (out, ok) = mac.run(
            &mac.demo,
            &["hook", "pre-command", "--if-active"],
            &input.to_string(),
        );
        assert!(ok, "{out}");
        let answer: Value = serde_json::from_str(&out).unwrap();
        let shown = answer["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        Command::new("sh").arg("-c").arg(&shown).output().unwrap()
    };
    let refused = run("cd /work && astrolabe effort save --status active");
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).starts_with("Effort command refused: "));
    let saved =
        run("astrolabe effort save --decision \"chose a test over none: it shows the status\"");
    assert!(saved.status.success());
    assert!(String::from_utf8_lossy(&saved.stdout).starts_with("Saved effort 1"));
}

/// Catches the friction log's 12:41 case: a commit that failed after the write was planned
/// said "Effort NOT saved" though the next notes write finished it. It now says the write
/// is kept, and running it again shows it landed.
#[cfg(unix)]
#[test]
fn a_kept_write_says_not_saved_yet_and_the_retry_shows_it_landed() {
    let (_world, mac) = bound();
    let once = mac.notes.join(".git/fail-once");
    write(&once, "");
    let hook = mac.notes.join(".git/hooks/pre-commit");
    write(
        &hook,
        &format!(
            "#!/bin/sh\nif [ -e '{}' ]; then rm '{}'; echo \"fatal: Unable to create '.git/index.lock': File exists.\" >&2; exit 1; fi\n",
            once.display(),
            once.display()
        ),
    );
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let save = "astrolabe effort save --status ready <<'ASTROLABE'\nReady once the hook lets it through.\n\nNothing chosen.\n\nNothing next.\nASTROLABE\n";
    let (out, ok) = mac.bash("s1", save);
    assert!(!ok, "{out}");
    assert!(
        out.starts_with("Effort 1 not saved yet: fatal: Unable to create '.git/index.lock': File exists. The write is kept"),
        "{out}"
    );
    assert!(!out.contains("NOT saved"), "{out}");
    let (out, ok) = mac.bash("s1", save);
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("Ready once the hook lets it through.")
    );
}

/// Catches a push claimed with no remote: every write's reply says what happened locally
/// and never that a sync or push started.
#[test]
fn no_write_claims_a_sync_with_no_remote() {
    let (_world, mac) = bound();
    git(&mac.notes, &["remote", "remove", "origin"]);
    for command in [
        "astrolabe effort save --status ready",
        "astrolabe effort save --decision \"chose local notes over none: they still save\"",
        "astrolabe effort save --journal \"Kept the notes local for the test.\"",
    ] {
        let (out, ok) = mac.bash("s1", command);
        assert!(ok, "{command}: {out}");
        assert!(
            !out.contains("sync started") && !out.contains("push"),
            "{command}: {out}"
        );
    }
}
