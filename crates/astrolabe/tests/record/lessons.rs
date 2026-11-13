use super::*;

const CLOSE: &str = "astrolabe lesson close lessons/friction.md --as \"Fixed the workflow because the hook can own it.\"";

fn seed_lesson(mac: &Machine) {
    write(
        &mac.notes.join("lessons/friction.md"),
        "# Friction\n\nIt cost a turn.\n",
    );
    git(&mac.notes, &["add", "lessons/friction.md"]);
    git(&mac.notes, &["commit", "-qm", "Add lesson"]);
}

#[test]
fn closing_records_the_disposition_then_removes_only_the_lesson_and_refreshes_the_board() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    write(&mac.notes.join("drafts/post.md"), "staged draft\n");
    git(&mac.notes, &["add", "drafts/post.md"]);
    let index = git(&mac.notes, &["diff", "--cached"]);
    let before = mac.log().lines().count();
    assert_eq!(astrolabe::record::lessons_in(&mac.notes).len(), 1);
    let (out, ok) = mac.bash("s", CLOSE);
    assert!(
        ok && out.contains("disposition committed, file removed; background sync started"),
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before + 2);
    assert!(!mac.notes.join("lessons/friction.md").exists());
    let first = git(&mac.notes, &["show", "HEAD~:lessons/friction.md"]);
    assert_eq!(
        first.lines().last(),
        Some("Fixed the workflow because the hook can own it.")
    );
    assert_eq!(first.matches("Fixed the workflow").count(), 1);
    for revision in ["HEAD", "HEAD~"] {
        let files = git(&mac.notes, &["show", "--name-only", "--format=", revision]);
        assert_eq!(files.trim(), "lessons/friction.md");
        assert!(git(&mac.notes, &["log", "-1", "--format=%s", revision]).contains("friction.md"));
    }
    assert_eq!(git(&mac.notes, &["diff", "--cached"]), index);
    assert!(mac.sync_requested());
    assert!(astrolabe::record::lessons_in(&mac.notes).is_empty());
    let page = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    assert!(
        !page.contains("friction.md"),
        "the regenerated board no longer lists it"
    );
    // Lost-reply retry and the plain result reader make no additional commits.
    assert!(mac.bash("s", CLOSE).1);
    assert!(mac.plain(CLOSE).1);
    assert_eq!(mac.log().lines().count(), before + 2);
}

#[test]
fn refused_closes_leave_the_notes_index_history_and_intents_untouched() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    write(&mac.notes.join("lessons/plain.txt"), "plain\n");
    let before = mac.log();
    let status = git(&mac.notes, &["status", "--porcelain"]);
    let original = std::fs::read(mac.notes.join("lessons/friction.md")).unwrap();
    for command in [
        "astrolabe lesson close lessons/missing.md --as done",
        "astrolabe lesson close knowledge/owner.md --as done",
        "astrolabe lesson close lessons/../knowledge/owner.md --as done",
        "astrolabe lesson close lessons/plain.txt --as done",
        "astrolabe lesson close lessons/friction.md --as \"   \"",
        "astrolabe lesson close lessons/friction.md --as \"a\nb\"",
        "astrolabe lesson close lessons/friction.md --as done && echo after",
        "echo before; astrolabe lesson close lessons/friction.md --as done",
    ] {
        let (out, ok) = mac.bash("s", command);
        assert!(!ok, "{command}: {out}");
        assert_eq!(mac.log(), before);
        assert_eq!(git(&mac.notes, &["status", "--porcelain"]), status);
        assert_eq!(
            std::fs::read(mac.notes.join("lessons/friction.md")).unwrap(),
            original
        );
        assert!(!mac.state().join("ops").exists());
    }
    let (out, ok) = mac.run(
        &mac.demo,
        &["lesson", "close", "lessons/friction.md", "--as", "done"],
        "",
    );
    assert!(!ok && out.contains("Nothing closed"), "{out}");
    assert_eq!(mac.log(), before);
}

