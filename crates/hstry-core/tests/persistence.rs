//! Persistence tests - verify data survives database closure and reopening

use chrono::Utc;
use hstry_core::Database;
use hstry_core::models::{Conversation, Message, MessageRole, Source};
use uuid::Uuid;

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!("hstry-persistence-test-{}.db", Uuid::new_v4());
    path.push(filename);
    path
}

#[tokio::test]
async fn source_persists_across_reopen() {
    let db_path = temp_db_path();

    // Phase 1: Create and populate
    {
        let db = Database::open(&db_path).await.expect("open db");

        let source = Source {
            id: "persist-source".to_string(),
            adapter: "opencode".to_string(),
            path: Some("/tmp/opencode".to_string()),
            last_sync_at: Some(Utc::now()),
            config: serde_json::json!({"setting": "value"}),
        };

        db.upsert_source(&source).await.expect("upsert");
        db.close().await;
    }

    // Phase 2: Reopen and verify
    {
        let db = Database::open(&db_path).await.expect("reopen db");

        let fetched = db
            .get_source("persist-source")
            .await
            .expect("get")
            .expect("exists");

        assert_eq!(fetched.id, "persist-source");
        assert_eq!(fetched.adapter, "opencode");
        assert_eq!(fetched.path, Some("/tmp/opencode".to_string()));
        assert!(fetched.last_sync_at.is_some());
        assert_eq!(fetched.config, serde_json::json!({"setting": "value"}));

        db.close().await;
    }
}

#[tokio::test]
async fn conversation_persists_across_reopen() {
    let db_path = temp_db_path();

    // Setup source first
    {
        let db = Database::open(&db_path).await.expect("open db");
        let source = Source {
            id: "test-source".to_string(),
            adapter: "test".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        };
        db.upsert_source(&source).await.expect("upsert source");
        db.close().await;
    }

    // Phase 1: Create conversation
    let conv_id = {
        let db = Database::open(&db_path).await.expect("open db");
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "test-source".to_string(),
            external_id: Some("persist-conv-1".to_string()),
            readable_id: None,
            title: Some("Persisted Conversation".to_string()),
            created_at: Utc::now(),
            updated_at: None,
            model: Some("gpt-4".to_string()),
            provider: Some("openai".to_string()),
            workspace: Some("/workspace".to_string()),
            tokens_in: Some(100),
            tokens_out: Some(200),
            cost_usd: Some(0.03),
            metadata: serde_json::json!({"key": "value"}),
            harness: Some("pi".to_string()),
        };
        db.upsert_conversation(&conv).await.expect("upsert");
        let id = conv.id;
        db.close().await;
        id
    };

    // Phase 2: Reopen and verify
    {
        let db = Database::open(&db_path).await.expect("reopen db");

        let fetched = db
            .get_conversation(conv_id)
            .await
            .expect("get")
            .expect("exists");

        assert_eq!(fetched.source_id, "test-source");
        assert_eq!(fetched.external_id, Some("persist-conv-1".to_string()));
        assert_eq!(fetched.title, Some("Persisted Conversation".to_string()));
        assert_eq!(fetched.model, Some("gpt-4".to_string()));
        assert_eq!(fetched.provider, Some("openai".to_string()));
        assert_eq!(fetched.workspace, Some("/workspace".to_string()));
        assert_eq!(fetched.tokens_in, Some(100));
        assert_eq!(fetched.tokens_out, Some(200));
        assert_eq!(fetched.cost_usd, Some(0.03));
        assert_eq!(fetched.harness, Some("pi".to_string()));

        db.close().await;
    }
}

