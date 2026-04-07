//! Configuration types and loading for hstry.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::error::Result;

/// Default GitHub repository for adapters.
pub const DEFAULT_ADAPTER_REPO: &str = "https://github.com/byteowlz/hstry";

/// Main application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the hstry database.
    pub database: PathBuf,

    /// Adapter directories to search for adapters.
    pub adapter_paths: Vec<PathBuf>,

    /// Repositories for downloading/updating adapters.
    /// Each repo can provide different adapters.
    pub adapter_repos: Vec<AdapterRepo>,

    /// JavaScript runtime preference: "bun", "deno", "node", or "auto".
    pub js_runtime: String,

    /// Embedding endpoint for semantic search (e.g., mmry's /v1/embeddings).
    pub embedding_endpoint: Option<String>,

    /// Workspace roots to scan recursively for session output.
    pub workspaces: Vec<String>,

    /// Adapter configuration overrides.
    pub adapters: Vec<AdapterConfig>,

    /// Service configuration.
    pub service: ServiceConfig,

    /// Sources configuration.
    pub sources: Vec<SourceConfig>,

    /// Remote hosts for syncing history across machines.
    pub remotes: Vec<RemoteConfig>,

    /// Sync configuration for hub/satellite mode.
    pub sync: SyncConfig,

    /// Search configuration.
    pub search: SearchConfig,

    /// Web automation configuration.
    pub web: WebConfig,

    /// Resume configuration for opening sessions in coding agents.
    pub resume: ResumeConfig,

    /// Storage knobs (message_events log, indexer outbox, etc.).
    #[serde(default)]
    pub storage: StorageConfig,
}

/// Storage-level knobs that control optional bookkeeping tables.
///
/// `message_events` is an append-only event log that mirrors every message
/// upsert. It is required by event-stream consumers (e.g. real-time UI clients)
/// but represents pure overhead for batch / search-only workloads. The default
/// favours the latter: events are *off*, and any existing rows are subject to
/// retention compaction during scheduled syncs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Configuration for the `message_events` log table.
    pub message_events: MessageEventsConfig,
    /// Configuration for the indexer outbox + worker.
    pub indexer_outbox: IndexerOutboxConfig,
}

/// Toggles the `message_events` append-only log on a per-deployment basis.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MessageEventsConfig {
    /// Append a row to `message_events` for every message upsert.
    /// Default: `false` — non-event consumers pay no overhead (trx-aa3m).
    pub enabled: bool,
    /// Maximum age of a `message_events` row before it becomes eligible for
    /// retention compaction. `0` disables age-based pruning.
    pub max_age_days: u32,
    /// Maximum number of `message_events` rows to retain per conversation.
    /// `0` disables per-conversation pruning.
    pub max_per_conversation: u32,
    /// Run retention compaction at most this often (seconds). The compactor is
    /// invoked from the service sync loop and respects this floor.
    pub compaction_interval_secs: u64,
}

impl Default for MessageEventsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: 30,
            max_per_conversation: 5_000,
            compaction_interval_secs: 3_600,
        }
    }
}

/// Configuration for the durable indexer outbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexerOutboxConfig {
    /// Enqueue indexer jobs on every message upsert. When disabled, the
    /// service falls back to the legacy "index everything that has no row
    /// in the search index" sweep.
    pub enabled: bool,
    /// How many outbox jobs the dedicated worker drains per tick.
    pub batch_size: usize,
    /// Sleep between worker ticks (milliseconds).
    pub poll_interval_ms: u64,
}

impl Default for IndexerOutboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            batch_size: 256,
            poll_interval_ms: 750,
        }
    }
}

/// Resume configuration for opening sessions in coding agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResumeConfig {
    /// Default agent to resume sessions in (e.g., "pi", "claude-code", "codex").
    pub default_agent: String,

    /// Per-agent resume configuration.
    pub agents: std::collections::HashMap<String, AgentResumeConfig>,
}

/// Configuration for resuming sessions in a specific coding agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResumeConfig {
    /// hstry export format name (must match an adapter name).
    pub format: String,

    /// Command template to launch the agent.
    /// Placeholders: {session_path}, {session_id}, {workspace}
    pub command: String,

    /// Directory where the agent stores sessions natively.
    /// Converted sessions are placed here so the agent discovers them.
    pub session_dir: String,
}

