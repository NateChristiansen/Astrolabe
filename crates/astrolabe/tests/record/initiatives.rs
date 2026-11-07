use super::*;

const INITIATIVE: &str = "astrolabe initiative new --title \"Smooth ink\" <<'ASTROLABE'\nMake ink smooth across frames, with café in it.\n\n## Efforts and order\nTether cap\nCap the tether so strokes stop splitting.\n\nGoo at speed\nKeep goo stable at speed,\ntwo frames at most.\n\n## What completion means\n- strokes never split\nASTROLABE\n";

#[test]
fn an_initiative_and_its_efforts_are_filed_in_one_step() {
    let world = World::new();
    let mac = world.machine("mac");
    // The spike that planned them is this session's effort, and stays so.
    assert!(mac.bash("s1", NEW).1);
    let before = mac.log().lines().count();
    let (out, ok) = mac.bash("s1", INITIATIVE);
    assert!(ok, "{out}");
    for line in [
        "Initiative \"Smooth ink\" filed as initiatives/smooth-ink.md with 2 efforts",
        "0002 Tether cap\n0003 Goo at speed",
        "This session stays on effort 1",
        "--initiative \"Smooth ink\"",
    ] {
        assert!(out.contains(line), "{line}\n{out}");
    }
    let tether = mac.effort_text(2);
    for line in [
        "status: proposed",
        "repo: demo",
        "initiative: Smooth ink",
        "involvement: check in at plan",
        "authority: edit",
        "saved_by: mac · claude",
        "session: s1",
        "# 0002 Tether cap",
        "## Current state\nCap the tether so strokes stop splitting.\n",
    ] {
        assert!(tether.contains(line), "{line}\n{tether}");
    }
    let goo = mac.effort_text(3);
    assert!(
        goo.contains("# 0003 Goo at speed")
            && goo.contains("## Current state\nKeep goo stable at speed,\ntwo frames at most.\n"),
        "{goo}"
    );
    let page = std::fs::read_to_string(mac.notes.join("initiatives/smooth-ink.md")).unwrap();
    assert!(
        page.starts_with("---\nstatus: ready\nrepo: demo\ncreated: 20"),
        "{page}"
    );
    let list = format!(
        "# Smooth ink\n\nMake ink smooth across frames, with café in it.\n\n## Efforts and order\n\n1. [Tether cap](../{}). Cap the tether so strokes stop splitting.\n2. [Goo at speed](../{}). Keep goo stable at speed, two frames at most.\n\n## What completion means\n- strokes never split\n",
        shelf("0002-tether-cap.md"),
        shelf("0003-goo-at-speed.md")
    );
    assert!(page.ends_with(&list), "{page}");
    // One commit for the lot, no journal line, and the session still on the spike.
    assert_eq!(mac.log().lines().count(), before + 1);
    assert_eq!(
        mac.log().lines().next(),
        Some("initiative Smooth ink: 2 efforts filed")
    );
    assert!(!mac.notes.join("journal").exists());
    // The page is in that commit, not left untracked for the sync to miss.
    assert!(
        git(&mac.notes, &["status", "--porcelain", "--", "initiatives"])
            .trim()
            .is_empty(),
        "the page is committed"
    );
    let state: Value =
        serde_json::from_slice(&std::fs::read(mac.state().join("s1.json")).unwrap()).unwrap();
    assert_eq!(state["effort"], "1");
    // The board groups them under the name.
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--all"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let grouped = v["efforts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["initiative"] == "Smooth ink")
        .count();
    assert_eq!(grouped, 2, "{out}");
    // The spike joins it and closes as their record; the next effort joins by name.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe effort save --status done --initiative \"Smooth ink\" <<'ASTROLABE'\nDone: the plan is filed as its efforts.\n\nNothing left here.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.effort_text(1).contains("initiative: Smooth ink"));
    // The same command typed again with no hook explains itself from the ticket.
    let (plain, ok) = mac.plain(INITIATIVE);
    assert!(
        ok && plain.contains("filed as initiatives/smooth-ink.md"),
        "{plain}"
    );
    // Filed once: the page is the page.
    let (out, ok) = mac.bash(
        "s1",
        "astrolabe initiative new --title \"Smooth ink\" <<'ASTROLABE'\nAgain.\n\n## Efforts\nMore\nmore goal\nASTROLABE\n",
    );
    assert!(!ok && out.contains("exists already"), "{out}");
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 3);
}

#[test]
fn a_refused_initiative_files_nothing() {
    let world = World::new();
    let mac = world.machine("mac");
    assert!(mac.bash("s1", NEW).1);
    let before = mac.log().lines().count();
    for (cmd, why) in [
        (
            "astrolabe initiative new --title \"No efforts\" <<'ASTROLABE'\nPurpose only.\nASTROLABE\n",
            "`## Efforts` section",
        ),
        (
            "astrolabe initiative new --title \"No goal\" <<'ASTROLABE'\n## Efforts\nTitle only\nASTROLABE\n",
            "has no goal",
        ),
        (
            "astrolabe initiative new --title \"Labelled\" <<'ASTROLABE'\n## Efforts\nA\n**Goal:** g.\nASTROLABE\n",
            "no **Label:** lines",
        ),
        (
            "astrolabe initiative new --title \"Flag\" --status proposed <<'ASTROLABE'\n## Efforts\nA\ng.\nASTROLABE\n",
            "isn't a flag of `initiative new`",
        ),
        (
            "astrolabe initiative new <<'ASTROLABE'\n## Efforts\nA\ng.\nASTROLABE\n",
            "--title",
        ),
        ("astrolabe initiative new --title \"No body\"", "## Efforts"),
    ] {
        let (out, ok) = mac.bash("s1", cmd);
        assert!(!ok && out.contains(why), "{cmd}: {out}");
    }
    // A worker files nothing; a subagent neither.
    assert!(mac.bash("w1", "astrolabe activate --effort 1 --worker").1);
    let (out, ok) = mac.bash("w1", INITIATIVE);
    assert!(!ok && out.contains("worker under effort 1"), "{out}");
    let (out, ok) = mac.bash_as("s1", INITIATIVE, Some("child-1"));
    assert!(!ok && out.contains("subagent"), "{out}");
    assert_eq!(mac.log().lines().count(), before);
    assert_eq!(astrolabe::notes::effort_files(&mac.notes).len(), 1);
    assert!(!mac.notes.join("initiatives").exists());
    // With no hook, nothing is filed and the command says so.
    let (plain, ok) = mac.plain(INITIATIVE);
    assert!(!ok && plain.contains("NOT recorded"), "{plain}");
}
