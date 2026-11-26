//! A member of an initiative with an integration branch that its orchestrator missed
//! closing. In one initiative, ten members merged into the playtest branch sat in review for days
//! with their worktrees, because nothing closed them on the merge or after the owner's
//! playtest (effort 502). The rule says to close them; this catches a close that didn't
//! happen, so it holds without anyone remembering.
use crate::effort::EffortFile;
use std::ffi::OsStr;
use std::path::Path;

/// Whether `commit` is in `branch` in the checkout: yes, no, or unknown when git couldn't
/// say (the commit or branch isn't there, or git failed). The local branch is asked first,
/// then the remote's copy, since a playtest branch can live only on the remote here.
fn contains(checkout: &Path, branch: &str, commit: &str) -> Option<bool> {
    let mut answer = None;
    for target in [branch.to_owned(), format!("refs/remotes/origin/{branch}")] {
        let args: [&OsStr; 4] = [
            "merge-base".as_ref(),
            "--is-ancestor".as_ref(),
            commit.as_ref(),
            target.as_ref(),
        ];
        match crate::worktree::git_or(crate::deadline::Git::Read, checkout, &args) {
            Ok(_) => return Some(true),
            // Exit 1 is git's "not an ancestor"; anything else means it couldn't tell.
            Err(failed) if failed.code == Some(1) => answer = Some(false),
            Err(_) => {}
        }
    }
    answer
}

/// The orchestrator's rule for a page with an integration branch. It's given where it's
/// acted on, filing the page and reading a member, because the initiative's session head
/// is at the hook's limit.
pub(crate) fn rule(branch: &str) -> String {
    format!(
        "Members merge into the integration branch {branch}. When a member's branch merges into it, in that same step close the member, or keep it in review waiting on \"owner: <what to look at> on {branch}\" when its finish line names the owner's look, and retire its worktree, because otherwise members sit in review with their worktrees until {branch} reaches main. After the owner playtests {branch}, record the commit they played with `astrolabe effort save --playtested <sha>` on the page and close every member that playtest covered."
    )
}

/// The integration branch a page names, if any.
pub(crate) fn integration(page: &EffortFile) -> Option<&str> {
    page.prop("integration")
        .map(str::trim)
        .filter(|b| !b.is_empty())
}

/// The commit of the integration branch the owner last playtested, and the day, from the
/// page's `playtested: <day> at <commit>`.
fn playtested(page: &EffortFile) -> Option<(&str, &str)> {
    let (day, commit) = page.prop("playtested")?.split_once(" at ")?;
    Some((day.trim(), commit.trim())).filter(|(_, c)| !c.is_empty())
}

/// What a member's note shows the orchestrator missed, if anything, read with git in the
/// member's repo checkout: still in review though its work is in the integration branch, or
/// still waiting on the owner's look though the owner has since played a commit that holds
/// it. None when there's nothing to flag or git can't tell.
pub(crate) fn missed(member: &EffortFile, page: &EffortFile, checkout: &Path) -> Option<String> {
    let branch = integration(page)?;
    let status = member.prop("status").unwrap_or_default();
    if matches!(status, "done" | "abandoned" | "proposed" | "ready") {
        return None;
    }
    // The commit the member last saved from, else its branch while that branch still exists.
    let commit = member
        .prop("head")
        .or_else(|| member.prop("branch"))
        .map(str::trim)
        .filter(|c| !c.is_empty())?;
    let waits_on_owner = member
        .prop("waits_for")
        .is_some_and(|w| w.starts_with("owner:"));
    if waits_on_owner {
        let (day, played) = playtested(page)?;
        let covered = contains(checkout, played, commit)?;
        return covered.then(|| format!(
            "The owner playtested {branch} on {day} at {played}, which holds this member's work, and it still waits on their look: close it if that playtest covered it."
        ));
    }
    if status != "review" {
        return None;
    }
    contains(checkout, branch, commit)?.then(|| format!(
        "Merged into {branch} but still in review: close it, or set it waiting on the owner's look on {branch}, and retire its worktree."
    ))
}

/// The member's repo checkout on this machine: the one registered for it, else `folder`
/// when that's a checkout or worktree of it. Repos are mostly known by their notes' remotes
/// and never registered, so the folder the reader is in is most often the one that answers.
fn checkout_for(
    repo: &str,
    repos: &crate::activate::Repos,
    notes: &Path,
    folder: Option<&Path>,
) -> Option<std::path::PathBuf> {
    if let Some(checkout) = repos.checkout(repo) {
        return Some(checkout);
    }
    let folder = folder?;
    let here = crate::activate::common_dir(folder)?;
    let names = crate::activate::repo_names(notes, repos, folder, &here).ok()?;
    names
        .iter()
        .any(|n| n == repo)
        .then(|| folder.to_path_buf())
}

/// The integration branch of the page an open member belongs to, if that page names one.
pub(crate) fn branch_of(
    member: &EffortFile,
    pages: &[crate::effort::Initiative],
) -> Option<String> {
    if matches!(member.prop("status"), Some("done" | "abandoned")) {
        return None;
    }
    let name = member.prop("initiative").filter(|n| !n.is_empty())?;
    let page = crate::effort::resolve_initiative(pages, name).ok()?;
    integration(&page.file).map(str::to_owned)
}

/// The same for a member found in the notes: its page's integration branch and its repo's
/// checkout here. None when it has no such page, no checkout of its repo is at hand, or
/// there's nothing to flag.
pub(crate) fn missed_in_notes(
    member: &EffortFile,
    pages: &[crate::effort::Initiative],
    repos: &crate::activate::Repos,
    notes: &Path,
    folder: Option<&Path>,
) -> Option<String> {
    let name = member.prop("initiative").filter(|n| !n.is_empty())?;
    let page = crate::effort::resolve_initiative(pages, name).ok()?;
    integration(&page.file)?;
    let repo = member.prop("repo")?.split(',').next()?.trim();
    let checkout = checkout_for(repo, repos, notes, folder)?;
    missed(member, &page.file, &checkout)
}
