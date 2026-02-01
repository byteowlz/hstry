//! Unit tests for the adapter runner.

#[cfg(test)]
mod runtime_tests {
    use super::super::Runtime;

    #[test]
    fn binary_names() {
        assert_eq!(Runtime::Bun.binary(), "bun");
        assert_eq!(Runtime::Deno.binary(), "deno");
        assert_eq!(Runtime::Node.binary(), "node");
    }

    #[test]
    fn run_args_bun() {
        let args = Runtime::Bun.run_args();
        assert_eq!(args, vec!["run"]);
    }

    #[test]
    fn run_args_deno() {
        let args = Runtime::Deno.run_args();
        assert!(args.contains(&"run"));
        assert!(args.contains(&"--allow-read"));
        assert!(args.contains(&"--allow-env"));
    }

    #[test]
    fn run_args_node() {
        let args = Runtime::Node.run_args();
        assert!(args.contains(&"--experimental-strip-types"));
    }

    #[test]
    fn parse_bun() {
        assert_eq!(Runtime::parse("bun"), Some(Runtime::Bun));
        assert_eq!(Runtime::parse("Bun"), Some(Runtime::Bun));
        assert_eq!(Runtime::parse("BUN"), Some(Runtime::Bun));
    }

    #[test]
    fn parse_deno() {
        assert_eq!(Runtime::parse("deno"), Some(Runtime::Deno));
        assert_eq!(Runtime::parse("Deno"), Some(Runtime::Deno));
    }

    #[test]
    fn parse_node() {
        assert_eq!(Runtime::parse("node"), Some(Runtime::Node));
        assert_eq!(Runtime::parse("Node"), Some(Runtime::Node));
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(Runtime::parse("unknown"), None);
        assert_eq!(Runtime::parse("python"), None);
        assert_eq!(Runtime::parse(""), None);
    }

    #[test]
    fn from_str_works() {
        assert_eq!("bun".parse::<Runtime>(), Ok(Runtime::Bun));
        assert_eq!("deno".parse::<Runtime>(), Ok(Runtime::Deno));
        assert_eq!("node".parse::<Runtime>(), Ok(Runtime::Node));
        assert!("unknown".parse::<Runtime>().is_err());
    }
}

#[cfg(test)]
mod adapter_request_tests {
    use super::super::{
        AdapterRequest, ExportConversation, ExportOptions, ParseOptions, ParsedMessage,
    };

    #[test]
    fn info_request_serializes() {
        let req = AdapterRequest::Info;
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["method"], "info");
    }

    #[test]
    fn detect_request_serializes() {
        let req = AdapterRequest::Detect {
            path: "/test/path".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["method"], "detect");
        assert_eq!(json["params"]["path"], "/test/path");
    }

    #[test]
    fn parse_request_serializes() {
        let req = AdapterRequest::Parse {
            path: "/data".to_string(),
            opts: ParseOptions {
                since: Some(1700000000000),
                limit: Some(100),
                include_tools: true,
                include_attachments: false,
                cursor: None,
                batch_size: Some(50),
            },
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["method"], "parse");
        assert_eq!(json["params"]["path"], "/data");
        assert_eq!(json["params"]["opts"]["since"], 1700000000000i64);
        assert_eq!(json["params"]["opts"]["limit"], 100);
        assert!(
            json["params"]["opts"]["include_tools"]
                .as_bool()
                .unwrap_or(false)
        );
    }

    #[test]
    fn parse_stream_request_serializes() {
        let req = AdapterRequest::ParseStream {
            path: "/data".to_string(),
            opts: ParseOptions {
                cursor: Some(serde_json::json!({"page": 2})),
                batch_size: Some(25),
                ..Default::default()
            },
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["method"], "parseStream");
        assert_eq!(json["params"]["opts"]["cursor"]["page"], 2);
    }

    #[test]
    fn export_request_serializes_with_conversations() {
        let conv = ExportConversation {
            external_id: Some("conv-1".to_string()),
            readable_id: Some("calm-builds-anchor".to_string()),
            title: Some("Test".to_string()),
            created_at: 1700000000000,
            updated_at: Some(1700001000000),
            model: Some("gpt-4".to_string()),
            provider: Some("openai".to_string()),
            workspace: Some("/project".to_string()),
            tokens_in: Some(100),
            tokens_out: Some(200),
            cost_usd: Some(0.05),
            messages: vec![
                ParsedMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    created_at: Some(1700000000000),
                    model: None,
                    tokens: Some(1),
                    cost_usd: None,
                    parts: None,
                    tool_calls: None,
                    metadata: None,
                },
                ParsedMessage {
                    role: "assistant".to_string(),
                    content: "Hi there!".to_string(),
                    created_at: Some(1700000001000),
                    model: Some("gpt-4".to_string()),
                    tokens: Some(3),
                    cost_usd: Some(0.001),
                    parts: None,
                    tool_calls: None,
                    metadata: None,
                },
            ],
            metadata: Some(serde_json::json!({"tags": ["test"]})),
        };

        let req = AdapterRequest::Export {
            conversations: vec![conv],
            opts: ExportOptions {
                format: "markdown".to_string(),
                pretty: Some(true),
                include_tools: Some(true),
                include_attachments: Some(false),
            },
        };

        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["method"], "export");
        assert!(json["params"]["conversations"].is_array());
        assert_eq!(json["params"]["conversations"][0]["title"], "Test");
        assert_eq!(
            json["params"]["conversations"][0]["messages"]
                .as_array()
                .expect("messages")
                .len(),
            2
        );
        assert_eq!(json["params"]["opts"]["format"], "markdown");
    }
}