#[test]
fn an_interrupted_close_finishes_on_a_new_call_without_repeating_the_disposition() {
    for file_still_present in [false, true] {
        let world = World::new();
        let mac = world.machine("mac");
        seed_lesson(&mac);
        let before = mac.log().lines().count();
        // Git refuses the removal commit, after the disposition commit has succeeded.
        let hook = mac.notes.join(".git/hooks/pre-commit");
        write(&hook, "#!/bin/sh\ntest -f lessons/friction.md\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let (out, ok) = mac.bash("s", CLOSE);
        assert!(!ok, "{out}");
        assert_eq!(mac.log().lines().count(), before + 1);
        let disposition = git(&mac.notes, &["show", "HEAD:lessons/friction.md"]);
        assert_eq!(disposition.matches("Fixed the workflow").count(), 1);
        std::fs::remove_file(hook).unwrap();
        if file_still_present {
            // Also cover interruption immediately after the first commit, before removal.
            write(&mac.notes.join("lessons/friction.md"), &disposition);
        }
        // A fresh tool call, not merely a retry carrying the old call's key.
        let input = json!({"session_id":"s", "cwd":mac.demo, "tool_name":"Bash", "tool_use_id":"retry-call", "tool_input":{"command":CLOSE}});
        let answer = mac.hook_json("pre-command", input).unwrap();
        let print = answer["hookSpecificOutput"]["updatedInput"]["command"]
            .as_str()
            .unwrap();
        let text = std::fs::read_to_string(
            print
                .strip_prefix("cat '")
                .unwrap()
                .strip_suffix('\'')
                .unwrap(),
        )
        .unwrap();
        assert!(text.contains("Closed lesson"), "{text}");
        assert_eq!(mac.log().lines().count(), before + 2);
        assert!(!mac.notes.join("lessons/friction.md").exists());
        assert_eq!(
            git(&mac.notes, &["show", "HEAD~:lessons/friction.md"]),
            disposition
        );
        assert!(
            mac.sync().1,
            "ordinary reconciliation also skips completed phases"
        );
        assert_eq!(mac.log().lines().count(), before + 2);
    }
}

#[test]
fn housekeeping_routes_both_shell_forms_to_read_only_replies() {
    for tool in ["Bash", "PowerShell"] {
        let world = World::new();
        let mac = world.machine("mac");
        seed_lesson(&mac);
        for command in [CLOSE.to_owned(), "astrolabe notes sync".to_owned()] {
            let command = if tool == "PowerShell" {
                format!("& {command}")
            } else {
                command
            };
            let answer = mac.hook_json("pre-command", json!({"session_id":"s", "cwd":mac.demo, "tool_name":tool, "tool_input":{"command":command}})).unwrap();
            let hso = &answer["hookSpecificOutput"];
            assert_eq!(hso["permissionDecision"], "allow", "{answer}");
            let print = hso["updatedInput"]["command"].as_str().unwrap();
            let prefix = if tool == "PowerShell" {
                "Get-Content -Raw -Encoding UTF8 -LiteralPath '"
            } else {
                "cat '"
            };
            let path = print
                .strip_prefix(prefix)
                .unwrap()
                .strip_suffix('\'')
                .unwrap();
            let ticket: Value = serde_json::from_str(
                &std::fs::read_to_string(path.replace(".txt", ".json")).unwrap(),
            )
            .unwrap();
            assert_eq!(ticket["ok"], true, "{ticket}");
            if command.contains("notes sync") {
                assert!(
                    ticket["text"]
                        .as_str()
                        .unwrap()
                        .contains("Background sync started"),
                    "{ticket}"
                );
                assert!(
                    ticket["text"]
                        .as_str()
                        .unwrap()
                        .contains("astrolabe notes status"),
                    "{ticket}"
                );
                assert!(mac.sync_requested());
                assert_ne!(
                    git(&mac.notes, &["rev-parse", "HEAD"]),
                    git(&mac.notes, &["rev-parse", "@{upstream}"]),
                    "the hook did not wait for a push"
                );
            } else {
                assert!(mac.sync_requested());
            }
        }
        assert!(!mac.notes.join("lessons/friction.md").exists());
        assert!(git(&mac.notes, &["status", "--porcelain"]).is_empty());
    }
}

#[test]
fn a_hook_sync_finishes_local_recovery_without_fetching_or_pushing() {
    let world = World::new();
    let mac = world.machine("mac");
    let before_remote = git(&mac.notes, &["rev-parse", "@{upstream}"]);
    let file = shelf("0001-recovered.md");
    let intent = json!({
        "key": "sync-recovery", "files": [[file, "# Recovered\n"]],
        "renames": [], "journal": [], "message": "Recover local write", "base": null,
        "states": [], "committed": false
    });
    write(
        &mac.state().join("ops/sync-recovery.json"),
        &intent.to_string(),
    );
    // A fetch here would fail and leave an offline record. The hook only launches it.
    git(
        &mac.notes,
        &[
            "remote",
            "set-url",
            "origin",
            mac.dir.0.join("missing.git").to_str().unwrap(),
        ],
    );
    let (out, ok) = mac.bash("s", "astrolabe notes sync");
    assert!(ok && out.contains("Background sync started"), "{out}");
    assert!(out.contains("astrolabe notes status"), "{out}");
    assert!(mac.notes.join(file).exists());
    assert!(mac.log().starts_with("Recover local write\n"));
    assert!(mac.sync_requested());
    assert_eq!(
        git(&mac.notes, &["rev-parse", "@{upstream}"]),
        before_remote
    );
    let record = std::fs::read_to_string(mac.state().join("sync.json")).unwrap_or_default();
    assert!(
        !record.contains("offline"),
        "the hook must not attempt the fetch: {record}"
    );
    assert!(!mac.notes.join(".git/FETCH_HEAD").exists());
}

#[test]
fn hook_sync_reports_no_remote_launch_failure_and_failed_recovery_honestly() {
    for outcome in ["no remote", "launch failed", "recovery failed"] {
        let world = World::new();
        let mac = world.machine("mac");
        match outcome {
            "no remote" => {
                git(&mac.notes, &["remote", "remove", "origin"]);
            }
            "launch failed" => {
                std::fs::create_dir(mac.state().join("sync-requested")).unwrap();
            }
            _ => {
                write(&mac.state().join("ops/broken.json"), "{");
            }
        }
        let before = mac.log();
        let (out, ok) = mac.bash("s", "astrolabe notes sync");
        assert!(out.contains("astrolabe notes status"), "{out}");
        assert!(!out.contains("sync started"), "{out}");
        match outcome {
            "no remote" => assert!(ok && out.contains("no remote configured"), "{out}"),
            "launch failed" => assert!(!ok && out.contains("sync could not start"), "{out}"),
            _ => assert!(
                !ok && out.contains("Unfinished writes failed again"),
                "{out}"
            ),
        }
        assert_eq!(mac.log(), before);
    }
}

#[test]
fn a_close_pulls_the_other_machines_lesson_changes_before_committing() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    git(&mac.notes, &["push", "-q", "origin", "main"]);
    let pc = world.machine("pc");
    let refreshed =
        "# Friction\n\nIt cost a turn.\nThe other machine added the missing evidence.\n";
    write(&pc.notes.join("lessons/friction.md"), refreshed);
    git(
        &pc.notes,
        &["commit", "-qam", "Add evidence from the other machine"],
    );
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    let published = git(&pc.notes, &["rev-parse", "HEAD"]);
    let (out, ok) = mac.bash("s", CLOSE);
    assert!(ok, "{out}");
    assert_eq!(git(&mac.notes, &["rev-parse", "HEAD~2"]), published);
    assert!(git(&mac.notes, &["show", "HEAD~:lessons/friction.md"]).starts_with(refreshed));
    let (out, ok) = mac.sync();
    assert!(ok, "the close remains publishable: {out}");
    assert_eq!(
        git(&mac.notes, &["rev-parse", "HEAD"]),
        git(&mac.notes, &["rev-parse", "@{upstream}"])
    );
}

