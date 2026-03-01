//! Integration tests for database operations.

use chrono::Utc;
use hstry_core::Database;
use hstry_core::db::{ListConversationsOptions, SearchMode, SearchOptions};
use hstry_core::models::{Conversation, Message, MessageRole, Source};
use uuid::Uuid;

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!("hstry-test-{}.db", Uuid::new_v4());
    path.push(filename);
    path
}

// ============================================================================
// Source Operations
// ============================================================================

#[tokio::test]
async fn upsert_source_creates_new_source() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let source = Source {
        id: "source-new".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/tmp/opencode".to_string()),
        last_sync_at: None,
        config: serde_json::json!({"setting": true}),
    };

    db.upsert_source(&source).await.expect("upsert");

    let fetched = db
        .get_source("source-new")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.id, source.id);
    assert_eq!(fetched.adapter, source.adapter);
    assert_eq!(fetched.path, source.path);
    assert_eq!(fetched.config, source.config);
}

#[tokio::test]
async fn upsert_source_updates_existing_source() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let source_v1 = Source {
        id: "source-update".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/tmp/v1".to_string()),
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    db.upsert_source(&source_v1).await.expect("upsert v1");

    let source_v2 = Source {
        id: "source-update".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/tmp/v2".to_string()),
        last_sync_at: Some(Utc::now()),
        config: serde_json::json!({"updated": true}),
    };
    db.upsert_source(&source_v2).await.expect("upsert v2");

    let fetched = db
        .get_source("source-update")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.path, Some("/tmp/v2".to_string()));
    assert!(fetched.last_sync_at.is_some());
}

#[tokio::test]
async fn list_sources_returns_all_sources() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    for i in 1..=3 {
        let source = Source {
            id: format!("source-{i}"),
            adapter: "test".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        };
        db.upsert_source(&source).await.expect("upsert");
    }

    let sources = db.list_sources().await.expect("list");
    assert_eq!(sources.len(), 3);
}

#[tokio::test]
async fn get_source_returns_none_for_missing() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let result = db.get_source("nonexistent").await.expect("get");
    assert!(result.is_none());
}

#[tokio::test]
async fn get_source_by_adapter_path_finds_matching() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let source = Source {
        id: "unique-source".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/specific/path".to_string()),
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    db.upsert_source(&source).await.expect("upsert");

    let found = db
        .get_source_by_adapter_path("opencode", "/specific/path")
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(found.id, "unique-source");

    let not_found = db
        .get_source_by_adapter_path("opencode", "/other/path")
        .await
        .expect("get");
    assert!(not_found.is_none());
}

// ============================================================================
// Conversation Operations
// ============================================================================

async fn setup_source(db: &Database) -> Source {
    let source = Source {
        id: "test-source".to_string(),
        adapter: "test".to_string(),
        path: None,
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    db.upsert_source(&source).await.expect("upsert source");
    source
}

#[tokio::test]
async fn upsert_conversation_creates_new() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    let conv = Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("ext-1".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Test Conversation".to_string()),
        created_at: Utc::now(),
        updated_at: None,
        model: Some("gpt-4".to_string()),
        provider: None,
        workspace: Some("/project".to_string()),
        tokens_in: Some(100),
        tokens_out: Some(200),
        cost_usd: Some(0.05),
        metadata: serde_json::json!({}),
        harness: None,
    };

    db.upsert_conversation(&conv).await.expect("upsert");

    let fetched = db
        .get_conversation(conv.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.title, conv.title);
    assert_eq!(fetched.model, conv.model);
    assert!(fetched.readable_id.is_some()); // Auto-generated
}

