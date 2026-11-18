//! Hand-off files under `.handoffs/` travel with the save that owns them, so a brief written
//! on one machine reaches the other with no hand git, and nothing of another's rides along.
use super::*;

/// Paths the last commit touched.
fn committed(m: &Machine) -> Vec<String> {
    let out = git(&m.notes, &["show", "--name-only", "--format=", "HEAD"]);
    // The first write anywhere adds the line-ending rules; that's not this change.
    (out.lines())
        .filter(|l| *l != ".gitattributes")
        .map(str::to_owned)
        .collect()
}

/// `git status --porcelain` lines, untracked files one by one.
fn status(m: &Machine) -> String {
    git(
        &m.notes,
        &["status", "--porcelain", "--untracked-files=all"],
    )
}

const PAGE: &str = "---\nstatus: active\nrepo: demo\n---\n# Quill\n\nQuill work.\n";

/// Effort 1 is bound to `s1` and a member of the initiative `quill`.
fn member(m: &Machine) {
    write(&m.notes.join("initiatives/quill.md"), PAGE);
    git(&m.notes, &["add", "initiatives"]);
    git(&m.notes, &["commit", "-qm", "page"]);
    let (out, ok) = m.bash("s1", NEW);
    assert!(ok, "{out}");
    let (out, ok) = m.bash("s1", "astrolabe effort save --initiative \"Quill\"");
    assert!(ok, "{out}");
}

/// Catches: a save that swept another worker's file, another folder's or a staged leftover
/// into its commit, or carried binary or oversized evidence, or left them unnamed.
#[test]
fn a_members_save_carries_only_its_own_numbered_text_files() {
    let world = World::new();
    let mac = world.machine("mac");
    member(&mac);
    let h = mac.notes.join(".handoffs");
    write(&h.join("quill/1-report.md"), "# Report\n");
    write(&h.join("quill/1-procedure/run.sh"), "echo run\n");
    write(
        &h.join("quill/2-report.md"),
        "half written by another worker\n",
    );
    write(&h.join("quill/worker-rules.md"), "the orchestrator's\n");
    write(&h.join("other/1-report.md"), "another folder\n");
    write(&h.join("1-loose.md"), "loose at the top\n");
    std::fs::write(h.join("quill/1-trace.bin"), b"PK\0\x03binary").unwrap();
    write(&h.join("quill/1-big.log"), &"x".repeat(256 * 1024 + 1));
    write(&h.join("quill/2-staged.md"), "staged by another\n");
    write(&mac.notes.join("knowledge/staged.md"), "staged by hand\n");
    git(
        &mac.notes,
        &["add", ".handoffs/quill/2-staged.md", "knowledge/staged.md"],
    );
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose ink over paint: it dries\"",
    );
    assert!(ok, "{out}");
    let mut files = committed(&mac);
    files.sort();
    assert_eq!(
        files,
        [
            ".handoffs/quill/1-procedure/run.sh",
            ".handoffs/quill/1-report.md",
            shelf("0001-smooth-ink.md").as_str(),
        ],
        "{out}"
    );
    assert!(
        out.contains(".handoffs/quill/1-trace.bin (binary)")
            && out.contains(".handoffs/quill/1-big.log (over 256 KiB)")
            && out.contains("stay on this machine"),
        "{out}"
    );
    for other in ["2-report.md", "worker-rules.md", "other/1-report.md"] {
        assert!(!out.contains(other), "only what it left of its own: {out}");
    }
    let status = status(&mac);
    for line in [
        "A  .handoffs/quill/2-staged.md",
        "A  knowledge/staged.md",
        "?? .handoffs/quill/2-report.md",
        "?? .handoffs/quill/worker-rules.md",
        "?? .handoffs/other/1-report.md",
        "?? .handoffs/1-loose.md",
        "?? .handoffs/quill/1-trace.bin",
        "?? .handoffs/quill/1-big.log",
    ] {
        assert!(status.contains(line), "{line}\n{status}");
    }
    // The next save names them again, since they still haven't left this machine.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose dry over wet: faster\"",
    );
    assert!(ok && out.contains("1-trace.bin (binary)"), "{out}");
    assert_eq!(committed(&mac), [shelf("0001-smooth-ink.md")]);
}