#[test]
fn refused_closes_do_not_pull_a_new_remote_commit() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    git(&mac.notes, &["push", "-q", "origin", "main"]);
    let pc = world.machine("pc");
    write(
        &pc.notes.join("knowledge/machines.md"),
        "New machine fact\n",
    );
    git(&pc.notes, &["commit", "-qam", "Update the machine fact"]);
    git(&pc.notes, &["push", "-q", "origin", "main"]);
    let before = git(&mac.notes, &["rev-parse", "HEAD"]);
    for command in [
        "astrolabe lesson close lessons/missing.md --as done",
        "astrolabe lesson close knowledge/owner.md --as done",
        "astrolabe lesson close lessons/friction.md --as \" \"",
    ] {
        let (out, ok) = mac.bash("s", command);
        assert!(!ok, "{out}");
        assert_eq!(git(&mac.notes, &["rev-parse", "HEAD"]), before);
        assert_eq!(git(&mac.notes, &["rev-parse", "@{upstream}"]), before);
        assert!(!mac.notes.join(".git/FETCH_HEAD").exists());
    }
}

#[test]
fn closing_reports_a_failed_pull_while_preserving_local_success() {
    for failure in ["offline", "diverged"] {
        let world = World::new();
        let mac = world.machine("mac");
        seed_lesson(&mac);
        git(&mac.notes, &["push", "-q", "origin", "main"]);
        if failure == "offline" {
            git(
                &mac.notes,
                &[
                    "remote",
                    "set-url",
                    "origin",
                    mac.dir.0.join("missing.git").to_str().unwrap(),
                ],
            );
        } else {
            let pc = world.machine("pc");
            write(
                &pc.notes.join("knowledge/owner.md"),
                "The other machine's change\n",
            );
            git(
                &pc.notes,
                &["commit", "-qam", "Change on the other machine"],
            );
            git(&pc.notes, &["push", "-q", "origin", "main"]);
            write(
                &mac.notes.join("knowledge/owner.md"),
                "This machine's change\n",
            );
            git(&mac.notes, &["commit", "-qam", "Conflicting local change"]);
        }
        let before = mac.log().lines().count();
        let (out, ok) = mac.bash("s", CLOSE);
        assert!(ok && out.contains("Closed lesson"), "{out}");
        assert!(out.contains("background sync started"), "{out}");
        assert!(
            !out.contains("no remote"),
            "a configured remote is distinct from an unavailable one: {out}"
        );
        assert_eq!(
            out.contains("Notes diverged from origin"),
            failure == "diverged",
            "{out}"
        );
        assert_eq!(mac.log().lines().count(), before + 2);
        assert!(!mac.notes.join("lessons/friction.md").exists());
        let record: Value =
            serde_json::from_slice(&std::fs::read(mac.state().join("sync.json")).unwrap()).unwrap();
        assert!(
            record[failure].is_object(),
            "the same warning record used by saves: {record}"
        );
        let (status, ok) = mac.run(&mac.notes, &["notes", "status"], "");
        assert!(ok && status.contains("not yet pushed"), "{status}");
    }
}