#[tokio::test]
async fn upsert_conversation_updates_by_external_id() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    let conv_v1 = Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("ext-update".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Original Title".to_string()),
        created_at: Utc::now(),
        updated_at: None,
        model: None,
        provider: None,
        workspace: None,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        harness: None,
    };
    db.upsert_conversation(&conv_v1).await.expect("upsert v1");

    // Same external_id, different UUID - should update
    let conv_v2 = Conversation {
        id: Uuid::new_v4(), // Different UUID
        source_id: "test-source".to_string(),
        external_id: Some("ext-update".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Updated Title".to_string()),
        created_at: conv_v1.created_at,
        updated_at: Some(Utc::now()),
        model: Some("gpt-4".to_string()),
        provider: None,
        workspace: None,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        harness: None,
    };
    db.upsert_conversation(&conv_v2).await.expect("upsert v2");

    let count = db.count_conversations().await.expect("count");
    assert_eq!(count, 1); // Should still be 1, not 2

    let id = db
        .get_conversation_id("test-source", "ext-update")
        .await
        .expect("get id")
        .expect("exists");
    let fetched = db.get_conversation(id).await.expect("get").expect("exists");
    assert_eq!(fetched.title, Some("Updated Title".to_string()));
    assert_eq!(fetched.model, Some("gpt-4".to_string()));
}

#[tokio::test]
async fn list_conversations_with_filters() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    // Create conversations with different workspaces
    for (i, ws) in ["ws1", "ws2", "ws1"].iter().enumerate() {
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "test-source".to_string(),
            external_id: Some(format!("ext-{i}")),
            readable_id: None,
            platform_id: None,
            title: Some(format!("Conv {i}")),
            created_at: Utc::now(),
            updated_at: None,
            model: None,
            provider: None,
            workspace: Some(ws.to_string()),
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            harness: None,
        };
        db.upsert_conversation(&conv).await.expect("upsert");
    }

    // Filter by workspace
    let opts = ListConversationsOptions {
        workspace: Some("ws1".to_string()),
        ..Default::default()
    };
    let convs = db.list_conversations(opts).await.expect("list");
    assert_eq!(convs.len(), 2);

    // Filter by limit
    let opts = ListConversationsOptions {
        limit: Some(2),
        ..Default::default()
    };
    let convs = db.list_conversations(opts).await.expect("list");
    assert_eq!(convs.len(), 2);
}

#[tokio::test]
async fn count_conversations_accurate() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    assert_eq!(db.count_conversations().await.expect("count"), 0);

    for i in 0..5 {
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "test-source".to_string(),
            external_id: Some(format!("ext-{i}")),
            readable_id: None,
            platform_id: None,
            title: None,
            created_at: Utc::now(),
            updated_at: None,
            model: None,
            provider: None,
            workspace: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            harness: None,
        };
        db.upsert_conversation(&conv).await.expect("upsert");
    }

    assert_eq!(db.count_conversations().await.expect("count"), 5);
}

// ============================================================================
// Message Operations
// ============================================================================

async fn setup_conversation(db: &Database) -> Conversation {
    setup_source(db).await;

    let conv = Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("conv-for-messages".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Conversation for messages".to_string()),
        created_at: Utc::now(),
        updated_at: None,
        model: None,
        provider: None,
        workspace: None,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        harness: None,
    };
    db.upsert_conversation(&conv).await.expect("upsert conv");
    conv
}

#[tokio::test]
async fn insert_message_creates_new() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::User,
        content: "Hello!".to_string(),
        parts_json: serde_json::json!([{"type": "text", "text": "Hello!"}]),
        created_at: Some(Utc::now()),
        model: None,
        tokens: Some(2),
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };

    db.insert_message(&msg).await.expect("insert");

    let messages = db.get_messages(conv.id).await.expect("get messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Hello!");
    assert_eq!(messages[0].role, MessageRole::User);
}

#[tokio::test]
async fn insert_message_upserts_by_idx() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg_v1 = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::User,
        content: "Original".to_string(),
        parts_json: serde_json::json!([]),
        created_at: None,
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg_v1).await.expect("insert v1");

    let msg_v2 = Message {
        id: Uuid::new_v4(), // Different ID
        conversation_id: conv.id,
        idx: 0, // Same idx
        role: MessageRole::User,
        content: "Updated".to_string(),
        parts_json: serde_json::json!([]),
        created_at: None,
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg_v2).await.expect("insert v2");

    let messages = db.get_messages(conv.id).await.expect("get");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Updated");
}

