//! Remote sync functionality over SSH.
//!
//! Provides fetching and bidirectional merging of hstry databases across machines.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::config::RemoteConfig;
use crate::db::{Database, SearchOptions};
use crate::error::{Error, Result};
use crate::models::{Conversation, ConversationWithMessages, Message, SearchHit};

/// Default remote database path (XDG standard).
pub const DEFAULT_REMOTE_DB_PATH: &str = "~/.local/share/hstry/hstry.db";

#[derive(Debug, Deserialize)]
struct JsonResponse<T> {
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RemoteSearchInput {
    query: String,
    limit: Option<i64>,
    offset: Option<i64>,
    source: Option<String>,
    workspace: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct RemoteShowInput {
    id: String,
}

/// Result of a fetch operation.
#[derive(Debug, Clone, Serialize)]
pub struct FetchResult {
    pub remote_name: String,
    pub local_cache_path: PathBuf,
    pub bytes_transferred: u64,
    pub fetched_at: DateTime<Utc>,
}

/// Result of a sync/merge operation.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub remote_name: String,
    pub conversations_added: usize,
    pub conversations_updated: usize,
    pub messages_added: usize,
    pub sources_added: usize,
    pub direction: SyncDirection,
}

/// Sync direction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection {
    /// Pull from remote to local.
    Pull,
    /// Push from local to remote.
    Push,
    /// Bidirectional merge.
    Bidirectional,
}

impl std::fmt::Display for SyncDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncDirection::Pull => write!(f, "pull"),
            SyncDirection::Push => write!(f, "push"),
            SyncDirection::Bidirectional => write!(f, "bidirectional"),
        }
    }
}

/// SSH transport for remote operations.
pub struct SshTransport {
    host: String,
    port: Option<u16>,
    identity_file: Option<String>,
}

