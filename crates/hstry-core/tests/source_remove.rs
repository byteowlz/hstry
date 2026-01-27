use anyhow::Result;
use hstry_core::models::{Conversation, Message, MessageRole, Source};
use hstry_core::{Database, Error};

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!("hstry-test-{}.db", uuid::Uuid::new_v4());
    path.push(filename);
    path
}

#[tokio::test]
async fn remove_source_deletes_conversations_and_messages() -> Result<()> {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await?;

    let source = Source {
        id: "source-1".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/tmp/opencode".to_string()),
        last_sync_at: None,
        config: serde_json::Value::Object(Default::default()),
    };
    db.upsert_source(&source).await?;

    let conversation = Conversation {
        id: uuid::Uuid::new_v4(),
        source_id: source.id.clone(),
        external_id: Some("ext-1".to_string()),
        title: Some("Test conversation".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: None,
        model: Some("test-model".to_string()),
        workspace: Some("/tmp".to_string()),
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
    };
    db.upsert_conversation(&conversation).await?;

    let message = Message {
        id: uuid::Uuid::new_v4(),
        conversation_id: conversation.id,
        idx: 0,
        role: MessageRole::User,
        content: "Hello".to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(chrono::Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
    };
    db.insert_message(&message).await?;

    assert_eq!(db.count_conversations().await?, 1);
    assert_eq!(db.count_messages().await?, 1);

    db.remove_source(&source.id).await?;

    assert!(db.get_source(&source.id).await?.is_none());
    assert_eq!(db.count_conversations().await?, 0);
    assert_eq!(db.count_messages().await?, 0);

    Ok(())
}

#[tokio::test]
async fn remove_source_missing_returns_not_found() -> Result<()> {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await?;

    let err = db.remove_source("missing-source").await.unwrap_err();
    match err {
        Error::NotFound(_) => Ok(()),
        _ => anyhow::bail!("unexpected error: {err}"),
    }
}

#[tokio::test]
async fn search_returns_snippet_and_ids() -> Result<()> {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await?;

    let source = Source {
        id: "source-1".to_string(),
        adapter: "opencode".to_string(),
        path: Some("/tmp/opencode".to_string()),
        last_sync_at: None,
        config: serde_json::Value::Object(Default::default()),
    };
    db.upsert_source(&source).await?;

    let conversation = Conversation {
        id: uuid::Uuid::new_v4(),
        source_id: source.id.clone(),
        external_id: Some("ext-1".to_string()),
        title: Some("Search test".to_string()),
        created_at: chrono::Utc::now(),
        updated_at: None,
        model: Some("test-model".to_string()),
        workspace: Some("/tmp/workspace".to_string()),
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
    };
    db.upsert_conversation(&conversation).await?;

    let message = Message {
        id: uuid::Uuid::new_v4(),
        conversation_id: conversation.id,
        idx: 2,
        role: MessageRole::Assistant,
        content: "Search target string".to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(chrono::Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
    };
    db.insert_message(&message).await?;

    let opts = hstry_core::db::SearchOptions {
        source_id: Some(source.id.clone()),
        workspace: Some("/tmp/workspace".to_string()),
        limit: Some(10),
        offset: None,
        mode: hstry_core::db::SearchMode::Auto,
    };
    let hits = db.search("target", opts).await?;

    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.message_id, message.id);
    assert_eq!(hit.conversation_id, conversation.id);
    assert_eq!(hit.message_idx, 2);
    assert!(hit.snippet.to_lowercase().contains("target"));
    assert_eq!(hit.source_id, source.id);
    assert_eq!(hit.external_id.as_deref(), Some("ext-1"));
    assert_eq!(hit.title.as_deref(), Some("Search test"));
    assert_eq!(hit.workspace.as_deref(), Some("/tmp/workspace"));
    assert_eq!(hit.source_adapter, "opencode");
    assert_eq!(hit.source_path.as_deref(), Some("/tmp/opencode"));

    Ok(())
}
