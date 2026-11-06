//! `astrolabe notes init`: a new owner's notes checkout, seeded with effort 1, "Fill in your
//! notes", so the board isn't empty and the owner learns the flow by doing it.
use crate::effort::EffortFile;
use crate::registry::AGENT;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const TITLE: &str = "Fill in your notes";

const README: &str = r#"# Your Astrolabe notes

This is your private record of work, preferences and decisions. Agents save here so a fresh session can pick up where you left off.

## Start here

For new notes, open an agent session in this folder and say **pick up 1**. The **Fill in your notes** effort walks you through your preferences, writing voice, machine details and first project, one piece at a time.

If the agent doesn't recognize Astrolabe, finish the [installation and first-session check](https://github.com/NateChristiansen/Astrolabe/blob/main/plugin/README.md).

## Find your work

- `efforts/`: where each piece of work stands and what comes next.
- `journal/`: a short history of what changed.
- `knowledge/`: your preferences and context for projects and machines.
- `lessons/`: friction waiting for review.

Open `home.html` for a view of your work, or follow [Using Astrolabe](https://github.com/NateChristiansen/Astrolabe/blob/main/docs/usage.md) to open the live board, resume an effort or connect another machine. Keep the Git remote for these notes private.

Instructions for agents are in [AGENTS.md](AGENTS.md).
"#;

const AGENTS: &str = "# Notes\n\nPrivate notes for the owner's agents. Astrolabe reads them at session start and writes them through its hook; what it reads and writes is in the [internals reference](https://github.com/NateChristiansen/Astrolabe/blob/main/docs/internals.md). Each session is told what it gets and how to record work, so this file only says what's here and how lessons move.\n\n## Folders\n\n- `efforts/`: one note per effort, written by the program. Current state is what a fresh session gets; Decisions and Pieces sit below it.\n- `journal/`: the owner's history, one line per effort per day.\n- `knowledge/`: what no model can learn on its own: `owner.md`, `voice.md`, `machines.md`, and one note per repo under `repos/`, named as the `repo` property of efforts names it. `repos/agent.md` is this checkout's own.\n- `lessons/`: friction waiting for triage. See below.\n- `drafts/`: text for the owner to post or decide on.\n- `initiatives/`: the board groups efforts by their `initiative` property.\n- `.agents/`: what agents write for each other.\n- `home.html`: the board, drawn by the program and out of git. The live page is the served one.\n\nRule files stay short, because instruction following falls as rules pile up. When a rule goes into one, check that file and drop a rule a newer model no longer needs.\n\n## Lessons\n\nA lesson is friction that cost a turn or more and that a fact, a rule or a fix would have saved. The session that hit it writes `lessons/YYYY-MM-DD-slug.md`, under about fifteen lines: a title, what happened with the evidence (what ran, what came back, what it cost), and one line on what it points at. No fix in it; keep the specifics, since a summary loses the gotcha.\n\nThe board counts lessons; nothing delivers them to a session. Triage runs here with the owner, and each lesson ends one way, with its reason: a fact or short rule in the file the model already reads (`machines.md`, the repo note, `owner.md` with the owner's OK); a program or repo fix, as an effort; or nothing. Append the disposition as the last line, commit, then delete the file in the next commit. `git log lessons/` is the trail.\n";

const OWNER: &str = "# Owner\n\nStanding preferences, read by every session. Current facts about work live in efforts and notes, not here. Agents change this file only with the owner's OK.\n\n## Boundaries\n\nWhat agents never do, and what needs the owner's OK first: posting as the owner, pushing, settings they keep for themselves.\n\n## How to talk to me\n\nHow replies should read: length, format, how questions are asked.\n\n## How I work\n\nHow the owner likes work run: when to check in, how much to verify, what the journal is for.\n\n## Projects and machines\n\nThe owner's projects as they call them, and which machines they work on.\n";

const VOICE: &str = "# Voice\n\nHow the owner writes, so agents match it in anything the owner reads and anything drafted for them. Samples teach style only: never take facts, people or history from them. Agents change this file only with the owner's OK.\n\n## Rules\n\nA few plain rules about the owner's writing: length, tone, what they never say.\n\n## Samples\n\nThe owner's own writing, pasted as it was sent: a chat message, a review comment, a note to a team.\n";

const MACHINES: &str = "# Machines\n\nFacts about the owner's machines that agents can't work out for themselves: the tools that are installed oddly, the commands that don't exist, the paths that differ. One section per machine.\n";

const NOTES_REPO: &str = "# Notes\n\nThis notes checkout, registered on each machine as `agent`. Efforts about the notes themselves name it in `repo: agent`, and a session here is the owner's seat, where any effort loads.\n";

const ATTRIBUTES: &str = "journal/*.md merge=union\njournal/**/*.md merge=union\n";

const CURRENT: &str = "New notes, with nothing filled in yet. This effort walks the owner through them, one file at a time, so agents start every session knowing who they work for.\n\nEach piece is a conversation: ask, write what the owner says in their words, show it, and tick the piece once they're happy. owner.md is how they want work run and replies written; voice.md is samples of their own writing; machines.md is what agents can't work out about their machines. The first repo note comes from `astrolabe repo add <name>`, run from that repo.\n\nNext: the owner file. Nothing filled in yet.";

const PIECES: &str = "- [ ] Owner: how I want work run and replies written\n- [ ] Voice: a few samples of my own writing\n- [ ] Machines: what agents can't work out about my machines\n- [ ] First repo note: `astrolabe repo add <name>` from that repo";

/// Creates the checkout at `folder` and registers it as this machine's notes. What it says,
/// line by line, for the owner.
pub fn init(folder: &Path) -> Result<Vec<String>, String> {
    if let Ok((_, notes)) = crate::activate::notes_and_repos() {
        return Err(format!(
            "this machine already has a notes checkout, {}. To use another, `astrolabe repo add agent <checkout>` replaces it",
            notes.display()
        ));
    }
    let folder = match folder.is_absolute() {
        true => folder.to_path_buf(),
        false => std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(folder),
    };
    let fresh_clone = check_empty(&folder)?;
    let made_folder = !folder.exists();
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    // Any failure from here puts the folder back as it was, so a rerun isn't refused as
    // someone's work.
    if let Err(why) = seed(&folder, fresh_clone) {
        undo(&folder, made_folder, fresh_clone);
        return Err(why);
    }
    let checkout = PathBuf::from(crate::registry::native(&folder));
    crate::activate::add_repo(AGENT, &checkout).map_err(|why| {
        format!("the notes were made at {} but the plugin's record couldn't be written ({why}); fix that and run `astrolabe repo add agent {}`", checkout.display(), checkout.display())
    })?;
    let mut said = vec![
        format!(
            "Notes made at {} and registered on this machine as the notes checkout.",
            checkout.display()
        ),
        format!(
            "Effort 1, \"{TITLE}\", is active: open a session in that folder and say \"pick up 1\"."
        ),
    ];
    if git(&folder, &["remote"]).is_ok_and(|r| r.trim().is_empty()) {
        said.push("It has no remote, so nothing leaves this machine yet: add a private one with `git remote add origin <url>` and `git push -u origin HEAD`.".to_owned());
    }
    Ok(said)
}

/// The files and the first commit. The identity is checked inside the new repo, where
/// `git commit` reads it: a parent folder's or a `gitdir:` include's config may differ.
fn seed(folder: &Path, fresh_clone: bool) -> Result<(), String> {
    if !fresh_clone {
        git(folder, &["init", "-q"])?;
    }
    for ident in ["GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"] {
        git(folder, &["var", ident]).map_err(|_| {
            "git has no name and email to commit with here; set `git config --global user.name` and `user.email`, then run this again".to_owned()
        })?;
    }
    let (day, _) = crate::notes::local_day_time();
    let effort = format!(
        "{}/0001-{}.md",
        crate::notes::effort_dir(&day),
        crate::effort::slug(TITLE)
    );
    let files = [
        ("README.md", README.to_owned()),
        ("AGENTS.md", AGENTS.to_owned()),
        (".gitattributes", ATTRIBUTES.to_owned()),
        ("knowledge/owner.md", OWNER.to_owned()),
        ("knowledge/voice.md", VOICE.to_owned()),
        ("knowledge/machines.md", MACHINES.to_owned()),
        ("knowledge/repos/agent.md", NOTES_REPO.to_owned()),
        ("lessons/.gitkeep", String::new()),
        ("drafts/.gitkeep", String::new()),
        ("initiatives/.gitkeep", String::new()),
        (effort.as_str(), first_effort(&day)),
    ];
    for (file, text) in &files {
        crate::notes::write_text(&folder.join(file), text)?;
    }
    git(folder, &["add", "-A"])?;
    git(folder, &["commit", "-q", "-m", "Seed the notes"])?;
    Ok(())
}

/// Back to what `check_empty` found: no folder, an empty one, or the clone's own `.git`.
fn undo(folder: &Path, made_folder: bool, fresh_clone: bool) {
    if made_folder {
        let _ = std::fs::remove_dir_all(folder);
        return;
    }
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        if fresh_clone && entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let _ = match path.is_dir() {
            true => std::fs::remove_dir_all(&path),
            false => std::fs::remove_file(&path),
        };
    }
}

/// A folder that's missing or empty, or a fresh clone of an empty repo (true): anything else
/// is someone's work, never written over.
fn check_empty(folder: &Path) -> Result<bool, String> {
    let entries: Vec<String> = match std::fs::read_dir(folder) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", folder.display())),
    };
    match entries.as_slice() {
        [] => Ok(false),
        [git_dir]
            if git_dir == ".git"
                && git(folder, &["rev-parse", "-q", "--verify", "HEAD"]).is_err() =>
        {
            Ok(true)
        }
        _ => Err(format!(
            "{} isn't empty; notes init only makes a new checkout, and `astrolabe repo add agent <checkout>` registers one that exists",
            folder.display()
        )),
    }
}

fn first_effort(day: &str) -> String {
    let mut file = EffortFile {
        title: format!("0001 {TITLE}"),
        current: CURRENT.to_owned(),
        ..Default::default()
    };
    for (k, v) in [
        ("status", "active"),
        ("repo", AGENT),
        ("initiative", ""),
        ("priority", "none"),
        ("involvement", "each step"),
        ("authority", "edit"),
        ("waits_for", ""),
        ("created", day),
    ] {
        file.set(k, v);
    }
    file.set_pieces(PIECES);
    file.render()
}

fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        false => Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
