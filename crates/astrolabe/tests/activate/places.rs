use super::*;

const SHARED: &str = "This folder is demo's main checkout, which other sessions share, so one of them can switch its branch or change files under you. Change no project file here: work in a worktree of your own, made before your first edit and named in any plan. demo's note says where worktrees go; ask first if its rules say to.";

const WRONG_FOLDER: &str = "This session started in the notes checkout, but effort 70's work is in demo. A session takes its repo's rules from the folder it starts in, so this one has the notes' rules and not demo's. Change no file in demo from here. Tell your dispatcher now that this worker has to be started again in a folder of demo, and stop until it answers.";

const SEAT: &str = "This initiative's work is all in demo, and this session is in the notes checkout. An orchestrator for one repo sits in that repo, so it reviews and lands with the repo's rules. Say so to the owner once, then carry on here unless they move it.";

/// The start of each notice about where a session is, for a text that should have none.
const OPENERS: [&str; 3] = [
    "main checkout, which other sessions share",
    "This session started in the notes checkout",
    "and this session is in the notes checkout",
];

fn quiet(text: &str, what: &str) {
    for opener in OPENERS {
        assert!(!text.contains(opener), "{what} has {opener:?}: {text}");
    }
}

fn worktree(repo: &Path, name: &str) -> PathBuf {
    let tree = repo.with_extension("worktrees").join(name);
    git(
        repo,
        &["worktree", "add", "-q", "-b", name, tree.to_str().unwrap()],
    );
    tree
}

#[test]
fn edit_authority_in_a_main_checkout_with_other_worktrees_is_told_to_use_its_own() {
    let f = Fixture::new();
    let load = "astrolabe activate --effort 70";
    // No other worktree: nothing says the checkout is shared.
    let (alone, ok) = f.bash("s0", &f.demo, load);
    assert!(ok, "{alone}");
    quiet(&alone, "a main checkout with no other worktree");
    let tree = worktree(&f.demo, "lane");
    let (text, ok) = f.bash("s1", &f.demo, load);
    assert!(ok && text.contains(&format!("\n\n{SHARED}")), "{text}");
    // Nothing else in the text moved: without the notice it's what the lone checkout got,
    // apart from the session's own name and probe.
    let head = |t: &str| -> String {
        let t = t.replace("session s1", "session s0");
        let t = t.replace(&format!("\n\n{SHARED}"), "");
        let head = t.split("\n\n---").next().unwrap();
        let lines = head.lines().filter(|l| !l.contains("astrolabe-probe"));
        lines.collect::<Vec<_>>().join("\n")
    };
    assert_eq!(head(&text), head(&alone));
    // A resumed session is told again.
    let resumed = f.session_start("s1", &f.demo, &[]).unwrap();
    assert!(resumed.contains(SHARED), "{resumed}");
    // A worker is told the same, ahead of its settings.
    let (worker, ok) = f.bash("w1", &f.demo, &format!("{load} --worker"));
    let (notice, settings) = (worker.find(SHARED), worker.find("Settings from"));
    assert!(ok && notice.is_some() && notice < settings, "{worker}");
    // In a worktree of its own, with read authority, or in the notes: nothing.
    let (text, ok) = f.bash("s2", &tree, load);
    assert!(ok, "{text}");
    quiet(&text, "a worktree");
    let (text, ok) = f.bash("s3", &f.demo, "astrolabe activate --effort 75");
    assert!(ok && text.contains("authority: read"), "{text}");
    quiet(&text, "read authority");
    let (text, ok) = f.bash("s4", &f.notes(), load);
    assert!(ok, "{text}");
    quiet(&text, "an owner's effort in the notes");
}

#[test]
fn a_worker_bound_in_the_notes_for_another_repos_effort_is_told_to_be_started_again() {
    let f = Fixture::new();
    let notes = f.notes();
    git(&notes, &["add", "-A"]);
    git(&notes, &["commit", "-q", "-m", "notes"]);
    let (text, ok) = f.bash("w1", &notes, "astrolabe activate --effort 70 --worker");
    let (notice, settings) = (text.find(WRONG_FOLDER), text.find("Settings from"));
    assert!(ok && notice.is_some() && notice < settings, "{text}");
    // The same worker in its effort's repo, and a worker on the notes' own effort: nothing.
    let (text, ok) = f.bash("w2", &f.demo, "astrolabe activate --effort 70 --worker");
    assert!(ok, "{text}");
    quiet(&text, "a worker in its repo");
    write(&notes.join("knowledge/repos/agent.md"), "REPO-NOTES-TEXT");
    write(
        &notes.join("efforts/0077-notes.md"),
        "---\nstatus: active\nrepo: agent\ninvolvement: autonomous\nauthority: edit\n---\n# 0077 Notes\n\n## Current state\n\nEFFORT-77\n",
    );
    git(&notes, &["add", "-A"]);
    git(&notes, &["commit", "-q", "-m", "77"]);
    let (text, ok) = f.bash("w3", &notes, "astrolabe activate --effort 77 --worker");
    assert!(ok, "{text}");
    quiet(&text, "a worker on the notes' own effort");
}

