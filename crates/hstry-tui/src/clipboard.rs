//! Export a session as markdown and copy it to the system clipboard.

use std::io::Write;

use base64::Engine;
use chrono::{DateTime, Local};
use hstry_core::models::{Conversation, Message, MessageRole};

const fn role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "User",
        MessageRole::Assistant => "Assistant",
        MessageRole::System => "System",
        MessageRole::Tool => "Tool",
        MessageRole::Other => "Other",
    }
}

/// Render a whole session as a markdown transcript.
pub fn session_markdown(conv: Option<&Conversation>, messages: &[Message]) -> String {
    let mut out = String::new();
    if let Some(conv) = conv {
        let title = conv.title.as_deref().unwrap_or("Untitled session");
        out.push_str(&format!("# {title}\n\n"));
        let created: DateTime<Local> = conv.created_at.into();
        out.push_str(&format!(
            "- Created: {}\n",
            created.format("%Y-%m-%d %H:%M")
        ));
        out.push_str(&format!("- Source: {}\n", conv.source_id));
        if let Some(model) = &conv.model {
            out.push_str(&format!("- Model: {model}\n"));
        }
        out.push_str(&format!("- Messages: {}\n\n", messages.len()));
    }
    for (i, msg) in messages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n---\n\n");
        }
        out.push_str(&format!("## {}", role_label(&msg.role)));
        if let Some(ts) = msg.created_at {
            let local: DateTime<Local> = ts.into();
            out.push_str(&format!(" · {}", local.format("%Y-%m-%d %H:%M")));
        }
        if let Some(model) = &msg.model {
            out.push_str(&format!(" · {model}"));
        }
        out.push_str("\n\n");
        out.push_str(msg.content.trim_end());
        out.push('\n');
    }
    out
}

/// Copy text to the clipboard. Tries the native clipboard first and falls
/// back to an OSC 52 escape sequence so remote (ssh/tmux) sessions still work.
pub fn copy_text(text: &str) -> Result<&'static str, String> {
    let native_err = match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
        Ok(()) => return Ok("system clipboard"),
        Err(e) => e.to_string(),
    };
    match copy_osc52(text) {
        Ok(()) => Ok("terminal (OSC 52)"),
        Err(e) => Err(format!("{native_err}; OSC 52: {e}")),
    }
}

fn copy_osc52(text: &str) -> std::io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let mut stdout = std::io::stdout().lock();
    // tmux needs the sequence wrapped in a DCS passthrough.
    if std::env::var_os("TMUX").is_some() {
        write!(stdout, "\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\")?;
    } else {
        write!(stdout, "\x1b]52;c;{encoded}\x07")?;
    }
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn msg(role: MessageRole, content: &str) -> Message {
        Message {
            id: Uuid::nil(),
            conversation_id: Uuid::nil(),
            idx: 0,
            role,
            content: content.to_string(),
            parts_json: serde_json::Value::Null,
            created_at: None,
            model: None,
            tokens: None,
            cost_usd: None,
            metadata: serde_json::Value::Null,
            sender: None,
            provider: None,
            harness: None,
            client_id: None,
        }
    }

    #[test]
    fn renders_full_transcript_without_conversation_header() {
        let messages = vec![
            msg(MessageRole::User, "hello\n"),
            msg(MessageRole::Assistant, "hi there"),
        ];
        let md = session_markdown(None, &messages);
        assert_eq!(md, "## User\n\nhello\n\n---\n\n## Assistant\n\nhi there\n");
    }

    #[test]
    fn renders_conversation_header() {
        let conv = Conversation {
            id: Uuid::nil(),
            source_id: "src".into(),
            external_id: None,
            readable_id: None,
            platform_id: None,
            title: Some("My chat".into()),
            created_at: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap(),
            updated_at: None,
            model: Some("gpt".into()),
            provider: None,
            workspace: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::Value::Null,
            harness: None,
            version: 0,
            message_count: 1,
            parent_conversation_id: None,
            parent_message_idx: None,
            fork_type: None,
        };
        let md = session_markdown(Some(&conv), &[msg(MessageRole::User, "x")]);
        assert!(md.starts_with("# My chat\n\n- Created: 2026-01-02 "));
        assert!(md.contains("- Source: src\n- Model: gpt\n- Messages: 1\n\n## User\n\nx\n"));
    }
}
