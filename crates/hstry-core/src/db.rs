//! Database operations for hstry.

use crate::error::Result;
use crate::models::*;
use crate::schema::SCHEMA;
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

/// Database handle for hstry.
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Open or create a database at the given path.
    pub async fn open(path: &Path) -> Result<Self> {
        let parent = path.parent().unwrap_or(Path::new("."));
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        let db = Self { pool };
        db.init().await?;
        Ok(db)
    }

    /// Initialize schema.
    async fn init(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA).execute(&self.pool).await?;
        Ok(())
    }

    /// Get the connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the database.
    pub async fn close(self) {
        self.pool.close().await;
    }

    // =========================================================================
    // Sources
    // =========================================================================

    /// Upsert a source.
    pub async fn upsert_source(&self, source: &Source) -> Result<()> {
        let last_sync = source.last_sync_at.map(|dt| dt.timestamp());
        sqlx::query(
            r#"
            INSERT INTO sources (id, adapter, path, last_sync_at, config)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                adapter = excluded.adapter,
                path = excluded.path,
                last_sync_at = excluded.last_sync_at,
                config = excluded.config
            "#,
        )
        .bind(&source.id)
        .bind(&source.adapter)
        .bind(&source.path)
        .bind(last_sync)
        .bind(source.config.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List all sources.
    pub async fn list_sources(&self) -> Result<Vec<Source>> {
        let rows = sqlx::query("SELECT * FROM sources ORDER BY adapter, id")
            .fetch_all(&self.pool)
            .await?;

        let mut sources = Vec::new();
        for row in rows {
            sources.push(Source {
                id: row.get("id"),
                adapter: row.get("adapter"),
                path: row.get("path"),
                last_sync_at: row.get::<Option<i64>, _>("last_sync_at").map(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .with_timezone(&Utc)
                }),
                config: row
                    .get::<Option<String>, _>("config")
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_default(),
            });
        }
        Ok(sources)
    }

    /// Get a source by ID.
    pub async fn get_source(&self, id: &str) -> Result<Option<Source>> {
        let row = sqlx::query("SELECT * FROM sources WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|row| Source {
            id: row.get("id"),
            adapter: row.get("adapter"),
            path: row.get("path"),
            last_sync_at: row.get::<Option<i64>, _>("last_sync_at").map(|ts| {
                chrono::DateTime::from_timestamp(ts, 0)
                    .unwrap_or_default()
                    .with_timezone(&Utc)
            }),
            config: row
                .get::<Option<String>, _>("config")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
        }))
    }

    // =========================================================================
    // Conversations
    // =========================================================================

    /// Insert a conversation (upsert by source_id + external_id).
    pub async fn upsert_conversation(&self, conv: &Conversation) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO conversations (id, source_id, external_id, title, created_at, updated_at, model, workspace, tokens_in, tokens_out, cost_usd, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, external_id) DO UPDATE SET
                title = excluded.title,
                updated_at = excluded.updated_at,
                model = excluded.model,
                workspace = excluded.workspace,
                tokens_in = excluded.tokens_in,
                tokens_out = excluded.tokens_out,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata
            "#,
        )
        .bind(conv.id.to_string())
        .bind(&conv.source_id)
        .bind(&conv.external_id)
        .bind(&conv.title)
        .bind(conv.created_at.timestamp())
        .bind(conv.updated_at.map(|dt| dt.timestamp()))
        .bind(&conv.model)
        .bind(&conv.workspace)
        .bind(conv.tokens_in)
        .bind(conv.tokens_out)
        .bind(conv.cost_usd)
        .bind(conv.metadata.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// List conversations with optional filters.
    pub async fn list_conversations(
        &self,
        opts: ListConversationsOptions,
    ) -> Result<Vec<Conversation>> {
        let mut sql = String::from("SELECT * FROM conversations WHERE 1=1");

        if opts.source_id.is_some() {
            sql.push_str(" AND source_id = ?");
        }
        if opts.workspace.is_some() {
            sql.push_str(" AND workspace = ?");
        }
        if opts.after.is_some() {
            sql.push_str(" AND created_at > ?");
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = opts.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut query = sqlx::query(&sql);

        if let Some(ref source_id) = opts.source_id {
            query = query.bind(source_id);
        }
        if let Some(ref workspace) = opts.workspace {
            query = query.bind(workspace);
        }
        if let Some(after) = opts.after {
            query = query.bind(after.timestamp());
        }

        let rows = query.fetch_all(&self.pool).await?;

        let mut convs = Vec::new();
        for row in rows {
            convs.push(conversation_from_row(&row)?);
        }
        Ok(convs)
    }

    /// Get a conversation by ID.
    pub async fn get_conversation(&self, id: Uuid) -> Result<Option<Conversation>> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => Ok(Some(conversation_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// Get conversation count.
    pub async fn count_conversations(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    // =========================================================================
    // Messages
    // =========================================================================

    /// Insert a message.
    pub async fn insert_message(&self, msg: &Message) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO messages (id, conversation_id, idx, role, content, created_at, model, tokens, cost_usd, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(conversation_id, idx) DO UPDATE SET
                role = excluded.role,
                content = excluded.content,
                created_at = excluded.created_at,
                model = excluded.model,
                tokens = excluded.tokens,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata
            "#,
        )
        .bind(msg.id.to_string())
        .bind(msg.conversation_id.to_string())
        .bind(msg.idx)
        .bind(msg.role.to_string())
        .bind(&msg.content)
        .bind(msg.created_at.map(|dt| dt.timestamp()))
        .bind(&msg.model)
        .bind(msg.tokens)
        .bind(msg.cost_usd)
        .bind(msg.metadata.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get messages for a conversation.
    pub async fn get_messages(&self, conversation_id: Uuid) -> Result<Vec<Message>> {
        let rows = sqlx::query("SELECT * FROM messages WHERE conversation_id = ? ORDER BY idx")
            .bind(conversation_id.to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(message_from_row(&row)?);
        }
        Ok(messages)
    }

    /// Get message count.
    pub async fn count_messages(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    // =========================================================================
    // Search
    // =========================================================================

    /// Full-text search across messages.
    pub async fn search(&self, query: &str, limit: i64) -> Result<Vec<Message>> {
        let rows = sqlx::query(
            r#"
            SELECT m.* FROM messages m
            JOIN messages_fts fts ON m.rowid = fts.rowid
            WHERE messages_fts MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )
        .bind(query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(message_from_row(&row)?);
        }
        Ok(messages)
    }
}

/// Options for listing conversations.
#[derive(Debug, Default)]
pub struct ListConversationsOptions {
    pub source_id: Option<String>,
    pub workspace: Option<String>,
    pub after: Option<chrono::DateTime<Utc>>,
    pub limit: Option<i64>,
}

fn conversation_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Conversation> {
    Ok(Conversation {
        id: Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default(),
        source_id: row.get("source_id"),
        external_id: row.get("external_id"),
        title: row.get("title"),
        created_at: chrono::DateTime::from_timestamp(row.get::<i64, _>("created_at"), 0)
            .unwrap_or_default()
            .with_timezone(&Utc),
        updated_at: row
            .get::<Option<i64>, _>("updated_at")
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.with_timezone(&Utc)),
        model: row.get("model"),
        workspace: row.get("workspace"),
        tokens_in: row.get("tokens_in"),
        tokens_out: row.get("tokens_out"),
        cost_usd: row.get("cost_usd"),
        metadata: row
            .get::<Option<String>, _>("metadata")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    })
}

fn message_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Message> {
    Ok(Message {
        id: Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default(),
        conversation_id: Uuid::parse_str(row.get::<&str, _>("conversation_id")).unwrap_or_default(),
        idx: row.get("idx"),
        role: MessageRole::from(row.get::<&str, _>("role")),
        content: row.get("content"),
        created_at: row
            .get::<Option<i64>, _>("created_at")
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.with_timezone(&Utc)),
        model: row.get("model"),
        tokens: row.get("tokens"),
        cost_usd: row.get("cost_usd"),
        metadata: row
            .get::<Option<String>, _>("metadata")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
    })
}