impl Default for ResumeConfig {
    fn default() -> Self {
        let mut agents = std::collections::HashMap::new();
        agents.insert(
            "pi".to_string(),
            AgentResumeConfig {
                format: "pi".to_string(),
                command: "pi --session {session_path}".to_string(),
                session_dir: "~/.pi/agent/sessions".to_string(),
            },
        );
        agents.insert(
            "claude-code".to_string(),
            AgentResumeConfig {
                format: "claude-code".to_string(),
                command: "claude --resume {session_id}".to_string(),
                session_dir: "~/.claude/projects".to_string(),
            },
        );
        agents.insert(
            "codex".to_string(),
            AgentResumeConfig {
                format: "codex".to_string(),
                command: "codex resume {session_id}".to_string(),
                session_dir: "~/.codex/sessions".to_string(),
            },
        );
        agents.insert(
            "opencode".to_string(),
            AgentResumeConfig {
                format: "opencode".to_string(),
                command: "opencode".to_string(),
                session_dir: "~/.local/share/opencode".to_string(),
            },
        );
        agents.insert(
            "goose".to_string(),
            AgentResumeConfig {
                format: "goose".to_string(),
                command: "goose session resume {session_id}".to_string(),
                session_dir: "~/.local/share/goose/sessions".to_string(),
            },
        );
        Self {
            default_agent: "pi".to_string(),
            agents,
        }
    }
}

/// Search configuration for indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Optional path to the Tantivy index directory.
    #[serde(default)]
    pub index_path: Option<PathBuf>,

    /// Batch size for background indexing.
    #[serde(default = "default_index_batch_size")]
    pub index_batch_size: usize,
}

fn default_index_batch_size() -> usize {
    500
}

/// Web automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Enable web automation.
    pub enabled: bool,

    /// Run web syncs in headless mode.
    pub headless: bool,

    /// Sync interval in seconds.
    pub sync_interval_secs: u64,

    /// Full refresh interval in seconds.
    pub full_refresh_interval_secs: u64,

    /// Optional storage directory for web exports.
    pub storage_dir: Option<String>,

    /// Provider configuration.
    pub providers: WebProvidersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WebProvidersConfig {
    pub chatgpt: WebProviderConfig,
    pub claude: WebProviderConfig,
    pub gemini: WebProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WebProviderConfig {
    /// Enable sync for this provider.
    pub enabled: bool,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            headless: true,
            sync_interval_secs: 900,
            full_refresh_interval_secs: 86_400,
            storage_dir: None,
            providers: WebProvidersConfig::default(),
        }
    }
}

/// Configuration for a remote host (SSH-based sync).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    /// Unique name for this remote (e.g., "laptop", "server").
    pub name: String,

    /// SSH host (e.g., "user@hostname", "hostname", or SSH config alias).
    pub host: String,

    /// Path to the hstry database on the remote (defaults to standard XDG path).
    #[serde(default)]
    pub database_path: Option<String>,

    /// SSH port (defaults to 22).
    #[serde(default)]
    pub port: Option<u16>,

    /// Path to SSH identity file (defaults to ~/.ssh/id_rsa or agent).
    #[serde(default)]
    pub identity_file: Option<String>,

    /// Whether this remote is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Configuration for an adapter repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterRepo {
    /// Unique name for this repo (e.g., "official", "community").
    pub name: String,

    /// Source type and location.
    #[serde(flatten)]
    pub source: AdapterRepoSource,

    /// Whether this repo is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Source types for adapter repositories.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AdapterRepoSource {
    /// Git repository (works with GitHub, GitLab, Gitea, self-hosted, etc.)
    Git {
        /// Repository URL (HTTPS or SSH).
        /// Examples:
        /// - https://github.com/byteowlz/hstry
        /// - https://gitlab.com/user/adapters
        /// - git@github.com:byteowlz/hstry.git
        /// - https://gitea.example.com/org/adapters
        url: String,

        /// Branch, tag, or commit to use.
        #[serde(default = "default_git_ref")]
        git_ref: String,

        /// Path within the repo where adapters are located.
        #[serde(default = "default_adapters_path")]
        path: String,
    },

    /// Direct URL to a tarball or zip archive.
    Archive {
        /// URL to the archive file (.tar.gz, .zip, .tgz).
        url: String,

        /// Path within the archive where adapters are located.
        #[serde(default = "default_adapters_path")]
        path: String,
    },

    /// Local filesystem path (for development or private adapters).
    Local {
        /// Absolute or relative path to adapters directory.
        path: String,
    },
}

impl AdapterRepoSource {
    /// Get the adapters path within the source.
    pub fn adapters_path(&self) -> &str {
        match self {
            Self::Git { path, .. } | Self::Archive { path, .. } | Self::Local { path } => path,
        }
    }
}