#[tokio::test]
async fn get_messages_ordered_by_idx() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    // Insert out of order
    for idx in [2, 0, 1] {
        let msg = Message {
            id: Uuid::new_v4(),
            conversation_id: conv.id,
            idx,
            role: if idx % 2 == 0 {
                MessageRole::User
            } else {
                MessageRole::Assistant
            },
            content: format!("Message {idx}"),
            parts_json: serde_json::json!([]),
            created_at: None,
            model: None,
            tokens: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            sender: None,
            provider: None,
            harness: None,
            client_id: None,
        };
        db.insert_message(&msg).await.expect("insert");
    }

    let messages = db.get_messages(conv.id).await.expect("get");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].idx, 0);
    assert_eq!(messages[1].idx, 1);
    assert_eq!(messages[2].idx, 2);
}

#[tokio::test]
async fn count_messages_accurate() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    assert_eq!(db.count_messages().await.expect("count"), 0);

    for idx in 0..3 {
        let msg = Message {
            id: Uuid::new_v4(),
            conversation_id: conv.id,
            idx,
            role: MessageRole::User,
            content: format!("Msg {idx}"),
            parts_json: serde_json::json!([]),
            created_at: None,
            model: None,
            tokens: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            sender: None,
            provider: None,
            harness: None,
            client_id: None,
        };
        db.insert_message(&msg).await.expect("insert");
    }

    assert_eq!(db.count_messages().await.expect("count"), 3);
}

#[tokio::test]
async fn message_events_include_inserted_messages() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::User,
        content: "Hello".to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({ "source": "test" }),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg).await.expect("insert");

    let events = db
        .get_message_events(conv.id, Some(-1), None, Some(10))
        .await
        .expect("events");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].idx, 0);
    assert!(events[0].payload_json.contains("\"Hello\""));
}

#[tokio::test]
async fn list_conversation_summaries_uses_cache() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::User,
        content: "First message".to_string(),
        parts_json: serde_json::json!([]),
        created_at: None,
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg).await.expect("insert");

    let summaries = db
        .list_conversation_summaries(ListConversationsOptions::default())
        .await
        .expect("summaries");

    let summary = summaries.iter().find(|s| s.conversation.id == conv.id);
    assert!(summary.is_some());
    let summary = summary.unwrap();
    assert_eq!(summary.message_count, 1);
    assert_eq!(summary.first_user_message.as_deref(), Some("First message"));
}

// ============================================================================
// Search Operations
// ============================================================================

#[tokio::test]
async fn search_finds_matching_content() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::Assistant,
        content: "The quick brown fox jumps over the lazy dog".to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg).await.expect("insert");

    let opts = SearchOptions {
        limit: Some(10),
        mode: SearchMode::NaturalLanguage,
        ..Default::default()
    };
    let hits = db.search("fox", opts).await.expect("search");

    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.to_lowercase().contains("fox"));
}

#[tokio::test]
async fn search_with_source_filter() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    // Create two sources
    let source1 = Source {
        id: "source-1".to_string(),
        adapter: "test".to_string(),
        path: None,
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    let source2 = Source {
        id: "source-2".to_string(),
        adapter: "test".to_string(),
        path: None,
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    db.upsert_source(&source1).await.expect("upsert");
    db.upsert_source(&source2).await.expect("upsert");

    // Create conversations in each source
    for (source_id, keyword) in [("source-1", "alpha"), ("source-2", "beta")] {
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: source_id.to_string(),
            external_id: Some(format!("{source_id}-conv")),
            readable_id: None,
            platform_id: None,
            title: None,
            created_at: Utc::now(),
            updated_at: None,
            model: None,
            provider: None,
            workspace: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            harness: None,
        };
        db.upsert_conversation(&conv).await.expect("upsert");

        let msg = Message {
            id: Uuid::new_v4(),
            conversation_id: conv.id,
            idx: 0,
            role: MessageRole::User,
            content: format!("Testing {keyword} content"),
            parts_json: serde_json::json!([]),
            created_at: None,
            model: None,
            tokens: None,
            cost_usd: None,
            metadata: serde_json::json!({}),
            sender: None,
            provider: None,
            harness: None,
            client_id: None,
        };
        db.insert_message(&msg).await.expect("insert");
    }

    // Search all sources
    let opts = SearchOptions {
        limit: Some(10),
        ..Default::default()
    };
    let all_hits = db.search("Testing", opts).await.expect("search");
    assert_eq!(all_hits.len(), 2);

    // Search only source-1
    let opts = SearchOptions {
        source_id: Some("source-1".to_string()),
        limit: Some(10),
        ..Default::default()
    };
    let filtered = db.search("Testing", opts).await.expect("search");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].source_id, "source-1");
}

