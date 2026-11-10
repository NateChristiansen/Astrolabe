//! Effort commands and the note format they update.
pub(crate) mod command;
mod file;
mod initiative;
pub mod roster;
pub use command::{DELIMITER, EffortCommand, FORMS, INITIATIVE_FLAGS, NEW_FLAGS, SAVE_FLAGS};
pub use file::{EffortFile, slug};
pub(crate) use file::{fence_mask, frontmatter, header_bounds, split_frontmatter};

/// What an effort's `involvement` may be: how often the owner wants to be asked.
pub(crate) const INVOLVEMENTS: [&str; 3] = ["autonomous", "check in at plan", "each step"];

/// What an effort's `authority` may be: how far an agent's changes may go.
pub(crate) const AUTHORITIES: [&str; 3] = ["read", "edit", "push"];
pub use initiative::{
    Initiative, InitiativePage, initiative_catalog, initiative_members, initiative_name,
    resolve_initiative, set_initiative_property,
};