#[cfg(unix)]
#[test]
fn a_lesson_symlink_cannot_close_a_file_outside_the_notes() {
    let world = World::new();
    let mac = world.machine("mac");
    let outside = mac.dir.0.join("outside.md");
    write(&outside, "outside\n");
    std::fs::create_dir_all(mac.notes.join("lessons")).unwrap();
    std::os::unix::fs::symlink(&outside, mac.notes.join("lessons/friction.md")).unwrap();
    let before = git(&mac.notes, &["status", "--porcelain"]);
    let (out, ok) = mac.bash("s", CLOSE);
    assert!(!ok && out.contains("symbolic link"), "{out}");
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "outside\n");
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), before);
}

#[test]
fn closing_without_a_remote_reports_local_success_honestly() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    git(&mac.notes, &["remote", "remove", "origin"]);
    let (out, ok) = mac.bash("s", CLOSE);
    assert!(
        ok && out.contains("sync unavailable: no remote configured"),
        "{out}"
    );
    assert!(!out.contains("sync started"));
    assert!(!mac.sync_requested());
}

#[test]
fn a_merge_conflict_refuses_a_close_before_any_lesson_write() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lesson(&mac);
    git(&mac.notes, &["checkout", "-qb", "side"]);
    write(&mac.notes.join("knowledge/owner.md"), "side\n");
    git(&mac.notes, &["commit", "-qam", "side"]);
    git(&mac.notes, &["checkout", "-q", "main"]);
    write(&mac.notes.join("knowledge/owner.md"), "main\n");
    git(&mac.notes, &["commit", "-qam", "main"]);
    let output = Command::new("git")
        .args(["merge", "side"])
        .current_dir(&mac.notes)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let history = mac.log();
    let status = git(&mac.notes, &["status", "--porcelain"]);
    let lesson = std::fs::read(mac.notes.join("lessons/friction.md")).unwrap();
    let (out, ok) = mac.bash("s", CLOSE);
    assert!(!ok && out.contains("unmerged"), "{out}");
    assert_eq!(mac.log(), history);
    assert_eq!(git(&mac.notes, &["status", "--porcelain"]), status);
    assert_eq!(
        std::fs::read(mac.notes.join("lessons/friction.md")).unwrap(),
        lesson
    );
    assert!(!mac.state().join("ops").exists());
}

