use super::*;

#[tokio::test]
async fn search_endpoint_returns_bounded_evidence_and_explicit_raw_opt_in() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let db = Arc::new(Database::open(&dir.path().join("api.db")).await?);
    db.upsert_source(&Source {
        id: "test".into(),
        adapter: "pi".into(),
        path: None,
        last_sync_at: None,
        config: serde_json::json!({}),
    })
    .await?;
    let conv = serde_json::from_value(
        serde_json::json!({"externalId":"test","createdAt":1767225600000_i64,"messages":[{"role":"tool","content":format!("{}needle@example.test{}", "noise ".repeat(1000), "noise ".repeat(1000))}]}),
    )?;
    ingest_batch(&db, "test", vec![conv]).await?;
    let state = AppState {
        config: Arc::new(Config::default()),
        db,
        ingest_token: Arc::new(None),
    };
    let query: SearchQuery =
        serde_json::from_value(serde_json::json!({"query":"needle@example.test","mode":"exact"}))?;
    let Json(result) = search(State(state.clone()), Query(query))
        .await
        .map_err(|s| anyhow::anyhow!("{s}"))?;
    assert!(result.to_string().chars().count() <= 3000);
    assert_eq!(result["result"]["hits"][0]["role"], "tool");
    assert!(result["result"]["hits"][0].get("content").is_none());
    let hit = &result["result"]["hits"][0];
    let request = serde_json::from_value(
        serde_json::json!({"id":hit["conversation_id"],"options":{"message_idx":hit["message_idx"],"field":"content","offset_chars":hit["match_position"]}}),
    )?;
    let Json(page) = read_evidence(State(state.clone()), Json(request))
        .await
        .map_err(|e| anyhow::anyhow!("{:?}", e.0))?;
    assert!(page.to_string().chars().count() <= 3000);
    assert!(
        page["result"]["records"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("needle@example.test")
    );
    let query = serde_json::from_value(
        serde_json::json!({"query":"needle@example.test","mode":"exact","raw":true}),
    )?;
    let Json(raw) = search(State(state), Query(query))
        .await
        .map_err(|s| anyhow::anyhow!("{s}"))?;
    assert!(raw["result"]["hits"][0]["content"].as_str().unwrap().len() > 10000);
    Ok(())
}
