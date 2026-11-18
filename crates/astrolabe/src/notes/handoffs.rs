//! The hand-off files a save carries: briefs, reports and small evidence agents write for
//! each other under `.handoffs/`. Without this they sat on one machine until someone ran git
//! by hand in the notes. A save takes only the files it owns, because many workers write in
//! one initiative folder at once and a half-written file of another's must not ride along.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

pub const FOLDER: &str = ".handoffs";
/// Every brief, report and review there was when this was set fits (the largest 218 KiB);
/// what doesn't is logs, ledgers and raw measurements, which stay where they were written.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Git's own test for binary: a NUL in the first 8000 bytes.
const SNIFF: usize = 8000;
/// Leftovers named in a reply before the rest are only counted.
const NAMED: usize = 5;

/// Which hand-off files a save owns, relative to the notes folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoffs {
    /// `.handoffs` or `.handoffs/<initiative slug>`.
    pub folder: String,
    /// Only the folder's entries named for this effort number, or all of them.
    pub number: Option<u32>,
}

impl Handoffs {
    /// An initiative's page: its whole folder.
    pub fn initiative(slug: &str) -> Self {
        Self {
            folder: format!("{FOLDER}/{slug}"),
            number: None,
        }
    }

    /// An effort: the entries named for it in its initiative's folder, or with none, the
    /// folders named for it at the top.
    pub fn effort(number: u32, initiative: Option<&str>) -> Self {
        Self {
            folder: match initiative {
                Some(slug) => format!("{FOLDER}/{slug}"),
                None => FOLDER.to_owned(),
            },
            number: Some(number),
        }
    }

    /// Whether `path`, relative to the notes folder with `/` between parts, is this save's.
    /// Loose files at the top of `.handoffs/` are nobody's.
    pub fn owns(&self, path: &str) -> bool {
        let loose = (path.strip_prefix(FOLDER))
            .and_then(|r| r.strip_prefix('/'))
            .is_some_and(|r| !r.contains('/'));
        let Some(inside) = (path.strip_prefix(self.folder.as_str()))
            .and_then(|r| r.strip_prefix('/'))
            .filter(|r| !r.is_empty() && !loose)
        else {
            return false;
        };
        match self.number {
            None => true,
            Some(n) => named_for(inside.split('/').next().unwrap_or(""), n),
        }
    }
}

/// `390-report.md`, `390`, `0390-eval` are named for 390; `3901-x` and `report-390` aren't.
fn named_for(name: &str, n: u32) -> bool {
    let digits = name.len() - name.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && name[..digits].parse::<u32>().ok() == Some(n)
}

/// Why a file the save owns stays out of its commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    Binary,
    Oversized,
}

/// Whether a changed file goes in: a deletion always does; a file only as text under the limit.
/// Unreadable is left without a word: it's no reason to fail a save.
pub fn fits(root: &Path, rel: &str, deleted: bool) -> Result<(), Option<Left>> {
    if deleted {
        return Ok(());
    }
    let mut file = std::fs::File::open(root.join(rel)).map_err(|_| None)?;
    let len = file.metadata().map_err(|_| None)?.len();
    if len > MAX_BYTES {
        return Err(Some(Left::Oversized));
    }
    let mut head = vec![0; SNIFF];
    let mut read = 0;
    while read < SNIFF {
        match file.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(k) => read += k,
            Err(_) => return Err(None),
        }
    }
    match head[..read].contains(&0) {
        true => Err(Some(Left::Binary)),
        false => Ok(()),
    }
}

/// The reply's line for files a save owns but left out, or None.
pub fn left_line(left: &[(String, Left)]) -> Option<String> {
    if left.is_empty() {
        return None;
    }
    let mut named: Vec<String> = (left.iter().take(NAMED))
        .map(|(path, why)| match why {
            Left::Binary => format!("{path} (binary)"),
            Left::Oversized => format!("{path} (over {} KiB)", MAX_BYTES / 1024),
        })
        .collect();
    if left.len() > NAMED {
        named.push(format!("{} more", left.len() - NAMED));
    }
    Some(format!(
        "Hand-off files left out of the commit, so they stay on this machine: {}.",
        named.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_owns_only_entries_named_for_its_number() {
        let h = Handoffs::effort(390, Some("audit"));
        for yes in [
            ".handoffs/audit/390-report.md",
            ".handoffs/audit/390-procedure/run.sh",
            ".handoffs/audit/0390-eval/x.txt",
            ".handoffs/audit/390/notes.md",
        ] {
            assert!(h.owns(yes), "{yes}");
        }
        for no in [
            ".handoffs/audit/391-report.md",
            ".handoffs/audit/3901-report.md",
            ".handoffs/audit/worker-rules.md",
            ".handoffs/audit/report-390.md",
            ".handoffs/other/390-report.md",
            ".handoffs/auditx/390-report.md",
            ".handoffs/390-loose.md",
            "efforts/390.md",
        ] {
            assert!(!h.owns(no), "{no}");
        }
    }

    #[test]
    fn an_effort_with_no_initiative_owns_the_folders_named_for_it_never_a_loose_file() {
        let h = Handoffs::effort(158, None);
        assert!(h.owns(".handoffs/0158-eval/results.md"));
        assert!(h.owns(".handoffs/158/brief.md"));
        assert!(!h.owns(".handoffs/0158-brief.md"));
        assert!(!h.owns(".handoffs/eval-harness/158-x.md"));
    }

    #[test]
    fn an_initiative_owns_its_whole_folder_and_nothing_else() {
        let h = Handoffs::initiative("audit");
        assert!(h.owns(".handoffs/audit/worker-rules.md"));
        assert!(h.owns(".handoffs/audit/391-report.md"));
        assert!(!h.owns(".handoffs/audit-two/x.md"));
        assert!(!h.owns(".handoffs/x.md"));
    }
}
