use hstry_runtime::{
    AdapterRequest, AdapterResponse, ExportConversation, ExportOptions, ExportResult, ParsedMessage,
};

#[test]
fn export_request_serializes() {
    let conv = ExportConversation {
        external_id: Some("conv-1".to_string()),
        title: Some("Test".to_string()),
        created_at: 1_700_000_000_000,
        updated_at: None,
        model: Some("test-model".to_string()),
        workspace: Some("/tmp".to_string()),
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        messages: vec![ParsedMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            created_at: Some(1_700_000_000_000),
            model: None,
            tokens: None,
            cost_usd: None,
            parts: None,
            tool_calls: None,
            metadata: None,
        }],
        metadata: None,
    };

    let req = AdapterRequest::Export {
        conversations: vec![conv],
        opts: ExportOptions {
            format: "markdown".to_string(),
            pretty: Some(true),
            include_tools: None,
            include_attachments: None,
        },
    };

    let value = serde_json::to_value(&req).expect("serialize export request");
    assert_eq!(value["method"], "export");
    assert!(value["params"]["conversations"].is_array());
    assert_eq!(value["params"]["opts"]["format"], "markdown");
}

#[test]
fn export_response_deserializes() {
    let json = r###"{"format":"markdown","content":"# Title","mimeType":"text/markdown"}"###;
    let response: AdapterResponse = serde_json::from_str(json).expect("deserialize response");

    match response {
        AdapterResponse::Export(ExportResult {
            format,
            content,
            mime_type,
            ..
        }) => {
            assert_eq!(format, "markdown");
            assert_eq!(content.as_deref(), Some("# Title"));
            assert_eq!(mime_type.as_deref(), Some("text/markdown"));
        }
        _ => panic!("unexpected response variant"),
    }
}