#[test]
fn housekeeping_names_in_text_pass_through_and_chained_sync_is_refused() {
    let world = World::new();
    let mac = world.machine("mac");
    for command in [
        "echo astrolabe lesson close",
        "echo \"astrolabe notes sync\"",
        "cat <<'EOF'\nastrolabe lesson close lessons/x.md --as done\nEOF",
        "Write-Output @'\nastrolabe notes sync\n'@",
    ] {
        assert!(mac.hook_json("pre-command", json!({"session_id":"s", "tool_name":"Bash", "cwd":mac.demo, "tool_input":{"command":command}})).is_none(), "{command}");
    }
    let before = mac.log();
    let (out, ok) = mac.bash("s", "astrolabe notes sync && echo after");
    assert!(!ok && out.contains("refused"), "{out}");
    assert_eq!(mac.log(), before);
}

fn seed_lessons(mac: &Machine, names: &[&str]) {
    for name in names {
        write(
            &mac.notes.join(format!("lessons/{name}.md")),
            &format!("# {name}\n\nIt cost a turn.\n"),
        );
    }
    git(&mac.notes, &["add", "lessons"]);
    git(&mac.notes, &["commit", "-qm", "Add lessons"]);
}

const DONE: &str = "astrolabe effort save --status done <<'ASTROLABE'\nDone.\n\nNothing chosen.\n\nNothing left.\nASTROLABE\n";

fn new_with(lessons: &str) -> String {
    format!(
        "astrolabe effort new --title \"Smooth ink\" {lessons} <<'ASTROLABE'\nsmooth the ink across frames\nASTROLABE\n"
    )
}

fn disposition_of(mac: &Machine, rel: &str) -> String {
    let sha = git(
        &mac.notes,
        &["log", "-1", "--format=%H", "--diff-filter=M", "--", rel],
    );
    git(&mac.notes, &["show", &format!("{}:{rel}", sha.trim())])
}