#[tokio::test]
async fn messages_persist_across_reopen() {
    let db_path = temp_db_path();

    // Setup source and conversation
    let conv_id = {
        let db = Database::open(&db_path).await.expect("open db");

        let source = Source {
            id: "test-source".to_string(),
            adapter: "test".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        };
        db.upsert_source(&source).await.expect("upsert source");

        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "test-source".to_string(),
            external_id: Some("persist-msgs-conv".to_string()),
            readable_id: None,
            title: Some("Message Persistence Test".to_string()),
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

        // Insert multiple messages
        for idx in 0..5 {
            let msg = Message {
                id: Uuid::new_v4(),
                conversation_id: conv.id,
                idx,
                role: if idx % 2 == 0 {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                content: format!("Message {idx}: Hello persistence!"),
                parts_json: serde_json::json!([{
                    "type": "text",
                    "text": format!("Message {idx}: Hello persistence!")
                }]),
                created_at: Some(Utc::now()),
                model: Some("gpt-4".to_string()),
                tokens: Some(((idx + 1) * 10) as i64),
                cost_usd: Some((idx + 1) as f64 * 0.001),
                metadata: serde_json::json!({"msg_idx": idx}),
                sender: None,
                provider: Some("openai".to_string()),
                harness: Some("pi".to_string()),
                client_id: Some(format!("client-{}", idx)),
            };
            db.insert_message(&msg).await.expect("insert");
        }

        let id = conv.id;
        db.close().await;
        id
    };

    // Phase 2: Reopen and verify all messages
    {
        let db = Database::open(&db_path).await.expect("reopen db");

        let messages = db.get_messages(conv_id).await.expect("get messages");

        assert_eq!(messages.len(), 5);

        for (idx, msg) in messages.iter().enumerate() {
            assert_eq!(msg.idx, idx as i32);
            assert_eq!(
                msg.content,
                format!("Message {idx}: Hello persistence!")
            );
            assert_eq!(msg.role, if idx % 2 == 0 { MessageRole::User } else { MessageRole::Assistant });
            assert_eq!(msg.model, Some("gpt-4".to_string()));
            assert_eq!(msg.tokens, Some(((idx + 1) * 10) as i64));
            assert_eq!(msg.cost_usd, Some((idx + 1) as f64 * 0.001));
            assert_eq!(msg.sender, None);
            assert_eq!(msg.provider, Some("openai".to_string()));
            assert_eq!(msg.harness, Some("pi".to_string()));
            assert_eq!(msg.client_id, Some(format!("client-{}", idx)));
        }

        db.close().await;
    }
}

#[tokio::test]
async fn full_scenario_persists_across_multiple_reopens() {
    let db_path = temp_db_path();

    // Round 1: Initial setup
    {
        let db = Database::open(&db_path).await.expect("open");

        // Source
        let source = Source {
            id: "scenario-source".to_string(),
            adapter: "pi".to_string(),
            path: Some("/home/user/project".to_string()),
            last_sync_at: Some(Utc::now()),
            config: serde_json::json!({"workspace": "active"}),
        };
        db.upsert_source(&source).await.expect("upsert source");

        // Conversation
        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "scenario-source".to_string(),
            external_id: Some("scenario-conv".to_string()),
            readable_id: None,
            title: Some("Full Persistence Scenario".to_string()),
            created_at: Utc::now(),
            updated_at: None,
            model: Some("claude-3-5-sonnet".to_string()),
            provider: Some("anthropic".to_string()),
            workspace: Some("/home/user/project".to_string()),
            tokens_in: Some(500),
            tokens_out: Some(800),
            cost_usd: Some(0.025),
            metadata: serde_json::json!({"test": "scenario"}),
            harness: Some("pi".to_string()),
        };
        db.upsert_conversation(&conv).await.expect("upsert conv");

        // Messages
        for idx in 0..3 {
            let msg = Message {
                id: Uuid::new_v4(),
                conversation_id: conv.id,
                idx,
                role: if idx % 2 == 0 { MessageRole::User } else { MessageRole::Assistant },
                content: format!("Scenario message {idx}"),
                parts_json: serde_json::json!([{"type": "text", "text": format!("Scenario message {idx}")}]),
                created_at: Some(Utc::now()),
                model: Some("claude-3-5-sonnet".to_string()),
                tokens: Some((100 * (idx + 1)) as i64),
                cost_usd: Some(0.005 * (idx + 1) as f64),
                metadata: serde_json::json!({"round": 1, "msg": idx}),
                sender: None,
                provider: Some("anthropic".to_string()),
                harness: Some("pi".to_string()),
                client_id: Some(format!("round1-msg{}", idx)),
            };
            db.insert_message(&msg).await.expect("insert");
        }

        db.close().await;
    }

    // Round 2: Verify and add more data
    let conv_id = {
        let db = Database::open(&db_path).await.expect("reopen");

        // Verify source
        let source = db.get_source("scenario-source").await.expect("get").expect("exists");
        assert_eq!(source.adapter, "pi");
        assert_eq!(source.path, Some("/home/user/project".to_string()));

        // Verify conversation
        let convs = db.list_conversations(Default::default()).await.expect("list");
        assert_eq!(convs.len(), 1);
        let conv = &convs[0];
        assert_eq!(conv.title, Some("Full Persistence Scenario".to_string()));

        // Verify messages
        let messages = db.get_messages(conv.id).await.expect("get");
        assert_eq!(messages.len(), 3);

        // Add more messages
        for idx in 3..5 {
            let msg = Message {
                id: Uuid::new_v4(),
                conversation_id: conv.id,
                idx,
                role: if idx % 2 == 0 { MessageRole::User } else { MessageRole::Assistant },
                content: format!("Round 2 message {idx}"),
                parts_json: serde_json::json!([{"type": "text", "text": format!("Round 2 message {idx}")}]),
                created_at: Some(Utc::now()),
                model: Some("claude-3-5-sonnet".to_string()),
                tokens: Some((100 * (idx + 1)) as i64),
                cost_usd: Some(0.005 * (idx + 1) as f64),
                metadata: serde_json::json!({"round": 2, "msg": idx}),
                sender: None,
                provider: Some("anthropic".to_string()),
                harness: Some("pi".to_string()),
                client_id: Some(format!("round2-msg{}", idx)),
            };
            db.insert_message(&msg).await.expect("insert round 2");
        }

        let id = conv.id;
        db.close().await;
        id
    };

    // Round 3: Final verification - should have all messages from both rounds
    {
        let db = Database::open(&db_path).await.expect("reopen final");

        let messages = db.get_messages(conv_id).await.expect("get final");
        assert_eq!(messages.len(), 5);

        // Verify all messages in order
        for (idx, msg) in messages.iter().enumerate() {
            if idx < 3 {
                assert_eq!(msg.metadata["round"], 1);
                assert_eq!(msg.client_id, Some(format!("round1-msg{}", idx)));
            } else {
                assert_eq!(msg.metadata["round"], 2);
                assert_eq!(msg.client_id, Some(format!("round2-msg{}", idx)));
            }
        }

        // Verify count
        let count = db.count_messages().await.expect("count");
        assert_eq!(count, 5);

        db.close().await;
    }
}

