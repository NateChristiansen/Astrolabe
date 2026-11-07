use super::*;
use serde_json::json;

const NOW: i64 = 1_800_000_010_000;
const AFTER: i64 = NOW - 5_000;

fn claude(at: i64) -> Value {
    json!({"type":"user", "sessionId":"s1", "origin":{"kind":"human"},
        "timestamp":chrono::DateTime::from_timestamp_millis(at).unwrap().to_rfc3339()})
}
fn codex(at: i64) -> Value {
    json!({"type":"response_item", "payload":{"type":"message", "role":"user",
        "internal_chat_message_metadata_passthrough":{"create_time": at as f64 / 1000.0,
        "content_item_metadata":[{"provenance":{"type":"user"}}]}}})
}

#[test]
fn only_explicit_human_provenance_with_original_time_counts() {
    assert_eq!(human_time(&claude(NOW), "claude", "s1"), Some(NOW));
    assert_eq!(human_time(&codex(NOW), "codex", "s1"), Some(NOW));
    for origin in ["peer", "task-notification", "unknown", ""] {
        let mut record = claude(NOW);
        record["origin"]["kind"] = origin.into();
        assert_eq!(human_time(&record, "claude", "s1"), None);
    }
    let mut record = claude(NOW);
    record["origin"] = Value::Null;
    assert_eq!(human_time(&record, "claude", "s1"), None);
    assert_eq!(human_time(&claude(NOW), "claude", "other"), None);
    record = claude(NOW);
    record["isSidechain"] = true.into();
    assert_eq!(human_time(&record, "claude", "s1"), None);
    record = json!({"type":"attachment", "sessionId":"s1", "timestamp":"2027-01-15T08:00:10Z",
        "attachment":{"type":"queued_command", "origin":{"kind":"human"},
        "timestamp":claude(AFTER)["timestamp"]}});
    assert_eq!(human_time(&record, "claude", "s1"), Some(AFTER));
    record["attachment"]["timestamp"] = Value::Null;
    assert_eq!(human_time(&record, "claude", "s1"), None);
    for provenance in ["harness", "agents_md", "unknown"] {
        let mut record = codex(NOW);
        record["payload"]["internal_chat_message_metadata_passthrough"]["content_item_metadata"]
            [0]["provenance"]["type"] = provenance.into();
        assert_eq!(human_time(&record, "codex", "s1"), None);
    }
    record = codex(AFTER);
    record["timestamp"] = claude(NOW)["timestamp"].clone();
    assert_eq!(human_time(&record, "codex", "s1"), Some(AFTER));
    record["payload"]["internal_chat_message_metadata_passthrough"]["create_time"] = Value::Null;
    assert_eq!(human_time(&record, "codex", "s1"), None);
    assert_eq!(human_time(&codex(NOW), "unknown", "s1"), None);
}

#[test]
fn transcript_is_bounded_complete_and_bound_to_the_session() {
    let dir = std::env::temp_dir().join(format!("astrolabe-reply-unit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("transcript.jsonl");
    for provider in ["claude", "codex"] {
        let header = if provider == "codex" {
            format!(
                "{}\n",
                json!({"type":"session_meta", "payload":{"id":"s1"}})
            )
        } else {
            String::new()
        };
        for (at, expected) in [
            (AFTER - 1, false),
            (AFTER, false),
            (AFTER + 1, true),
            (NOW, true),
            (NOW + 1, false),
        ] {
            let record = if provider == "codex" {
                codex(at)
            } else {
                claude(at)
            };
            std::fs::write(&path, format!("{header}{record}\n")).unwrap();
            assert_eq!(has_reply(&path, provider, "s1", AFTER, NOW), Some(expected));
            assert!(!has_reply(&path, provider, "other", AFTER, NOW).unwrap_or(false));
        }
        let record = if provider == "codex" {
            codex(NOW)
        } else {
            claude(NOW)
        };
        // An unflushed last record, even valid JSON, is retried by a later hook.
        std::fs::write(&path, format!("{header}{record}")).unwrap();
        assert!(!has_reply(&path, provider, "s1", AFTER, NOW).unwrap_or(false));
        std::fs::write(
            &path,
            format!("{header}{record}\n{}\n", "x".repeat(WINDOW as usize)),
        )
        .unwrap();
        assert!(!has_reply(&path, provider, "s1", AFTER, NOW).unwrap_or(false));
        // Partial first record and malformed lines do not hide a complete recent reply.
        std::fs::write(
            &path,
            format!(
                "{header}{}\nnot json\n{record}\n",
                "x".repeat(WINDOW as usize)
            ),
        )
        .unwrap();
        assert_eq!(has_reply(&path, provider, "s1", AFTER, NOW), Some(true));
        assert_eq!(has_reply(&path, provider, "s1", NOW + 1, NOW), None);
    }
    std::fs::remove_file(&path).unwrap();
    assert_eq!(has_reply(&path, "claude", "s1", AFTER, NOW), None);
    std::fs::remove_dir(&dir).unwrap();
}
