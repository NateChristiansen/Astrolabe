//! An initiative can name its integration branch, the playtest branch its members merge
//! into. Its members finish on that merge, and a member its orchestrator missed closing is
//! flagged where the next reader looks: in one initiative, ten merged members sat in review for
//! days with their worktrees because their finish line was main (effort 502).
use super::*;

const TOWER: &str = "astrolabe initiative new --title \"Tower\" --integration playtest/tower <<'ASTROLABE'\nThe tower, piece by piece.\n\n## Efforts\nWinch\nBuild the winch.\n\nRopes\nDraw the ropes.\nfinish: look: the rope sag\n\nRelease note\nWrite the release note.\nfinish: main\n\nSurvey\nSurvey the rooms.\nfinish: findings\nASTROLABE\n";

fn finish_of(mac: &Machine, n: u32) -> String {
    let text = mac.effort_text(n);
    text.lines()
        .find_map(|l| l.strip_prefix("finish: "))
        .unwrap_or_default()
        .to_owned()
}

/// Catches an integration branch that isn't kept on the page, members that still finish on
/// main, a `look:` that doesn't say where to look, `main` not opting out, and a member filed
/// later under the page missing the branch.
#[test]
fn members_of_an_initiative_with_an_integration_branch_finish_on_its_merge() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash("s1", TOWER);
    assert!(ok, "{out}");
    // The rule comes with the filing, since the session head has no room for it.
    assert!(
        out.contains("Members merge into the integration branch playtest/tower. When a member's branch merges into it, in that same step close the member")
            && out.contains("retire its worktree"),
        "{out}"
    );
    let page = std::fs::read_to_string(mac.notes.join("initiatives/tower.md")).unwrap();
    assert!(page.contains("integration: playtest/tower\n"), "{page}");
    assert_eq!(finish_of(&mac, 1), "done when merged into playtest/tower");
    assert_eq!(
        finish_of(&mac, 2),
        "done when merged into playtest/tower and the owner has looked at the rope sag there"
    );
    assert_eq!(finish_of(&mac, 3), "done when merged");
    assert_eq!(
        finish_of(&mac, 4),
        "nothing to land; done when the findings are written"
    );
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Grate bell\" --initiative tower <<'ASTROLABE'\nRing the grate bell.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert_eq!(finish_of(&mac, 5), "done when merged into playtest/tower");
    // Without the flag nothing changes.
    let (out, ok) = mac.bash(
        "s3",
        "astrolabe effort new --title \"Loose\" <<'ASTROLABE'\nA loose end.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert_eq!(finish_of(&mac, 6), "done when merged");
}

/// Catches a branch name that couldn't be read back as one being written, and the page's
/// own flags being taken on an effort's save.
#[test]
fn a_bad_integration_branch_or_a_page_flag_on_an_effort_is_refused() {
    let world = World::new();
    let mac = world.machine("mac");
    let before = mac.log().lines().count();
    for name in ["play test", "-x", "a..b", "x.lock", "ends/"] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe initiative new --title \"Bad\" --integration \"{name}\" <<'ASTROLABE'\n## Efforts\nOne\nDo one.\nASTROLABE\n"),
        );
        assert!(!ok && out.contains("isn't a branch name"), "{name}: {out}");
    }
    assert_eq!(mac.log().lines().count(), before);
    assert!(mac.bash("s1", NEW).1);
    for flag in ["--integration playtest", "--playtested abcdef1"] {
        let (out, ok) = mac.bash(
            "s1",
            &format!("astrolabe effort save {flag} <<'ASTROLABE'\nStill going.\nASTROLABE\n"),
        );
        assert!(
            !ok && out.contains("belong to an initiative's page"),
            "{flag}: {out}"
        );
    }
}

