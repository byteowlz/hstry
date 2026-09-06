//! Search contract regression tests; benchmarks live in byteowlz/bench.
use hstry_core::{
    Database,
    db::{SearchMode, SearchOptions},
    models::{Conversation, Message, Source},
    recall::{Budget, Provenance, is_needle, project, window},
};
use serde_json::json;

async fn fixture() -> anyhow::Result<(tempfile::TempDir, Database, uuid::Uuid)> {
    let dir = tempfile::tempdir()?;
    let db = Database::open(&dir.path().join("recall.db")).await?;
    db.upsert_source(&Source {
        id: "lab:pi".into(),
        adapter: "pi".into(),
        path: None,
        last_sync_at: None,
        config: json!({}),
    })
    .await?;
    let conv: Conversation = serde_json::from_value(json!({
        "id": "00000000-0000-4000-8000-000000000001", "source_id": "lab:pi", "title": "Immich installation", "external_id": "fixture", "readable_id": "fixture-session",
        "created_at": "2026-01-01T00:00:00Z", "workspace": "/fixture", "model": "fixture-model", "harness": "pi",
        "metadata": {"completeness": "partial"}
    }))?;
    db.upsert_conversation(&conv).await?;
    let messages = [
        ("user", "Set up Immich please".to_string()),
        ("assistant", "Creating the account".to_string()),
        (
            "tool",
            format!(
                "{}admin@fixture.local APP_ADMIN_PASSWORD=FixtureOnly123 /srv/photos/admin.env parseSessionHeader edge-node-7 Error: SQLITE_BUSY a+b[0]{}",
                "Routine build output. ".repeat(200),
                "Routine build output. ".repeat(200)
            ),
        ),
        (
            "assistant",
            "The pull-only registry keeps allocation policy in consumers".to_string(),
        ),
        ("user", "GPUI rendering discussion".to_string()),
    ];
    for (idx, (role, content)) in messages.into_iter().enumerate() {
        let msg: Message = serde_json::from_value(json!({
            "id": uuid::Uuid::new_v4(), "conversation_id": conv.id, "idx": idx,
            "role": role, "content": content, "parts_json": [], "metadata": {},
            "created_at": "2026-01-01T00:00:00Z"
        }))?;
        db.insert_message(&msg).await?;
    }
    Ok((dir, db, conv.id))
}

