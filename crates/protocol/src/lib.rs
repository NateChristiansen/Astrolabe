//! Versioned handoff contracts. See the crate README for authorship boundaries.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Extensions are retained, including nested JSON values.
pub type Extensions = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum EnvelopeVersion {
    #[serde(rename = "1")]
    V1,
}

/// Payload versions evolve independently of the envelope version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum PayloadVersion {
    #[serde(rename = "1")]
    V1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Links {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Header {
    pub schema_version: EnvelopeVersion,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
    pub producer: String,
    /// Software-supplied RFC 3339 timestamp in UTC.
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub body: String,
    #[serde(flatten)]
    pub extra: Extensions,
}

/// The discriminator selects exactly one typed payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Envelope {
    ReviewFinding {
        #[serde(flatten)]
        envelope: Header,
        payload: ReviewFinding,
    },
    AskOwnerRequest {
        #[serde(flatten)]
        envelope: Header,
        payload: AskOwnerRequest,
    },
    AskOwnerAnswer {
        #[serde(flatten)]
        envelope: Header,
        payload: AskOwnerAnswer,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Executed,
    Inferred,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Uncalibrated self-report; never use as a probability or approval threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Confidence {
    pub value: f64,
    pub method: String,
    pub model: String,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReviewFinding {
    pub payload_version: PayloadVersion,
    pub revision: String,
    pub file: String,
    /// One-based source line.
    pub line: u32,
    pub trigger: String,
    pub expected: String,
    pub actual: String,
    pub impact: String,
    pub evidence: Evidence,
    pub severity: Severity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Recommendation {
    pub action: String,
    pub reason: String,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskOwnerRequest {
    pub payload_version: PayloadVersion,
    /// Exactly one self-contained question, never a list of questions.
    pub question: String,
    pub context: String,
    pub recommendation: Recommendation,
    pub what_waits: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskOwnerAnswer {
    pub payload_version: PayloadVersion,
    pub request_id: String,
    pub answer: String,
    #[serde(flatten)]
    pub extra: Extensions,
}

/// One local WebSocket between the app and the core. Text frames, one JSON message each.
/// `Auth` must be the first frame; nothing else is read or sent before it succeeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeRequest {
    Auth {
        secret: String,
    },
    Start {
        repo: String,
        provider: BridgeProvider,
    },
    /// Asks for a session's transcript so far.
    Open {
        session: i64,
    },
    Prompt {
        session: i64,
        text: String,
    },
    Answer {
        request: String,
        choice: BridgeChoice,
    },
    /// Closes a request no process can receive any more. Nothing reaches an agent.
    Dismiss {
        request: String,
    },
    KeepWaiting {
        card: i64,
    },
    Stop {
        session: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BridgeProvider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeChoice {
    Allow,
    Reject,
    /// Rejects, then sends the words as the next prompt once the turn has ended.
    RejectAndSay {
        words: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeEvent {
    /// Everything the window shows except transcripts. Sent whole on connect and on change.
    State {
        state: BridgeState,
    },
    Transcript {
        session: i64,
        items: Vec<TranscriptItem>,
    },
    Item {
        session: i64,
        item: TranscriptItem,
    },
    /// The core's own words about something it could not do.
    Problem {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BridgeState {
    pub repos: Vec<String>,
    pub providers: Vec<BridgeProvider>,
    pub sessions: Vec<SessionRow>,
    pub plain: Vec<PlainRow>,
    /// What this core's restart decided, when it decided anything.
    pub restart: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    CouldNotStart,
    Working,
    NeedsYou,
    YourTurn,
    Ended,
    Failed,
    Interrupted,
    StillRunning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionRow {
    pub id: i64,
    pub repo: String,
    pub provider: BridgeProvider,
    pub title: Option<String>,
    pub state: SessionState,
    /// Why it could not start or failed, in the core's own words.
    pub why: Option<String>,
    pub can_prompt: bool,
    pub can_stop: bool,
    pub requests: Vec<ApprovalRow>,
    pub cards: Vec<StallRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRow {
    pub id: String,
    pub asking: String,
    /// What it asks to run, as the adapter sent it. A title can differ.
    pub detail: Option<String>,
    /// False when the process that asked has gone, so no answer can reach it.
    pub live: bool,
    /// When not live, what happened to the session.
    pub decided: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StallRow {
    pub id: i64,
    pub summary: String,
}

/// A session outside the app whose hooks reported in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlainRow {
    pub folder: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRole {
    You,
    Agent,
    Tool,
    Note,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptItem {
    /// Rises through a session's transcript.
    pub seq: i64,
    pub role: TranscriptRole,
    pub text: String,
    /// A tool's updates share this; the latest of each field stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}
