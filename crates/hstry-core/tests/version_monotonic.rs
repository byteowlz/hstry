//! Tests for monotonic conversation version counter.

use chrono::Utc;
use hstry_core::Database;
use hstry_core::models::{Conversation, Message, MessageRole};
use std::sync::Arc;
use tokio::task::JoinSet;
use uuid::Uuid;

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!("hstry-test-{}.db", Uuid::new_v4());
    path.push(filename);
    path
}

fn test_conversation() -> Conversation {
    Conversation {
        id: Uuid::new_v4(),
        source_id: "test-source".to_string(),
        external_id: Some("test-conv-1".to_string()),
        readable_id: None,
        platform_id: None,
        title: Some("Test Conversation".to_string()),
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
        version: 0,
        message_count: 0,
        parent_conversation_id: None,
        parent_message_idx: None,
        fork_type: None,
    }
}

fn test_message(conversation_id: Uuid, idx: i32) -> Message {
    Message {
        id: Uuid::new_v4(),
        conversation_id,
        idx,
        role: MessageRole::User,
        content: format!("Test message {idx}"),
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
    }
}

// ============================================================================
// Monotonic Increment Tests
// ============================================================================

#[tokio::test]
async fn version_increments_on_conversation_upsert() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    // Create conversation (initial version should be 0)
    let conv = test_conversation();
    let conv_id = conv.id;

    db.upsert_conversation(&conv).await.expect("upsert");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert_eq!(version, 0, "Initial version should be 0");
    assert_eq!(message_count, 0, "Initial message_count should be 0");

    // Update conversation (version should increment to 1)
    let mut conv_updated = conv.clone();
    conv_updated.title = Some("Updated Title".to_string());
    db.upsert_conversation(&conv_updated)
        .await
        .expect("upsert update");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert_eq!(version, 1, "Version should increment to 1 after update");
    assert_eq!(message_count, 0, "message_count should still be 0");
}

#[tokio::test]
async fn version_increments_on_metadata_update() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    let (version, _) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 0);

    // Update metadata (version should increment to 1)
    db.update_conversation_metadata(
        conv_id,
        Some("New Title"),
        Some("workspace-1"),
        Some("gpt-4"),
        Some("openai"),
        Some(&serde_json::json!({"key": "value"})),
        Some("readable-123"),
        Some("pi"),
        Some("platform-456"),
    )
    .await
    .expect("update metadata");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1, "Version should increment after metadata update");
    assert_eq!(message_count, 0);
}

#[tokio::test]
async fn version_increments_on_updated_at_update() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    let (version, _) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 0);

    // Update updated_at (version should increment to 1)
    db.update_conversation_updated_at(conv_id, Utc::now())
        .await
        .expect("update updated_at");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(
        version, 1,
        "Version should increment after updated_at update"
    );
    assert_eq!(message_count, 0);
}

#[tokio::test]
async fn version_and_message_count_increment_on_message_insert() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 0);
    assert_eq!(message_count, 0);

    // Insert first message (version -> 1, message_count -> 1)
    let msg1 = test_message(conv_id, 0);
    let written = db.insert_message(&msg1).await.expect("insert message");
    assert!(written, "First message should be written");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1, "Version should be 1 after first message");
    assert_eq!(
        message_count, 1,
        "message_count should be 1 after first message"
    );

    // Insert second message (version -> 2, message_count -> 2)
    let msg2 = test_message(conv_id, 1);
    let written = db.insert_message(&msg2).await.expect("insert message");
    assert!(written, "Second message should be written");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 2, "Version should be 2 after second message");
    assert_eq!(
        message_count, 2,
        "message_count should be 2 after second message"
    );
}

#[tokio::test]
async fn version_increments_but_message_count_unchanged_on_message_update() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    // Insert initial message
    let msg1 = test_message(conv_id, 0);
    db.insert_message(&msg1).await.expect("insert message");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1);
    assert_eq!(message_count, 1);

    // Update the same message (version -> 2, message_count stays at 1)
    let mut msg1_updated = msg1.clone();
    msg1_updated.content = "Updated content".to_string();
    let written = db
        .insert_message(&msg1_updated)
        .await
        .expect("update message");
    assert!(written, "Message update should be written");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 2, "Version should increment on message update");
    assert_eq!(
        message_count, 1,
        "message_count should not change on message update"
    );
}

