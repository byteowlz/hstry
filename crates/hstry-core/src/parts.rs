//! Typed message parts for structured content.
//!
//! This module defines the canonical `Part` enum used in `Message.parts_json`.
//! All AI agent adapters and clients (like Octo) serialize parts to this format.
//!
//! ## Binary Data Handling
//!
//! For media content (images, audio, video, attachments), hstry supports three modes:
//!
//! 1. **URL reference** - Points to external URL or file:// path
//! 2. **Attachment reference** - Points to binary data in `attachments` table (preferred)
//! 3. **Base64 inline** - Embedded in JSON (convenient but bloats data)
//!
//! When writing messages via the gRPC API, binary data should be sent separately
//! using `BinaryData` and the API will store it in the `attachments` table,
//! returning an `attachment_id` to use in the `MediaSource::AttachmentRef`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Binary data for upload to the attachments table.
///
/// Used in the write API to efficiently store binary content.
/// After upload, reference via `MediaSource::AttachmentRef`.
#[derive(Debug, Clone)]
pub struct BinaryData {
    /// Unique ID for this binary (becomes attachments.id).
    pub id: String,
    /// Associated message ID.
    pub message_id: String,
    /// MIME type.
    pub mime_type: String,
    /// Original filename (optional).
    pub filename: Option<String>,
    /// The binary content.
    pub data: Vec<u8>,
}

/// Tool execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    #[default]
    Pending,
    Running,
    Success,
    Error,
}

impl ToolStatus {
    /// Parse from various status strings.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pending" | "queued" => Self::Pending,
            "running" | "in_progress" | "executing" => Self::Running,
            "success" | "completed" | "done" | "ok" => Self::Success,
            "error" | "failed" | "failure" => Self::Error,
            _ => Self::Pending,
        }
    }
}

/// Source for media content (images, audio, video, files).
///
/// Supports three storage modes:
/// - **Url**: External or internal URL (http://, https://, file://)
/// - **AttachmentRef**: Reference to binary data in `attachments` table (efficient for DB storage)
/// - **Base64**: Inline base64 data (convenient for streaming, but bloats JSON)
///
/// When writing to hstry, prefer `AttachmentRef` for binary data - the write API
/// will store the actual bytes in the `attachments` table and return the ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum MediaSource {
    /// URL reference (external link or file:// path).
    Url {
        url: String,
        #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },

    /// Reference to attachment in DB (preferred for binary storage).
    /// The `attachmentId` points to the `attachments.id` column.
    AttachmentRef {
        #[serde(rename = "attachmentId")]
        attachment_id: String,
        #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },

    /// Base64-encoded inline data (for streaming/transport, not storage).
    Base64 {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

impl MediaSource {
    /// Create a URL source.
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url {
            url: url.into(),
            mime_type: None,
        }
    }

    /// Create an attachment reference.
    pub fn attachment_ref(attachment_id: impl Into<String>, mime_type: Option<String>) -> Self {
        Self::AttachmentRef {
            attachment_id: attachment_id.into(),
            mime_type,
        }
    }

    /// Create a base64 source (use sparingly, prefer attachment_ref for storage).
    pub fn base64(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self::Base64 {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// Get the MIME type if known.
    pub fn mime_type(&self) -> Option<&str> {
        match self {
            Self::Url { mime_type, .. } => mime_type.as_deref(),
            Self::AttachmentRef { mime_type, .. } => mime_type.as_deref(),
            Self::Base64 { mime_type, .. } => Some(mime_type),
        }
    }

    /// Check if this is an attachment reference (binary stored in DB).
    pub fn is_attachment_ref(&self) -> bool {
        matches!(self, Self::AttachmentRef { .. })
    }

    /// Get the attachment ID if this is an attachment reference.
    pub fn attachment_id(&self) -> Option<&str> {
        match self {
            Self::AttachmentRef { attachment_id, .. } => Some(attachment_id),
            _ => None,
        }
    }
}

/// A range within a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRange {
    #[serde(rename = "startLine", skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(rename = "endLine", skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
}

/// A content part within a message.
///
/// Messages contain a list of parts representing different content types:
/// text, thinking/reasoning, tool calls/results, file references, media, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    /// Plain text or markdown content.
    Text {
        id: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<String>,
    },

    /// Thinking/reasoning content (chain-of-thought).
    Thinking { id: String, text: String },

    /// A tool call (request to execute a tool).
    ToolCall {
        id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        #[serde(default, skip_serializing_if = "is_default_status")]
        status: ToolStatus,
    },

    /// A tool result (output from executing a tool).
    ToolResult {
        id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
        #[serde(rename = "isError", default)]
        is_error: bool,
        #[serde(
            rename = "durationMs",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        duration_ms: Option<u64>,
    },

    /// A file reference.
    FileRef {
        id: String,
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<FileRange>,
    },

    /// Image content.
    Image {
        id: String,
        #[serde(flatten)]
        source: MediaSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
    },

    /// Audio content.
    Audio {
        id: String,
        #[serde(flatten)]
        source: MediaSource,
        #[serde(
            rename = "durationSec",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        duration_sec: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transcript: Option<String>,
    },

    /// Video content.
    Video {
        id: String,
        #[serde(flatten)]
        source: MediaSource,
        #[serde(
            rename = "durationSec",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        duration_sec: Option<f64>,
    },

    /// Generic file attachment.
    Attachment {
        id: String,
        #[serde(flatten)]
        source: MediaSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(rename = "sizeBytes", default, skip_serializing_if = "Option::is_none")]
        size_bytes: Option<u64>,
    },
}

fn is_default_status(s: &ToolStatus) -> bool {
    *s == ToolStatus::Pending
}

impl Part {
    /// Create a text part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            id: generate_id(),
            text: text.into(),
            format: None,
        }
    }

    /// Create a thinking part.
    pub fn thinking(text: impl Into<String>) -> Self {
        Self::Thinking {
            id: generate_id(),
            text: text.into(),
        }
    }

    /// Create a tool call part.
    pub fn tool_call(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        input: Option<Value>,
    ) -> Self {
        Self::ToolCall {
            id: generate_id(),
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            input,
            status: ToolStatus::Pending,
        }
    }

    /// Create a tool result part.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        output: Option<Value>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            id: generate_id(),
            tool_call_id: tool_call_id.into(),
            name: None,
            output,
            is_error,
            duration_ms: None,
        }
    }

    /// Get the part ID.
    pub fn id(&self) -> &str {
        match self {
            Self::Text { id, .. }
            | Self::Thinking { id, .. }
            | Self::ToolCall { id, .. }
            | Self::ToolResult { id, .. }
            | Self::FileRef { id, .. }
            | Self::Image { id, .. }
            | Self::Audio { id, .. }
            | Self::Video { id, .. }
            | Self::Attachment { id, .. } => id,
        }
    }

    /// Extract text content from this part.
    pub fn text_content(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. } | Self::Thinking { text, .. } => Some(text),
            Self::ToolResult { output, .. } => output.as_ref().and_then(|v| v.as_str()),
            Self::Audio { transcript, .. } => transcript.as_deref(),
            _ => None,
        }
    }
}