#[test]
fn an_orchestrator_is_told_where_a_worker_starts_and_when_it_sits_in_the_notes() {
    let f = Fixture::new();
    let notes = f.notes();
    let page = |title: &str, repo: &str| {
        format!("---\nstatus: active\nrepo: {repo}\n---\n# {title}\n\n## Current state\n\nPAGE\n")
    };
    write(
        &notes.join("initiatives/one-repo.md"),
        &page("One repo", "demo"),
    );
    write(
        &notes.join("initiatives/two-repos.md"),
        &page("Two repos", "demo, other"),
    );
    let (text, ok) = f.bash("o1", &notes, "astrolabe activate --initiative one-repo");
    assert!(ok && text.contains(&format!("\n\n{SEAT}")), "{text}");
    for said in [
        "Give each one a start folder in its effort's repo, never just your own folder, because a session takes its repo's rules from the folder it starts in: a fresh worktree of that repo where the app makes one, else the repo's checkout with a brief that has it make a worktree before it edits.",
        "That repo's note under `knowledge/repos/` says where its checkout and worktrees are on this machine; ask the owner if it doesn't.",
        "alone first and names its brief by full path.",
    ] {
        assert!(text.contains(said), "lacks {said:?}: {text}");
    }
    // The folder of briefs in full, as the notes checkout is on record, so a worker started
    // anywhere can open its brief.
    let (_, after) = text
        .split_once("Briefs and reports live in the notes under `")
        .unwrap();
    let (folder, rest) = after.split_once('`').unwrap();
    let sep = std::path::MAIN_SEPARATOR;
    let root = folder.strip_suffix(&format!("{sep}.handoffs{sep}one-repo{sep}"));
    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok();
    assert!(
        root.is_some_and(|root| Path::new(root).is_absolute() && same(Path::new(root), &notes))
            && rest.starts_with(" so another session can resume."),
        "{folder}"
    );
    // Across repos the notes are its seat, and in its one repo it's where it should be.
    let (text, ok) = f.bash("o2", &notes, "astrolabe activate --initiative two-repos");
    assert!(ok && text.contains("never just your own folder"), "{text}");
    quiet(&text, "an orchestrator across repos");
    let (text, ok) = f.bash("o3", &f.demo, "astrolabe activate --initiative one-repo");
    assert!(ok && text.contains("never just your own folder"), "{text}");
    quiet(&text, "an orchestrator in its repo");
}

#[test]
fn a_git_read_the_hook_has_no_time_for_leaves_the_notice_out() {
    let f = Fixture::new();
    worktree(&f.demo, "lane");
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && text.contains(SHARED), "{text}");
    // At the look for other worktrees the hook's clock is moved on 15 s, past its time for
    // reading repos, so that look isn't started. What it would have shown isn't guessed:
    // the session gets its effort's text with no word about its folder.
    let env = [("ASTROLABE_HOOK_TEST_STALL", "worktree:+15000")];
    let resumed = f.session_start("s1", &f.demo, &env).unwrap();
    assert!(
        resumed.contains("Astrolabe is on in this session for effort 70"),
        "{resumed}"
    );
    quiet(&resumed, "a session whose folder couldn't be read in time");
}

#[test]
fn the_notes_under_another_name_are_still_the_notes() {
    let f = Fixture::new();
    let notes = f.notes();
    // The owner's notes are also registered as `notes`, and an effort and an initiative
    // name them that way: their work is in the notes, so the notes are the right folder.
    let (out, ok) = f.run(
        &notes,
        &["repo", "add", "notes", notes.to_str().unwrap()],
        "",
        &[],
    );
    assert!(ok, "{out}");
    write(&notes.join("knowledge/repos/notes.md"), "REPO-NOTES-TEXT");
    write(
        &notes.join("efforts/0078-tidy.md"),
        "---\nstatus: active\nrepo: notes\ninvolvement: autonomous\nauthority: edit\n---\n# 0078 Tidy\n\n## Current state\n\nEFFORT-78\n",
    );
    write(
        &notes.join("initiatives/tidy-notes.md"),
        "---\nstatus: active\nrepo: notes\n---\n# Tidy notes\n\n## Current state\n\nPAGE\n",
    );
    git(&notes, &["add", "-A"]);
    git(&notes, &["commit", "-q", "-m", "notes"]);
    let (text, ok) = f.bash("w1", &notes, "astrolabe activate --effort 78 --worker");
    assert!(ok && text.contains("repo notes"), "{text}");
    quiet(
        &text,
        "a worker on the notes' own effort, by their other name",
    );
    let (text, ok) = f.bash("o1", &notes, "astrolabe activate --initiative tidy-notes");
    assert!(ok, "{text}");
    quiet(&text, "an orchestrator for the notes, by their other name");
}