#[tokio::test]
async fn search_state_persists_across_reopen() {
    let db_path = temp_db_path();

    // Round 1: Set search state
    {
        let db = Database::open(&db_path).await.expect("open");
        db.set_search_state("test-key", "test-value").await.expect("set");
        db.set_search_state("another-key", "another-value").await.expect("set 2");
        db.close().await;
    }

    // Round 2: Verify search state persists
    {
        let db = Database::open(&db_path).await.expect("reopen");

        let val1 = db.get_search_state("test-key").await.expect("get");
        assert_eq!(val1, Some("test-value".to_string()));

        let val2 = db.get_search_state("another-key").await.expect("get 2");
        assert_eq!(val2, Some("another-value".to_string()));

        // Verify missing key returns None
        let missing = db.get_search_state("nonexistent").await.expect("get missing");
        assert!(missing.is_none());

        db.close().await;
    }

    // Round 3: Update and verify update persists
    {
        let db = Database::open(&db_path).await.expect("reopen 2");
        db.set_search_state("test-key", "updated-value").await.expect("update");

        let val = db.get_search_state("test-key").await.expect("get updated");
        assert_eq!(val, Some("updated-value".to_string()));

        // Other key unchanged
        let val2 = db.get_search_state("another-key").await.expect("get other");
        assert_eq!(val2, Some("another-value".to_string()));

        db.close().await;
    }
}

#[tokio::test]
async fn partial_update_persists_across_reopen() {
    let db_path = temp_db_path();

    // Setup
    let conv_id = {
        let db = Database::open(&db_path).await.expect("open");

        let source = Source {
            id: "partial-source".to_string(),
            adapter: "test".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        };
        db.upsert_source(&source).await.expect("upsert");

        let conv = Conversation {
            id: Uuid::new_v4(),
            source_id: "partial-source".to_string(),
            external_id: Some("partial-conv".to_string()),
            readable_id: None,
            title: Some("Original Title".to_string()),
            created_at: Utc::now(),
            updated_at: None,
            model: Some("gpt-3.5".to_string()),
            provider: Some("openai".to_string()),
            workspace: Some("/original".to_string()),
            tokens_in: Some(100),
            tokens_out: Some(200),
            cost_usd: Some(0.01),
            metadata: serde_json::json!({"original": true}),
            harness: Some("old-harness".to_string()),
        };
        db.upsert_conversation(&conv).await.expect("upsert");
        let id = conv.id;
        db.close().await;
        id
    };

    // Update metadata partially
    {
        let db = Database::open(&db_path).await.expect("reopen");
        db.update_conversation_metadata(
            conv_id,
            Some("New Title"),
            Some("/new-workspace"),
            Some("claude-3-5-sonnet"),
            Some("anthropic"),
            None,
            None,
            Some("new-harness"),
        )
        .await
        .expect("update");
        db.close().await;
    }

    // Verify update persisted and original fields preserved
    {
        let db = Database::open(&db_path).await.expect("reopen 2");
        let conv = db.get_conversation(conv_id).await.expect("get").expect("exists");

        // Updated fields
        assert_eq!(conv.title, Some("New Title".to_string()));
        assert_eq!(conv.model, Some("claude-3-5-sonnet".to_string()));
        assert_eq!(conv.provider, Some("anthropic".to_string()));
        assert_eq!(conv.workspace, Some("/new-workspace".to_string()));
        assert_eq!(conv.harness, Some("new-harness".to_string()));

        // Preserved fields
        assert_eq!(conv.tokens_in, Some(100));
        assert_eq!(conv.tokens_out, Some(200));
        assert_eq!(conv.cost_usd, Some(0.01));
        assert_eq!(conv.metadata, serde_json::json!({"original": true}));

        db.close().await;
    }
}