#[tokio::test]
async fn literal_tool_evidence_is_anchored_and_budgeted() -> anyhow::Result<()> {
    let (_dir, db, id) = fixture().await?;
    for q in [
        "admin@fixture.local",
        "/srv/photos/admin.env",
        "parseSessionHeader",
        "edge-node-7",
        "Error: SQLITE_BUSY",
        "a+b[0]",
    ] {
        let report = db
            .search_report(
                q,
                SearchOptions {
                    mode: SearchMode::Exact,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(report.attempts, vec!["exact"]);
        let hit = &report.hits[0];
        assert_eq!(
            (hit.conversation_id, hit.message_idx, hit.role.to_string()),
            (id, 2, "tool".into())
        );
        assert!(hit.snippet.contains(q));
        assert!(hit.match_position.is_some());
        let value = project(&report, Budget::default(), false)?;
        assert!(value.to_string().chars().count() <= 3000);
        assert!(value["result"]["hits"][0].get("content").is_none());
        assert!(
            value["result"]["hits"][0]["snippet"]
                .as_str()
                .unwrap()
                .contains(q)
        );
        assert_eq!(
            hit.provenance,
            Provenance {
                source: "lab:pi".into(),
                machine: Some("lab".into()),
                completeness: "partial".into(),
                last_sync_at: None,
                snapshot_at: None,
                basis: None
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn automatic_fallback_is_reported_but_exact_never_interprets_regex() -> anyhow::Result<()> {
    let (_dir, db, _) = fixture().await?;
    let q = r"admin@fixture\.local";
    let exact = db
        .search_report(
            q,
            SearchOptions {
                mode: SearchMode::Exact,
                ..Default::default()
            },
        )
        .await?;
    assert!(exact.hits.is_empty());
    assert_eq!(exact.attempts, ["exact"]);
    let auto = db.search_report(q, SearchOptions::default()).await?;
    assert_eq!(auto.attempts, ["exact", "regex"]);
    assert_eq!(auto.hits.len(), 1);
    let absent = db
        .search_report("absent@fixture.local", SearchOptions::default())
        .await?;
    assert!(absent.hits.is_empty());
    let value = project(&absent, Budget::default(), false)?;
    assert_eq!(value["result"]["absence_is_global"], false);
    assert_eq!(value["result"]["stores"][0]["completeness"], "unknown");
    assert!(value["result"]["attempts"].as_array().unwrap().len() >= 2);
    Ok(())
}

#[tokio::test]
async fn fallback_and_exact_keep_filters_before_limits() -> anyhow::Result<()> {
    let (_dir, db, _) = fixture().await?;
    let opts = SearchOptions {
        role: Some("assistant,user".into()),
        limit: Some(1),
        mode: SearchMode::Exact,
        ..Default::default()
    };
    assert!(db.search("admin@fixture.local", opts).await?.is_empty());
    for opts in [
        SearchOptions {
            source_id: Some("missing".into()),
            ..Default::default()
        },
        SearchOptions {
            workspace: Some("/other".into()),
            ..Default::default()
        },
        SearchOptions {
            model: Some("other".into()),
            ..Default::default()
        },
        SearchOptions {
            harness: Some("other".into()),
            ..Default::default()
        },
        SearchOptions {
            tag: Some("other".into()),
            ..Default::default()
        },
        SearchOptions {
            after: Some("2027-01-01T00:00:00Z".parse()?),
            ..Default::default()
        },
        SearchOptions {
            before: Some("2025-01-01T00:00:00Z".parse()?),
            ..Default::default()
        },
    ] {
        assert!(db.search("admin@fixture.local", opts).await?.is_empty());
    }
    Ok(())
}

#[tokio::test]
async fn conversational_recall_surfaces_the_answer_not_just_the_topic() -> anyhow::Result<()> {
    let (_dir, db, _) = fixture().await?;
    for (q, target) in [
        ("what password did we set for immich", "FixtureOnly123"),
        ("which session did we discuss gpui in", "GPUI"),
        ("what did we decide about allocation policy", "consumers"),
    ] {
        let report = db.search_report(q, SearchOptions::default()).await?;
        let value = project(&report, Budget::default(), false)?;
        assert!(
            value["result"]["hits"]
                .as_array()
                .unwrap()
                .iter()
                .take(3)
                .any(|h| h["snippet"].as_str().unwrap().contains(target)),
            "query {q}: {value}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn copied_conversation_reports_snapshot_completeness() -> anyhow::Result<()> {
    let (source_dir, source, _) = fixture().await?;
    let target_dir = tempfile::tempdir()?;
    let target = Database::open(&target_dir.path().join("copy.db")).await?;
    source.close().await;
    hstry_core::remote::merge_databases(&target, &source_dir.path().join("recall.db"), "satellite")
        .await?;
    let report = target
        .search_report("admin@fixture.local", SearchOptions::default())
        .await?;
    let provenance = &report.hits[0].provenance;
    assert_eq!(
        (
            provenance.machine.as_deref(),
            provenance.completeness.as_str(),
            provenance.basis.as_deref()
        ),
        (Some("satellite"), "full", Some("copied_snapshot_only"))
    );
    assert!(provenance.snapshot_at.is_some());
    let mut partial = provenance.clone();
    partial.apply_snapshot(
        &json!({"hstry_sync":{"message_count":100,"machine":"satellite"}}),
        5,
    );
    assert_eq!(partial.completeness, "partial");
    Ok(())
}

#[tokio::test]
async fn stemming_only_matches_still_have_an_evidence_window() -> anyhow::Result<()> {
    let (_dir, db, _) = fixture().await?;
    let report = db
        .search_report(
            "allocating",
            SearchOptions {
                mode: SearchMode::NaturalLanguage,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(report.hits.len(), 1);
    assert!(report.hits[0].snippet.contains("allocation"));
    assert!(report.hits[0].match_position.is_some());
    Ok(())
}

#[tokio::test]
async fn exact_positive_filters_and_pagination_are_preserved() -> anyhow::Result<()> {
    let (_dir, db, id) = fixture().await?;
    db.add_conversation_tag(id, "recall").await?;
    let options = SearchOptions {
        mode: SearchMode::Exact,
        role: Some("tool,assistant".into()),
        source_id: Some("lab:pi".into()),
        workspace: Some("/fixture".into()),
        model: Some("fixture-model".into()),
        harness: Some("pi".into()),
        tag: Some("recall".into()),
        after: Some("2025-01-01T00:00:00Z".parse()?),
        before: Some("2027-01-01T00:00:00Z".parse()?),
        limit: Some(1),
        ..Default::default()
    };
    let report = db
        .search_report("admin@fixture.local", options.clone())
        .await?;
    assert_eq!(report.hits.len(), 1);
    assert!(
        db.search(
            "admin@fixture.local",
            SearchOptions {
                offset: Some(1),
                ..options
            }
        )
        .await?
        .is_empty()
    );
    Ok(())
}

#[test]
fn classifier_and_unicode_windows_are_predictable() {
    for q in [
        "a@b.test",
        "parseHeader",
        "edge-node-7",
        "APP_PASSWORD=value",
        "Error: broken",
        "GPUI",
        r"x\.(a|b)",
    ] {
        assert!(is_needle(q), "{q}");
    }
    for q in ["allocation policy registry", "what did we decide about bb"] {
        assert!(!is_needle(q), "{q}");
    }
    assert_eq!(window("日本語🔑abc", 3, 4), "語🔑ab");
}

#[tokio::test]
async fn budgets_include_json_escaping_and_metadata() -> anyhow::Result<()> {
    let (_dir, db, _) = fixture().await?;
    let mut report = db
        .search_report("admin@fixture.local", SearchOptions::default())
        .await?;
    report.hits[0].content = "\"\\\n🔑".repeat(1000);
    report.hits[0].match_position = Some(0);
    report.hits = vec![report.hits[0].clone(); 30];
    for total in [512, 1000, 3000] {
        let value = project(
            &report,
            Budget {
                total,
                snippet: 300,
            },
            false,
        )?;
        assert!(value.to_string().chars().count() <= total);
        assert_eq!(value["result"]["truncated"], true);
    }
    assert!(
        Budget {
            total: 1,
            snippet: 300
        }
        .validate()
        .is_err()
    );
    Ok(())
}
