//! Sync helpers shared between CLI and service.

use anyhow::{Context, Result};
use chrono::Utc;
use hstry_core::{Database, ingest::ingest_batch, models::Source};
use hstry_runtime::{
    AdapterRunner,
    runner::{ParseOptions, ParseStreamResult},
};

/// Number of conversations to buffer per streaming batch. Larger batches
/// trade memory for fewer transaction commits and let the bulk-insert path
/// pack more rows per multi-row INSERT statement.
const DEFAULT_BATCH_SIZE: usize = 200;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncStats {
    pub source_id: String,
    pub conversations: usize,
    pub messages: usize,
}

/// Optional progress callback invoked once per committed batch.
///
/// Arguments are the running total of conversations and messages written so
/// far. Callers wire this to whatever UI they prefer (indicatif bar, plain
/// stdout, structured logs). Returning is fast and infallible by contract —
/// the callback must not block on I/O.
pub type ProgressCallback<'a> = &'a (dyn Fn(usize, usize) + Send + Sync);

pub async fn sync_source(
    db: &Database,
    runner: &AdapterRunner,
    source: &Source,
) -> Result<SyncStats> {
    sync_source_with_progress(db, runner, source, None).await
}

pub async fn sync_source_with_progress(
    db: &Database,
    runner: &AdapterRunner,
    source: &Source,
    progress: Option<ProgressCallback<'_>>,
) -> Result<SyncStats> {
    let adapter_path = runner
        .find_adapter(&source.adapter)
        .with_context(|| format!("Adapter '{}' not found", source.adapter))?;

    let path = source
        .path
        .as_ref()
        .with_context(|| format!("No path configured for source '{}'", source.id))?;

    let mut new_count = 0usize;
    let mut message_count = 0usize;
    let mut affected_conversation_ids: Vec<uuid::Uuid> = Vec::new();

    let mut cursor = source.config.get("cursor").cloned();

    let mut parsed_stream = runner
        .parse_stream(
            &adapter_path,
            path,
            ParseOptions {
                since: source.last_sync_at.map(|dt| dt.timestamp_millis()),
                limit: None,
                include_tools: true,
                include_attachments: true,
                cursor: cursor.clone(),
                batch_size: Some(DEFAULT_BATCH_SIZE),
            },
        )
        .await?;

    if parsed_stream.is_none() {
        let conversations = runner
            .parse(
                &adapter_path,
                path,
                ParseOptions {
                    since: source.last_sync_at.map(|dt| dt.timestamp_millis()),
                    limit: None,
                    include_tools: true,
                    include_attachments: true,
                    cursor: None,
                    batch_size: None,
                },
            )
            .await?;
        parsed_stream = Some(ParseStreamResult {
            conversations,
            cursor: None,
            done: Some(true),
        });
    }

    while let Some(batch) = parsed_stream.take() {
        if let Some(next_cursor) = batch.cursor.clone() {
            cursor = Some(next_cursor);
        }

        let outcome = ingest_batch(db, &source.id, batch.conversations).await?;
        new_count += outcome.conversations;
        message_count += outcome.messages;
        affected_conversation_ids.extend(outcome.affected_conversation_ids);

        if outcome.conversations > 0
            && let Some(cb) = progress
        {
            cb(new_count, message_count);
        }

        let done = batch.done.unwrap_or(false);
        if done {
            break;
        }

        parsed_stream = runner
            .parse_stream(
                &adapter_path,
                path,
                ParseOptions {
                    since: source.last_sync_at.map(|dt| dt.timestamp_millis()),
                    limit: None,
                    include_tools: true,
                    include_attachments: true,
                    cursor: cursor.clone(),
                    batch_size: Some(DEFAULT_BATCH_SIZE),
                },
            )
            .await?;
        if parsed_stream.is_none() {
            break;
        }
    }

    // Rebuild summary caches for all affected conversations in one pass
    if !affected_conversation_ids.is_empty() {
        db.rebuild_conversation_summaries(&affected_conversation_ids)
            .await?;
    }

    let mut updated = source.clone();
    if let serde_json::Value::Object(mut config) = updated.config.clone() {
        if let Some(cursor) = cursor {
            config.insert("cursor".to_string(), cursor);
        } else {
            config.remove("cursor");
        }
        updated.config = serde_json::Value::Object(config);
    } else if let Some(cursor) = cursor {
        let mut config = serde_json::Map::new();
        config.insert("cursor".to_string(), cursor);
        updated.config = serde_json::Value::Object(config);
    }
    updated.last_sync_at = Some(Utc::now());
    db.upsert_source(&updated).await?;

    Ok(SyncStats {
        source_id: source.id.clone(),
        conversations: new_count,
        messages: message_count,
    })
}
