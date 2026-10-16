use hstry_core::{Config, Database, ingest::ingest_batch, models::Source};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

async fn fixture() -> anyhow::Result<(tempfile::TempDir, PathBuf, String, String)> {
    let dir = tempfile::tempdir()?;
    let mut config = Config {
        database: dir.path().join("history.db"),
        ..Default::default()
    };
    config.adapter_paths = vec![
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../adapters")
            .canonicalize()?,
    ];
    let db = Database::open(&config.database).await?;
    db.upsert_source(&Source {
        id: "pick-source".into(),
        adapter: "codex".into(),
        path: Some("/work/pick-source.jsonl".into()),
        last_sync_at: None,
        config: json!({}),
    })
    .await?;

    let auth = serde_json::from_value(json!({
        "externalId": "auth-refactor-session",
        "title": "Refactor auth middleware",
        "workspace": "/work/auth-service",
        "createdAt": 1767225600000_i64,
        "messages": [
            {"role": "user", "content": "Refactor the auth middleware to use the new token store"},
            {"role": "assistant", "content": "Switched the middleware to the token store."}
        ]
    }))?;
    let billing = serde_json::from_value(json!({
        "externalId": "billing-report-session",
        "title": "Monthly billing report",
        "workspace": "/work/billing",
        "createdAt": 1767225600000_i64,
        "messages": [
            {"role": "user", "content": "Generate the monthly billing report"},
            {"role": "assistant", "content": "Here is the billing report."}
        ]
    }))?;
    ingest_batch(&db, "pick-source", vec![auth, billing]).await?;
    let ids = db.list_conversations(Default::default()).await?;
    let auth_id = ids
        .iter()
        .find(|c| c.title.as_deref() == Some("Refactor auth middleware"))
        .unwrap()
        .id
        .to_string();
    let billing_id = ids
        .iter()
        .find(|c| c.title.as_deref() == Some("Monthly billing report"))
        .unwrap()
        .id
        .to_string();

    let path = dir.path().join("config.toml");
    fs::write(&path, toml::to_string(&config)?)?;
    Ok((dir, path, auth_id, billing_id))
}

fn cmd(config: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hstry"));
    command
        .arg("--config")
        .arg(config)
        .env("HSTRY_NO_SERVICE", "1")
        .env("HSTRY_NO_API", "1");
    command.args(args);
    command
}

fn stdout(mut command: Command) -> anyhow::Result<String> {
    let output = command.output()?;
    anyhow::ensure!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tokio::test]
async fn pick_no_pick_selects_best_query_match() -> anyhow::Result<()> {
    let (_dir, config, auth_id, _billing_id) = fixture().await?;
    let out = stdout(cmd(
        &config,
        &["pick", "--no-pick", "--id-only", "auth refactor"],
    ))?;
    assert_eq!(
        out.trim(),
        auth_id,
        "fuzzy query should pick the auth session"
    );
    Ok(())
}

#[tokio::test]
async fn pick_print_emits_transcript() -> anyhow::Result<()> {
    let (_dir, config, _auth_id, _) = fixture().await?;
    let out = stdout(cmd(
        &config,
        &["pick", "--no-pick", "--print", "billing report"],
    ))?;
    assert!(
        out.contains("# Monthly billing report"),
        "title missing: {out}"
    );
    assert!(
        out.contains("Generate the monthly billing report"),
        "user turn missing"
    );
    assert!(
        out.contains("Here is the billing report."),
        "assistant turn missing"
    );
    Ok(())
}

#[tokio::test]
async fn pick_path_prints_source_path() -> anyhow::Result<()> {
    let (_dir, config, _auth_id, _) = fixture().await?;
    let out = stdout(cmd(&config, &["pick", "--no-pick", "--path", "billing"]))?;
    assert_eq!(out.trim(), "/work/pick-source.jsonl");
    Ok(())
}

#[tokio::test]
async fn pick_unmatched_query_fails() -> anyhow::Result<()> {
    let (_dir, config, _auth_id, _) = fixture().await?;
    let output = cmd(&config, &["pick", "--no-pick", "--id-only", "zzzzz"]).output()?;
    assert!(!output.status.success(), "unmatched query must fail");
    assert!(String::from_utf8_lossy(&output.stderr).contains("No session matches"));
    Ok(())
}

#[tokio::test]
async fn pick_workspace_scope_filters_candidates() -> anyhow::Result<()> {
    let (_dir, config, auth_id, billing_id) = fixture().await?;
    let auth = stdout(cmd(
        &config,
        &[
            "pick",
            "--no-pick",
            "--id-only",
            "--workspace",
            "auth-service",
            "report",
        ],
    ))?;
    // The only in-scope session is the auth one, even though "report" matches billing.
    assert_eq!(
        auth.trim(),
        auth_id,
        "workspace scope must restrict candidates"
    );
    let billing = stdout(cmd(
        &config,
        &["pick", "--no-pick", "--id-only", "--workspace", "billing"],
    ))?;
    assert_eq!(billing.trim(), billing_id);
    Ok(())
}
