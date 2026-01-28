//! Domain models for normalized chat history entities.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A source of chat history (e.g., ChatGPT export, OpenCode local).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub adapter: String,
    pub path: Option<String>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub config: serde_json::Value,
}

/// A conversation from any source, normalized to a common format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub source_id: String,
    pub external_id: Option<String>,
    pub readable_id: Option<String>,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
    pub metadata: serde_json::Value,
}

/// A message within a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub idx: i32,
    pub role: MessageRole,
    pub content: String,
    pub parts_json: serde_json::Value,
    pub created_at: Option<DateTime<Utc>>,
    pub model: Option<String>,
    pub tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub metadata: serde_json::Value,
}

/// Message roles across different sources.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
    #[serde(other)]
    Other,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::System => write!(f, "system"),
            MessageRole::Tool => write!(f, "tool"),
            MessageRole::Other => write!(f, "other"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "user" | "human" => MessageRole::User,
            "assistant" | "agent" | "ai" | "bot" => MessageRole::Assistant,
            "system" => MessageRole::System,
            "tool" | "function" => MessageRole::Tool,
            _ => MessageRole::Other,
        }
    }
}

/// A tool call within a message (for agent interactions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: Uuid,
    pub message_id: Uuid,
    pub tool_name: String,
    pub input: Option<serde_json::Value>,
    pub output: Option<String>,
    pub status: Option<ToolStatus>,
    pub duration_ms: Option<i64>,
}

/// Tool call status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Pending,
    Success,
    Error,
}

/// An attachment to a message (file, image, code block).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Uuid,
    pub message_id: Uuid,
    #[serde(rename = "type")]
    pub attachment_type: AttachmentType,
    pub name: Option<String>,
    pub mime_type: Option<String>,
    pub content: Option<Vec<u8>>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub metadata: serde_json::Value,
}

/// Attachment types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentType {
    File,
    Image,
    Code,
}

/// A tag for organizing conversations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

/// Conversation with all its messages (for full retrieval).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationWithMessages {
    #[serde(flatten)]
    pub conversation: Conversation,
    pub messages: Vec<MessageWithExtras>,
}

/// Message with tool calls and attachments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithExtras {
    #[serde(flatten)]
    pub message: Message,
    pub tool_calls: Vec<ToolCall>,
    pub attachments: Vec<Attachment>,
}

/// Search hit for message-level queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub message_id: Uuid,
    pub conversation_id: Uuid,
    pub message_idx: i32,
    pub role: MessageRole,
    pub content: String,
    pub snippet: String,
    pub created_at: Option<DateTime<Utc>>,
    pub conv_created_at: DateTime<Utc>,
    pub conv_updated_at: Option<DateTime<Utc>>,
    pub score: f32,
    pub source_id: String,
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub source_adapter: String,
    pub source_path: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
