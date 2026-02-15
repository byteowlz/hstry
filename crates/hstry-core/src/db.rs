//! Database operations for hstry.

use crate::error::{Error, Result};
use crate::models::{
    Conversation, ConversationSnapshot, Message, MessageEvent, MessageRole, SearchHit, Source,
};
use crate::schema::SCHEMA;
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::fmt::Write;
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

/// Database handle for hstry.
pub struct Database {
    pool: SqlitePool,
}

/// Normalize a source path for consistent comparison.
/// Trims trailing slashes and handles path normalization.
fn normalize_source_path(path: Option<&String>) -> Option<String> {
    path.map(|p| p.trim_end_matches('/').to_string())
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

    /// Initialize schema and run migrations.
    async fn init(&self) -> Result<()> {
        sqlx::raw_sql(SCHEMA).execute(&self.pool).await?;
        self.run_migrations().await?;
        self.ensure_conversations_readable_id_column().await?;
        self.ensure_conversations_provider_column().await?;
        self.ensure_messages_parts_column().await?;
        self.ensure_fts_schema_optimized().await?;
        Ok(())
    }

    /// Run all pending migrations from the migrations directory.
    async fn run_migrations(&self) -> Result<()> {
        // Try to find migrations directory:
        // 1. HSTRY_MIGRATIONS_DIR environment variable
        // 2. CARGO_MANIFEST_DIR/migrations (when running from source)
        // 3. XDG_DATA_HOME/hstry/migrations (user data directory)
        // 4. ./migrations (fallback for development)

        let migrations_dir = if let Ok(dir) = std::env::var("HSTRY_MIGRATIONS_DIR") {
            std::path::PathBuf::from(dir)
        } else if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            let path = std::path::PathBuf::from(manifest_dir).join("migrations");
            if path.exists() {
                path
            } else {
                // Fallback to embedded migrations for production builds
                return self.run_embedded_migrations().await;
            }
        } else if let Some(data_dir) = dirs::data_dir() {
            let path = data_dir.join("hstry").join("migrations");
            if path.exists() {
                path
            } else {
                return self.run_embedded_migrations().await;
            }
        } else {
            std::path::PathBuf::from("migrations")
        };

        if !migrations_dir.exists() {
            return self.run_embedded_migrations().await;
        }

        let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.path());

        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sql") {
                continue;
            }

            let filename = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| Error::Other("Invalid migration filename".to_string()))?;

            // Parse version from filename (e.g., "001_initial_schema.sql" -> 1)
            let version = filename
                .split('_')
                .next()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| Error::Other(format!("Invalid migration filename: {filename}")))?;

            // Check if already applied
            let applied = sqlx::query("SELECT 1 FROM schema_migrations WHERE version = ?")
                .bind(version)
                .fetch_optional(&self.pool)
                .await?;

            if applied.is_some() {
                continue; // Already applied
            }

            // Read and execute migration
            let migration_sql = std::fs::read_to_string(&path)?;
            tracing::info!("Running migration: {filename}");

            let mut tx = self.pool.begin().await?;
            sqlx::raw_sql(&migration_sql).execute(&mut *tx).await?;

            // Record migration
            sqlx::query(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
            )
            .bind(version)
            .bind(filename)
            .bind(Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            tracing::info!("Applied migration: {filename}");
        }

        Ok(())
    }

    /// Run embedded migrations (compiled into the binary).
    async fn run_embedded_migrations(&self) -> Result<()> {
        // Embedded migrations - update this list when adding new migrations
        let migrations: &[(&str, &str)] = &[
            (
                "001_initial_schema.sql",
                include_str!("../migrations/001_initial_schema.sql"),
            ),
            (
                "002_add_provider_column.sql",
                include_str!("../migrations/002_add_provider_column.sql"),
            ),
            (
                "003_add_provider_index.sql",
                include_str!("../migrations/003_add_provider_index.sql"),
            ),
            (
                "004_add_events_and_snapshots.sql",
                include_str!("../migrations/004_add_events_and_snapshots.sql"),
            ),
            (
                "005_add_conversation_summary_cache.sql",
                include_str!("../migrations/005_add_conversation_summary_cache.sql"),
            ),
            (
                "006_add_sender_and_provider_to_messages.sql",
                include_str!("../migrations/006_add_sender_and_provider_to_messages.sql"),
            ),
            (
                "007_add_harness_column.sql",
                include_str!("../migrations/007_add_harness_column.sql"),
            ),
            (
                "008_add_client_id_to_messages.sql",
                include_str!("../migrations/008_add_client_id_to_messages.sql"),
            ),
            (
                "009_performance_indexes.sql",
                include_str!("../migrations/009_performance_indexes.sql"),
            ),
            (
                "010_add_platform_id.sql",
                include_str!("../migrations/010_add_platform_id.sql"),
            ),
        ];

        for (filename, sql) in migrations {
            // Parse version from filename
            let version = filename
                .split('_')
                .next()
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| Error::Other(format!("Invalid migration filename: {filename}")))?;

            // Check if already applied
            let applied = sqlx::query("SELECT 1 FROM schema_migrations WHERE version = ?")
                .bind(version)
                .fetch_optional(&self.pool)
                .await?;

            if applied.is_some() {
                continue; // Already applied
            }

            // Execute migration
            tracing::info!("Running embedded migration: {filename}");

            let mut tx = self.pool.begin().await?;
            sqlx::raw_sql(sql).execute(&mut *tx).await?;

            // Record migration
            sqlx::query(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)",
            )
            .bind(version)
            .bind(*filename)
            .bind(Utc::now().timestamp())
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            tracing::info!("Applied embedded migration: {filename}");
        }

        Ok(())
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
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
                    platform_id: None,
                    title: title.clone(),
                    created_at: Utc::now(),
                    updated_at: None,
                    model: None,
                    provider: None,
                    workspace: None,
                    tokens_in: None,
                    tokens_out: None,
                    cost_usd: None,
                    metadata: metadata.clone(),
                    harness: None,
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

    async fn ensure_conversations_provider_column(&self) -> Result<()> {
        let rows = sqlx::query("PRAGMA table_info(conversations)")
            .fetch_all(&self.pool)
            .await?;

        let has_provider = rows
            .iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .any(|name| name == "provider");

        if !has_provider {
            sqlx::query("ALTER TABLE conversations ADD COLUMN provider TEXT")
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
        let normalized_path = normalize_source_path(source.path.as_ref());
        sqlx::query(
            r"
            INSERT INTO sources (id, adapter, path, last_sync_at, config)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                adapter = excluded.adapter,
                path = excluded.path,
                last_sync_at = excluded.last_sync_at,
                config = excluded.config
            ",
        )
        .bind(&source.id)
        .bind(&source.adapter)
        .bind(&normalized_path)
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
        let normalized_path = path.trim_end_matches('/');
        let row = sqlx::query("SELECT * FROM sources WHERE adapter = ? AND path = ?")
            .bind(adapter)
            .bind(normalized_path)
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
        let readable_id = if let Some(id) = conv.readable_id.clone() {
            Some(id)
        } else {
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
        };

        sqlx::query(
            r"
            INSERT INTO conversations (id, source_id, external_id, readable_id, platform_id, title, created_at, updated_at, model, provider, workspace, tokens_in, tokens_out, cost_usd, metadata, harness)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, external_id) DO UPDATE SET
                readable_id = COALESCE(excluded.readable_id, conversations.readable_id),
                platform_id = COALESCE(excluded.platform_id, conversations.platform_id),
                title = excluded.title,
                updated_at = excluded.updated_at,
                model = excluded.model,
                provider = excluded.provider,
                workspace = excluded.workspace,
                tokens_in = excluded.tokens_in,
                tokens_out = excluded.tokens_out,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata,
                harness = COALESCE(excluded.harness, conversations.harness)
            ",
        )
        .bind(conv.id.to_string())
        .bind(&conv.source_id)
        .bind(&conv.external_id)
        .bind(&readable_id)
        .bind(&conv.platform_id)
        .bind(&conv.title)
        .bind(conv.created_at.timestamp())
        .bind(conv.updated_at.map(|dt| dt.timestamp()))
        .bind(&conv.model)
        .bind(&conv.provider)
        .bind(&conv.workspace)
        .bind(conv.tokens_in)
        .bind(conv.tokens_out)
        .bind(conv.cost_usd)
        .bind(conv.metadata.to_string())
        .bind(&conv.harness)
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
        if let Some(ref workspace) = opts.workspace {
            if is_like_pattern(workspace) {
                sql.push_str(" AND workspace LIKE ?");
            } else {
                sql.push_str(" AND workspace = ?");
            }
        }
        if opts.after.is_some() {
            sql.push_str(" AND created_at > ?");
        }

        sql.push_str(" ORDER BY COALESCE(updated_at, created_at) DESC");

        if let Some(limit) = opts.limit {
            let _ = write!(sql, " LIMIT {limit}");
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
            convs.push(conversation_from_row(&row));
        }
        Ok(convs)
    }

    /// List conversations with optional filters and include first user message.
    pub async fn list_conversation_previews(
        &self,
        opts: ListConversationsOptions,
    ) -> Result<Vec<ConversationPreview>> {
        let mut sql = String::from(
            "SELECT c.*, (SELECT content FROM messages m WHERE m.conversation_id = c.id AND m.role = 'user' ORDER BY m.idx ASC LIMIT 1) AS first_user_message FROM conversations c WHERE 1=1",
        );

        if opts.source_id.is_some() {
            sql.push_str(" AND c.source_id = ?");
        }
        if let Some(ref workspace) = opts.workspace {
            if is_like_pattern(workspace) {
                sql.push_str(" AND c.workspace LIKE ?");
            } else {
                sql.push_str(" AND c.workspace = ?");
            }
        }
        if opts.after.is_some() {
            sql.push_str(" AND c.created_at > ?");
        }

        sql.push_str(" ORDER BY COALESCE(c.updated_at, c.created_at) DESC");

        if let Some(limit) = opts.limit {
            let _ = write!(sql, " LIMIT {limit}");
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

        let mut previews = Vec::new();
        for row in rows {
            previews.push(ConversationPreview {
                conversation: conversation_from_row(&row),
                first_user_message: row.get("first_user_message"),
            });
        }

        Ok(previews)
    }

    /// List conversations with message counts and first user message.
    pub async fn list_conversation_summaries(
        &self,
        opts: ListConversationsOptions,
    ) -> Result<Vec<ConversationSummary>> {
        let mut sql = String::from(
            "SELECT c.*, \
             COALESCE(cs.message_count, (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id)) AS message_count, \
             COALESCE(cs.first_user_message, (SELECT content FROM messages m WHERE m.conversation_id = c.id AND m.role = 'user' ORDER BY m.idx ASC LIMIT 1)) AS first_user_message \
             FROM conversations c \
             LEFT JOIN conversation_summary_cache cs ON cs.conversation_id = c.id \
             WHERE 1=1",
        );

        if opts.source_id.is_some() {
            sql.push_str(" AND c.source_id = ?");
        }
        if let Some(ref workspace) = opts.workspace {
            if is_like_pattern(workspace) {
                sql.push_str(" AND c.workspace LIKE ?");
            } else {
                sql.push_str(" AND c.workspace = ?");
            }
        }
        if opts.after.is_some() {
            sql.push_str(" AND c.created_at > ?");
        }

        sql.push_str(" ORDER BY COALESCE(c.updated_at, c.created_at) DESC");

        if let Some(limit) = opts.limit {
            let _ = write!(sql, " LIMIT {limit}");
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
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            summaries.push(ConversationSummary {
                conversation: conversation_from_row(&row),
                message_count: row.get::<i64, _>("message_count"),
                first_user_message: row.get::<Option<String>, _>("first_user_message"),
            });
        }

        Ok(summaries)
    }

    /// Resolve a conversation using source_id and identifiers.
    pub async fn get_conversation_by_reference(
        &self,
        source_id: Option<&str>,
        external_id: Option<&str>,
        readable_id: Option<&str>,
        conversation_id: Option<&str>,
        workspace: Option<&str>,
    ) -> Result<Option<Conversation>> {
        let mut sql = String::from("SELECT * FROM conversations WHERE 1=1");

        if source_id.is_some() {
            sql.push_str(" AND source_id = ?");
        }
        if workspace.is_some() {
            sql.push_str(" AND workspace = ?");
        }

        if external_id.is_none() && readable_id.is_none() && conversation_id.is_none() {
            return Ok(None);
        }

        // Build OR clause matching any of: external_id, readable_id, id (UUID),
        // or platform_id.  The same value is tested against all applicable columns
        // so callers can pass a single identifier without knowing which column
        // it belongs to.
        sql.push_str(" AND (");
        let mut or_count = 0;

        // Helper macro to append "column = ?" with proper OR separator.
        macro_rules! or_clause {
            ($col:expr) => {
                if or_count > 0 {
                    write!(sql, " OR {} = ?", $col).unwrap();
                } else {
                    write!(sql, "{} = ?", $col).unwrap();
                }
                or_count += 1;
            };
        }

        if external_id.is_some() {
            or_clause!("external_id");
            or_clause!("platform_id"); // also try platform_id with the same value
        }
        if readable_id.is_some() {
            or_clause!("readable_id");
        }
        if conversation_id.is_some() {
            or_clause!("id");
        }
        sql.push_str(") LIMIT 1");

        let mut query = sqlx::query(&sql);

        if let Some(source_id) = source_id {
            query = query.bind(source_id);
        }
        if let Some(workspace) = workspace {
            query = query.bind(workspace);
        }
        if let Some(external_id) = external_id {
            query = query.bind(external_id);
            query = query.bind(external_id); // bind for platform_id check too
        }
        if let Some(readable_id) = readable_id {
            query = query.bind(readable_id);
        }
        if let Some(conversation_id) = conversation_id {
            query = query.bind(conversation_id);
        }

        let row = query.fetch_optional(&self.pool).await?;
        Ok(row.map(|row| conversation_from_row(&row)))
    }

    /// Get a conversation by ID.
    pub async fn get_conversation(&self, id: Uuid) -> Result<Option<Conversation>> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        match row {
            Some(row) => Ok(Some(conversation_from_row(&row))),
            None => Ok(None),
        }
    }

    /// Get conversation ID by source_id + external_id (or platform_id).
    pub async fn get_conversation_id(
        &self,
        source_id: &str,
        external_id: &str,
    ) -> Result<Option<Uuid>> {
        let row = sqlx::query(
            "SELECT id FROM conversations WHERE source_id = ? AND (external_id = ? OR platform_id = ?)",
        )
        .bind(source_id)
        .bind(external_id)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default()))
    }

    /// Check if a conversation already exists for a given adapter source.
    ///
    /// Checks `external_id` (primary key for dedup) and `readable_id` (which
    /// Octo uses to store its session UUID for reverse-lookup). This handles
    /// both new sessions (where Octo writes `external_id` = Pi native ID) and
    /// legacy sessions (where `external_id` was Octo's UUID and `readable_id`
    /// might not be set yet).
    pub async fn conversation_exists_for_session(
        &self,
        source_id: &str,
        external_id: &str,
    ) -> Result<bool> {
        let exists = sqlx::query(
            "SELECT 1 FROM conversations \
             WHERE source_id = ? \
             AND (external_id = ? OR readable_id = ?) \
             LIMIT 1",
        )
        .bind(source_id)
        .bind(external_id)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(exists.is_some())
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

    /// Count conversations and messages for a specific source.
    pub async fn count_source_data(&self, source_id: &str) -> Result<(i64, i64)> {
        let conv_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE source_id = ?")
                .bind(source_id)
                .fetch_one(&self.pool)
                .await?;

        let msg_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM messages m
             JOIN conversations c ON m.conversation_id = c.id
             WHERE c.source_id = ?",
        )
        .bind(source_id)
        .fetch_one(&self.pool)
        .await?;

        Ok((conv_count.0, msg_count.0))
    }

    /// Get detailed statistics per source.
    pub async fn get_source_stats(&self) -> Result<Vec<SourceStats>> {
        let rows = sqlx::query(
            r"
            SELECT
                s.id as source_id,
                s.adapter,
                COUNT(DISTINCT c.id) as conversations,
                COUNT(m.id) as messages,
                MIN(c.created_at) as oldest,
                MAX(c.created_at) as newest,
                s.last_sync_at
            FROM sources s
            LEFT JOIN conversations c ON c.source_id = s.id
            LEFT JOIN messages m ON m.conversation_id = c.id
            GROUP BY s.id, s.adapter, s.last_sync_at
            ORDER BY conversations DESC
            ",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(SourceStats {
                source_id: row.get("source_id"),
                adapter: row.get("adapter"),
                conversations: row.get::<i64, _>("conversations"),
                messages: row.get::<i64, _>("messages"),
                oldest: row
                    .get::<Option<i64>, _>("oldest")
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc)),
                newest: row
                    .get::<Option<i64>, _>("newest")
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc)),
                last_sync_at: row
                    .get::<Option<i64>, _>("last_sync_at")
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.with_timezone(&Utc)),
            });
        }
        Ok(stats)
    }

    /// Get activity stats (conversations per day/week/month).
    pub async fn get_activity_stats(&self, days: i64) -> Result<ActivityStats> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let cutoff_ts = cutoff.timestamp();

        let today_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE created_at >= ?")
                .bind((Utc::now() - chrono::Duration::days(1)).timestamp())
                .fetch_one(&self.pool)
                .await?;

        let week_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE created_at >= ?")
                .bind((Utc::now() - chrono::Duration::days(7)).timestamp())
                .fetch_one(&self.pool)
                .await?;

        let month_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE created_at >= ?")
                .bind((Utc::now() - chrono::Duration::days(30)).timestamp())
                .fetch_one(&self.pool)
                .await?;

        let period_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversations WHERE created_at >= ?")
                .bind(cutoff_ts)
                .fetch_one(&self.pool)
                .await?;

        Ok(ActivityStats {
            today: today_count.0,
            week: week_count.0,
            month: month_count.0,
            period: period_count.0,
            period_days: days,
        })
    }

    /// Delete a conversation and all its messages.
    pub async fn delete_conversation(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        // Delete messages first (foreign key)
        sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        // Delete conversation
        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Delete multiple conversations and all their associated data in a single transaction.
    /// Much faster than calling `delete_conversation` in a loop because it avoids
    /// per-row transaction overhead.
    pub async fn delete_conversations_batch(&self, ids: &[Uuid]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await?;

        // Process in chunks to stay within SQLite variable limits (max 999)
        for chunk in ids.chunks(500) {
            let placeholders: String = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");

            // Delete related tables first (message_events, snapshots, summary cache, messages)
            let sql = format!(
                "DELETE FROM message_events WHERE conversation_id IN ({placeholders})"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id.to_string());
            }
            query.execute(&mut *tx).await?;

            let sql = format!(
                "DELETE FROM conversation_snapshots WHERE conversation_id IN ({placeholders})"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id.to_string());
            }
            query.execute(&mut *tx).await?;

            let sql = format!(
                "DELETE FROM conversation_summary_cache WHERE conversation_id IN ({placeholders})"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id.to_string());
            }
            query.execute(&mut *tx).await?;

            let sql = format!(
                "DELETE FROM messages WHERE conversation_id IN ({placeholders})"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id.to_string());
            }
            query.execute(&mut *tx).await?;

            let sql = format!(
                "DELETE FROM conversations WHERE id IN ({placeholders})"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(id.to_string());
            }
            query.execute(&mut *tx).await?;
        }

        tx.commit().await?;
        Ok(ids.len())
    }

    /// Update a conversation's updated_at timestamp.
    pub async fn update_conversation_updated_at(
        &self,
        id: Uuid,
        updated_at: chrono::DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(updated_at.timestamp())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Partial update of conversation metadata.
    /// Only fields that are `Some` will be updated; `None` fields are left unchanged.
    /// Always bumps `updated_at` to now.
    pub async fn update_conversation_metadata(
        &self,
        id: Uuid,
        title: Option<&str>,
        workspace: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        metadata_json: Option<&serde_json::Value>,
        readable_id: Option<&str>,
        harness: Option<&str>,
        platform_id: Option<&str>,
    ) -> Result<()> {
        // Use COALESCE pattern: each field only updates if a non-NULL value is provided.
        // We pass NULL for fields that shouldn't change, and the COALESCE keeps the old value.
        let now = Utc::now().timestamp();
        let id_str = id.to_string();
        let metadata_str = metadata_json.map(|v| v.to_string());

        sqlx::query(
            r"UPDATE conversations SET
                title = COALESCE(?, title),
                workspace = COALESCE(?, workspace),
                model = COALESCE(?, model),
                provider = COALESCE(?, provider),
                metadata = COALESCE(?, metadata),
                readable_id = COALESCE(?, readable_id),
                harness = COALESCE(?, harness),
                platform_id = COALESCE(?, platform_id),
                updated_at = ?
            WHERE id = ?",
        )
        .bind(title)
        .bind(workspace)
        .bind(model)
        .bind(provider)
        .bind(metadata_str.as_deref())
        .bind(readable_id)
        .bind(harness)
        .bind(platform_id)
        .bind(now)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // =========================================================================
    // Messages
    // =========================================================================

    /// Insert a message.
    pub async fn insert_message(&self, msg: &Message) -> Result<()> {
        let exists = self.message_exists(msg.conversation_id, msg.idx).await?;
        let parts_json = normalize_parts_json(&msg.parts_json);
        let content = project_content(&msg.content, &parts_json);
        let sender_json = msg
            .sender
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());
        sqlx::query(
            r"
            INSERT INTO messages (id, conversation_id, idx, role, content, parts_json, created_at, model, tokens, cost_usd, metadata, sender_json, provider, harness, client_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(conversation_id, idx) DO UPDATE SET
                role = excluded.role,
                content = excluded.content,
                parts_json = excluded.parts_json,
                created_at = excluded.created_at,
                model = excluded.model,
                tokens = excluded.tokens,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata,
                sender_json = excluded.sender_json,
                provider = excluded.provider,
                harness = excluded.harness,
                client_id = excluded.client_id
            ",
        )
        .bind(msg.id.to_string())
        .bind(msg.conversation_id.to_string())
        .bind(msg.idx)
        .bind(msg.role.to_string())
        .bind(&content)
        .bind(parts_json.to_string())
        .bind(msg.created_at.map(|dt| dt.timestamp()))
        .bind(&msg.model)
        .bind(msg.tokens)
        .bind(msg.cost_usd)
        .bind(msg.metadata.to_string())
        .bind(&sender_json)
        .bind(&msg.provider)
        .bind(&msg.harness)
        .bind(&msg.client_id)
        .execute(&self.pool)
        .await?;
        self.insert_message_event(msg).await?;
        self.invalidate_conversation_snapshot(msg.conversation_id)
            .await?;
        if exists {
            self.rebuild_conversation_summary(msg.conversation_id)
                .await?;
        } else {
            self.bump_conversation_summary(msg, &content).await?;
        }
        Ok(())
    }

    /// Begin an explicit transaction. The caller must call `commit()` or `rollback()`.
    /// Use this to wrap multiple operations (e.g., bulk sync) in a single transaction.
    pub async fn begin(&self) -> Result<sqlx::Transaction<'_, sqlx::Sqlite>> {
        Ok(self.pool.begin().await?)
    }

    /// Insert a message within an existing transaction, skipping per-message
    /// event/snapshot/summary bookkeeping. Call `rebuild_conversation_summaries`
    /// after committing the transaction to reconcile caches.
    pub async fn insert_message_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        msg: &Message,
    ) -> Result<()> {
        let parts_json = normalize_parts_json(&msg.parts_json);
        let content = project_content(&msg.content, &parts_json);
        let sender_json = msg
            .sender
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());
        sqlx::query(
            r"
            INSERT INTO messages (id, conversation_id, idx, role, content, parts_json, created_at, model, tokens, cost_usd, metadata, sender_json, provider, harness, client_id)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(conversation_id, idx) DO UPDATE SET
                role = excluded.role,
                content = excluded.content,
                parts_json = excluded.parts_json,
                created_at = excluded.created_at,
                model = excluded.model,
                tokens = excluded.tokens,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata,
                sender_json = excluded.sender_json,
                provider = excluded.provider,
                harness = excluded.harness,
                client_id = excluded.client_id
            ",
        )
        .bind(msg.id.to_string())
        .bind(msg.conversation_id.to_string())
        .bind(msg.idx)
        .bind(msg.role.to_string())
        .bind(&content)
        .bind(parts_json.to_string())
        .bind(msg.created_at.map(|dt| dt.timestamp()))
        .bind(&msg.model)
        .bind(msg.tokens)
        .bind(msg.cost_usd)
        .bind(msg.metadata.to_string())
        .bind(&sender_json)
        .bind(&msg.provider)
        .bind(&msg.harness)
        .bind(&msg.client_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Upsert a conversation within an existing transaction.
    pub async fn upsert_conversation_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        conv: &Conversation,
    ) -> Result<()> {
        let readable_id = if let Some(id) = conv.readable_id.clone() {
            Some(id)
        } else {
            let from_meta = readable_id_from_metadata(&conv.metadata);
            if from_meta.is_some() {
                from_meta
            } else {
                Some(generate_readable_id(conv))
            }
        };

        sqlx::query(
            r"
            INSERT INTO conversations (id, source_id, external_id, readable_id, platform_id, title, created_at, updated_at, model, provider, workspace, tokens_in, tokens_out, cost_usd, metadata, harness)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, external_id) DO UPDATE SET
                readable_id = COALESCE(excluded.readable_id, conversations.readable_id),
                platform_id = COALESCE(excluded.platform_id, conversations.platform_id),
                title = excluded.title,
                updated_at = excluded.updated_at,
                model = excluded.model,
                provider = excluded.provider,
                workspace = excluded.workspace,
                tokens_in = excluded.tokens_in,
                tokens_out = excluded.tokens_out,
                cost_usd = excluded.cost_usd,
                metadata = excluded.metadata,
                harness = COALESCE(excluded.harness, conversations.harness)
            ",
        )
        .bind(conv.id.to_string())
        .bind(&conv.source_id)
        .bind(&conv.external_id)
        .bind(&readable_id)
        .bind(&conv.platform_id)
        .bind(&conv.title)
        .bind(conv.created_at.timestamp())
        .bind(conv.updated_at.map(|dt| dt.timestamp()))
        .bind(&conv.model)
        .bind(&conv.provider)
        .bind(&conv.workspace)
        .bind(conv.tokens_in)
        .bind(conv.tokens_out)
        .bind(conv.cost_usd)
        .bind(conv.metadata.to_string())
        .bind(&conv.harness)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    /// Rebuild conversation summary caches for the given conversation IDs.
    /// Call this after bulk-inserting messages via `insert_message_in_tx`.
    pub async fn rebuild_conversation_summaries(&self, conversation_ids: &[Uuid]) -> Result<()> {
        for id in conversation_ids {
            self.rebuild_conversation_summary(*id).await?;
            self.invalidate_conversation_snapshot(*id).await?;
        }
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
            messages.push(message_from_row(&row));
        }
        Ok(messages)
    }

    /// Get messages with snapshot caching.
    pub async fn get_messages_cached(&self, conversation_id: Uuid) -> Result<Vec<Message>> {
        let message_count = self
            .count_messages_for_conversation(conversation_id)
            .await?;
        if let Some(snapshot) = self.get_conversation_snapshot(conversation_id).await? {
            if snapshot.message_count == message_count {
                return Ok(snapshot.messages);
            }
        }

        let messages = self.get_messages(conversation_id).await?;
        self.upsert_conversation_snapshot(conversation_id, message_count, &messages)
            .await?;
        Ok(messages)
    }

    /// Get message events for a conversation with optional cursor/limit.
    pub async fn get_message_events(
        &self,
        conversation_id: Uuid,
        after_idx: Option<i32>,
        after_created_at_ms: Option<i64>,
        limit: Option<i64>,
    ) -> Result<Vec<MessageEvent>> {
        let mut sql = String::from(
            "SELECT id, conversation_id, idx, payload_json, created_at, metadata \
             FROM message_events WHERE conversation_id = ?",
        );

        if after_idx.is_some() {
            sql.push_str(" AND idx > ?");
        }
        if after_created_at_ms.is_some() {
            sql.push_str(" AND created_at > ?");
        }

        sql.push_str(" ORDER BY idx");

        if let Some(limit) = limit {
            let _ = write!(sql, " LIMIT {limit}");
        }

        let mut query = sqlx::query(&sql).bind(conversation_id.to_string());
        if let Some(after_idx) = after_idx {
            query = query.bind(after_idx);
        }
        if let Some(after_created_at_ms) = after_created_at_ms {
            query = query.bind(after_created_at_ms / 1000);
        }

        let rows = query.fetch_all(&self.pool).await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let created_at = row
                .get::<Option<i64>, _>("created_at")
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.with_timezone(&Utc));
            let metadata = row
                .get::<Option<String>, _>("metadata")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            events.push(MessageEvent {
                id: Uuid::parse_str(row.get::<String, _>("id").as_str()).unwrap_or_default(),
                conversation_id,
                idx: row.get("idx"),
                payload_json: row.get("payload_json"),
                created_at,
                metadata,
            });
        }
        Ok(events)
    }

    pub async fn count_messages_for_conversation(&self, conversation_id: Uuid) -> Result<i64> {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM messages WHERE conversation_id = ?")
                .bind(conversation_id.to_string())
                .fetch_one(&self.pool)
                .await?;
        Ok(count.0)
    }

    /// Get message count.
    pub async fn count_messages(&self) -> Result<i64> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0)
    }

    // =========================================================================
    // Message Events + Snapshots
    // =========================================================================

    async fn insert_message_event(&self, msg: &Message) -> Result<()> {
        let payload = serde_json::to_string(msg)?;
        sqlx::query(
            r"
            INSERT INTO message_events (id, conversation_id, idx, payload_json, created_at, metadata)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                conversation_id = excluded.conversation_id,
                idx = excluded.idx,
                payload_json = excluded.payload_json,
                created_at = excluded.created_at,
                metadata = excluded.metadata
            ",
        )
        .bind(msg.id.to_string())
        .bind(msg.conversation_id.to_string())
        .bind(msg.idx)
        .bind(payload)
        .bind(msg.created_at.map(|dt| dt.timestamp()))
        .bind(msg.metadata.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn message_exists(&self, conversation_id: Uuid, idx: i32) -> Result<bool> {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM messages WHERE conversation_id = ? AND idx = ?")
                .bind(conversation_id.to_string())
                .bind(idx)
                .fetch_one(&self.pool)
                .await?;
        Ok(count.0 > 0)
    }

    async fn invalidate_conversation_snapshot(&self, conversation_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM conversation_snapshots WHERE conversation_id = ?")
            .bind(conversation_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_conversation_snapshot(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<ConversationSnapshot>> {
        let row = sqlx::query(
            "SELECT conversation_id, message_count, payload_json, updated_at FROM conversation_snapshots WHERE conversation_id = ?",
        )
        .bind(conversation_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let payload: String = row.get("payload_json");
        let messages: Vec<Message> = serde_json::from_str(&payload).unwrap_or_default();
        Ok(Some(ConversationSnapshot {
            conversation_id,
            message_count: row.get::<i64, _>("message_count"),
            messages,
        }))
    }

    async fn upsert_conversation_snapshot(
        &self,
        conversation_id: Uuid,
        message_count: i64,
        messages: &[Message],
    ) -> Result<()> {
        let payload = serde_json::to_string(messages)?;
        let updated_at = chrono::Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO conversation_snapshots (conversation_id, message_count, payload_json, updated_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(conversation_id) DO UPDATE SET
                message_count = excluded.message_count,
                payload_json = excluded.payload_json,
                updated_at = excluded.updated_at
            ",
        )
        .bind(conversation_id.to_string())
        .bind(message_count)
        .bind(payload)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn bump_conversation_summary(&self, msg: &Message, content: &str) -> Result<()> {
        let updated_at = chrono::Utc::now().timestamp();
        let first_user_message = if msg.role == MessageRole::User {
            Some(content.to_string())
        } else {
            None
        };

        sqlx::query(
            r"
            INSERT INTO conversation_summary_cache (conversation_id, message_count, first_user_message, updated_at)
            VALUES (?, 1, ?, ?)
            ON CONFLICT(conversation_id) DO UPDATE SET
                message_count = message_count + 1,
                first_user_message = COALESCE(first_user_message, excluded.first_user_message),
                updated_at = excluded.updated_at
            ",
        )
        .bind(msg.conversation_id.to_string())
        .bind(first_user_message)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn rebuild_conversation_summary(&self, conversation_id: Uuid) -> Result<()> {
        let updated_at = chrono::Utc::now().timestamp();
        sqlx::query(
            r"
            INSERT INTO conversation_summary_cache (conversation_id, message_count, first_user_message, updated_at)
            SELECT
                c.id,
                COUNT(m.id) AS message_count,
                (
                    SELECT content
                    FROM messages m2
                    WHERE m2.conversation_id = c.id
                      AND m2.role = 'user'
                    ORDER BY m2.idx ASC
                    LIMIT 1
                ) AS first_user_message,
                ?
            FROM conversations c
            LEFT JOIN messages m ON m.conversation_id = c.id
            WHERE c.id = ?
            GROUP BY c.id
            ON CONFLICT(conversation_id) DO UPDATE SET
                message_count = excluded.message_count,
                first_user_message = excluded.first_user_message,
                updated_at = excluded.updated_at
            ",
        )
        .bind(updated_at)
        .bind(conversation_id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // =========================================================================
    // Attachments
    // =========================================================================

    /// Insert a binary attachment for a message.
    pub async fn insert_attachment(
        &self,
        id: &str,
        message_id: Uuid,
        mime_type: &str,
        filename: Option<&str>,
        data: &[u8],
    ) -> Result<()> {
        // Determine type from mime_type
        let attachment_type = if mime_type.starts_with("image/") {
            "image"
        } else if mime_type.starts_with("audio/") {
            "audio"
        } else if mime_type.starts_with("video/") {
            "video"
        } else {
            "file"
        };

        sqlx::query(
            r"
            INSERT INTO attachments (id, message_id, type, name, mime_type, content)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(id)
        .bind(message_id.to_string())
        .bind(attachment_type)
        .bind(filename)
        .bind(mime_type)
        .bind(data)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get attachment binary data by ID.
    pub async fn get_attachment(&self, id: &str) -> Result<Option<(String, Vec<u8>)>> {
        let row = sqlx::query("SELECT mime_type, content FROM attachments WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|row| {
            let mime_type: String = row.get("mime_type");
            let content: Vec<u8> = row.get("content");
            (mime_type, content)
        }))
    }

    // =========================================================================
    // Search state
    // =========================================================================

    pub async fn get_search_state(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM search_state WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| row.get::<String, _>("value")))
    }

    pub async fn set_search_state(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO search_state (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // =========================================================================
    // Search
    // =========================================================================

    /// Full-text search across messages with snippet and provenance.
    pub async fn search(&self, query: &str, opts: SearchOptions) -> Result<Vec<SearchHit>> {
        let mode = opts.mode.resolve(query);
        let table = mode.table_name();
        let query = sanitize_fts_query(query);

        let mut sql = format!(
            r"
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
            "
        );

        if opts.source_id.is_some() {
            sql.push_str(" AND c.source_id = ?");
        }
        if opts.workspace.is_some() {
            sql.push_str(" AND c.workspace = ?");
        }

        sql.push_str(" ORDER BY score ASC");

        if let Some(limit) = opts.limit {
            let _ = write!(sql, " LIMIT {limit}");
        }
        if let Some(offset) = opts.offset {
            let _ = write!(sql, " OFFSET {offset}");
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
                        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc))
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
                host: None,
            });
        }

        Ok(hits)
    }

    /// Optimized FTS schema initialization that checks integrity only when needed.
    async fn ensure_fts_schema_optimized(&self) -> Result<()> {
        // Check if we've already validated this database version
        let fts_key = "fts_last_integrity_check_v1";
        let last_check = self.get_search_state(fts_key).await?;

        // Only run integrity check if: never checked, or DB was recently modified
        let should_check = match last_check {
            None => true,
            Some(ts) => {
                // Parse timestamp and check if > 1 hour old
                match ts.parse::<i64>() {
                    Ok(secs) => {
                        let now = Utc::now().timestamp();
                        (now - secs) > 3600 // Only check every hour
                    }
                    Err(_) => true,
                }
            }
        };

        if !should_check {
            // Skip expensive checks, just ensure tables exist
            self.ensure_fts_tables_exist().await?;
            return Ok(());
        }

        // Run full check
        self.ensure_fts_schema().await?;

        // Mark as checked
        self.set_search_state(fts_key, &Utc::now().timestamp().to_string())
            .await?;

        Ok(())
    }

    /// Ensure FTS tables exist without expensive integrity checks.
    async fn ensure_fts_tables_exist(&self) -> Result<()> {
        for table_name in ["messages_fts", "messages_code_fts"] {
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT sql FROM sqlite_master WHERE type='table' AND name = ?")
                    .bind(table_name)
                    .fetch_optional(&self.pool)
                    .await?;

            if existing.is_none() {
                // Create table if missing
                let (create_sql, trigger_sql, trigger_names) = match table_name {
                    "messages_fts" => (
                        r"
                        CREATE VIRTUAL TABLE messages_fts USING fts5(
                            content,
                            content=messages,
                            content_rowid=rowid,
                            tokenize = 'porter',
                            prefix = '2 3 4'
                        );
                        ",
                        &[
                            r"
                            CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
                                INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                            END;
                            ",
                            r"
                            CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
                                INSERT INTO messages_fts(messages_fts, rowid, content)
                                VALUES('delete', OLD.rowid, OLD.content);
                            END;
                            ",
                            r"
                            CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
                                INSERT INTO messages_fts(messages_fts, rowid, content)
                                VALUES('delete', OLD.rowid, OLD.content);
                                INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                            END;
                            ",
                        ] as &[&str],
                        &["messages_ai", "messages_ad", "messages_au"] as &[&str],
                    ),
                    "messages_code_fts" => (
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
                            r"
                            CREATE TRIGGER messages_code_ai AFTER INSERT ON messages BEGIN
                                INSERT INTO messages_code_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                            END;
                            ",
                            r"
                            CREATE TRIGGER messages_code_ad AFTER DELETE ON messages BEGIN
                                INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                                VALUES('delete', OLD.rowid, OLD.content);
                            END;
                            ",
                            r"
                            CREATE TRIGGER messages_code_au AFTER UPDATE ON messages BEGIN
                                INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                                VALUES('delete', OLD.rowid, OLD.content);
                                INSERT INTO messages_code_fts(rowid, content)
                                VALUES (NEW.rowid, NEW.content);
                            END;
                            ",
                        ] as &[&str],
                        &["messages_code_ai", "messages_code_ad", "messages_code_au"] as &[&str],
                    ),
                    _ => unreachable!(),
                };

                let mut conn = self.pool.acquire().await?;
                for trigger in trigger_names {
                    let drop_sql = format!("DROP TRIGGER IF EXISTS {trigger}");
                    sqlx::raw_sql(&drop_sql).execute(&mut *conn).await?;
                }
                let drop_table = format!("DROP TABLE IF EXISTS {table_name}");
                sqlx::raw_sql(&drop_table).execute(&mut *conn).await?;
                sqlx::raw_sql(create_sql).execute(&mut *conn).await?;
                for sql in trigger_sql {
                    sqlx::raw_sql(sql).execute(&mut *conn).await?;
                }

                // Rebuild from existing messages
                let messages_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
                    .fetch_one(&self.pool)
                    .await?;
                if messages_count.0 > 0 {
                    let rebuild =
                        format!("INSERT INTO {table_name}({table_name}) VALUES('rebuild')");
                    sqlx::raw_sql(&rebuild).execute(&self.pool).await?;
                }

                tracing::info!("Created FTS table {table_name}");
            }
        }

        Ok(())
    }

    async fn ensure_fts_schema(&self) -> Result<()> {
        let messages_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM messages")
            .fetch_one(&self.pool)
            .await?;

        self.ensure_fts_table(
            "messages_fts",
            r"
            CREATE VIRTUAL TABLE messages_fts USING fts5(
                content,
                content=messages,
                content_rowid=rowid,
                tokenize = 'porter',
                prefix = '2 3 4'
            );
            ",
            &[
                r"
                CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                ",
                r"
                CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                END;
                ",
                r"
                CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                ",
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
                r"
                CREATE TRIGGER messages_code_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_code_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
                END;
                ",
                r"
                CREATE TRIGGER messages_code_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                END;
                ",
                r"
                CREATE TRIGGER messages_code_au AFTER UPDATE ON messages BEGIN
                    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
                    VALUES('delete', OLD.rowid, OLD.content);
                    INSERT INTO messages_code_fts(rowid, content)
                    VALUES (NEW.rowid, NEW.content);
                END;
                ",
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

/// Conversation preview with first user message.
#[derive(Debug, Clone)]
pub struct ConversationPreview {
    pub conversation: Conversation,
    pub first_user_message: Option<String>,
}

/// Conversation summary with message counts.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub conversation: Conversation,
    pub message_count: i64,
    pub first_user_message: Option<String>,
}

/// Statistics for a single source.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SourceStats {
    pub source_id: String,
    pub adapter: String,
    pub conversations: i64,
    pub messages: i64,
    pub oldest: Option<chrono::DateTime<Utc>>,
    pub newest: Option<chrono::DateTime<Utc>>,
    pub last_sync_at: Option<chrono::DateTime<Utc>>,
}

/// Activity statistics over time periods.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityStats {
    pub today: i64,
    pub week: i64,
    pub month: i64,
    pub period: i64,
    pub period_days: i64,
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
#[derive(Debug, Default, Clone)]
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

fn sanitize_fts_query(query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        return String::new();
    }

    if query.contains(':') || query.contains('"') {
        let escaped = query.replace('"', "\"\"");
        return format!("\"{escaped}\"");
    }

    query.to_string()
}

fn is_like_pattern(value: &str) -> bool {
    value.contains('%') || value.contains('_')
}

fn conversation_from_row(row: &sqlx::sqlite::SqliteRow) -> Conversation {
    Conversation {
        id: Uuid::parse_str(row.get::<&str, _>("id")).unwrap_or_default(),
        source_id: row.get("source_id"),
        external_id: row.get("external_id"),
        readable_id: row.try_get("readable_id").ok(),
        platform_id: row.try_get("platform_id").ok().flatten(),
        title: row.get("title"),
        created_at: chrono::DateTime::from_timestamp(row.get::<i64, _>("created_at"), 0)
            .unwrap_or_default()
            .with_timezone(&Utc),
        updated_at: row
            .get::<Option<i64>, _>("updated_at")
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.with_timezone(&Utc)),
        model: row.get("model"),
        provider: row.try_get("provider").ok(),
        workspace: row.get("workspace"),
        tokens_in: row.get("tokens_in"),
        tokens_out: row.get("tokens_out"),
        cost_usd: row.get("cost_usd"),
        metadata: row
            .get::<Option<String>, _>("metadata")
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default(),
        harness: row.try_get("harness").ok().flatten(),
    }
}

fn message_from_row(row: &sqlx::sqlite::SqliteRow) -> Message {
    Message {
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
        sender: row
            .try_get::<Option<String>, _>("sender_json")
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok()),
        provider: row.try_get("provider").ok().flatten(),
        harness: row.try_get("harness").ok().flatten(),
        client_id: row.try_get("client_id").ok().flatten(),
    }
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
    }

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

    let adj_index = usize::try_from(hash % ADJECTIVES.len() as u64).unwrap_or_default();
    let verb_index = usize::try_from((hash >> 8) % VERBS.len() as u64).unwrap_or_default();
    let noun_index = usize::try_from((hash >> 16) % NOUNS.len() as u64).unwrap_or_default();
    let adj = ADJECTIVES[adj_index];
    let verb = VERBS[verb_index];
    let noun = NOUNS[noun_index];
    format!("{adj}-{verb}-{noun}")
}