#[test]
fn an_effort_names_its_lessons_and_marking_it_done_closes_each_one() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction", "second"]);
    // A lesson that isn't there is refused with the open ones listed, and writes nothing.
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", &new_with("--lesson lessons/nope.md"));
    assert!(
        !ok && out.contains("lessons/nope.md, which isn't in the notes")
            && out.contains("Open lessons:\n- lessons/friction.md\n- lessons/second.md"),
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before);
    let (out, ok) = mac.bash("s1", &new_with("--lesson lessons/x.txt"));
    assert!(!ok && out.contains("Markdown"), "{out}");
    let (out, ok) = mac.bash("s1", &new_with("--lesson lessons/friction.md"));
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("\nlessons: lessons/friction.md\n")
    );
    // A later save adds to the list; one named again isn't doubled.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --lesson lessons/second.md --lesson lessons/friction.md",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("s1", "astrolabe effort save --lesson lessons/gone.md");
    assert!(!ok && out.contains("isn't in the notes"), "{out}");
    assert!(
        mac.effort_text(1)
            .contains("\nlessons: lessons/friction.md, lessons/second.md\n")
    );
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", DONE);
    let line = format!("Fixed by effort 1, Smooth ink, done {}.", today());
    assert!(
        ok && out.contains(&format!(
            "Closed lesson friction.md: \"{line}\" committed, file removed."
        )) && out.contains("Closed lesson second.md"),
        "{out}"
    );
    // The save's commit, then two per lesson, as `lesson close` makes them.
    assert_eq!(mac.log().lines().count(), before + 5);
    for rel in ["lessons/friction.md", "lessons/second.md"] {
        assert!(!mac.notes.join(rel).exists());
        let disposed = disposition_of(&mac, rel);
        assert_eq!(disposed.lines().last(), Some(line.as_str()), "{disposed}");
        assert_eq!(disposed.matches("Fixed by").count(), 1);
    }
    let trail = git(&mac.notes, &["log", "--format=%s", "--", "lessons/"]);
    assert!(
        trail.starts_with("Remove closed lesson lessons/second.md\nRecord disposition for lesson lessons/second.md\nRemove closed lesson lessons/friction.md\nRecord disposition for lesson lessons/friction.md\n"),
        "{trail}"
    );
    for (revision, rel) in [
        ("HEAD", "lessons/second.md"),
        ("HEAD~", "lessons/second.md"),
        ("HEAD~2", "lessons/friction.md"),
        ("HEAD~3", "lessons/friction.md"),
    ] {
        let files = git(&mac.notes, &["show", "--name-only", "--format=", revision]);
        assert_eq!(files.trim(), rel, "{revision}");
    }
    // The list stays as the record of what the effort closed; done again closes nothing.
    assert!(
        mac.effort_text(1)
            .contains("\nlessons: lessons/friction.md, lessons/second.md\n")
    );
    let count = mac.log().lines().count();
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done <<'ASTROLABE'\nDone, still.\n\nNothing chosen.\n\nNothing left.\nASTROLABE\n",
    );
    assert!(ok && !out.contains("lesson"), "{out}");
    assert_eq!(mac.log().lines().count(), count + 1);
}

#[test]
fn a_dispatcher_marking_done_by_number_closes_it_and_names_other_open_efforts() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction"]);
    assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
    // A worker may name lessons on its own save.
    assert!(
        mac.bash(
            "w1",
            "astrolabe activate --effort 1 --worker --model opus --reasoning medium"
        )
        .1
    );
    seed_lessons(&mac, &["worker"]);
    let (out, ok) = mac.bash("w1", "astrolabe effort save --lesson lessons/worker.md");
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Other ink\" --lesson lessons/friction.md <<'ASTROLABE'\nanother take on it\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.bash("s2", "astrolabe effort read 1").1);
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort save --effort 1 --status done <<'ASTROLABE'\nDone and landed.\n\nNothing chosen.\n\nNothing left.\nASTROLABE\n",
    );
    assert!(
        ok && out.contains("Closed lesson friction.md")
            && out.contains("Closed lesson worker.md")
            && out.contains("Effort 2, Other ink, also names lesson friction.md and is active"),
        "{out}"
    );
    assert!(!mac.notes.join("lessons/friction.md").exists());
    assert!(!mac.notes.join("lessons/worker.md").exists());
    assert!(
        disposition_of(&mac, "lessons/worker.md").ends_with(&format!(
            "Fixed by effort 1, Smooth ink, done {}.\n",
            today()
        ))
    );
}

#[test]
fn a_gone_lesson_is_skipped_abandoned_closes_nothing_and_no_lessons_saves_as_before() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction", "kept"]);
    assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
    git(&mac.notes, &["rm", "-q", "lessons/friction.md"]);
    git(&mac.notes, &["commit", "-qm", "Closed by hand"]);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", DONE);
    assert!(
        ok && out.contains("Lesson friction.md was already gone, so nothing was closed for it."),
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before + 1);
    // Abandoned closes nothing.
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Kept ink\" --lesson lessons/kept.md <<'ASTROLABE'\nkeep it\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort save --status abandoned --reason \"not needed\" <<'ASTROLABE'\nAbandoned.\n\nNot needed.\n\nNothing left.\nASTROLABE\n",
    );
    assert!(ok && !out.contains("lesson"), "{out}");
    assert!(mac.notes.join("lessons/kept.md").exists());
    // No lessons: one commit, and the file has no lessons line.
    assert!(
        mac.bash(
            "s3",
            "astrolabe effort new --title \"Plain ink\" <<'ASTROLABE'\nplain\nASTROLABE\n"
        )
        .1
    );
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s3", DONE);
    assert!(ok && !out.contains("lesson"), "{out}");
    assert_eq!(mac.log().lines().count(), before + 1);
    assert!(!mac.effort_text(3).contains("lessons:"));
}

