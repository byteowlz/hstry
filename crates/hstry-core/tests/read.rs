use hstry_core::{
    Database,
    models::{Conversation, Message, Source},
    read::ReadOptions,
};
use serde_json::json;

async fn fixture() -> anyhow::Result<(tempfile::TempDir, Database, uuid::Uuid)> {
    let dir = tempfile::tempdir()?;
    let db = Database::open(&dir.path().join("read.db")).await?;
    db.upsert_source(&Source {
        id: "fixture".into(),
        adapter: "pi".into(),
        path: None,
        last_sync_at: None,
        config: json!({}),
    })
    .await?;
    let id = uuid::Uuid::new_v4();
    let conv: Conversation = serde_json::from_value(
        json!({"id":id,"source_id":"fixture","created_at":"2026-01-01T00:00:00Z","metadata":{}}),
    )?;
    db.upsert_conversation(&conv).await?;
    for idx in 0..3 {
        let msg: Message = serde_json::from_value(
            json!({"id":uuid::Uuid::new_v4(),"conversation_id":id,"idx":idx,"role":"tool","content":if idx==1 {"needle evidence".into()} else {"\"界\\\n".repeat(5000)},"parts_json":[],"metadata":{}}),
        )?;
        db.insert_message(&msg).await?;
    }
    Ok((dir, db, id))
}

#[tokio::test]
async fn bounded_context_keeps_anchor_before_large_neighbors() -> anyhow::Result<()> {
    let (_dir, db, id) = fixture().await?;
    let page = db
        .read_page(
            id,
            ReadOptions {
                message_idx: Some(1),
                before: 1,
                after: 1,
                max_chars: 1500,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        (
            &page.records[0].field,
            page.records[0].message_idx,
            &page.records[0].text
        ),
        (&"content".to_owned(), 1, &"needle evidence".to_owned())
    );
    assert!(page.to_wire()?.chars().count() + 1 <= 1500);
    assert!(page.truncated);
    Ok(())
}

#[tokio::test]
async fn field_continuations_reconstruct_unicode_without_gaps() -> anyhow::Result<()> {
    let (_dir, db, id) = fixture().await?;
    let mut offset = 0;
    let mut text = String::new();
    loop {
        let page = db
            .read_page(
                id,
                ReadOptions {
                    message_idx: Some(0),
                    field: Some("content".into()),
                    offset_chars: offset,
                    max_chars: 1100,
                    ..Default::default()
                },
            )
            .await?;
        assert!(page.to_wire()?.chars().count() + 1 <= 1100);
        text.push_str(&page.records[0].text);
        match page.records[0].next_offset_chars {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(text, "\"界\\\n".repeat(5000));
    Ok(())
}

#[tokio::test]
async fn interaction_expansion_follows_only_direct_ids_and_rejects_stale_pages()
-> anyhow::Result<()> {
    let (_dir, db, id) = fixture().await?;
    for (idx, parts) in [
        (
            3,
            json!([{"type":"tool_call","toolCallId":"owned","input":{"command":"fixture"}}]),
        ),
        (
            4,
            json!([{"type":"tool_result","toolCallId":"owned","output":"result"},{"type":"tool_call","toolCallId":"other","input":{}}]),
        ),
        (
            5,
            json!([{"type":"tool_result","toolCallId":"other","output":"not directly owned"}]),
        ),
    ] {
        let m: Message = serde_json::from_value(
            json!({"id":uuid::Uuid::new_v4(),"conversation_id":id,"idx":idx,"role":"assistant","content":"fixture","parts_json":parts,"metadata":{}}),
        )?;
        db.insert_message(&m).await?;
    }
    let page = db
        .read_page(
            id,
            ReadOptions {
                message_idx: Some(3),
                expand_interactions: true,
                max_chars: 12000,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(
        page.records
            .iter()
            .map(|r| (r.message_idx, r.field.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (3, "content"),
            (3, "parts/0/input"),
            (4, "content"),
            (4, "parts/0/output"),
            (4, "parts/1/input")
        ]
    );
    assert!(
        db.read_page(
            id,
            ReadOptions {
                version: Some(-1),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    Ok(())
}
