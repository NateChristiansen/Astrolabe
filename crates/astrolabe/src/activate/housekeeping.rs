//! Standalone notes housekeeping runs in the hook, with a read-only result for the app.
use super::{routing, state_dir, tickets};
use crate::record::lesson;
use serde_json::Value;

pub(super) fn command(input: &Value, session: Option<&str>, child: bool) -> Option<Value> {
    let tool_input = &input["tool_input"];
    let key = crate::guard::command_key(tool_input);
    let command = tool_input[key].as_str()?;
    if !lesson::claims(command) {
        return None;
    }
    let session = session?;
    let parsed = lesson::parse(command);
    let closing = parsed.as_ref().is_ok_and(|args| args[1] == "lesson");
    if closing
        && (child
            || state_dir()
                .and_then(|dir| super::read_state(&dir, session).ok().flatten())
                .is_some_and(|s| s.worker.is_true()))
    {
        return Some(crate::hook::deny(
            "A worker leaves notes housekeeping to its dispatcher.",
        ));
    }
    let cwd = crate::hook::folder(input).unwrap_or_else(|| ".".into());
    let powershell = super::proof::powershell_input(input);
    let mut op_key = None;
    let mut digest = None;
    let (text, ok) = match parsed {
        Ok(args) if args[1] == "notes" => {
            match crate::record::notes_here()
                .and_then(|(_, notes)| crate::application::request_sync(&notes))
            {
                Ok(text) => (text, true),
                Err(why) => (why, false),
            }
        }
        Ok(args) => {
            let hash = crate::notes::hash(&format!("lesson\0{}\0{}", args[3], args[5]));
            let key = match input["tool_use_id"].as_str().filter(|id| !id.is_empty()) {
                Some(id) => crate::notes::hash(&format!("{session} {id} {hash}")),
                None => super::random_hex().unwrap_or_else(|_| hash.clone()),
            };
            let done = lesson::close(&args[3], &args[5], &key, &hash, &cwd);
            op_key = Some(key);
            digest = Some(hash);
            (done.text, done.ok)
        }
        Err(why) => (
            format!("Notes command refused: {}.", why.trim_end_matches('.')),
            false,
        ),
    };
    let left = state_dir()
        .ok_or_else(|| "no home folder".to_owned())
        .and_then(|dir| tickets::leave_with(&dir, &text, ok, digest.as_deref(), Some(&cwd)));
    Some(match left {
        Ok(ticket) => {
            if let (Some(key), true) = (op_key, ok) {
                crate::record::forget(&key);
            }
            routing::rewrite_to_show(tool_input, key, &ticket, powershell)
        }
        Err(why) => crate::hook::deny(&format!(
            "Astrolabe couldn't show the notes command's result ({why}); run astrolabe notes status."
        )),
    })
}
