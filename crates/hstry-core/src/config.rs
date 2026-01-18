//! Configuration types and loading for hstry.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::error::Result;

/// Main application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the hstry database.
    pub database: PathBuf,

    /// Adapter directories to search for adapters.
    pub adapter_paths: Vec<PathBuf>,

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
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hstry");

        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hstry");

        // Include both user config adapters and system-wide adapters
        // Also check for adapters in the executable's directory for dev mode
        let mut adapter_paths = vec![config_dir.join("adapters")];

        // Add exe-relative adapters (for development and bundled distribution)
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                // Development: target/debug/../../adapters (goes to project root)
                let dev_adapters = exe_dir.join("../..").join("adapters");
                if dev_adapters.exists() {
                    if let Ok(canonical) = dev_adapters.canonicalize() {
                        adapter_paths.push(canonical);
                    }
                }
                // Bundled: exe_dir/adapters
                let bundled_adapters = exe_dir.join("adapters");
                if bundled_adapters.exists() {
                    adapter_paths.push(bundled_adapters);
                }
            }
        }

        Self {
            database: data_dir.join("hstry.db"),
            adapter_paths,
            js_runtime: "auto".to_string(),
            embedding_endpoint: None,
            workspaces: Vec::new(),
            adapters: Vec::new(),
            service: ServiceConfig::default(),
            sources: Vec::new(),
        }
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
        config.expand_paths()?;
        Ok(config)
    }

    /// Get the default config file path.
    pub fn default_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hstry")
            .join("config.toml")
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
            config.expand_paths()?;
            config.save_to_path(path)?;
            Ok(config)
        }
    }

    /// Expand a path, replacing ~ with home directory.
    pub fn expand_path(path: &str) -> PathBuf {
        let expanded = shellexpand::full(path)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| path.to_string());
        PathBuf::from(expanded)
    }

    fn expand_paths(&mut self) -> Result<()> {
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
        Ok(())
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

/// Background service configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceConfig {
    /// Whether the service should auto-run when started.
    pub enabled: bool,

    /// Poll interval in seconds (fallback when filesystem events are missed).
    pub poll_interval_secs: u64,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: 30,
        }
    }
}

fn default_true() -> bool {
    true
}