#[cfg(test)]
mod adapter_response_tests {
    use super::super::AdapterResponse;

    #[test]
    fn info_response_deserializes() {
        let json = r#"{"name":"opencode","displayName":"OpenCode","version":"1.0.0","defaultPaths":["~/.opencode"]}"#;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Info(info) => {
                assert_eq!(info.name, "opencode");
                assert_eq!(info.display_name, "OpenCode");
                assert_eq!(info.version, "1.0.0");
                assert_eq!(info.default_paths, vec!["~/.opencode"]);
            }
            _ => panic!("Expected Info response"),
        }
    }

    #[test]
    fn detect_response_deserializes_confidence() {
        let json = "0.95";
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Detect(confidence) => {
                assert!((confidence.unwrap_or(0.0) - 0.95).abs() < f32::EPSILON);
            }
            _ => panic!("Expected Detect response"),
        }
    }

    #[test]
    fn detect_response_deserializes_null() {
        let json = "null";
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Detect(confidence) => {
                assert!(confidence.is_none());
            }
            _ => panic!("Expected Detect response"),
        }
    }

    #[test]
    fn parse_response_deserializes() {
        let json = r#"[{"externalId":"conv-1","title":"Test","createdAt":1700000000000,"messages":[{"role":"user","content":"Hi"}]}]"#;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Parse(convs) => {
                assert_eq!(convs.len(), 1);
                assert_eq!(convs[0].external_id, Some("conv-1".to_string()));
                assert_eq!(convs[0].messages.len(), 1);
            }
            _ => panic!("Expected Parse response"),
        }
    }

    #[test]
    fn parse_stream_response_deserializes() {
        let json = r#"{"conversations":[{"externalId":"c1","createdAt":1700000000000,"messages":[]}],"cursor":{"page":2},"done":false}"#;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::ParseStream(result) => {
                assert_eq!(result.conversations.len(), 1);
                assert_eq!(result.cursor, Some(serde_json::json!({"page": 2})));
                assert_eq!(result.done, Some(false));
            }
            _ => panic!("Expected ParseStream response"),
        }
    }

    #[test]
    fn export_response_deserializes_content() {
        let json =
            r##"{"format":"markdown","content":"# Title\n\nContent","mimeType":"text/markdown"}"##;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Export(result) => {
                assert_eq!(result.format, "markdown");
                assert!(result.content.as_ref().expect("content").contains("Title"));
                assert_eq!(result.mime_type, Some("text/markdown".to_string()));
            }
            _ => panic!("Expected Export response"),
        }
    }

    #[test]
    fn export_response_deserializes_files() {
        let json = r##"{"format":"obsidian","files":[{"path":"2024/01/conv.md","content":"# Conv"},{"path":"2024/01/meta.json","content":"{}"}]}"##;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Export(result) => {
                assert_eq!(result.format, "obsidian");
                let files = result.files.expect("files");
                assert_eq!(files.len(), 2);
                assert_eq!(files[0].path, "2024/01/conv.md");
            }
            _ => panic!("Expected Export response"),
        }
    }

    #[test]
    fn error_response_deserializes() {
        let json = r#"{"error":"Something went wrong"}"#;
        let response: AdapterResponse = serde_json::from_str(json).expect("deserialize");

        match response {
            AdapterResponse::Error { error } => {
                assert_eq!(error, "Something went wrong");
            }
            _ => panic!("Expected Error response"),
        }
    }
}

#[cfg(test)]
mod parsed_conversation_tests {
    use super::super::ParsedConversation;

    #[test]
    fn full_conversation_deserializes() {
        let json = r#"{
            "externalId": "conv-123",
            "readableId": "calm-builds-anchor",
            "title": "Test Conversation",
            "createdAt": 1700000000000,
            "updatedAt": 1700001000000,
            "model": "gpt-4",
            "workspace": "/project",
            "tokensIn": 100,
            "tokensOut": 200,
            "costUsd": 0.05,
            "messages": [
                {
                    "role": "user",
                    "content": "Hello",
                    "createdAt": 1700000000000,
                    "tokens": 1
                }
            ],
            "metadata": {"source": "test"}
        }"#;

        let conv: ParsedConversation = serde_json::from_str(json).expect("deserialize");
        assert_eq!(conv.external_id, Some("conv-123".to_string()));
        assert_eq!(conv.readable_id, Some("calm-builds-anchor".to_string()));
        assert_eq!(conv.tokens_in, Some(100));
        assert_eq!(conv.cost_usd, Some(0.05));
        assert_eq!(conv.messages.len(), 1);
    }

    #[test]
    fn minimal_conversation_deserializes() {
        let json = r#"{"createdAt": 1700000000000, "messages": []}"#;
        let conv: ParsedConversation = serde_json::from_str(json).expect("deserialize");
        assert!(conv.external_id.is_none());
        assert!(conv.title.is_none());
        assert!(conv.messages.is_empty());
    }
}