/// Catches: a brief that sat on one machine because only hand git would have carried it.
#[test]
fn a_brief_written_on_one_machine_reads_on_the_other_after_the_next_save() {
    let world = World::new();
    let mac = world.machine("mac");
    let pc = world.machine("pc");
    member(&mac);
    write(
        &mac.notes.join(".handoffs/quill/1-brief.md"),
        "# Brief\n\nDo the ink.\n",
    );
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --decision \"chose a brief over chat: it lasts\""
        )
        .1
    );
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    assert_eq!(
        std::fs::read_to_string(pc.notes.join(".handoffs/quill/1-brief.md")).unwrap(),
        "# Brief\n\nDo the ink.\n"
    );
    // An edit and a removal travel the same way.
    write(
        &mac.notes.join(".handoffs/quill/1-brief.md"),
        "# Brief\n\nDone.\n",
    );
    write(&mac.notes.join(".handoffs/quill/1-old.md"), "old\n");
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --decision \"chose done over more: enough\""
        )
        .1
    );
    std::fs::remove_file(mac.notes.join(".handoffs/quill/1-old.md")).unwrap();
    assert!(
        mac.bash(
            "s1",
            "astrolabe effort save --decision \"chose less over more: tidy\""
        )
        .1
    );
    assert!(committed(&mac).contains(&".handoffs/quill/1-old.md".to_owned()));
    assert!(mac.sync().1);
    assert!(pc.sync().1);
    assert!(
        std::fs::read_to_string(pc.notes.join(".handoffs/quill/1-brief.md"))
            .unwrap()
            .contains("Done.")
    );
    assert!(!pc.notes.join(".handoffs/quill/1-old.md").exists());
}

/// Catches: an orchestrator's page save leaving its folder's briefs behind, or reaching into
/// another initiative's folder or the loose files at the top.
#[test]
fn an_initiatives_save_carries_its_whole_folder() {
    let world = World::new();
    let mac = world.machine("mac");
    write(&mac.notes.join("initiatives/quill.md"), PAGE);
    git(&mac.notes, &["add", "initiatives"]);
    git(&mac.notes, &["commit", "-qm", "page"]);
    let (out, ok) = mac.bash("lead", "astrolabe activate --initiative quill");
    assert!(ok, "{out}");
    let h = mac.notes.join(".handoffs");
    write(&h.join("quill/worker-rules.md"), "rules\n");
    write(&h.join("quill/2-brief.md"), "brief\n");
    write(&h.join("quill-two/1-brief.md"), "another initiative\n");
    write(&h.join("loose.md"), "loose\n");
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --decision \"chose two workers over one: parallel\"",
    );
    assert!(ok, "{out}");
    let mut files = committed(&mac);
    files.sort();
    assert_eq!(
        files,
        [
            ".handoffs/quill/2-brief.md",
            ".handoffs/quill/worker-rules.md",
            "initiatives/quill.md",
        ]
    );
    let status = status(&mac);
    assert!(
        status.contains("?? .handoffs/quill-two/1-brief.md")
            && status.contains("?? .handoffs/loose.md"),
        "{status}"
    );
}

/// Catches: an effort with no initiative carrying nothing, or a loose file or another
/// effort's folder at the top of `.handoffs/`.
#[test]
fn an_effort_with_no_initiative_carries_the_folders_named_for_its_number() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let h = mac.notes.join(".handoffs");
    write(&h.join("1/brief.md"), "brief\n");
    write(&h.join("0001-eval/results.md"), "results\n");
    write(&h.join("0001-cleanup-brief.md"), "loose\n");
    write(&h.join("12-eval/results.md"), "effort 12's\n");
    write(&h.join("eval-harness/1-notes.md"), "a freeform folder\n");
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --decision \"chose eval over guess: evidence\"",
    );
    assert!(ok, "{out}");
    let mut files = committed(&mac);
    files.sort();
    assert_eq!(
        files,
        [
            ".handoffs/0001-eval/results.md",
            ".handoffs/1/brief.md",
            shelf("0001-smooth-ink.md").as_str(),
        ]
    );
}
