//! Regression suite for incremental sync correctness and missed-event recovery.
//!
//! Covers:
//! - trx-hjjw.2: source-scoped purge primitives
//! - trx-hjjw.4: stable, content-addressable message ids
//! - trx-hjjw.5: conversation-local duplicate turn dedup
//! - trx-hjjw.6: bulk reseed mode (begin/end)
//! - trx-aa3m: message_events feature flag
//! - trx-jtxf: message_events retention/compaction
//! - trx-z42c.3: per-source watermarks
//! - trx-z42c.5/.6: indexer outbox enqueue/drain semantics
//! - trx-z42c.9: end-to-end "missed event recovery" scenario

use anyhow::Result;
use hstry_core::Database;
use hstry_core::models::{Conversation, Message, MessageRole, Source};

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    let filename = format!("hstry-incsync-{}.db", uuid::Uuid::new_v4());
    path.push(filename);
    path
}

fn make_source(id: &str) -> Source {
    Source {
        id: id.to_string(),
        adapter: "pi".to_string(),
        path: Some(format!("/tmp/{id}")),
        last_sync_at: None,
        config: serde_json::Value::Object(Default::default()),
    }
}

fn make_conversation(source_id: &str, ext: &str) -> Conversation {
    Conversation {
        id: uuid::Uuid::new_v4(),
        source_id: source_id.to_string(),
        external_id: Some(ext.to_string()),
        readable_id: None,
        platform_id: None,
        title: Some(format!("conv {ext}")),
        created_at: chrono::Utc::now(),
        updated_at: None,
        model: None,
        provider: None,
        workspace: None,
        tokens_in: None,
        tokens_out: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
        harness: None,
        version: 0,
        message_count: 0,
        parent_conversation_id: None,
        parent_message_idx: None,
        fork_type: None,
    }
}

fn make_message(conv_id: uuid::Uuid, idx: i32, role: MessageRole, content: &str) -> Message {
    Message {
        id: uuid::Uuid::new_v4(),
        conversation_id: conv_id,
        idx,
        role,
        content: content.to_string(),
        parts_json: serde_json::json!([]),
        created_at: Some(chrono::Utc::now()),
        model: None,
        tokens: None,
        cost_usd: None,
        metadata: serde_json::Value::Object(Default::default()),
        sender: None,
        provider: None,
        harness: None,
        client_id: None,
    }
}

#[tokio::test]
async fn stable_message_id_is_deterministic_and_path_independent() {
    let a = hstry_core::stable_message_id("pi", Some("conv-1"), 0, "user", "hello", None);
    let b = hstry_core::stable_message_id("pi", Some("conv-1"), 0, "user", "hello", None);
    assert_eq!(a, b, "same inputs must produce same id");

    // Different idx => different id
    let c = hstry_core::stable_message_id("pi", Some("conv-1"), 1, "user", "hello", None);
    assert_ne!(a, c);

    // client_id takes priority
    let d = hstry_core::stable_message_id("pi", Some("conv-1"), 0, "user", "hello", Some("X"));
    let e = hstry_core::stable_message_id("pi", Some("conv-2"), 5, "assistant", "world", Some("X"));
    assert_eq!(d, e, "client_id must dominate over content/idx");
}

#[tokio::test]
async fn purge_source_removes_only_targeted_rows() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;

    let s_keep = make_source("keep");
    let s_drop = make_source("drop");
    db.upsert_source(&s_keep).await?;
    db.upsert_source(&s_drop).await?;

    for source in [&s_keep, &s_drop] {
        let conv = make_conversation(&source.id, &format!("{}-c1", source.id));
        db.upsert_conversation(&conv).await?;
        for i in 0..3 {
            let m = make_message(conv.id, i, MessageRole::User, &format!("hi {i}"));
            db.insert_message(&m).await?;
        }
    }

    let purged = db.purge_source("drop", false).await?;
    assert_eq!(purged.conversations, 1);
    assert_eq!(purged.messages, 3);

    let (keep_c, keep_m) = db.count_source_data("keep").await?;
    let (drop_c, drop_m) = db.count_source_data("drop").await?;
    assert_eq!((keep_c, keep_m), (1, 3));
    assert_eq!((drop_c, drop_m), (0, 0));

    // Source row preserved unless explicitly dropped.
    assert!(db.get_source("drop").await?.is_some());

    let purged2 = db.purge_source("drop", true).await?;
    assert_eq!(purged2.conversations, 0);
    assert!(db.get_source("drop").await?.is_none());

    Ok(())
}

#[tokio::test]
async fn message_events_off_by_default() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    let s = make_source("pi-1");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;
    db.insert_message(&make_message(conv.id, 0, MessageRole::User, "hi"))
        .await?;

    // trx-aa3m: feature flag default is OFF.
    assert_eq!(db.count_message_events().await?, 0);

    // Flip on, insert another message, expect a row.
    db.set_message_events_enabled(true);
    db.insert_message(&make_message(conv.id, 1, MessageRole::Assistant, "yo"))
        .await?;
    assert!(db.count_message_events().await? >= 1);
    Ok(())
}