/// Catches a member merged into the integration branch but left in review going unflagged,
/// a flag before the merge, the owner's-look wait being flagged as a miss, and a member
/// still waiting on that look after the owner played a commit holding it going unflagged.
#[test]
fn a_member_its_orchestrator_missed_closing_is_flagged() {
    let world = World::new();
    let mac = world.machine("mac");
    // The demo repo is known by its note's remote and never registered, as most repos are:
    // the folder the reader is in answers.
    git(&mac.demo, &["branch", "playtest/tower"]);
    assert!(mac.bash("lead", TOWER).1);
    // The worker builds on its own branch and saves in review from there.
    git(&mac.demo, &["checkout", "-q", "-b", "winch"]);
    git(&mac.demo, &["commit", "-q", "--allow-empty", "-m", "winch"]);
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe activate --effort 1 --worker --model opus --reasoning medium",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash(
        "w1",
        "astrolabe effort save --status review --waits-for \"dispatcher: review and land\" <<'ASTROLABE'\nIn review: the winch is built.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert!(mac.bash("lead", "astrolabe activate --initiative tower").1);
    let (out, ok) = mac.bash("lead", "astrolabe effort read 1");
    assert!(
        ok && !out.contains("Missed close")
            && out.contains("Members merge into the integration branch playtest/tower."),
        "{out}"
    );
    // The orchestrator merges it into the playtest branch and doesn't close it.
    git(&mac.demo, &["checkout", "-q", "playtest/tower"]);
    git(&mac.demo, &["merge", "-q", "--ff-only", "winch"]);
    let (out, ok) = mac.bash("lead", "astrolabe effort read 1");
    assert!(
        ok && out.contains("Missed close: Merged into playtest/tower but still in review"),
        "{out}"
    );
    let (out, ok) = mac.run(&mac.demo, &["efforts", "--repo", "demo"], "");
    assert!(ok, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let row = (v["efforts"].as_array().unwrap().iter())
        .find(|e| e["number"] == 1)
        .unwrap()
        .clone();
    assert!(
        row["missed"]
            .as_str()
            .is_some_and(|m| m.starts_with("Merged into playtest/tower")),
        "{row}"
    );
    // The board is drawn on the next write, from the folder that write was made in.
    let (out, ok) = mac.bash(
        "other",
        "astrolabe effort new --title \"Unrelated\" <<'ASTROLABE'\nSomething else.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let board = std::fs::read_to_string(mac.notes.join("home.html")).unwrap();
    assert!(
        board.contains("<p class=\"missed\">Merged into playtest/tower"),
        "{board}"
    );
    // Waiting on the owner's look is the allowed state, until the owner has played it.
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --effort 1 --waits-for \"owner: the winch on playtest/tower\" <<'ASTROLABE'\nIn review: merged into the playtest branch, waiting on my look.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("lead", "astrolabe effort read 1");
    assert!(ok && !out.contains("Missed close"), "{out}");
    let played = git(&mac.demo, &["rev-parse", "playtest/tower"]);
    let (out, ok) = mac.bash(
        "lead",
        &format!(
            "astrolabe effort save --playtested {} <<'ASTROLABE'\nI played the tower.\nASTROLABE\n",
            played.trim()
        ),
    );
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(mac.notes.join("initiatives/tower.md")).unwrap();
    assert!(
        page.contains(&format!(" at {}\n", played.trim().to_ascii_lowercase())),
        "{page}"
    );
    let (out, ok) = mac.bash("lead", "astrolabe effort read 1");
    assert!(
        ok && out.contains("Missed close: The owner playtested playtest/tower on"),
        "{out}"
    );
    // Closed, it's flagged no more.
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --effort 1 --status done <<'ASTROLABE'\nDone: merged into the playtest branch and played.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let (out, ok) = mac.bash("lead", "astrolabe effort read 1");
    assert!(
        ok && !out.contains("Missed close") && !out.contains("Members merge into"),
        "{out}"
    );
}

/// Catches an orchestrator unable to name the branch after filing, or to change it.
#[test]
fn an_orchestrator_can_set_the_integration_branch_on_a_filed_page() {
    let world = World::new();
    let mac = world.machine("mac");
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe initiative new --title \"Ink\" <<'ASTROLABE'\nSmooth ink.\n\n## Efforts\nTether cap\nCap the tether.\nASTROLABE\n",
    );
    assert!(
        ok && out.contains("name that branch with `astrolabe effort save --integration <branch>`"),
        "{out}"
    );
    assert!(mac.bash("lead", "astrolabe activate --initiative ink").1);
    let (out, ok) = mac.bash(
        "lead",
        "astrolabe effort save --integration playtest/ink <<'ASTROLABE'\nMembers merge into the ink playtest branch.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    let page = std::fs::read_to_string(mac.notes.join("initiatives/ink.md")).unwrap();
    assert!(page.contains("integration: playtest/ink\n"), "{page}");
    let (out, ok) = mac.bash(
        "s2",
        "astrolabe effort new --title \"Goo\" --initiative ink <<'ASTROLABE'\nTime the goo.\nASTROLABE\n",
    );
    assert!(ok, "{out}");
    assert_eq!(finish_of(&mac, 2), "done when merged into playtest/ink");
}