#[test]
fn a_worktree_whose_folder_is_gone_is_no_one_elses_work() {
    let f = Fixture::new();
    // Locked, then its folder deleted: git keeps the entry and never calls it prunable.
    let tree = worktree(&f.demo, "gone");
    git(&f.demo, &["worktree", "lock", tree.to_str().unwrap()]);
    std::fs::remove_dir_all(&tree).unwrap();
    let (text, ok) = f.bash("s1", &f.demo, "astrolabe activate --effort 70");
    assert!(ok, "{text}");
    quiet(&text, "a main checkout whose only other worktree is gone");
    // One that is there still counts.
    worktree(&f.demo, "lane");
    let (text, ok) = f.bash("s2", &f.demo, "astrolabe activate --effort 70");
    assert!(ok && text.contains(SHARED), "{text}");
}

/// A session's text with what differs between any two runs taken out: the fixture's folder
/// and the lines of the guard's probe.
#[cfg(unix)]
fn masked(f: &Fixture, text: &str) -> String {
    let real = std::fs::canonicalize(&f.dir.0).unwrap();
    let text = text.replace(real.to_str().unwrap(), "<dir>");
    let text = text.replace(f.dir.0.to_str().unwrap(), "<dir>");
    let lines = text.lines().filter(|l| !l.contains("astrolabe-probe"));
    lines.collect::<Vec<_>>().join("\n")
}

/// What a real binding sends where no notice fires, by length and SHA-256, as it stands since
/// Needs you came to mean something the owner can act on now (before that, as main sent it
/// at 8766a3e, 94 bytes shorter for an effort and 287 for a worker): the whole reply and the whole
/// resumed text, through the hook, not only the rules. Not on Windows, where the text has
/// both shells' proof steps and its own path separators.
#[cfg(unix)]
#[test]
fn a_binding_no_notice_fires_for_gets_the_bytes_main_gave_it() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    let notes = f.notes();
    git(&notes, &["add", "-A"]);
    git(&notes, &["commit", "-q", "-m", "notes"]);
    let load = "astrolabe activate --effort 70";
    let mut got = Vec::new();
    let mut keep = |what: &str, text: String| {
        quiet(&text, what);
        let text = masked(&f, &text);
        let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        got.push((what.to_owned(), text.len(), hash));
    };
    // A main checkout with no other worktree: an owner's effort, its resume, and a worker.
    let (text, ok) = f.bash("s1", &f.demo, load);
    assert!(ok, "{text}");
    keep("an owner's effort in a lone checkout", text);
    keep(
        "that effort resumed",
        f.session_start("s1", &f.demo, &[]).unwrap(),
    );
    let (text, ok) = f.bash("w1", &f.demo, &format!("{load} --worker"));
    assert!(ok, "{text}");
    keep("a worker in a lone checkout", text);
    // The owner's seat, and a worktree of its own beside a shared main checkout.
    let (text, ok) = f.bash("s2", &notes, load);
    assert!(ok, "{text}");
    keep("an owner's effort in the notes", text);
    let tree = worktree(&f.demo, "lane");
    let (text, ok) = f.bash("s3", &tree, load);
    assert!(ok, "{text}");
    keep("an owner's effort in a worktree", text);
    let want = [
        (
            "an owner's effort in a lone checkout",
            7435,
            "cbbcb9f47e3afc87d99ead0b3cb708477291b196da612899f35fd02415531356",
        ),
        (
            "that effort resumed",
            7967,
            "a041e381ad3cac9ca3422b57b42a53c76009b9aaff688971d88a8f4cad14c9f8",
        ),
        (
            "a worker in a lone checkout",
            6027,
            "707eb7de7925e3d1a507598cde102fc7364481b0728b0afbe15e46cacd7f34a2",
        ),
        (
            "an owner's effort in the notes",
            7435,
            "1d24e56954b69b33552ff9ea9fa41e12ff9500849792f03f0d86c08b1b04bbb0",
        ),
        (
            "an owner's effort in a worktree",
            7435,
            "d2e00b8141047871fd2a5f466d6dca721882c32a2650d5f5dd4a2943cdf18e75",
        ),
    ];
    let got: Vec<(&str, usize, &str)> = (got.iter())
        .map(|(what, len, hash)| (what.as_str(), *len, hash.as_str()))
        .collect();
    assert_eq!(got, want);
}
