use super::*;

#[tokio::test]
async fn search_and_expand_share_a_real_message_anchor() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db = hstry_core::Database::open(&dir.path().join("mcp.db")).await?;
    db.upsert_source(&hstry_core::models::Source {
        id: "test".into(),
        adapter: "pi".into(),
        path: None,
        last_sync_at: None,
        config: serde_json::json!({}),
    })
    .await?;
    let conv = serde_json::from_value(
        serde_json::json!({"externalId":"test","createdAt":1767225600000_i64,"messages":[{"role":"tool","content":"needle@example.test FixtureOnlyValue"}]}),
    )?;
    hstry_core::ingest::ingest_batch(&db, "test", vec![conv]).await?;
    let server = McpServer::new(Config::default(), db);
    let request = serde_json::from_value(serde_json::json!({"query":"needle@example.test"}))?;
    let output = server.search(Parameters(request)).await;
    assert!(output.chars().count() <= 3000);
    let result: serde_json::Value = serde_json::from_str(&output)?;
    let hit = &result["result"]["hits"][0];
    let request = serde_json::from_value(
        serde_json::json!({"conversation_id":hit["conversation_id"],"message_idx":hit["message_idx"]}),
    )?;
    let output = server.expand(Parameters(request)).await;
    assert!(output.chars().count() <= 3000);
    let expanded: serde_json::Value = serde_json::from_str(&output)?;
    assert_eq!(
        expanded["result"]["records"][0]["text"],
        "needle@example.test FixtureOnlyValue"
    );
    Ok(())
}
