//! Read owner and voice from the registered notes checkout, fresh for each session.
use std::path::Path;

/// Both clients took 9,850 bytes whole and silently cut 10,875, so it's refused here.
const LIMIT: usize = 9800;
const HEADER: &str = "Owner and voice files from the agent repo. Samples in them teach style only and are never a source of facts.";

pub fn read(agent: &Path) -> Result<String, String> {
    let mut text = HEADER.to_owned();
    for file in ["knowledge/owner.md", "knowledge/voice.md"] {
        let body = std::fs::read_to_string(agent.join(file));
        let body = body.map_err(|e| format!("{file} in {}: {e}", agent.display()))?;
        text += &format!("\n\n--- {file}\n\n{}", body.trim());
    }
    match text.len() {
        size if size > LIMIT => Err(format!("owner and voice: {size} bytes, limit {LIMIT}")),
        _ => Ok(text),
    }
}
