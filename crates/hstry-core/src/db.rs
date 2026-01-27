//! Database operations for hstry.

use crate::error::{Error, Result};
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
        self.ensure_conversations_readable_id_column().await?;
        self.ensure_messages_parts_column().await?;
        self.ensure_fts_schema().await?;
        Ok(())
    }

    async fn ensure_conversations_readable_id_column(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(conversations)")
            .fetch_all(&self.pool)
            .await?;

        let has_readable_id = rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .any(|name| name == "readable_id");

        if !has_readable_id {
            sqlx::query("ALTER TABLE conversations ADD COLUMN readable_id TEXT")
                .execute(&self.pool)
                .await?;
        }

        // Backfill missing readable IDs deterministically.
        let rows = sqlx::query(
            "SELECT id, source_id, external_id, title, metadata FROM conversations WHERE readable_id IS NULL OR readable_id = ''",
        )
        .fetch_all(&self.pool)
        .await?;

        for row in rows {
            let id = Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default();
            let source_id: String = row.get("source_id");
            let external_id: Option<String> = row.get("external_id");
            let title: Option<String> = row.get("title");
            let metadata = row
                .get::<Option<String>, _>("metadata")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            let readable_id = readable_id_from_metadata(&metadata).unwrap_or_else(|| {
                generate_readable_id(&Conversation {
                    id,
                    source_id: source_id.clone(),
                    external_id: external_id.clone(),
                    readable_id: None,
                    title: title.clone(),
                    created_at: Utc::now(),
                    updated_at: None,
                    model: None,
                    workspace: None,
                    tokens_in: None,
                    tokens_out: None,
                    cost_usd: None,
                    metadata: metadata.clone(),
                })
            });

            sqlx::query("UPDATE conversations SET readable_id = ? WHERE id = ?")
                .bind(readable_id)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?;
        }

        Ok(())
    }

    async fn ensure_messages_parts_column(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(messages)")
            .fetch_all(&self.pool)
            .await?;

        let has_parts_json = rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .any(|name| name == "parts_json");

        if !has_parts_json {
            sqlx::query("ALTER TABLE messages ADD COLUMN parts_json JSON NOT NULL DEFAULT '[]'")
                .execute(&self.pool)
                .await?;
        }

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

    /// Get a source by adapter and path.
    pub async fn get_source_by_adapter_path(
        &self,
        adapter: &str,
        path: &str,
    ) -> Result<Option<Source>> {
        let row = sqlx::query("SELECT * FROM sources WHERE adapter = ? AND path = ?")
            .bind(adapter)
            .bind(path)
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

    /// Remove a source and all associated data.
    pub async fn remove_source(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        sqlx::query("DELETE FROM conversations WHERE source_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        let result = sqlx::query("DELETE FROM sources WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        if result.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(Error::NotFound(format!("source '{id}'")));
        }

        tx.commit().await?;
        Ok(())
    }

    // =========================================================================
    // Conversations
    // =========================================================================

    /// Insert a conversation (upsert by source_id + external_id).
    pub async fn upsert_conversation(&self, conv: &Conversation) -> Result<()> {
        let readable_id = match conv.readable_id.clone() {
            Some(id) => Some(id),
            None => {
                let from_meta = readable_id_from_metadata(&conv.metadata);
                if from_meta.is_some() {
                    from_meta
                } else if let Some(external_id) = conv.external_id.as_deref() {
                    self.get_conversation_readable_id(&conv.source_id, external_id)
                        .await?
                        .or_else(|| Some(generate_readable_id(conv)))
                } else {
                    Some(generate_readable_id(conv))
                }
            }
        };

        sqlx::query(
            r#"
            INSERT INTO conversations (id, source_id, external_id, readable_id, title, created_at, updated_at, model, workspace, tokens_in, tokens_out, cost_usd, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, external_id) DO UPDATE SET
                readable_id = COALESCE(excluded.readable_id, conversations.readable_id),
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
        .bind(&readable_id)
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

    /// Get conversation ID by source_id + external_id.
    pub async fn get_conversation_id(
        &self,
        source_id: &str,
        external_id: &str,
    ) -> Result<Option<Uuid>> {
        let row =
            sqlx::query("SELECT id FROM conversations WHERE source_id = ? AND external_id = ?")
                .bind(source_id)
                .bind(external_id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|row| Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default()))
    }

    async fn get_conversation_readable_id(
        &self,
        source_id: &str,
        external_id: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT readable_id FROM conversations WHERE source_id = ? AND external_id = ?",
        )
        .bind(source_id)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|row| row.get::<Option<String>, _>("readable_id")))
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
        let parts_json = normalize_parts_json(&msg.parts_json);
        let content = project_content(&msg.content, &parts_json);
        sqlx::query(
            r#"
            INSERT INTO messages (id, conversation_id, idx, role, content, parts_json, created_at, model, tokens, cost_usd, metadata)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(conversation_id, idx) DO UPDATE SET
                role = excluded.role,
                content = excluded.content,
                parts_json = excluded.parts_json,
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
        .bind(content)
        .bind(parts_json.to_string())
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

    /// Full-text search across messages with snippet and provenance.
    pub async fn search(&self, query: &str, opts: SearchOptions) -> Result<Vec<SearchHit>> {
        let mode = opts.mode.resolve(query);
        let table = mode.table_name();

        let mut sql = format!(
            r#"
            SELECT
                m.id AS message_id,
                m.conversation_id AS conversation_id,
                m.idx AS message_idx,
                m.role AS role,
                m.content AS content,
                m.created_at AS created_at,
                c.created_at AS conv_created_at,
                c.updated_at AS conv_updated_at,
                c.source_id AS source_id,
                c.external_id AS external_id,
                c.title AS title,
                c.workspace AS workspace,
                s.adapter AS source_adapter,
                s.path AS source_path,
                snippet({table}, 0, '[', ']', '…', 12) AS snippet,
                bm25({table}) AS score
            FROM {table}
            JOIN messages m ON m.rowid = {table}.rowid
            JOIN conversations c ON c.id = m.conversation_id
            JOIN sources s ON s.id = c.source_id
            WHERE {table} MATCH ?
            "#
        );

        if opts.source_id.is_some() {
            sql.push_str(" AND c.source_id = ?");
        }
        if opts.workspace.is_some() {
            sql.push_str(" AND c.workspace = ?");
        }

        sql.push_str(" ORDER BY score ASC");

        if let Some(limit) = opts.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        if let Some(offset) = opts.offset {
            sql.push_str(&format!(" OFFSET {offset}"));
        }

        let mut query_builder = sqlx::query(&sql);
        query_builder = query_builder.bind(query);

        if let Some(ref source_id) = opts.source_id {
            query_builder = query_builder.bind(source_id);
        }
        if let Some(ref workspace) = opts.workspace {
            query_builder = query_builder.bind(workspace);
        }

        let rows = query_builder.fetch_all(&self.pool).await?;

        let mut hits = Vec::new();
        for row in rows {
            hits.push(SearchHit {
                message_id: Uuid::parse_str(row.get::<&str, _>("message_id")).unwrap_or_default(),
                conversation_id: Uuid::parse_str(row.get::<&str, _>("conversation_id"))
                    .unwrap_or_default(),
                message_idx: row.get("message_idx"),
                role: MessageRole::from(row.get::<&str, _>("role")),
                content: row.get("content"),
                snippet: row.get::<String, _>("snippet"),
                created_at: row
                    .get::<Option<i64>, _>("created_at")
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc)),
                conv_created_at: {
                    let ts = row.get::<i64, _>("conv_created_at");
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now)
                },
                conv_updated_at: row
                    .get::<Option<i64>, _>("conv_updated_at")
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc)),
                score: row.get::<f32, _>("score"),
                source_id: row.get("source_id"),
                external_id: row.get("external_id"),
                title: row.get("title"),
                workspace: row.get("workspace"),
                source_adapter: row.get("source_adapter"),
                source_path: row.get("source_path"),
            });
        }

        Ok(hits)
    }

    async fn ensure_fts_schema(&self) -> Result<()> {
        let messages_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await?;

        self.ensure_fts_table(
            "messages_fts",
            r#"
            CREATE VIRTUAL TABLE messages_fts USING fts5(
                content,
                content=messages,
                content_rowid=rowid,
                tokenize = 'porter',
                prefix = '2 3 4'
            );
            "#,
            &[
                r#"
                CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                "#,
                r#"
                CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                END;
                "#,
                r#"
                CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                "#,
            ],
            &["messages_ai", "messages_ad", "messages_au"],
            messages_count.0,
            |sql| sql.contains("tokenize = 'porter'") && sql.contains("prefix = '2 3 4'"),
        )
        .await?;

        self.ensure_fts_table(
            "messages_code_fts",
            r#"
            CREATE VIRTUAL TABLE messages_code_fts USING fts5(
                content,
                content=messages,
                content_rowid=rowid,
                tokenize = "unicode61 tokenchars '_./:'",
                prefix = '2 3 4'
            );
            "#,
            &[
                r#"
                CREATE TRIGGER messages_code_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_code_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                "#,
                r#"
                CREATE TRIGGER messages_code_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                END;
                "#,
                r#"
                CREATE TRIGGER messages_code_au AFTER UPDATE ON messages BEGIN
                    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                    INSERT INTO messages_code_fts(rowid, content)
                    VALUES (NEW.rowid, NEW.content);
                END;
                "#,
            ],
            &["messages_code_ai", "messages_code_ad", "messages_code_au"],
            messages_count.0,
            |sql| sql.contains("unicode61") && sql.contains("tokenchars") && sql.contains("prefix"),
        )
        .await?;

        Ok(())
    }

    async fn ensure_fts_table<F>(
        &self,
        name: &str,
        create_sql: &str,
        trigger_sql: &[&str],
        trigger_names: &[&str],
        messages_count: i64,
        schema_ok: F,
    ) -> Result<()>
    where
        F: Fn(&str) -> bool,
    {
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT sql FROM sqlite_master WHERE type='table' AND name = ?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;

        let mut should_recreate = match existing {
            Some((sql,)) => !schema_ok(&sql),
            None => true,
        };

        // If the table exists and schema is OK, run a thorough integrity check.
        // The basic integrity-check can miss corruption that the ranked version catches.
        if !should_recreate {
            match self.fts_integrity_check(name).await {
                Ok(true) => {} // Healthy
                Ok(false) => {
                    tracing::warn!("FTS table {name} is corrupted, will rebuild");
                    should_recreate = true;
                }
                Err(e) => {
                    tracing::warn!(
                        "FTS table {name} integrity check failed ({}), will rebuild",
                        e
                    );
                    should_recreate = true;
                }
            }
        }

        if should_recreate {
            self.rebuild_fts_table(name, create_sql, trigger_sql, trigger_names)
                .await?;
        }

        // Rebuild index if table is empty but messages exist
        if messages_count > 0 {
            let row_count: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {name}"))
                .fetch_one(&self.pool)
                .await?;
            if row_count.0 == 0 {
                let rebuild = format!("INSERT INTO {name}({name}) VALUES('rebuild')");
                sqlx::raw_sql(&rebuild).execute(&self.pool).await?;
            }
        }

        Ok(())
    }

    /// Run a thorough FTS5 integrity check.
    /// The `rank` parameter makes the check verify content matches the index.
    /// Returns Ok(true) if healthy, Ok(false) if corrupted.
    async fn fts_integrity_check(&self, name: &str) -> Result<bool> {
        // Use a dedicated connection to avoid transaction state issues
        let mut conn = self.pool.acquire().await?;

        let check_sql = format!("INSERT INTO {name}({name}, rank) VALUES('integrity-check', 1)");
        match sqlx::raw_sql(&check_sql).execute(&mut *conn).await {
            Ok(_) => Ok(true),
            Err(e) => {
                // Check if it's a corruption error (code 267 = SQLITE_CORRUPT)
                let err_str = e.to_string();
                if err_str.contains("malformed") || err_str.contains("corrupt") {
                    Ok(false)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Drop and recreate an FTS table with its triggers.
    async fn rebuild_fts_table(
        &self,
        name: &str,
        create_sql: &str,
        trigger_sql: &[&str],
        trigger_names: &[&str],
    ) -> Result<()> {
        // Use a dedicated connection for the entire rebuild to avoid lock issues
        let mut conn = self.pool.acquire().await?;

        for trigger in trigger_names {
            let drop_sql = format!("DROP TRIGGER IF EXISTS {trigger}");
            sqlx::raw_sql(&drop_sql).execute(&mut *conn).await?;
        }

        let drop_table = format!("DROP TABLE IF EXISTS {name}");
        sqlx::raw_sql(&drop_table).execute(&mut *conn).await?;

        sqlx::raw_sql(create_sql).execute(&mut *conn).await?;

        for sql in trigger_sql {
            sqlx::raw_sql(sql).execute(&mut *conn).await?;
        }

        // Rebuild the index from the content table
        let rebuild = format!("INSERT INTO {name}({name}) VALUES('rebuild')");
        sqlx::raw_sql(&rebuild).execute(&mut *conn).await?;

        tracing::info!("Rebuilt FTS table {name}");
        Ok(())
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

/// Options for search queries.
#[derive(Debug, Default)]
pub struct SearchOptions {
    pub source_id: Option<String>,
    pub workspace: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub mode: SearchMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    #[default]
    Auto,
    NaturalLanguage,
    Code,
}

impl SearchMode {
    fn resolve(self, query: &str) -> SearchMode {
        match self {
            SearchMode::Auto => detect_search_mode(query),
            other => other,
        }
    }

    fn table_name(self) -> &'static str {
        match self {
            SearchMode::Auto | SearchMode::NaturalLanguage => "messages_fts",
            SearchMode::Code => "messages_code_fts",
        }
    }
}

fn detect_search_mode(query: &str) -> SearchMode {
    let has_path = query.contains('/') || query.contains('\\');
    let has_scope = query.contains("::") || query.contains("->");
    let has_snake = query.contains('_');
    let has_dot = query.contains('.');
    let has_camel = query
        .chars()
        .zip(query.chars().skip(1))
        .any(|(a, b)| a.is_lowercase() && b.is_uppercase());

    if has_path || has_scope || has_snake || has_dot || has_camel {
        SearchMode::Code
    } else {
        SearchMode::NaturalLanguage
    }
}

fn conversation_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Conversation> {
    Ok(Conversation {
        id: Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default(),
        source_id: row.get("source_id"),
        external_id: row.get("external_id"),
        readable_id: row.get("readable_id"),
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
        parts_json: row
            .get::<Option<String>, _>("parts_json")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!([])),
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

fn normalize_parts_json(parts_json: &serde_json::Value) -> serde_json::Value {
    match parts_json {
        serde_json::Value::Array(_) => parts_json.clone(),
        _ => serde_json::json!([]),
    }
}

fn project_content(content: &str, parts_json: &serde_json::Value) -> String {
    let serde_json::Value::Array(parts) = parts_json else {
        return content.to_string();
    };

    let should_project = content.trim().is_empty() || should_project_from_parts(content, parts);
    if !should_project {
        return content.to_string();
    };

    let mut text_parts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut total_chars = 0usize;
    for part in parts {
        let serde_json::Value::Object(obj) = part else {
            continue;
        };
        let part_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        let raw_text = match part_type {
            "text" | "thinking" => obj.get("text").and_then(|v| v.as_str()),
            "status" | "error" => obj
                .get("message")
                .or_else(|| obj.get("text"))
                .and_then(|v| v.as_str()),
            _ => None,
        };

        let Some(raw_text) = raw_text else { continue };
        let sanitized = sanitize_projection_text(raw_text);
        if sanitized.is_empty() {
            continue;
        }
        let key = sanitized.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        total_chars += sanitized.chars().count();
        if total_chars > 4000 {
            break;
        }
        text_parts.push(sanitized);
    }

    if text_parts.is_empty() {
        content.to_string()
    } else {
        text_parts.join("\n\n")
    }
}

fn should_project_from_parts(content: &str, parts: &[serde_json::Value]) -> bool {
    if parts.is_empty() {
        return false;
    }

    let line_count = content.lines().count();
    let char_count = content.chars().count();
    char_count > 2000 || line_count > 80
}

fn sanitize_projection_text(text: &str) -> String {
    const MAX_CHARS: usize = 500;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let truncated: String = trimmed.chars().take(MAX_CHARS).collect();
    truncated
}

fn readable_id_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    let serde_json::Value::Object(map) = metadata else {
        return None;
    };
    map.get("readableId")
        .or_else(|| map.get("readable_id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

fn generate_readable_id(conv: &Conversation) -> String {
    // Deterministic, human-readable IDs based on stable identifiers.
    const ADJECTIVES: &[&str] = &[
        "amber", "brisk", "calm", "daring", "eager", "fuzzy", "gentle", "hazy", "icy", "jolly",
        "keen", "lucky", "mellow", "nimble", "proud", "swift",
    ];
    const VERBS: &[&str] = &[
        "builds", "checks", "crafts", "drives", "explores", "fixes", "guides", "helps", "joins",
        "keeps", "learns", "moves", "patches", "routes", "shapes", "tests",
    ];
    const NOUNS: &[&str] = &[
        "anchor", "beacon", "circuit", "delta", "ember", "forest", "galaxy", "harbor", "island",
        "junction", "kernel", "ladder", "matrix", "nebula", "orchid", "pioneer",
    ];

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    conv.id.hash(&mut hasher);
    conv.source_id.hash(&mut hasher);
    conv.external_id.hash(&mut hasher);
    conv.title.hash(&mut hasher);
    let hash = hasher.finish();

    let adj = ADJECTIVES[(hash as usize) % ADJECTIVES.len()];
    let verb = VERBS[((hash >> 8) as usize) % VERBS.len()];
    let noun = NOUNS[((hash >> 16) as usize) % NOUNS.len()];
    format!("{adj}-{verb}-{noun}")
}
