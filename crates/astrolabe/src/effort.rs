//! Effort commands and the note format they update.
pub(crate) mod command;
mod file;
mod initiative;
mod refusal;
pub mod roster;
pub use command::{DELIMITER, EffortCommand, FORMS, INITIATIVE_FLAGS, NEW_FLAGS, SAVE_FLAGS};
pub use file::{EffortFile, plain_goal, slug};
pub(crate) use file::{
    fence_mask, frontmatter, header_bounds, piece_box, piece_key, split_frontmatter,
};
pub use refusal::{form, refusal};

/// What an effort's `involvement` may be: how often the owner wants to be asked.
pub(crate) const INVOLVEMENTS: [&str; 4] = [
    "autonomous",
    "check in on bigger plans",
    "check in at plan",
    "each step",
];

/// The involvement a new effort or initiative gets when the owner names none: the owner chose
/// a plan first on bigger work only (2026-11-18). Notes keep whatever value is written on them.
pub(crate) const DEFAULT_INVOLVEMENT: &str = INVOLVEMENTS[1];

/// What an effort's `authority` may be: how far an agent's changes may go.
pub(crate) const AUTHORITIES: [&str; 3] = ["read", "edit", "push"];

/// The event that finishes an effort when its filing named none, and when a note filed
/// before the property existed is read.
pub const FINISH_DEFAULT: &str = "done when merged";

/// The forms `--finish` (and an initiative member's `finish:` line) take, as a refusal
/// lists them.
pub(crate) const FINISH_FORMS: &str = "merged (the default; under an initiative with an integration branch, merged into that branch), main (merged into main even under an integration branch), look: <what the owner looks at after the merge>, findings (nothing to land; done when the findings are written) or accepted (nothing to land; done when the owner accepts the findings)";

/// The finish line a filing form names, as the note keeps it: one line naming the event,
/// so whoever reads the note can tell when it has happened. Under an initiative with an
/// integration branch, a merge means a merge into that branch, because its members wait
/// in review with their worktrees until main otherwise (effort 502); `main` keeps the
/// merge into main for a member that has to reach it on its own.
pub(crate) fn finish_line(form: &str, integration: Option<&str>) -> Result<String, String> {
    let form = form.trim();
    let merged = match integration {
        Some(branch) => format!("done when merged into {branch}"),
        None => FINISH_DEFAULT.to_owned(),
    };
    match form {
        "merged" => Ok(merged),
        "main" => Ok(FINISH_DEFAULT.to_owned()),
        "findings" => Ok("nothing to land; done when the findings are written".to_owned()),
        "accepted" => Ok("nothing to land; done when the owner accepts the findings".to_owned()),
        _ => match form.strip_prefix("look:").map(str::trim) {
            Some(what) if !what.is_empty() && !what.contains(['\n', '\r']) => {
                Ok(match integration {
                    Some(_) => format!("{merged} and the owner has looked at {what} there"),
                    None => format!("{merged} and the owner has looked at {what}"),
                })
            }
            _ => Err(format!(
                "finish {form:?} isn't a finish line; use {FINISH_FORMS}"
            )),
        },
    }
}

/// Whether a name can be an integration branch as a note keeps it: one git ref name on one
/// line, so a finish line that names it reads back the same.
pub(crate) fn integration_branch(name: &str) -> Result<String, String> {
    let name = name.trim();
    let bad = name.is_empty()
        || name.starts_with(['-', '/', '.'])
        || name.ends_with(['/', '.'])
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("@{")
        || name.contains("//")
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\\"'`".contains(c));
    if bad {
        return Err(format!(
            "integration {name:?} isn't a branch name: give the playtest branch's name as git knows it, such as playtest/levels"
        ));
    }
    Ok(name.to_owned())
}

pub use initiative::{
    Initiative, InitiativePage, initiative_catalog, initiative_members, initiative_name,
    resolve_initiative, set_initiative_property,
};
