use protocol::{AskOwnerAnswer, AskOwnerRequest, Envelope, ReviewFinding};
use schemars::{JsonSchema, schema_for};
use std::{error::Error, fs, path::Path};

fn write<T: JsonSchema>(dir: &Path, name: &str) -> Result<(), Box<dyn Error>> {
    let json = serde_json::to_string_pretty(&schema_for!(T))? + "\n";
    fs::write(dir.join(format!("{name}.json")), json)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/schemas");
    fs::create_dir_all(&dir)?;
    write::<Envelope>(&dir, "envelope-v1")?;
    write::<ReviewFinding>(&dir, "review-finding-v1")?;
    write::<AskOwnerRequest>(&dir, "ask-owner-request-v1")?;
    write::<AskOwnerAnswer>(&dir, "ask-owner-answer-v1")?;
    Ok(())
}