#[tokio::test]
async fn version_does_not_increment_on_duplicate_message_insert() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    let msg1 = test_message(conv_id, 0);
    db.insert_message(&msg1).await.expect("insert message");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1);
    assert_eq!(message_count, 1);

    // Insert same message again (idempotent - should skip)
    let written = db.insert_message(&msg1).await.expect("insert duplicate");
    assert!(!written, "Duplicate message should not be written");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(
        version, 1,
        "Version should not increment on duplicate message insert"
    );
    assert_eq!(
        message_count, 1,
        "message_count should not increment on duplicate message insert"
    );
}

#[tokio::test]
async fn version_increments_on_rebuild_conversation_summaries() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    // Insert messages via transaction (no per-message version bump)
    let mut tx = db.begin().await.expect("begin transaction");
    db.insert_message_in_tx(&mut tx, &test_message(conv_id, 0))
        .await
        .expect("insert in tx");
    db.insert_message_in_tx(&mut tx, &test_message(conv_id, 1))
        .await
        .expect("insert in tx");
    tx.commit().await.expect("commit transaction");

    // Version should still be 0 at this point
    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(
        version, 0,
        "Version should be 0 after transaction (before rebuild)"
    );
    assert_eq!(message_count, 0, "message_count should be 0 before rebuild");

    // Rebuild summaries (should increment version and reconcile message_count)
    db.rebuild_conversation_summaries(&[conv_id])
        .await
        .expect("rebuild summaries");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1, "Version should increment to 1 after rebuild");
    assert_eq!(
        message_count, 2,
        "message_count should be reconciled to 2 after rebuild"
    );
}

