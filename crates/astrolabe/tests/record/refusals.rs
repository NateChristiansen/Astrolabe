//! A refused effort command names its real cause first and offers the one line to run
//! instead. The families from the audit's refusal ledger (2026-11-18) are unit tests on
//! `effort::refusal`; these check one of each kind reaches the reply through the hook:
//! refused with nothing written, help answered as worked, and PowerShell's read.
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

/// PowerShell gets its own body marker and closing line. Its read stays a plain read that
/// succeeds: Claude's read-only check doesn't pass `exit 1`, and a read stopped there would
/// hide the reply (review of effort 401).
#[test]
fn powershell_is_offered_a_here_string() {
    let (_world, mac) = bound();
    let command =
        "cd C:\\work\\repo; astrolabe effort save --status active @'\nWhere it stands.\n'@";
    let input = json!({"session_id": "s1", "cwd": mac.demo, "tool_name": "PowerShell", "tool_use_id": "call-ps", "tool_input": {"command": command}});
    let (out, ok) = mac.shell(input.clone(), &mac.demo, command, &[]);
    assert!(!ok, "{out}");
    let (answer, _) = mac.run(
        &mac.demo,
        &["hook", "pre-command", "--if-active"],
        &input.to_string(),
    );
    let answer: Value = serde_json::from_str(&answer).unwrap();
    let read = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    assert!(
        read.starts_with("Get-Content -Raw -Encoding UTF8 -LiteralPath '") && read.ends_with('\''),
        "{read}"
    );
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

/// The same for a new effort, an initiative and a standalone journal line: each said "NOT
/// created/filed/written" for a write that then landed (the lock fix's report). Each retry is
/// matched to the kept write, so it lands once.
#[cfg(unix)]
#[test]
fn a_kept_new_effort_initiative_or_journal_line_says_not_done_yet() {
    let world = World::new();
    let mac = world.machine("mac");
    let once = mac.notes.join(".git/fail-once");
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
    for (session, command, said) in [
        (
            "s2",
            NEW,
            "Effort 1 not created yet: fatal: Unable to create",
        ),
        (
            "s3",
            "astrolabe initiative new --title \"Ink work\" <<'ASTROLABE'\nMake the ink feel right.\nASTROLABE\n",
            "Initiative ink-work not filed yet: fatal: Unable to create",
        ),
        (
            "s4",
            "astrolabe journal \"Found the ink preset in the settings.\"",
            " not written yet: fatal: Unable to create",
        ),
    ] {
        write(&once, "");
        let before = mac.log().lines().count();
        let (out, ok) = mac.bash(session, command);
        assert!(!ok, "{out}");
        assert!(out.starts_with(said) || out.contains(said), "{out}");
        assert!(
            out.contains("The write is kept") && !out.contains(" NOT "),
            "{out}"
        );
        let (out, ok) = mac.bash(session, command);
        assert!(ok, "{out}");
        assert_eq!(
            mac.log().lines().count(),
            before + 1,
            "it landed once: {out}"
        );
    }
    assert_eq!(
        mac.journal_today().matches("Found the ink preset").count(),
        1
    );
}

/// A new effort refused for what its load would refuse: nothing filed, nothing committed.
fn refused_filing(mac: &Machine, command: &str) -> String {
    let before = mac.log();
    let (out, ok) = mac.bash("s1", command);
    assert!(!ok, "{out}");
    assert!(out.starts_with("Effort NOT created: "), "{out}");
    assert_eq!(mac.log(), before, "a refused filing commits nothing: {out}");
    assert!(
        astrolabe::notes::effort_files(&mac.notes).is_empty(),
        "a refused filing writes no note: {out}"
    );
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), "", "{out}");
    out
}

/// Catches effort 494: a body with its own Current state heading printed "Effort NOT created"
/// yet committed a note with the section twice, which every later read refused. The heading
/// opening the body is taken as the section, once.
#[test]
fn a_body_opening_with_its_current_state_heading_is_that_section() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Smooth ink\" <<'ASTROLABE'\n## Current state\nSmooth the ink across frames.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let text = mac.effort_text(1);
    assert_eq!(text.matches("## Current state").count(), 1, "{text}");
    assert!(
        text.contains("## Current state\nSmooth the ink across frames."),
        "{text}"
    );
    let (out, ok) = mac.bash("s1", "astrolabe effort read 1");
    assert!(ok, "{out}");
    assert!(out.contains("Smooth the ink across frames."), "{out}");
}

/// The same fault from a heading further in: a body that would give the note a section twice
/// is refused, and the refusal writes and commits nothing.
#[test]
fn a_body_that_would_hold_a_section_twice_is_refused_and_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    for (body, said) in [
        (
            "## Current state\nSmooth the ink.\n\n## Current state\nAgain.",
            "that section twice",
        ),
        (
            "Smooth the ink.\n\n## Current state\nAgain.",
            "that section twice",
        ),
        (
            "Smooth the ink.\n\n## Plan\n1. Cap it.\n\n## Plan\n1. Cap it again.",
            "`## Plan` twice",
        ),
    ] {
        let out = refused_filing(
            &mac,
            &format!(
                "astrolabe effort new --title \"Smooth ink\" <<'ASTROLABE'\n{body}\nASTROLABE\n"
            ),
        );
        assert!(out.contains(said), "{out}");
    }
    // A heading inside a code fence is text, as the note's reader takes it.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort new --title \"Smooth ink\" <<'ASTROLABE'\nSmooth the ink.\n\n```\n## Current state\n```\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort read 1");
    assert!(ok, "{out}");
}

/// Catches effort 494's other fault: the checks of the load a filing ends in ran on the note
/// after its commit, so a refusal there left the effort filed. A notes checkout missing a note
/// every load reads is refused before anything is written.
#[test]
fn a_filing_its_load_would_refuse_writes_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    git(&mac.notes, &["rm", "-q", "knowledge/machines.md"]);
    git(&mac.notes, &["commit", "-q", "-m", "drop machines"]);
    let out = refused_filing(&mac, NEW);
    assert!(out.contains("knowledge/machines.md"), "{out}");
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
