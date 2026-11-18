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
pub(crate) const INVOLVEMENTS: [&str; 3] = ["autonomous", "check in at plan", "each step"];

/// What an effort's `authority` may be: how far an agent's changes may go.
pub(crate) const AUTHORITIES: [&str; 3] = ["read", "edit", "push"];

/// The event that finishes an effort when its filing named none, and when a note filed
/// before the property existed is read.
pub const FINISH_DEFAULT: &str = "done when merged";

/// The forms `--finish` (and an initiative member's `finish:` line) take, as a refusal
/// lists them.
pub(crate) const FINISH_FORMS: &str = "merged (the default), look: <what the owner looks at after the merge>, findings (nothing to land; done when the findings are written) or accepted (nothing to land; done when the owner accepts the findings)";

/// The finish line a filing form names, as the note keeps it: one line naming the event,
/// so whoever reads the note can tell when it has happened.
pub(crate) fn finish_line(form: &str) -> Result<String, String> {
    let form = form.trim();
    match form {
        "merged" => Ok(FINISH_DEFAULT.to_owned()),
        "findings" => Ok("nothing to land; done when the findings are written".to_owned()),
        "accepted" => Ok("nothing to land; done when the owner accepts the findings".to_owned()),
        _ => match form.strip_prefix("look:").map(str::trim) {
            Some(what) if !what.is_empty() && !what.contains(['\n', '\r']) => Ok(format!(
                "{FINISH_DEFAULT} and the owner has looked at {what}"
            )),
            _ => Err(format!(
                "finish {form:?} isn't a finish line; use {FINISH_FORMS}"
            )),
        },
    }
}
pub use initiative::{
    Initiative, InitiativePage, initiative_catalog, initiative_members, initiative_name,
    resolve_initiative, set_initiative_property,
};
