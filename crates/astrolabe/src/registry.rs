//! Local repo registration helpers; records live in the plugin's state folder.
use std::path::Path;

pub const AGENT: &str = "agent";

/// Canonical folder name without the Windows verbatim prefix, for a local repo record.
pub fn native(path: &Path) -> String {
    let path = crate::hook::comparable(path);
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
}