#[tokio::test]
async fn search_mode_code_explicit() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::Assistant,
        content: "Use MyClassName for this task".to_string(),
        parts_json: serde_json::json!([]),
        created_at: None,
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg).await.expect("insert");

    // Explicit code mode search
    let opts = SearchOptions {
        mode: SearchMode::Code,
        limit: Some(10),
        ..Default::default()
    };
    // FTS5 code tokenizer should handle camelCase
    let hits = db.search("MyClassName", opts).await.expect("search");
    assert_eq!(hits.len(), 1);
}

// ============================================================================
// Search State
// ============================================================================

#[tokio::test]
async fn search_state_get_set() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    assert!(db.get_search_state("key1").await.expect("get").is_none());

    db.set_search_state("key1", "value1").await.expect("set");
    assert_eq!(
        db.get_search_state("key1").await.expect("get"),
        Some("value1".to_string())
    );

    // Update existing key
    db.set_search_state("key1", "value2").await.expect("set");
    assert_eq!(
        db.get_search_state("key1").await.expect("get"),
        Some("value2".to_string())
    );
}

// ============================================================================
// Partial Metadata Update
// ============================================================================

#[tokio::test]
async fn update_conversation_metadata_partial_title() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    let conv = Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("ext-meta-1".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Original".to_string()),
        created_at: Utc::now(),
        updated_at: None,
        model: Some("gpt-4".to_string()),
        provider: Some("openai".to_string()),
        workspace: Some("/project".to_string()),
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        harness: Some("pi".to_string()),
    };
    db.upsert_conversation(&conv).await.expect("upsert");

    // Update only title -- other fields should be preserved
    db.update_conversation_metadata(
        conv.id,
        Some("Updated Title"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("update");

    let fetched = db
        .get_conversation(conv.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.title, Some("Updated Title".to_string()));
    assert_eq!(fetched.model, Some("gpt-4".to_string())); // Preserved
    assert_eq!(fetched.provider, Some("openai".to_string())); // Preserved
    assert_eq!(fetched.workspace, Some("/project".to_string())); // Preserved
    assert_eq!(fetched.harness, Some("pi".to_string())); // Preserved
    assert!(fetched.updated_at.is_some()); // Bumped
}

#[tokio::test]
async fn update_conversation_metadata_partial_harness() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    setup_source(&db).await;

    let conv = Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("ext-meta-2".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Keep This".to_string()),
        created_at: Utc::now(),
        updated_at: None,
        model: None,
        provider: None,
        workspace: None,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        harness: Some("pi".to_string()),
    };
    db.upsert_conversation(&conv).await.expect("upsert");

    // Update only harness -- title should be preserved
    db.update_conversation_metadata(
        conv.id,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("claude"),
        None,
    )
    .await
    .expect("update");

    let fetched = db
        .get_conversation(conv.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(fetched.title, Some("Keep This".to_string())); // Preserved
    assert_eq!(fetched.harness, Some("claude".to_string())); // Updated
}

#[tokio::test]
async fn delete_conversation_removes_messages() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");
    let conv = setup_conversation(&db).await;

    let msg = Message {
        id: Uuid::new_v4(),
        conversation_id: conv.id,
        idx: 0,
        role: MessageRole::User,
        content: "hello".to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::json!({}),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    };
    db.insert_message(&msg).await.expect("insert msg");

    // Verify message exists
    let messages = db.get_messages(conv.id).await.expect("get msgs");
    assert_eq!(messages.len(), 1);

    // Delete conversation
    db.delete_conversation(conv.id).await.expect("delete");

    // Verify conversation gone
    assert!(db.get_conversation(conv.id).await.expect("get").is_none());

    // Verify messages gone
    let messages = db.get_messages(conv.id).await.expect("get msgs");
    assert!(messages.is_empty());
}

// ============================================================================
// Database Lifecycle
// ============================================================================

#[tokio::test]
async fn database_creates_parent_directories() {
    let mut path = std::env::temp_dir();
    path.push(format!("hstry-nested/{}/test.db", Uuid::new_v4()));

    let db = Database::open(&path).await.expect("open");
    assert!(path.exists());
    db.close().await;
}
