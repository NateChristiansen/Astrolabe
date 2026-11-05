//! Local repo registration helpers; records live in the plugin's state folder.
use std::path::Path;

pub const AGENT: &str = "agent";

/// Canonical folder name without the Windows verbatim prefix, for a local repo record.
pub fn native(path: &Path) -> String {
    strip_verbatim(&crate::hook::comparable(path))
}

/// A canonical Windows path as git and the owner write it: `\\?\C:\x` is `C:\x`, and
/// `\\?\UNC\host\share` is `\\host\share`. Any other text is left as it is.
fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
}

#[cfg(test)]
mod tests {
    use super::strip_verbatim;

    #[test]
    fn verbatim_prefixes_keep_a_real_path() {
        assert_eq!(strip_verbatim(r"\\?\C:\x"), r"C:\x");
        assert_eq!(strip_verbatim(r"\\?\UNC\host\share\x"), r"\\host\share\x");
        assert_eq!(strip_verbatim("/x"), "/x");
    }
}