fn default_git_ref() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn default_adapters_path() -> String {
    "adapters".to_string()
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = xdg_data_dir().join("hstry");
        let config_dir = xdg_config_dir().join("hstry");

        // Include both user config adapters and system-wide adapters
        // Also check for adapters in the executable's directory for dev mode
        let mut adapter_paths = vec![config_dir.join("adapters")];

        // Add exe-relative adapters (for development and bundled distribution)
        if let Ok(exe_path) = std::env::current_exe()
            && let Some(exe_dir) = exe_path.parent()
        {
            // Development: target/debug/../../adapters (goes to project root)
            let dev_adapters = exe_dir.join("../..").join("adapters");
            if dev_adapters.exists()
                && let Ok(canonical) = dev_adapters.canonicalize()
            {
                adapter_paths.push(canonical);
            }
            // Bundled: exe_dir/adapters
            let bundled_adapters = exe_dir.join("adapters");
            if bundled_adapters.exists() {
                adapter_paths.push(bundled_adapters);
            }
        }

        Self {
            database: data_dir.join("hstry.db"),
            adapter_paths,
            adapter_repos: vec![AdapterRepo {
                name: "official".to_string(),
                source: AdapterRepoSource::Git {
                    url: DEFAULT_ADAPTER_REPO.to_string(),
                    git_ref: "main".to_string(),
                    path: "adapters".to_string(),
                },
                enabled: true,
            }],
            js_runtime: "auto".to_string(),
            embedding_endpoint: None,
            workspaces: Vec::new(),
            adapters: Vec::new(),
            service: ServiceConfig::default(),
            sources: Vec::new(),
            remotes: Vec::new(),
            sync: SyncConfig::default(),
            search: SearchConfig::default(),
            web: WebConfig::default(),
            resume: ResumeConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            index_path: None,
            index_batch_size: default_index_batch_size(),
        }
    }
}

/// Hub/satellite sync configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    /// Sync mode: standalone, hub, or satellite.
    pub mode: SyncMode,
    /// Optional device identifier for sync provenance.
    pub device_id: Option<String>,
    /// Preferred hub remote name (for satellite mode).
    pub hub_remote: Option<String>,
    /// Auto-sync remotes in the background service.
    pub auto_sync: bool,
    /// Auto-sync interval (seconds).
    pub auto_sync_interval_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    Standalone,
    Hub,
    Satellite,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            mode: SyncMode::Standalone,
            device_id: None,
            hub_remote: None,
            auto_sync: false,
            auto_sync_interval_secs: 300,
        }
    }
}

impl Config {
    /// Resolve Tantivy index path from config.
    pub fn search_index_path(&self) -> PathBuf {
        if let Some(path) = &self.search.index_path {
            return path.clone();
        }

        let base_dir = self.database.parent().unwrap_or_else(|| Path::new("."));
        base_dir.join("index").join("tantivy")
    }
}

impl Config {
    /// Load configuration from the default config file.
    pub fn load() -> Result<Self> {
        let config_path = Self::default_config_path();
        if config_path.exists() {
            Self::load_from_path(&config_path)
        } else {
            Ok(Self::default())
        }
    }

    /// Load configuration from a specific file.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)
            .map_err(|e| Error::Config(format!("Failed to parse config: {e}")))?;
        config.expand_paths();
        Ok(config)
    }

    /// Get the default config file path.
    pub fn default_config_path() -> PathBuf {
        xdg_config_dir().join("hstry").join("config.toml")
    }

    /// Save configuration to a specific file path.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Ensure config exists at the given path, creating defaults if missing.
    pub fn ensure_at(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load_from_path(path)
        } else {
            let mut config = Self::default();
            config.expand_paths();
            config.save_to_path(path)?;
            Ok(config)
        }
    }

    /// Expand a path, replacing ~ with home directory.
    pub fn expand_path(path: &str) -> PathBuf {
        let expanded =
            shellexpand::full(path).map_or_else(|_| path.to_string(), std::borrow::Cow::into_owned);
        PathBuf::from(expanded)
    }

    fn expand_paths(&mut self) {
        self.database = Self::expand_path(&self.database.to_string_lossy());
        self.adapter_paths = self
            .adapter_paths
            .iter()
            .map(|p| Self::expand_path(&p.to_string_lossy()))
            .collect();
        self.workspaces = self
            .workspaces
            .iter()
            .map(|p| Self::expand_path(p).to_string_lossy().to_string())
            .collect();
        self.sources = self
            .sources
            .iter()
            .map(|source| SourceConfig {
                id: source.id.clone(),
                adapter: source.adapter.clone(),
                path: Self::expand_path(&source.path)
                    .to_string_lossy()
                    .to_string(),
                auto_sync: source.auto_sync,
            })
            .collect();

        self.web.storage_dir = self
            .web
            .storage_dir
            .as_ref()
            .map(|path| Self::expand_path(path).to_string_lossy().to_string());
    }

    /// Check whether a given adapter is enabled.
    pub fn adapter_enabled(&self, name: &str) -> bool {
        if let Some(entry) = self.adapters.iter().find(|adapter| adapter.name == name) {
            entry.enabled
        } else {
            true
        }
    }
}

