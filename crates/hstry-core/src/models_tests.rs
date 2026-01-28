//! Unit tests for domain models.

use super::*;

#[cfg(test)]
mod message_role_tests {
    use super::*;

    #[test]
    fn display_user() {
        assert_eq!(MessageRole::User.to_string(), "user");
    }

    #[test]
    fn display_assistant() {
        assert_eq!(MessageRole::Assistant.to_string(), "assistant");
    }

    #[test]
    fn display_system() {
        assert_eq!(MessageRole::System.to_string(), "system");
    }

    #[test]
    fn display_tool() {
        assert_eq!(MessageRole::Tool.to_string(), "tool");
    }

    #[test]
    fn display_other() {
        assert_eq!(MessageRole::Other.to_string(), "other");
    }

    #[test]
    fn from_user_variants() {
        assert_eq!(MessageRole::from("user"), MessageRole::User);
        assert_eq!(MessageRole::from("User"), MessageRole::User);
        assert_eq!(MessageRole::from("USER"), MessageRole::User);
        assert_eq!(MessageRole::from("human"), MessageRole::User);
        assert_eq!(MessageRole::from("Human"), MessageRole::User);
    }

    #[test]
    fn from_assistant_variants() {
        assert_eq!(MessageRole::from("assistant"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("Assistant"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("agent"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("ai"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("bot"), MessageRole::Assistant);
    }

    #[test]
    fn from_system() {
        assert_eq!(MessageRole::from("system"), MessageRole::System);
        assert_eq!(MessageRole::from("System"), MessageRole::System);
    }

    #[test]
    fn from_tool_variants() {
        assert_eq!(MessageRole::from("tool"), MessageRole::Tool);
        assert_eq!(MessageRole::from("function"), MessageRole::Tool);
    }

    #[test]
    fn from_unknown_returns_other() {
        assert_eq!(MessageRole::from("unknown"), MessageRole::Other);
        assert_eq!(MessageRole::from("random"), MessageRole::Other);
        assert_eq!(MessageRole::from(""), MessageRole::Other);
    }

    #[test]
    fn serde_roundtrip_all_variants() {
        for role in [
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::System,
            MessageRole::Tool,
            MessageRole::Other,
        ] {
            let json = serde_json::to_string(&role).expect("serialize");
            let parsed: MessageRole = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, role);
        }
    }

    #[test]
    fn serde_deserializes_unknown_as_other() {
        let json = r#""unknown_role""#;
        let parsed: MessageRole = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed, MessageRole::Other);
    }
}

#[cfg(test)]
mod tool_status_tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        for status in [ToolStatus::Pending, ToolStatus::Success, ToolStatus::Error] {
            let json = serde_json::to_string(&status).expect("serialize");
            let parsed: ToolStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, status);
        }
    }
}

#[cfg(test)]
mod attachment_type_tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        for attachment_type in [
            AttachmentType::File,
            AttachmentType::Image,
            AttachmentType::Code,
        ] {
            let json = serde_json::to_string(&attachment_type).expect("serialize");
            let parsed: AttachmentType = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, attachment_type);
        }
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let source = Source {
            id: "test-source".to_string(),
            adapter: "opencode".to_string(),
            path: Some("/home/user/.opencode".to_string()),
            last_sync_at: Some(chrono::Utc::now()),
            config: serde_json::json!({"key": "value"}),
        };

        let json = serde_json::to_string(&source).expect("serialize");
        let parsed: Source = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.id, source.id);
        assert_eq!(parsed.adapter, source.adapter);
        assert_eq!(parsed.path, source.path);
        assert_eq!(parsed.config, source.config);
    }

    #[test]
    fn serde_with_optional_fields_none() {
        let source = Source {
            id: "minimal".to_string(),
            adapter: "chatgpt".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::Value::Object(Default::default()),
        };

        let json = serde_json::to_string(&source).expect("serialize");
        let parsed: Source = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.path, None);
        assert_eq!(parsed.last_sync_at, None);
    }
}

#[cfg(test)]
mod conversation_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn serde_roundtrip() {
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "source-1".to_string(),
            external_id: Some("ext-123".to_string()),
            readable_id: Some("calm-builds-anchor".to_string()),
            title: Some("Test conversation".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: Some(chrono::Utc::now()),
            model: Some("gpt-4".to_string()),
            workspace: Some("/project".to_string()),
            tokens_in: Some(100),
            tokens_out: Some(200),
            cost_usd: Some(0.05),
            metadata: serde_json::json!({"tags": ["test"]}),
        };

        let json = serde_json::to_string(&conv).expect("serialize");
        let parsed: Conversation = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.id, conv.id);
        assert_eq!(parsed.source_id, conv.source_id);
        assert_eq!(parsed.title, conv.title);
        assert_eq!(parsed.tokens_in, conv.tokens_in);
        assert_eq!(parsed.cost_usd, conv.cost_usd);
    }

    #[test]
    fn serde_with_minimal_fields() {
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "src".to_string(),
            external_id: None,
            readable_id: None,
            title: None,
            created_at: chrono::Utc::now(),
            updated_at: None,
            model: None,
            workspace: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::Value::Object(Default::default()),
        };

        let json = serde_json::to_string(&conv).expect("serialize");
        let parsed: Conversation = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.external_id, None);
        assert_eq!(parsed.title, None);
    }
}

#[cfg(test)]
mod message_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn serde_roundtrip() {
        let msg = Message {
            id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            idx: 0,
            role: MessageRole::User,
            content: "Hello, world!".to_string(),
            parts_json: serde_json::json!([{"type": "text", "text": "Hello, world!"}]),
            created_at: Some(chrono::Utc::now()),
            model: Some("gpt-4".to_string()),
            tokens: Some(5),
            cost_usd: Some(0.001),
            metadata: serde_json::json!({}),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: Message = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.id, msg.id);
        assert_eq!(parsed.role, msg.role);
        assert_eq!(parsed.content, msg.content);
        assert_eq!(parsed.idx, msg.idx);
    }
}

#[cfg(test)]
mod search_hit_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn serde_roundtrip() {
        let hit = SearchHit {
            message_id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            message_idx: 5,
            role: MessageRole::Assistant,
            content: "Here's the answer".to_string(),
            snippet: "Here's the [answer]".to_string(),
            created_at: Some(chrono::Utc::now()),
            conv_created_at: chrono::Utc::now(),
            conv_updated_at: None,
            score: 0.95,
            source_id: "source-1".to_string(),
            external_id: Some("ext-1".to_string()),
            title: Some("Q&A Session".to_string()),
            workspace: Some("/project".to_string()),
            source_adapter: "opencode".to_string(),
            source_path: Some("/home/user/.opencode".to_string()),
            host: None,
        };

        let json = serde_json::to_string(&hit).expect("serialize");
        let parsed: SearchHit = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.message_id, hit.message_id);
        assert_eq!(parsed.snippet, hit.snippet);
        assert!((parsed.score - hit.score).abs() < f32::EPSILON);
    }
}