impl SshTransport {
    /// Create a new SSH transport from remote config.
    pub fn from_config(config: &RemoteConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            identity_file: config.identity_file.clone(),
        }
    }

    /// Build SSH command with common options.
    fn ssh_command(&self) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=10");

        if let Some(port) = self.port {
            cmd.arg("-p").arg(port.to_string());
        }

        if let Some(ref identity) = self.identity_file {
            let expanded = shellexpand::full(identity)
                .map_or_else(|_| identity.clone(), std::borrow::Cow::into_owned);
            cmd.arg("-i").arg(expanded);
        }

        cmd
    }

    /// Build SCP command with common options.
    fn scp_command(&self) -> Command {
        let mut cmd = Command::new("scp");
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg("-C"); // Enable compression

        if let Some(port) = self.port {
            cmd.arg("-P").arg(port.to_string());
        }

        if let Some(ref identity) = self.identity_file {
            let expanded = shellexpand::full(identity)
                .map_or_else(|_| identity.clone(), std::borrow::Cow::into_owned);
            cmd.arg("-i").arg(expanded);
        }

        cmd
    }

    /// Test connection to the remote host.
    pub fn test_connection(&self) -> Result<()> {
        let mut cmd = self.ssh_command();
        cmd.arg(&self.host).arg("echo").arg("ok");

        let output = cmd
            .output()
            .map_err(|e| Error::Remote(format!("Failed to execute ssh: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Remote(format!(
                "SSH connection failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }

    /// Fetch a file from the remote host to a local path.
    pub fn fetch_file(&self, remote_path: &str, local_path: &Path) -> Result<u64> {
        // Ensure parent directory exists
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Expand remote path (shell expansion happens on remote)
        let remote_spec = format!("{}:{}", self.host, remote_path);

        let mut cmd = self.scp_command();
        cmd.arg(&remote_spec).arg(local_path);

        let output = cmd
            .output()
            .map_err(|e| Error::Remote(format!("Failed to execute scp: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Remote(format!(
                "SCP fetch failed: {}",
                stderr.trim()
            )));
        }

        // Return file size
        let metadata = std::fs::metadata(local_path)?;
        Ok(metadata.len())
    }

    /// Push a file from local to remote.
    pub fn push_file(&self, local_path: &Path, remote_path: &str) -> Result<u64> {
        let metadata = std::fs::metadata(local_path)?;
        let size = metadata.len();

        let remote_spec = format!("{}:{}", self.host, remote_path);

        let mut cmd = self.scp_command();
        cmd.arg(local_path).arg(&remote_spec);

        let output = cmd
            .output()
            .map_err(|e| Error::Remote(format!("Failed to execute scp: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Remote(format!("SCP push failed: {}", stderr.trim())));
        }

        Ok(size)
    }

    /// Execute a command on the remote host and return stdout.
    pub fn exec(&self, command: &str) -> Result<String> {
        let mut cmd = self.ssh_command();
        cmd.arg(&self.host).arg(command);

        let output = cmd
            .output()
            .map_err(|e| Error::Remote(format!("Failed to execute ssh: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Remote(format!(
                "Remote command failed: {}",
                stderr.trim()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Check if a file exists on the remote.
    pub fn file_exists(&self, remote_path: &str) -> Result<bool> {
        let cmd = format!("test -f {remote_path} && echo yes || echo no");
        let output = self.exec(&cmd)?;
        Ok(output.trim() == "yes")
    }

    /// Get the expanded path on the remote (resolves ~ and env vars).
    pub fn expand_remote_path(&self, path: &str) -> Result<String> {
        let cmd = format!("echo {path}");
        let output = self.exec(&cmd)?;
        Ok(output.trim().to_string())
    }
}

/// Get the cache directory for remote databases.
pub fn remote_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hstry")
        .join("remotes")
}

/// Get the cached database path for a remote.
pub fn cached_db_path(remote_name: &str) -> PathBuf {
    remote_cache_dir().join(format!("{remote_name}.db"))
}

/// Fetch a remote database to local cache.
pub fn fetch_remote(config: &RemoteConfig) -> Result<FetchResult> {
    let transport = SshTransport::from_config(config);

    // Test connection first
    transport.test_connection()?;

    // Determine remote database path
    let remote_db_path = config
        .database_path
        .as_deref()
        .unwrap_or(DEFAULT_REMOTE_DB_PATH);

    // Expand the path on the remote
    let expanded_path = transport.expand_remote_path(remote_db_path)?;

    // Check if remote database exists
    if !transport.file_exists(&expanded_path)? {
        return Err(Error::Remote(format!(
            "Remote database not found at: {expanded_path}",
        )));
    }

    // Fetch to local cache
    let cache_path = cached_db_path(&config.name);
    let bytes = transport.fetch_file(&expanded_path, &cache_path)?;

    Ok(FetchResult {
        remote_name: config.name.clone(),
        local_cache_path: cache_path,
        bytes_transferred: bytes,
        fetched_at: Utc::now(),
    })
}

/// Merge conversations from a source database into a target database.
/// Uses updated_at for conflict resolution (newer wins).
pub async fn merge_databases(
    target: &Database,
    source_path: &Path,
    remote_name: &str,
) -> Result<SyncResult> {
    // Open the source database
    let source = Database::open(source_path).await?;

    let mut conversations_added = 0usize;
    let mut conversations_updated = 0usize;
    let mut messages_added = 0usize;
    let mut sources_added = 0usize;

    // Merge sources (prefixed with remote name to avoid conflicts)
    let remote_sources = source.list_sources().await?;
    for mut remote_source in remote_sources {
        // Prefix source ID with remote name to namespace it
        let namespaced_id = format!("{}:{}", remote_name, remote_source.id);
        remote_source.id = namespaced_id;

        // Check if source already exists
        let existing = target.get_source(&remote_source.id).await?;
        if existing.is_none() {
            target.upsert_source(&remote_source).await?;
            sources_added += 1;
        }
    }

    // Get all conversations from source
    let source_conversations = source
        .list_conversations(crate::db::ListConversationsOptions::default())
        .await?;

    for conv in source_conversations {
        // Namespace the source_id
        let namespaced_source_id = format!("{}:{}", remote_name, conv.source_id);

        // Check if conversation already exists (by external_id within namespaced source)
        let existing_id = if let Some(ref external_id) = conv.external_id {
            target
                .get_conversation_id(&namespaced_source_id, external_id)
                .await?
        } else {
            None
        };

        let (should_insert, conv_id) = if let Some(existing_uuid) = existing_id {
            // Conversation exists, check if we should update
            if let Some(existing_conv) = target.get_conversation(existing_uuid).await? {
                // Compare updated_at timestamps (newer wins)
                let should_update = match (conv.updated_at, existing_conv.updated_at) {
                    (Some(new_ts), Some(old_ts)) => new_ts > old_ts,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => conv.created_at > existing_conv.created_at,
                };
                if should_update {
                    conversations_updated += 1;
                    (true, existing_uuid)
                } else {
                    (false, existing_uuid)
                }
            } else {
                (true, existing_uuid)
            }
        } else {
            // New conversation
            conversations_added += 1;
            (true, Uuid::new_v4())
        };

        if should_insert {
            let merged_conv = Conversation {
                id: conv_id,
                source_id: namespaced_source_id.clone(),
                external_id: conv.external_id,
                readable_id: conv.readable_id,
                title: conv.title,
                created_at: conv.created_at,
                updated_at: conv.updated_at,
                model: conv.model,
                provider: conv.provider,
                workspace: conv.workspace,
                tokens_in: conv.tokens_in,
                tokens_out: conv.tokens_out,
                cost_usd: conv.cost_usd,
                metadata: conv.metadata,
            };

            target.upsert_conversation(&merged_conv).await?;

            // Merge messages
            let source_messages = source.get_messages(conv.id).await?;
            for msg in source_messages {
                let merged_msg = Message {
                    id: Uuid::new_v4(), // Generate new ID for target
                    conversation_id: conv_id,
                    idx: msg.idx,
                    role: msg.role,
                    content: msg.content,
                    parts_json: msg.parts_json,
                    created_at: msg.created_at,
                    model: msg.model,
                    tokens: msg.tokens,
                    cost_usd: msg.cost_usd,
                    metadata: msg.metadata,
                };
                target.insert_message(&merged_msg).await?;
                messages_added += 1;
            }
        }
    }

    source.close().await;

    Ok(SyncResult {
        remote_name: remote_name.to_string(),
        conversations_added,
        conversations_updated,
        messages_added,
        sources_added,
        direction: SyncDirection::Pull,
    })
}

/// Full sync operation: fetch remote DB and merge into local.
pub async fn sync_from_remote(
    local_db: &Database,
    config: &RemoteConfig,
) -> Result<(FetchResult, SyncResult)> {
    // Fetch the remote database
    let fetch_result = fetch_remote(config)?;

    // Merge into local
    let sync_result =
        merge_databases(local_db, &fetch_result.local_cache_path, &config.name).await?;

    Ok((fetch_result, sync_result))
}

/// Push local database to remote and merge.
pub async fn sync_to_remote(local_db_path: &Path, config: &RemoteConfig) -> Result<SyncResult> {
    let transport = SshTransport::from_config(config);

    // Test connection
    transport.test_connection()?;

    // Determine paths
    let remote_db_path = config
        .database_path
        .as_deref()
        .unwrap_or(DEFAULT_REMOTE_DB_PATH);
    let expanded_path = transport.expand_remote_path(remote_db_path)?;

    // Create a temporary merged database
    let temp_dir = tempfile::tempdir()?;
    let temp_db_path = temp_dir.path().join("merged.db");

    // If remote DB exists, fetch it first
    let remote_exists = transport.file_exists(&expanded_path)?;
    if remote_exists {
        transport.fetch_file(&expanded_path, &temp_db_path)?;
    }

    // Open/create the temp database
    let temp_db = Database::open(&temp_db_path).await?;

    // Merge local into temp (with namespace "local" for tracking)
    let sync_result = merge_databases(&temp_db, local_db_path, "local").await?;

    temp_db.close().await;

    // Push back to remote
    transport.push_file(&temp_db_path, &expanded_path)?;

    Ok(SyncResult {
        remote_name: config.name.clone(),
        conversations_added: sync_result.conversations_added,
        conversations_updated: sync_result.conversations_updated,
        messages_added: sync_result.messages_added,
        sources_added: sync_result.sources_added,
        direction: SyncDirection::Push,
    })
}

pub async fn search_remote(
    config: &RemoteConfig,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let transport = SshTransport::from_config(config);
    let input = RemoteSearchInput {
        query: query.to_string(),
        limit: opts.limit,
        offset: opts.offset,
        source: opts.source_id.clone(),
        workspace: opts.workspace.clone(),
        mode: Some(
            match opts.mode {
                crate::db::SearchMode::Auto => "auto",
                crate::db::SearchMode::NaturalLanguage => "natural",
                crate::db::SearchMode::Code => "code",
            }
            .to_string(),
        ),
    };
    let payload = serde_json::to_vec(&input)?;
    let host_name = config.name.clone();
    let host = config.host.clone();

    let hits = tokio::task::spawn_blocking(move || {
        let mut cmd = transport.ssh_command();
        cmd.arg(host)
            .arg("hstry")
            .arg("search")
            .arg("--json")
            .arg("--input")
            .arg("-");

        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::Remote(format!("Failed to start ssh: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&payload)
                .map_err(|e| Error::Remote(format!("Failed writing stdin: {e}")))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| Error::Remote(format!("SSH failed: {e}")))?;

        if !output.status.success() {
            return Err(Error::Remote(format!(
                "Remote search failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let response: JsonResponse<Vec<SearchHit>> = serde_json::from_slice(&output.stdout)
            .map_err(|e| Error::Remote(format!("Failed parsing remote response: {e}")))?;

        if !response.ok {
            return Err(Error::Remote(
                response
                    .error
                    .unwrap_or_else(|| "Remote search error".to_string()),
            ));
        }

        Ok(response.result.unwrap_or_default())
    })
    .await
    .map_err(|e| Error::Remote(format!("Remote search join error: {e}")))??;

    Ok(hits
        .into_iter()
        .map(|mut hit| {
            hit.host = Some(host_name.clone());
            hit
        })
        .collect())
}

pub async fn search_remotes(
    remotes: &[RemoteConfig],
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let mut set = JoinSet::new();
    for remote in remotes.iter().filter(|r| r.enabled) {
        let remote = remote.clone();
        let query = query.to_string();
        let opts = opts.clone();
        set.spawn(async move { search_remote(&remote, &query, &opts).await });
    }

    let mut hits = Vec::new();
    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(remote_hits)) => hits.extend(remote_hits),
            Ok(Err(err)) => return Err(err),
            Err(err) => return Err(Error::Remote(format!("Remote search task failed: {err}"))),
        }
    }

    Ok(hits)
}

pub async fn show_remote(
    config: &RemoteConfig,
    conversation_id: &str,
) -> Result<ConversationWithMessages> {
    let transport = SshTransport::from_config(config);
    let input = RemoteShowInput {
        id: conversation_id.to_string(),
    };
    let payload = serde_json::to_vec(&input)?;
    let host = config.host.clone();

    tokio::task::spawn_blocking(move || {
        let mut cmd = transport.ssh_command();
        cmd.arg(host)
            .arg("hstry")
            .arg("show")
            .arg("--json")
            .arg("--input")
            .arg("-");

        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::Remote(format!("Failed to start ssh: {e}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&payload)
                .map_err(|e| Error::Remote(format!("Failed writing stdin: {e}")))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| Error::Remote(format!("SSH failed: {e}")))?;

        if !output.status.success() {
            return Err(Error::Remote(format!(
                "Remote show failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let response: JsonResponse<ConversationWithMessages> =
            serde_json::from_slice(&output.stdout)
                .map_err(|e| Error::Remote(format!("Failed parsing remote response: {e}")))?;

        if !response.ok {
            return Err(Error::Remote(
                response
                    .error
                    .unwrap_or_else(|| "Remote show error".to_string()),
            ));
        }

        response
            .result
            .ok_or_else(|| Error::Remote("Remote show returned no result".to_string()))
    })
    .await
    .map_err(|e| Error::Remote(format!("Remote show join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cached_db_path() {
        let path = cached_db_path("laptop");
        assert!(path.to_string_lossy().contains("laptop.db"));
    }

    #[test]
    fn test_ssh_transport_command_building() {
        let config = RemoteConfig {
            name: "test".to_string(),
            host: "user@example.com".to_string(),
            database_path: None,
            port: Some(2222),
            identity_file: Some("~/.ssh/custom_key".to_string()),
            enabled: true,
        };

        let transport = SshTransport::from_config(&config);
        assert_eq!(transport.host, "user@example.com");
        assert_eq!(transport.port, Some(2222));
    }
}