/// Configuration for a single source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Unique identifier for this source.
    pub id: String,

    /// Adapter to use (e.g., "opencode", "chatgpt").
    pub adapter: String,

    /// Path to the source data.
    pub path: String,

    /// Whether to auto-sync this source.
    #[serde(default = "default_true")]
    pub auto_sync: bool,
}

/// Configuration for a single adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterConfig {
    /// Adapter name (e.g., "codex", "claude-web").
    pub name: String,

    /// Whether this adapter is enabled for imports.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Transport type for gRPC service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTransport {
    /// TCP on localhost (default, backward compatible).
    #[default]
    Tcp,
    /// Unix domain socket (more secure for multi-user).
    Unix,
}

/// Background service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    /// Whether the service should auto-run when started.
    pub enabled: bool,

    /// Safety poll interval in seconds (fallback when filesystem events are missed).
    ///
    /// This is intentionally infrequent: filesystem events are the primary trigger.
    pub poll_interval_secs: u64,

    /// Expose local search API (localhost only).
    #[serde(default = "default_true")]
    pub search_api: bool,

    /// Optional port for search API (defaults to dynamic if unset).
    /// Only used when transport = "tcp".
    #[serde(default)]
    pub search_port: Option<u16>,

    /// Transport type: "tcp" (localhost:port) or "unix" (domain socket).
    /// Unix socket provides better security for multi-user systems.
    #[serde(default)]
    pub transport: ServiceTransport,

    /// Per-source adaptive scheduling parameters (trx-z42c.1).
    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// Resource controls for the sync loop (trx-z42c.7).
    #[serde(default)]
    pub resources: ResourceConfig,
}

/// Per-source adaptive cadence configuration. The scheduler keeps a per-source
/// `next_due_at` deadline. After every successful sync, the cadence is
/// multiplied by `idle_backoff` (clamped to `max_interval_secs`) when no new
/// data was returned, and reset to `min_interval_secs` otherwise.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SchedulerConfig {
    /// Lower bound on per-source cadence (seconds). Active sources fire at
    /// this rate.
    pub min_interval_secs: u64,
    /// Upper bound on per-source cadence (seconds). Idle sources slowly back
    /// off to this value.
    pub max_interval_secs: u64,
    /// Multiplier applied to the cadence when a sync produced zero new
    /// conversations / messages.
    pub idle_backoff: f32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            min_interval_secs: 30,
            max_interval_secs: 1_800,
            idle_backoff: 1.5,
        }
    }
}

/// Resource controls applied to the sync loop. These bound concurrency and
/// give the operator a kill switch for runaway CPU/IO usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceConfig {
    /// Maximum number of sources synced concurrently.
    pub max_concurrent_syncs: usize,
    /// Hard time budget for a single source sync (milliseconds). `0` disables.
    pub per_source_time_budget_ms: u64,
    /// Quality-of-service class for sync work: `interactive` runs at normal
    /// priority, `background` yields to other work via larger sleeps.
    pub qos: QosClass,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            max_concurrent_syncs: 4,
            per_source_time_budget_ms: 60_000,
            qos: QosClass::Background,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QosClass {
    Interactive,
    #[default]
    Background,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: 1_200,
            search_api: true,
            search_port: None,
            transport: ServiceTransport::Tcp,
            scheduler: SchedulerConfig::default(),
            resources: ResourceConfig::default(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Get XDG-compliant config directory.
/// Checks `$XDG_CONFIG_HOME` first, then falls back to platform default.
fn xdg_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    // Fallback: ~/.config on Unix, platform default elsewhere
    if cfg!(unix) {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".config"))
    } else {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Get XDG-compliant data directory.
/// Checks `$XDG_DATA_HOME` first, then falls back to `~/.local/share`.
fn xdg_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    // Fallback: ~/.local/share on Unix, platform default elsewhere
    if cfg!(unix) {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".local").join("share"))
    } else {
        dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