fn generate_id() -> String {
    format!("part_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_status_parse() {
        assert_eq!(ToolStatus::parse("pending"), ToolStatus::Pending);
        assert_eq!(ToolStatus::parse("running"), ToolStatus::Running);
        assert_eq!(ToolStatus::parse("in_progress"), ToolStatus::Running);
        assert_eq!(ToolStatus::parse("success"), ToolStatus::Success);
        assert_eq!(ToolStatus::parse("error"), ToolStatus::Error);
    }

    #[test]
    fn test_part_serialization() {
        let part = Part::text("Hello, world!");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("Hello, world!"));
    }

    #[test]
    fn test_tool_call_part() {
        let part = Part::tool_call("call_123", "bash", Some(serde_json::json!({"cmd": "ls"})));
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"tool_call\""));
        assert!(json.contains("\"toolCallId\":\"call_123\""));
        assert!(json.contains("\"name\":\"bash\""));
    }

    #[test]
    fn test_part_text_content() {
        let text_part = Part::text("hello");
        assert_eq!(text_part.text_content(), Some("hello"));

        let thinking_part = Part::thinking("reasoning");
        assert_eq!(thinking_part.text_content(), Some("reasoning"));

        let tool_result =
            Part::tool_result("call_1", Some(serde_json::json!("output text")), false);
        assert_eq!(tool_result.text_content(), Some("output text"));
    }

    #[test]
    fn test_media_source_url() {
        let source = MediaSource::url("https://example.com/image.png");
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"url\""));
        assert!(json.contains("\"url\":\"https://example.com/image.png\""));
    }

    #[test]
    fn test_media_source_attachment_ref() {
        let source = MediaSource::attachment_ref("att_123", Some("image/png".to_string()));
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"attachmentRef\""));
        assert!(json.contains("\"attachmentId\":\"att_123\""));
        assert!(json.contains("\"mimeType\":\"image/png\""));
        assert!(source.is_attachment_ref());
        assert_eq!(source.attachment_id(), Some("att_123"));
    }

    #[test]
    fn test_media_source_base64() {
        let source = MediaSource::base64("SGVsbG8=", "text/plain");
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"source\":\"base64\""));
        assert!(json.contains("\"data\":\"SGVsbG8=\""));
        assert_eq!(source.mime_type(), Some("text/plain"));
    }

    #[test]
    fn test_image_part_with_attachment_ref() {
        let part = Part::Image {
            id: "part_123".to_string(),
            source: MediaSource::attachment_ref("att_456", Some("image/jpeg".to_string())),
            alt: Some("A test image".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("\"type\":\"image\""));
        assert!(json.contains("\"source\":\"attachmentRef\""));
        assert!(json.contains("\"attachmentId\":\"att_456\""));
    }
}
