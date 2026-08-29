//! Shared ingest pipeline: write a batch of [`ParsedConversation`]s into the
//! database with conversation upsert, stable-id message dedupe, and parent
//! resolution. Used by adapter sync (hstry-cli) and the HTTP ingest endpoint
//! (hstry-api).

use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::Database;
use crate::parsed::ParsedConversation;
use crate::stable_message_id;

#[derive(Debug, Clone, Default)]
pub struct IngestOutcome {
    /// Conversations written (new or updated) in this batch.
    pub conversations: usize,
    /// Conversations inserted for the first time in this batch.
    pub created: usize,
    /// Existing conversations refreshed in this batch.
    pub updated: usize,
    /// Messages submitted in this batch (duplicates dedupe at the DB layer).
    pub messages: usize,
    /// Conversation ids touched by this batch; pass to
    /// [`Database::rebuild_conversation_summaries`] once syncing completes.
    pub affected_conversation_ids: Vec<Uuid>,
}

/// Write one batch of parsed conversations for `source_id` inside a single
/// transaction. Callers are responsible for rebuilding conversation summaries
/// afterwards (batching several calls into one rebuild is fine).
pub async fn ingest_batch(
    db: &Database,
    source_id: &str,
    conversations: Vec<ParsedConversation>,
) -> Result<IngestOutcome> {
    let mut outcome = IngestOutcome::default();

    let mut batch_convs: Vec<crate::models::Conversation> = Vec::new();
    let mut batch_msgs: Vec<crate::models::Message> = Vec::new();

    // Track conversation ids chosen in this batch by external_id so duplicate
    // rows for the same conversation map to one hstry conversation id.
    let mut batch_external_to_conv_id: HashMap<String, Uuid> = HashMap::new();

    // Collect parent resolution info for second pass (before consuming the batch)
    let parent_resolutions: Vec<_> = conversations
        .iter()
        .filter_map(|conv| {
            let parent_ext = conv.parent_external_id.as_ref()?;
            let child_ext = conv.external_id.as_ref()?;
            Some((
                child_ext.clone(),
                parent_ext.clone(),
                conv.parent_message_idx,
                conv.fork_type.clone(),
            ))
        })
        .collect();

    for conv in conversations {
        let mut conv_id = Uuid::new_v4();
        let mut existing_conv: Option<crate::models::Conversation> = None;
        let mut seen_in_batch = false;

        if let Some(external_id) = conv.external_id.as_deref() {
            if let Some(existing_in_batch) = batch_external_to_conv_id.get(external_id) {
                conv_id = *existing_in_batch;
                seen_in_batch = true;
            } else if let Some(existing) = db.get_conversation_id(source_id, external_id).await? {
                conv_id = existing;
                existing_conv = db
                    .get_conversation_by_reference(
                        Some(source_id),
                        Some(external_id),
                        None,
                        None,
                        None,
                    )
                    .await?;
                batch_external_to_conv_id.insert(external_id.to_string(), conv_id);
            } else if db
                .conversation_exists_for_session(source_id, external_id)
                .await?
            {
                tracing::debug!("Skipping session {} - already exists in hstry", external_id);
                continue;
            } else {
                batch_external_to_conv_id.insert(external_id.to_string(), conv_id);
            }
        }

        let mut metadata = conv.metadata.unwrap_or_default();
        let mut readable_id = conv.readable_id;
        let mut title = conv.title;
        let mut model = conv.model;
        let mut provider = conv.provider;
        let mut workspace = conv.workspace;

        let is_existing = existing_conv.is_some();
        if let Some(existing) = existing_conv {
            if let serde_json::Value::Object(mut existing_map) = existing.metadata {
                if let serde_json::Value::Object(new_map) = metadata {
                    for (k, v) in new_map {
                        existing_map.entry(k).or_insert(v);
                    }
                    metadata = serde_json::Value::Object(existing_map);
                } else {
                    metadata = serde_json::Value::Object(existing_map);
                }
            }

            if readable_id.as_deref().unwrap_or_default().is_empty() {
                readable_id = existing.readable_id;
            }
            if title.as_deref().unwrap_or_default().is_empty() {
                title = existing.title;
            }
            if model.is_none() {
                model = existing.model;
            }
            if provider.is_none() {
                provider = existing.provider;
            }
            if workspace.is_none() {
                workspace = existing.workspace;
            }
        }

        let hstry_conv = crate::models::Conversation {
            id: conv_id,
            source_id: source_id.to_string(),
            external_id: conv.external_id,
            readable_id,
            platform_id: None,
            title,
            created_at: chrono::DateTime::from_timestamp_millis(conv.created_at)
                .unwrap_or_default()
                .with_timezone(&Utc),
            updated_at: conv.updated_at.and_then(|ts| {
                chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&Utc))
            }),
            model,
            provider,
            workspace,
            tokens_in: conv.tokens_in,
            tokens_out: conv.tokens_out,
            cost_usd: conv.cost_usd,
            metadata,
            harness: None,
            version: 0,
            message_count: 0,
            parent_conversation_id: None, // Resolved in a second pass below
            parent_message_idx: conv.parent_message_idx,
            fork_type: conv.fork_type.clone(),
        };

        if !seen_in_batch {
            if is_existing {
                outcome.updated += 1;
            } else {
                outcome.created += 1;
            }
            outcome.affected_conversation_ids.push(hstry_conv.id);
            batch_convs.push(hstry_conv.clone());
            outcome.conversations += 1;
        }

        for (idx, msg) in conv.messages.iter().enumerate() {
            let Ok(idx) = i32::try_from(idx) else {
                continue;
            };
            let parts_json = msg.parts.clone().unwrap_or_else(|| serde_json::json!([]));
            let role_str = msg.role.as_str();
            // Stable, content-addressable message id (trx-hjjw.4): replays
            // of the same data produce the same row id, so the existing
            // ON CONFLICT clauses naturally dedupe.
            let stable_id = stable_message_id(
                source_id,
                hstry_conv.external_id.as_deref(),
                idx,
                role_str,
                &msg.content,
                None,
            );
            let hstry_msg = crate::models::Message {
                id: stable_id,
                conversation_id: hstry_conv.id,
                idx,
                role: crate::models::MessageRole::from(role_str),
                content: msg.content.clone(),
                parts_json,
                created_at: msg.created_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&Utc))
                }),
                model: msg.model.clone(),
                tokens: msg.tokens,
                cost_usd: msg.cost_usd,
                metadata: serde_json::Value::Object(serde_json::Map::default()),
                sender: None,
                provider: None,
                harness: None,
                client_id: None,
            };
            batch_msgs.push(hstry_msg);
        }
    }

    // Write the entire batch inside a single transaction. Messages go through
    // the multi-row bulk path so we hit ~60 rows per INSERT statement instead
    // of one per message.
    if !batch_convs.is_empty() {
        // Parsing remains parallel, but SQLite has only one writer. Queue bulk
        // transactions in-process instead of making connections spin on the
        // database busy timeout and occasionally fail with SQLITE_BUSY.
        let _writer = db.lock_ingest_writer().await;
        let mut tx = db.begin().await?;

        for conv in &batch_convs {
            db.upsert_conversation_in_tx(&mut tx, conv).await?;
        }
        db.bulk_insert_messages_in_tx(&mut tx, &batch_msgs).await?;
        outcome.messages += batch_msgs.len();

        tx.commit().await?;
    }

    // Second pass: resolve parent_external_id -> parent_conversation_id.
    // Sources provide the parent's external ID, but hstry needs the internal
    // conversation UUID, resolvable only after the batch is imported.
    for (child_ext_id, parent_ext_id, parent_msg_idx, fork_type) in &parent_resolutions {
        let parent_conv_id = db
            .get_conversation_id(source_id, parent_ext_id)
            .await
            .ok()
            .flatten();
        let child_conv_id = db
            .get_conversation_id(source_id, child_ext_id)
            .await
            .ok()
            .flatten();

        if let (Some(parent_id), Some(child_id)) = (parent_conv_id, child_conv_id) {
            let _ = db
                .update_conversation_metadata_full(
                    child_id,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&parent_id.to_string()),
                    *parent_msg_idx,
                    fork_type.as_deref(),
                )
                .await;
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::models::Source;
    use crate::parsed::{ParsedConversation, ParsedMessage};

    fn parsed_conversation() -> ParsedConversation {
        ParsedConversation {
            external_id: Some("progress-fixture".to_string()),
            readable_id: None,
            title: Some("Progress fixture".to_string()),
            created_at: 1_700_000_000_000,
            updated_at: None,
            model: None,
            provider: None,
            workspace: None,
            tokens_in: None,
            tokens_out: None,
            cost_usd: None,
            messages: vec![ParsedMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
                created_at: None,
                model: None,
                tokens: None,
                cost_usd: None,
                parts: None,
                tool_calls: None,
                metadata: None,
            }],
            metadata: None,
            version: None,
            message_count: None,
            parent_external_id: None,
            parent_message_idx: None,
            fork_type: None,
        }
    }

    #[tokio::test]
    async fn concurrent_batches_queue_without_database_locked_errors() -> Result<()> {
        use std::sync::Arc;

        let path =
            std::env::temp_dir().join(format!("hstry-ingest-concurrent-{}.db", Uuid::new_v4()));
        let db = Arc::new(Database::open(&path).await?);

        for source_idx in 0..8 {
            db.upsert_source(&Source {
                id: format!("source-{source_idx}"),
                adapter: "test".to_string(),
                path: None,
                last_sync_at: None,
                config: serde_json::json!({}),
            })
            .await?;
        }

        let mut tasks = tokio::task::JoinSet::new();
        for source_idx in 0..8 {
            let db = Arc::clone(&db);
            tasks.spawn(async move {
                let source_id = format!("source-{source_idx}");
                let conversations = (0..50)
                    .map(|conversation_idx| {
                        let mut conversation = parsed_conversation();
                        conversation.external_id =
                            Some(format!("conversation-{source_idx}-{conversation_idx}"));
                        conversation
                    })
                    .collect();
                ingest_batch(&db, &source_id, conversations).await
            });
        }

        while let Some(result) = tasks.join_next().await {
            let outcome = result??;
            assert_eq!((outcome.conversations, outcome.messages), (50, 50));
        }
        assert_eq!(db.count_conversations().await?, 400);

        let db = Arc::try_unwrap(db)
            .unwrap_or_else(|_| panic!("all ingestion tasks should release the database"));
        db.close().await;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[tokio::test]
    async fn outcome_distinguishes_created_from_updated_conversations() -> Result<()> {
        let path = std::env::temp_dir().join(format!("hstry-ingest-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).await?;
        db.upsert_source(&Source {
            id: "web".to_string(),
            adapter: "web".to_string(),
            path: None,
            last_sync_at: None,
            config: serde_json::json!({}),
        })
        .await?;

        let first = ingest_batch(&db, "web", vec![parsed_conversation()]).await?;
        assert_eq!(
            (first.conversations, first.created, first.updated),
            (1, 1, 0)
        );

        let second = ingest_batch(&db, "web", vec![parsed_conversation()]).await?;
        assert_eq!(
            (second.conversations, second.created, second.updated),
            (1, 0, 1)
        );
        Ok(())
    }
}
