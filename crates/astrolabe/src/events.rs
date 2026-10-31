//! Maps ACP-shaped JSON-RPC messages to normalized core events.
use crate::model::ToolStatus;
use serde_json::Value;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnd {
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEvent {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub status: Option<ToolStatus>,
    /// Digest of the tool's reported content; none when the update carries none.
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEvent {
    /// New agent output: a message, a thought or a plan.
    Content,
    Tool(ToolEvent),
    /// The adapter is alive. Says nothing about whether the work progresses.
    Keepalive,
    RequestOpened {
        rpc_id: String,
        title: String,
    },
    TurnEnded(TurnEnd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessage {
    /// Which agent session spoke; a child's differs from its parent's.
    pub agent_session_id: Option<String>,
    pub event: CoreEvent,
}

/// Returns none for messages that carry no live state, such as other responses.
pub fn map_message(message: &Value) -> Option<AgentMessage> {
    let params = message.get("params");
    let agent_session_id = params
        .and_then(|params| text(params, "sessionId"))
        .map(str::to_owned);
    let event = match message.get("method").and_then(Value::as_str) {
        Some("session/update") => map_update(params?.get("update")?)?,
        Some("session/request_permission") => {
            let tool_call = params?.get("toolCall");
            CoreEvent::RequestOpened {
                rpc_id: message.get("id")?.to_string(),
                title: tool_call
                    .and_then(|call| text(call, "title"))
                    .unwrap_or("permission requested")
                    .to_owned(),
            }
        }
        Some(_) => return None,
        None => map_response(message)?,
    };
    Some(AgentMessage {
        agent_session_id,
        event,
    })
}

fn map_update(update: &Value) -> Option<CoreEvent> {
    Some(match text(update, "sessionUpdate")? {
        "agent_message_chunk" | "agent_thought_chunk" | "plan" => CoreEvent::Content,
        "tool_call" | "tool_call_update" => CoreEvent::Tool(ToolEvent {
            tool_call_id: text(update, "toolCallId")?.to_owned(),
            title: text(update, "title").map(str::to_owned),
            status: text(update, "status").and_then(tool_status),
            fingerprint: fingerprint(update),
        }),
        // A replayed user message is not agent progress.
        "user_message_chunk" => return None,
        // Usage and metadata prove only that the adapter is alive. So does any
        // kind this version does not know: an unknown kind must not hide a stall.
        _ => CoreEvent::Keepalive,
    })
}

fn map_response(message: &Value) -> Option<CoreEvent> {
    if message.get("error").is_some() {
        return Some(CoreEvent::TurnEnded(TurnEnd::Failed));
    }
    let reason = text(message.get("result")?, "stopReason")?;
    Some(CoreEvent::TurnEnded(match reason {
        "end_turn" => TurnEnd::Completed,
        "cancelled" => TurnEnd::Cancelled,
        _ => TurnEnd::Failed,
    }))
}

fn tool_status(status: &str) -> Option<ToolStatus> {
    match status {
        "pending" => Some(ToolStatus::Pending),
        "in_progress" => Some(ToolStatus::Running),
        "completed" | "failed" => Some(ToolStatus::Finished),
        _ => None,
    }
}

/// Keepalives may repeat earlier content, so progress means a changed digest
/// rather than a present field.
fn fingerprint(update: &Value) -> Option<String> {
    let reported = ["content", "rawOutput"].map(|field| update.get(field));
    let empty = |value: &Option<&Value>| match value {
        None | Some(Value::Null) => true,
        Some(Value::Array(items)) => items.is_empty(),
        Some(_) => false,
    };
    if reported.iter().all(empty) {
        return None;
    }
    let mut hasher = DefaultHasher::new();
    format!("{reported:?}").hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
