//! Sync helpers shared between CLI and service.

use anyhow::{Context, Result};
use chrono::Utc;
use hstry_core::{Database, models::Source};
use hstry_runtime::{AdapterRunner, runner::ParseOptions};

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

    let parse_opts = ParseOptions {
        since: source.last_sync_at.map(|dt| dt.timestamp_millis()),
        limit: None,
        include_tools: true,
        include_attachments: true,
    };

    let conversations = runner.parse(&adapter_path, path, parse_opts).await?;

    let mut new_count = 0usize;
    let mut message_count = 0usize;
    for conv in conversations {
        let mut conv_id = uuid::Uuid::new_v4();
        if let Some(external_id) = conv.external_id.as_deref() {
            if let Some(existing) = db.get_conversation_id(&source.id, external_id).await? {
                conv_id = existing;
            }
        }

        let hstry_conv = hstry_core::models::Conversation {
            id: conv_id,
            source_id: source.id.clone(),
            external_id: conv.external_id,
            title: conv.title,
            created_at: chrono::DateTime::from_timestamp_millis(conv.created_at as i64)
                .unwrap_or_default()
                .with_timezone(&Utc),
            updated_at: conv.updated_at.and_then(|ts| {
                chrono::DateTime::from_timestamp_millis(ts as i64).map(|dt| dt.with_timezone(&Utc))
            }),
            model: conv.model,
            workspace: conv.workspace,
            tokens_in: conv.tokens_in.map(|t| t as i64),
            tokens_out: conv.tokens_out.map(|t| t as i64),
            cost_usd: conv.cost_usd,
            metadata: conv
                .metadata
                .map(|m| serde_json::to_value(m).unwrap_or_default())
                .unwrap_or_default(),
        };

        db.upsert_conversation(&hstry_conv).await?;

        for (idx, msg) in conv.messages.iter().enumerate() {
            let hstry_msg = hstry_core::models::Message {
                id: uuid::Uuid::new_v4(),
                conversation_id: hstry_conv.id,
                idx: idx as i32,
                role: hstry_core::models::MessageRole::from(msg.role.as_str()),
                content: msg.content.clone(),
                created_at: msg.created_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp_millis(ts as i64)
                        .map(|dt| dt.with_timezone(&Utc))
                }),
                model: msg.model.clone(),
                tokens: msg.tokens.map(|t| t as i64),
                cost_usd: msg.cost_usd,
                metadata: serde_json::Value::Object(Default::default()),
            };
            db.insert_message(&hstry_msg).await?;
            message_count += 1;
        }

        new_count += 1;
    }

    let mut updated = source.clone();
    updated.last_sync_at = Some(Utc::now());
    db.upsert_source(&updated).await?;

    Ok(SyncStats {
        source_id: source.id.clone(),
        conversations: new_count,
        messages: message_count,
    })
}
