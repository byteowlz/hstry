//! Sync helpers shared between CLI and service.

use anyhow::{Context, Result};
use chrono::Utc;
use hstry_core::{Database, models::Source};
use hstry_runtime::{
    AdapterRunner,
    runner::{ParseOptions, ParseStreamResult},
};

const DEFAULT_BATCH_SIZE: usize = 25;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncStats {
    pub source_id: String,
    pub conversations: usize,
    pub messages: usize,
}

pub async fn sync_source(
    db: &Database,
    runner: &AdapterRunner,
    source: &Source,
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

        for conv in batch.conversations {
            let mut conv_id = uuid::Uuid::new_v4();
            if let Some(external_id) = conv.external_id.as_deref()
                && let Some(existing) = db.get_conversation_id(&source.id, external_id).await?
            {
                conv_id = existing;
            }

            let hstry_conv = hstry_core::models::Conversation {
                id: conv_id,
                source_id: source.id.clone(),
                external_id: conv.external_id,
                readable_id: conv.readable_id,
                title: conv.title,
                created_at: chrono::DateTime::from_timestamp_millis(conv.created_at)
                    .unwrap_or_default()
                    .with_timezone(&Utc),
                updated_at: conv.updated_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&Utc))
                }),
                model: conv.model,
                workspace: conv.workspace,
                tokens_in: conv.tokens_in,
                tokens_out: conv.tokens_out,
                cost_usd: conv.cost_usd,
                metadata: conv
                    .metadata
                    .map(|m| serde_json::to_value(m).unwrap_or_default())
                    .unwrap_or_default(),
            };

            db.upsert_conversation(&hstry_conv).await?;

            for (idx, msg) in conv.messages.iter().enumerate() {
                let parts_json = msg.parts.clone().unwrap_or_else(|| serde_json::json!([]));
                let hstry_msg = hstry_core::models::Message {
                    id: uuid::Uuid::new_v4(),
                    conversation_id: hstry_conv.id,
                    idx: idx as i32,
                    role: hstry_core::models::MessageRole::from(msg.role.as_str()),
                    content: msg.content.clone(),
                    parts_json,
                    created_at: msg.created_at.and_then(|ts| {
                        chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&Utc))
                    }),
                    model: msg.model.clone(),
                    tokens: msg.tokens,
                    cost_usd: msg.cost_usd,
                    metadata: serde_json::Value::Object(Default::default()),
                };
                db.insert_message(&hstry_msg).await?;
                message_count += 1;
            }

            new_count += 1;
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
