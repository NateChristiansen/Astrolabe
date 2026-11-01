use protocol::Envelope;
use serde_json::{Value, json};
use std::{fs, path::Path};

#[test]
fn fixtures_round_trip_with_extensions() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../protocol/fixtures");
    let mut count = 0;
    for file in fs::read_dir(dir).unwrap() {
        let path = file.unwrap().path();
        // Subdirectories hold ACP recordings, not protocol envelopes.
        if path.is_dir() {
            continue;
        }
        let input: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let envelope: Envelope = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(serde_json::to_value(envelope).unwrap(), input);
        count += 1;
    }
    assert_eq!(count, 3);
}

#[test]
fn discriminator_and_versions_reject_mismatches() {
    let mut input: Value = serde_json::from_str(include_str!(
        "../../../protocol/fixtures/ask-owner-answer.json"
    ))
    .unwrap();
    input["kind"] = json!("review_finding");
    assert!(serde_json::from_value::<Envelope>(input.clone()).is_err());
    input["kind"] = json!("ask_owner_answer");
    input["payload"]["payload_version"] = json!("2");
    assert!(serde_json::from_value::<Envelope>(input.clone()).is_err());
    input["payload"]["payload_version"] = json!("1");
    input["schema_version"] = json!("2");
    assert!(serde_json::from_value::<Envelope>(input).is_err());
}