#[cfg(test)]
mod parsed_message_tests {
    use super::super::ParsedMessage;

    #[test]
    fn message_with_tool_calls_deserializes() {
        let json = r#"{
            "role": "assistant",
            "content": "Let me check that",
            "toolCalls": [
                {
                    "toolName": "read_file",
                    "input": {"path": "/test.txt"},
                    "output": "file contents",
                    "status": "success",
                    "durationMs": 150
                }
            ]
        }"#;

        let msg: ParsedMessage = serde_json::from_str(json).expect("deserialize");
        assert_eq!(msg.role, "assistant");
        let tool_calls = msg.tool_calls.expect("tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].tool_name, "read_file");
        assert_eq!(tool_calls[0].status, Some("success".to_string()));
    }

    #[test]
    fn message_with_parts_deserializes() {
        let json = r#"{
            "role": "assistant",
            "content": "",
            "parts": [
                {"type": "thinking", "text": "Let me think..."},
                {"type": "text", "text": "Here's my answer"}
            ]
        }"#;

        let msg: ParsedMessage = serde_json::from_str(json).expect("deserialize");
        let parts = msg.parts.expect("parts");
        assert!(parts.is_array());
        assert_eq!(parts.as_array().expect("array").len(), 2);
    }
}

#[cfg(test)]
mod adapter_runner_tests {
    use super::super::{AdapterRunner, Runtime};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn create_test_adapter(dir: &TempDir, name: &str) -> PathBuf {
        let adapter_dir = dir.path().join(name);
        std::fs::create_dir_all(&adapter_dir).expect("create dir");
        let adapter_file = adapter_dir.join("adapter.ts");
        std::fs::write(&adapter_file, "// test adapter").expect("write");
        adapter_file
    }

    #[test]
    fn find_adapter_finds_existing() {
        let dir = TempDir::new().expect("tempdir");
        create_test_adapter(&dir, "test-adapter");

        let runner = AdapterRunner::new(Runtime::Bun, vec![dir.path().to_path_buf()]);
        let found = runner.find_adapter("test-adapter");
        assert!(found.is_some());
        assert!(
            found
                .expect("found")
                .to_string_lossy()
                .contains("adapter.ts")
        );
    }

    #[test]
    fn find_adapter_returns_none_for_missing() {
        let dir = TempDir::new().expect("tempdir");
        let runner = AdapterRunner::new(Runtime::Bun, vec![dir.path().to_path_buf()]);
        assert!(runner.find_adapter("nonexistent").is_none());
    }

    #[test]
    fn list_adapters_finds_all() {
        let dir = TempDir::new().expect("tempdir");
        create_test_adapter(&dir, "adapter-a");
        create_test_adapter(&dir, "adapter-b");
        create_test_adapter(&dir, "adapter-c");

        let runner = AdapterRunner::new(Runtime::Bun, vec![dir.path().to_path_buf()]);
        let adapters = runner.list_adapters();
        assert_eq!(adapters.len(), 3);
        assert!(adapters.contains(&"adapter-a".to_string()));
        assert!(adapters.contains(&"adapter-b".to_string()));
        assert!(adapters.contains(&"adapter-c".to_string()));
    }

    #[test]
    fn list_adapters_sorted_and_deduped() {
        let dir1 = TempDir::new().expect("tempdir");
        let dir2 = TempDir::new().expect("tempdir");

        // Same adapter in both directories
        create_test_adapter(&dir1, "shared-adapter");
        create_test_adapter(&dir2, "shared-adapter");
        create_test_adapter(&dir1, "zebra");
        create_test_adapter(&dir2, "alpha");

        let runner = AdapterRunner::new(
            Runtime::Bun,
            vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()],
        );
        let adapters = runner.list_adapters();

        // Should be deduped
        assert_eq!(
            adapters.iter().filter(|a| *a == "shared-adapter").count(),
            1
        );

        // Should be sorted
        let mut sorted = adapters.clone();
        sorted.sort();
        assert_eq!(adapters, sorted);
    }

    #[test]
    fn list_adapters_ignores_non_adapter_dirs() {
        let dir = TempDir::new().expect("tempdir");

        // Valid adapter
        create_test_adapter(&dir, "valid");

        // Directory without adapter.ts
        let invalid_dir = dir.path().join("invalid");
        std::fs::create_dir_all(&invalid_dir).expect("create");
        std::fs::write(invalid_dir.join("other.ts"), "// not an adapter").expect("write");

        let runner = AdapterRunner::new(Runtime::Bun, vec![dir.path().to_path_buf()]);
        let adapters = runner.list_adapters();

        assert_eq!(adapters.len(), 1);
        assert!(adapters.contains(&"valid".to_string()));
        assert!(!adapters.contains(&"invalid".to_string()));
    }
}
