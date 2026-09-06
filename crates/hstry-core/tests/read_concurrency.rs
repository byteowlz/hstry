use hstry_core::{
    Database, ingest::ingest_batch, models::Source, read::ReadOptions, stable_message_id,
};
use serde_json::json;
use std::sync::Arc;

#[test]
fn native_identity_survives_position_content_and_role_enrichment() {
    let initial = stable_message_id(
        "source",
        Some("session"),
        0,
        "assistant",
        "before",
        Some("native-id"),
    );
    let enriched = stable_message_id(
        "source",
        Some("session"),
        9,
        "tool",
        "corrected payload",
        Some("native-id"),
    );
    assert_eq!(initial, enriched);
    assert_ne!(
        initial,
        stable_message_id(
            "another-source",
            Some("session"),
            0,
            "assistant",
            "before",
            Some("native-id")
        )
    );
}

#[tokio::test]
async fn readers_keep_bounded_consistent_pages_during_ingestion() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db = Arc::new(Database::open(&dir.path().join("concurrent.db")).await?);
    db.upsert_source(&Source {
        id: "source".into(),
        adapter: "pi".into(),
        path: None,
        last_sync_at: None,
        config: json!({}),
    })
    .await?;
    let conversation = |n: usize| {
        serde_json::from_value(
            json!({"externalId":"session","createdAt":1767225600000_i64,"messages":(0..n).map(|i|json!({"role":"user","content":format!("message {i}")})).collect::<Vec<_>>()}),
        )
    };
    ingest_batch(&db, "source", vec![conversation(1)?]).await?;
    let id = db.list_conversations(Default::default()).await?[0].id;
    let writer = db.clone();
    let writing = tokio::spawn(async move {
        for n in 2..20 {
            ingest_batch(&writer, "source", vec![conversation(n)?]).await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut last_version = 0;
    for _ in 0..30 {
        let page = db
            .read_page(
                id,
                ReadOptions {
                    message_idx: Some(0),
                    after: 5,
                    max_chars: 1200,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(page.records[0].text, "message 0");
        assert!(page.version >= last_version);
        assert!(page.to_wire()?.chars().count() + 1 <= 1200);
        last_version = page.version;
    }
    writing.await??;
    Ok(())
}