#[test]
fn an_interrupted_done_save_finishes_its_closes_without_repeating_a_disposition() {
    for file_still_present in [false, true] {
        let world = World::new();
        let mac = world.machine("mac");
        seed_lessons(&mac, &["friction"]);
        assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
        let before = mac.log().lines().count();
        // Git refuses the removal commit, after the save and the disposition committed.
        let hook = mac.notes.join(".git/hooks/pre-commit");
        write(&hook, "#!/bin/sh\ntest -f lessons/friction.md\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let (out, ok) = mac.bash("s1", DONE);
        assert!(
            !ok && out.starts_with("Saved effort 1, but closing its lessons stopped: git refused a commit. Don't save it again"),
            "{out}"
        );
        assert_eq!(mac.log().lines().count(), before + 2);
        let disposed = git(&mac.notes, &["show", "HEAD:lessons/friction.md"]);
        assert_eq!(disposed.matches("Fixed by effort 1").count(), 1);
        std::fs::remove_file(hook).unwrap();
        if file_still_present {
            write(&mac.notes.join("lessons/friction.md"), &disposed);
        }
        assert!(mac.sync().1, "the next notes write finishes it");
        assert_eq!(mac.log().lines().count(), before + 3);
        assert!(!mac.notes.join("lessons/friction.md").exists());
        assert_eq!(disposition_of(&mac, "lessons/friction.md"), disposed);
        assert!(mac.effort_text(1).contains("status: done"));
        assert!(mac.sync().1);
        assert_eq!(mac.log().lines().count(), before + 3);
    }
}

#[test]
fn the_board_and_efforts_json_say_which_effort_answers_each_lesson() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction", "nobody"]);
    assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
    let (out, ok) = mac.run(&mac.demo, &["efforts"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["efforts"][0]["lessons"], json!(["lessons/friction.md"]));
    let lessons = v["lessons"].as_array().unwrap();
    let answer = &lessons[0]["answered_by"];
    assert_eq!(lessons[0]["file"], "lessons/friction.md");
    assert_eq!(answer["number"], 1);
    assert_eq!(answer["title"], "Smooth ink");
    assert_eq!(answer["status"], "active");
    assert!(
        answer["file"]
            .as_str()
            .unwrap()
            .ends_with("0001-smooth-ink.md")
    );
    assert!(lessons[1].get("answered_by").is_none(), "{}", lessons[1]);
    assert!(
        mac.bash(
            "s2",
            "astrolabe effort new --title \"Plain ink\" <<'ASTROLABE'\nplain\nASTROLABE\n"
        )
        .1
    );
    let (out, _) = mac.run(&mac.demo, &["efforts"], "");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(
        v["efforts"][1].get("lessons").is_none(),
        "{}",
        v["efforts"][1]
    );
    let page = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    assert!(
        page.contains(">answered by 1 Smooth ink, active</a>")
            && page.contains("class=\"pill answered\""),
        "{page}"
    );
}

/// After a done save, the notes still take writes: nothing is left for recovery to fail on.
fn later_writes_work(mac: &Machine) {
    let (out, ok) = mac.sync();
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "later",
        "astrolabe effort new --title \"Later ink\" <<'ASTROLABE'\nlater\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let status = git(&mac.notes, &["status", "--porcelain", "--", "lessons"]);
    assert!(status.trim().is_empty(), "lessons left dirty: {status}");
}

#[test]
fn one_lesson_under_two_spellings_closes_once_and_leaves_the_notes_writable() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction"]);
    let (out, ok) = mac.bash(
        "s1",
        &new_with("--lesson lessons/friction.md --lesson lessons/./friction.md --lesson lessons//friction.md"),
    );
    assert!(ok, "{out}");
    assert!(
        mac.effort_text(1)
            .contains("\nlessons: lessons/friction.md\n")
    );
    let (out, ok) = mac.bash("s1", DONE);
    assert!(
        ok && out.matches("Closed lesson friction.md").count() == 1,
        "{out}"
    );
    assert!(!mac.notes.join("lessons/friction.md").exists());
    assert_eq!(
        disposition_of(&mac, "lessons/friction.md")
            .matches("Fixed by")
            .count(),
        1
    );
    later_writes_work(&mac);
}

