//! Wire types for parsed conversations, shared by every producer that feeds
//! the ingest pipeline: TS adapters (via hstry-runtime), the HTTP ingest
//! endpoint (hstry-api), and any future push-based source.

use serde::{Deserialize, Serialize};

/// Parsed conversation from an external source (matches the TS adapter types).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedConversation {
    pub external_id: Option<String>,
    pub readable_id: Option<String>,
    pub title: Option<String>,
    pub created_at: i64, // Unix ms
    pub updated_at: Option<i64>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub workspace: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
    pub messages: Vec<ParsedMessage>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Monotonic version counter (read-only hint from source; DB counters are authoritative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// Denormalized message count (read-only hint; DB maintains authoritative count).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_count: Option<u32>,
    /// Parent conversation external ID (for session tree tracking).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_external_id: Option<String>,
    /// Message index in the parent where this conversation branched off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_idx: Option<i32>,
    /// Why this child was created: "fork", "thread", "resume".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_type: Option<String>,
}

/// Parsed message from an external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMessage {
    pub role: String,
    pub content: String,
    pub created_at: Option<i64>,
    pub model: Option<String>,
    pub tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub parts: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ParsedToolCall>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Parsed tool call from an external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedToolCall {
    pub tool_name: String,
    pub input: Option<serde_json::Value>,
    pub output: Option<String>,
    pub status: Option<String>,
    pub duration_ms: Option<i64>,
}