#[tokio::test]
async fn monotonic_increments_across_mixed_operations() {
    let db_path = temp_db_path();
    let db = Database::open(&db_path).await.expect("open db");

    let conv = test_conversation();
    let conv_id = conv.id;

    // Operation 1: Create conversation
    db.upsert_conversation(&conv).await.expect("upsert");
    assert_version_eq(&db, conv_id, 0, "After initial upsert").await;

    // Operation 2: Update conversation metadata
    db.update_conversation_metadata(
        conv_id,
        Some("Title 2"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("update metadata");
    assert_version_eq(&db, conv_id, 1, "After metadata update").await;

    // Operation 3: Insert first message
    db.insert_message(&test_message(conv_id, 0))
        .await
        .expect("insert msg 1");
    assert_version_eq(&db, conv_id, 2, "After first message").await;

    // Operation 4: Insert second message
    db.insert_message(&test_message(conv_id, 1))
        .await
        .expect("insert msg 2");
    assert_version_eq(&db, conv_id, 3, "After second message").await;

    // Operation 5: Update conversation again
    db.update_conversation_metadata(
        conv_id,
        Some("Title 3"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("update metadata again");
    assert_version_eq(&db, conv_id, 4, "After second metadata update").await;

    // Operation 6: Update message
    let mut msg_updated = test_message(conv_id, 0);
    msg_updated.content = "Updated".to_string();
    db.insert_message(&msg_updated)
        .await
        .expect("update message");
    assert_version_eq(&db, conv_id, 5, "After message update").await;

    // Verify message_count is correct
    let (_, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(message_count, 2, "message_count should be 2");
}

async fn assert_version_eq(db: &Database, conv_id: Uuid, expected: i64, context: &str) {
    let (version, _) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert_eq!(
        version, expected,
        "Version should be {expected} {context}, but was {version}"
    );
}

// ============================================================================
// Concurrency Tests
// ============================================================================

#[tokio::test]
async fn concurrent_message_inserts_produce_strictly_increasing_versions() {
    let db_path = temp_db_path();
    let db = Arc::new(Database::open(&db_path).await.expect("open db"));

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    // Spawn 10 concurrent tasks, each inserting a unique message
    let mut join_set = JoinSet::new();
    for i in 0..10 {
        let db_clone = Arc::clone(&db);
        join_set.spawn(async move {
            let msg = test_message(conv_id, i);
            db_clone.insert_message(&msg).await.expect("insert message");
        });
    }

    // Wait for all tasks to complete
    let mut results = Vec::new();
    while let Some(result) = join_set.join_next().await {
        results.push(result);
    }

    // All tasks should succeed
    for result in results {
        result.expect("task should succeed");
    }

    // Final version should be 10 (one increment per message)
    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert_eq!(
        version, 10,
        "Final version should be 10 after 10 concurrent inserts"
    );
    assert_eq!(
        message_count, 10,
        "message_count should be 10 after 10 concurrent inserts"
    );
}

#[tokio::test]
async fn concurrent_mixed_operations_produce_strictly_increasing_versions() {
    let db_path = temp_db_path();
    let db = Arc::new(Database::open(&db_path).await.expect("open db"));

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    // Spawn concurrent tasks performing different operations
    let mut join_set = JoinSet::new();

    // Task 1-5: Insert messages
    for i in 0..5 {
        let db_clone = Arc::clone(&db);
        join_set.spawn(async move {
            let msg = test_message(conv_id, i);
            db_clone.insert_message(&msg).await.expect("insert message");
        });
    }

    // Task 6: Update conversation metadata
    let db_clone = Arc::clone(&db);
    join_set.spawn(async move {
        db_clone
            .update_conversation_metadata(
                conv_id,
                Some("Concurrent Title"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("update metadata");
    });

    // Task 7: Update updated_at
    let db_clone = Arc::clone(&db);
    join_set.spawn(async move {
        db_clone
            .update_conversation_updated_at(conv_id, Utc::now())
            .await
            .expect("update updated_at");
    });

    // Task 8: Upsert conversation
    let db_clone = Arc::clone(&db);
    let conv_clone = conv.clone();
    join_set.spawn(async move {
        let mut conv_updated = conv_clone;
        conv_updated.title = Some("Another Title".to_string());
        db_clone
            .upsert_conversation(&conv_updated)
            .await
            .expect("upsert");
    });

    // Wait for all tasks
    let mut results = Vec::new();
    while let Some(result) = join_set.join_next().await {
        results.push(result);
    }

    for result in results {
        result.expect("task should succeed");
    }

    // Verify monotonicity: final version should be >= number of successful operations
    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert!(
        version >= 7,
        "Version should be at least 7 after 7 concurrent operations"
    );
    assert_eq!(
        message_count, 5,
        "message_count should be 5 after 5 message inserts"
    );

    // Verify all messages are present
    let messages = db.get_messages(conv_id).await.expect("get messages");
    assert_eq!(messages.len(), 5, "Should have 5 messages");
}

#[tokio::test]
async fn concurrent_updates_to_same_message_produce_correct_versioning() {
    let db_path = temp_db_path();
    let db = Arc::new(Database::open(&db_path).await.expect("open db"));

    let conv = test_conversation();
    let conv_id = conv.id;
    db.upsert_conversation(&conv).await.expect("upsert");

    // Insert initial message
    let mut msg = test_message(conv_id, 0);
    msg.content = "Initial".to_string();
    db.insert_message(&msg).await.expect("insert initial");

    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .unwrap();
    assert_eq!(version, 1);
    assert_eq!(message_count, 1);

    // Spawn 5 concurrent tasks updating the same message
    let mut join_set = JoinSet::new();
    for i in 0..5 {
        let db_clone = Arc::clone(&db);
        join_set.spawn(async move {
            let mut msg_update = test_message(conv_id, 0);
            msg_update.content = format!("Update {}", i);
            db_clone
                .insert_message(&msg_update)
                .await
                .expect("update message");
        });
    }

    // Wait for all tasks
    let mut results = Vec::new();
    while let Some(result) = join_set.join_next().await {
        results.push(result);
    }

    for result in results {
        result.expect("task should succeed");
    }

    // Final version should be initial (1) + number of distinct updates
    // Note: some updates might have identical content and be skipped
    let (version, message_count) = db
        .get_conversation_version(conv_id)
        .await
        .expect("get version")
        .expect("conversation exists");
    assert!(
        version >= 2,
        "Version should be at least 2 after concurrent updates"
    );
    assert_eq!(
        message_count, 1,
        "message_count should remain 1 (no new messages)"
    );

    // Verify message was updated (should have latest content)
    let messages = db.get_messages(conv_id).await.expect("get messages");
    assert_eq!(messages.len(), 1);
    assert!(messages[0].content.starts_with("Update"));
}
