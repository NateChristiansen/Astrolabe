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