#[test]
fn a_lesson_named_in_the_wrong_case_is_refused() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction"]);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", &new_with("--lesson lessons/Friction.md"));
    assert!(
        !ok && out.contains("lessons/Friction.md") && out.contains("lessons/friction.md"),
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before);
    assert!(mac.bash("s1", &new_with("")).1);
    let (out, ok) = mac.bash("s1", "astrolabe effort save --lesson lessons/FRICTION.md");
    assert!(!ok && out.contains("lessons/friction.md"), "{out}");
    later_writes_work(&mac);
}

#[test]
fn a_hand_edited_lessons_line_with_repeats_or_the_wrong_case_closes_each_file_once() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction", "second"]);
    assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
    // The owner edits the property by hand, then reloads the effort.
    let (_, rel) = astrolabe::notes::effort_files(&mac.notes)
        .into_iter()
        .find(|(n, _)| *n == 1)
        .unwrap();
    let path = mac.notes.join(&rel);
    let text = std::fs::read_to_string(&path).unwrap().replace(
        "lessons: lessons/friction.md\n",
        "lessons: lessons/friction.md, lessons/friction.md, lessons/./friction.md, lessons/Second.md, lessons/second.md\n",
    );
    write(&path, &text);
    git(&mac.notes, &["commit", "-qam", "Hand edit"]);
    assert!(mac.bash("s1", "astrolabe activate --effort 1").1);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", DONE);
    assert!(
        ok && out.matches("Closed lesson friction.md").count() == 1
            && out.matches("Closed lesson second.md").count() == 1,
        "{out}"
    );
    assert_eq!(mac.log().lines().count(), before + 5);
    for rel in ["lessons/friction.md", "lessons/second.md"] {
        assert!(!mac.notes.join(rel).exists());
        assert_eq!(disposition_of(&mac, rel).matches("Fixed by").count(), 1);
    }
    later_writes_work(&mac);
}

#[test]
fn a_planned_close_whose_lesson_is_gone_is_skipped_and_a_failed_commit_leaves_no_line() {
    let world = World::new();
    let mac = world.machine("mac");
    seed_lessons(&mac, &["friction"]);
    assert!(mac.bash("s1", &new_with("--lesson lessons/friction.md")).1);
    // Git refuses every commit that touches the lesson: the save commits, the close fails.
    let hook = mac.notes.join(".git/hooks/pre-commit");
    write(
        &hook,
        "#!/bin/sh\n! git diff --cached --name-only | grep -q '^lessons/'\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let (out, ok) = mac.bash("s1", DONE);
    assert!(
        !ok && out.contains("but closing its lessons stopped"),
        "{out}"
    );
    // No disposition line left uncommitted in the lesson.
    let status = git(&mac.notes, &["status", "--porcelain", "--", "lessons"]);
    assert!(status.trim().is_empty(), "{status}");
    assert!(
        !std::fs::read_to_string(mac.notes.join("lessons/friction.md"))
            .unwrap()
            .contains("Fixed by")
    );
    // The owner removes the lesson by hand meanwhile: the planned close is skipped.
    std::fs::remove_file(hook).unwrap();
    git(&mac.notes, &["rm", "-q", "lessons/friction.md"]);
    git(&mac.notes, &["commit", "-qm", "Closed by hand"]);
    later_writes_work(&mac);
    assert!(mac.effort_text(1).contains("status: done"));
}