#[tokio::test]
async fn message_events_compaction_caps_per_conversation() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    db.set_message_events_enabled(true);

    let s = make_source("pi-c");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;

    for i in 0..10i32 {
        db.insert_message(&make_message(
            conv.id,
            i,
            MessageRole::User,
            &format!("m{i}"),
        ))
        .await?;
    }

    let before = db.count_message_events().await?;
    assert!(before >= 10);

    // Cap to 5 per conversation; expect at least 5 removed.
    let removed = db.compact_message_events(0, 5).await?;
    assert!(removed >= 5);
    let after = db.count_message_events().await?;
    assert!(after <= 5);
    Ok(())
}

#[tokio::test]
async fn dedup_collapses_duplicate_turns_in_window() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    let s = make_source("pi-d");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;

    // Two identical user turns at adjacent indices, then a different one.
    db.insert_message(&make_message(conv.id, 0, MessageRole::User, "ping"))
        .await?;
    db.insert_message(&make_message(conv.id, 1, MessageRole::User, "ping"))
        .await?;
    db.insert_message(&make_message(conv.id, 2, MessageRole::User, "pong"))
        .await?;

    let removed = db.dedup_conversation_messages(conv.id, 3600, false).await?;
    assert_eq!(removed, 1);

    let (_, msgs) = db.count_source_data(&s.id).await?;
    assert_eq!(msgs, 2);
    Ok(())
}

#[tokio::test]
async fn watermarks_round_trip_through_source_config() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    let s = make_source("wm-1");
    db.upsert_source(&s).await?;

    assert_eq!(db.get_source_watermark(&s.id).await?, None);
    db.set_source_watermark(&s.id, 1_700_000_000_000).await?;
    assert_eq!(
        db.get_source_watermark(&s.id).await?,
        Some(1_700_000_000_000)
    );
    db.set_source_watermark(&s.id, 1_700_000_005_000).await?;
    assert_eq!(
        db.get_source_watermark(&s.id).await?,
        Some(1_700_000_005_000)
    );
    Ok(())
}

#[tokio::test]
async fn indexer_outbox_drain_semantics() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    let s = make_source("ob-1");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;

    db.set_indexer_outbox_enabled(true);
    db.insert_message(&make_message(conv.id, 0, MessageRole::User, "queued"))
        .await?;
    db.insert_message(&make_message(conv.id, 1, MessageRole::Assistant, "ok"))
        .await?;

    let depth = db.indexer_outbox_depth().await?;
    assert!(depth >= 2);

    let jobs = db.fetch_indexer_jobs(10).await?;
    assert!(jobs.len() >= 2);
    let ids: Vec<i64> = jobs.iter().map(|j| j.id).collect();
    db.ack_indexer_jobs(&ids).await?;
    assert_eq!(db.indexer_outbox_depth().await?, 0);
    Ok(())
}

#[tokio::test]
async fn bulk_reseed_mode_round_trips() -> Result<()> {
    let db = Database::open(&temp_db_path()).await?;
    db.begin_bulk_reseed().await?;
    db.end_bulk_reseed().await?;
    // Indexes should be queryable after end (no panic / sql error).
    let s = make_source("brs-1");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;
    db.insert_message(&make_message(conv.id, 0, MessageRole::User, "hi"))
        .await?;
    Ok(())
}

#[tokio::test]
async fn missed_event_recovery_via_purge_and_reimport() -> Result<()> {
    // Simulates: a Pi source ingested 3 turns, then the watcher missed a write
    // and the on-disk file has 5 turns. A reseed (purge + re-import) recovers
    // the missing tail.
    let db = Database::open(&temp_db_path()).await?;
    let s = make_source("recover-1");
    db.upsert_source(&s).await?;
    let conv = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv).await?;

    for i in 0..3 {
        db.insert_message(&make_message(
            conv.id,
            i,
            MessageRole::User,
            &format!("v1-{i}"),
        ))
        .await?;
    }
    let (_, before) = db.count_source_data(&s.id).await?;
    assert_eq!(before, 3);

    // Purge and re-import 5 turns (the "fixed" on-disk state).
    db.purge_source(&s.id, false).await?;
    let conv2 = make_conversation(&s.id, "c1");
    db.upsert_conversation(&conv2).await?;
    for i in 0..5 {
        db.insert_message(&make_message(
            conv2.id,
            i,
            MessageRole::User,
            &format!("v2-{i}"),
        ))
        .await?;
    }

    let (_, after) = db.count_source_data(&s.id).await?;
    assert_eq!(after, 5);
    Ok(())
}
