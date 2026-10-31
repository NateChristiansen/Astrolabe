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
