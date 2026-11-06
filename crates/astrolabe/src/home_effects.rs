//! Board persistence and server startup after completed note operations.
use crate::notes::{Notes, write_text};
use std::path::{Path, PathBuf};

/// The board follows every change to the checkout, drawn once the change is whole:
/// after an op's result is recorded, after a pull that brought commits, after a sync.
/// Under the caller's lock. A failure is recorded for the status and session text,
/// never raised: the notes are the record, the board is a view of them.
pub fn redraw(notes: &Notes) {
    if let Err(why) = draw(notes) {
        notes.record_sync("board-failed", &why);
    }
    // Every change is a chance to have the home up, so the owner never starts it.
    crate::home::ensure_serving(notes);
}

/// The board, redrawn by hand: `astrolabe home`.
pub fn board(notes: &Notes) -> Result<PathBuf, String> {
    let _held = notes.lock()?;
    draw(notes).inspect_err(|why| notes.record_sync("board-failed", why))
}

/// An earlier failure is cleared first, so a board that draws never shows it.
fn draw(notes: &Notes) -> Result<PathBuf, String> {
    notes.clear_sync("board-failed");
    ensure_ignored(notes)?;
    crate::home::write(notes)
}

/// The board is drawn per machine, so it stays out of git through the checkout's own
/// exclude file, never a committed `.gitignore`: nothing to stage, nothing that can
/// leave the tree dirty, and the owner's ignore rules are untouched.
fn ensure_ignored(notes: &Notes) -> Result<(), String> {
    let exclude = notes.git(&["rev-parse", "--git-path", "info/exclude"])?;
    let exclude = exclude.trim();
    let path = if Path::new(exclude).is_absolute() {
        PathBuf::from(exclude)
    } else {
        notes.root.join(exclude)
    };
    let have = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let line = format!("/{}", crate::home::FILE);
    if !have.lines().any(|l| l.trim() == line) {
        let mut text = have;
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        write_text(&path, &text)?;
    }
    // A `.gitignore` negation outranks the exclude file; then the board isn't drawn,
    // because an unignored board is one `git add .` from origin.
    if notes
        .git(&["check-ignore", "-q", "--no-index", "--", crate::home::FILE])
        .is_err()
    {
        return Err(format!(
            "{} isn't ignored in the notes checkout (a .gitignore rule overrides the exclude file), so it isn't drawn; drop that rule and the board is drawn at the next save",
            crate::home::FILE
        ));
    }
    Ok(())
}
