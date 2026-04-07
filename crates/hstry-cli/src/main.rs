#![allow(clippy::print_stdout, clippy::print_stderr)]
//! hstry CLI - Universal AI chat history

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::stream::{self, StreamExt};
use hstry_core::config::{AdapterRepo, AdapterRepoSource};
use hstry_core::models::{Conversation, Message, MessageRole, SearchHit, Source};
use hstry_core::{Config, Database};
use hstry_runtime::{AdapterRunner, ExportConversation, ExportOptions, ParsedMessage, Runtime};

/// Apply storage feature flags from `config` to a freshly opened `Database`.
/// Centralised so every entry point honours the trx-aa3m / trx-z42c contracts.
fn apply_storage_config(db: &Database, config: &Config) {
    db.set_message_events_enabled(config.storage.message_events.enabled);
    db.set_indexer_outbox_enabled(config.storage.indexer_outbox.enabled);
}

mod adapter_manifest;
use serde::{Serialize, de::DeserializeOwned};

mod pretty;
mod service;
mod sync;

#[derive(Debug, serde::Deserialize)]
struct SyncInput {
    source: Option<String>,
    parallel: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct SearchInput {
    query: String,
    limit: Option<i64>,
    source: Option<String>,
    workspace: Option<String>,
    mode: Option<SearchModeArg>,
    scope: Option<SearchScopeArg>,
    remotes: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct ListInput {
    source: Option<String>,
    workspace: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, serde::Deserialize)]
struct ShowInput {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct SourceAddInput {
    path: String,
    adapter: Option<String>,
    id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SourceRemoveInput {
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct AdapterAddInput {
    path: String,
}

#[derive(Debug, serde::Deserialize)]
struct AdapterToggleInput {
    name: String,
}

#[derive(Debug, serde::Serialize)]
struct JsonResponse<T> {
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct SyncSummary {
    sources: Vec<sync::SyncStats>,
    total_sources: usize,
    total_conversations: usize,
    total_messages: usize,
}

#[derive(Debug, serde::Serialize)]
struct StatsSummary {
    sources: i64,
    conversations: i64,
    messages: i64,
    per_source: Vec<hstry_core::db::SourceStats>,
    activity: hstry_core::db::ActivityStats,
}

#[derive(Debug, serde::Serialize)]
struct ScanHit {
    adapter: String,
    display_name: String,
    path: String,
    confidence: f32,
}

#[derive(Debug, serde::Serialize)]
struct AdapterStatus {
    name: String,
    enabled: bool,
}
#[derive(Debug, Parser)]
#[command(
    name = "hstry",
    author,
    version,
    about = "Universal AI chat history database",
    propagate_version = true
)]
struct Cli {
    /// Config file path
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Output JSON for programmatic use
    #[arg(long, global = true)]
    json: bool,

    /// Increase verbosity
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Sync chat history from all sources
    Sync {
        /// Only sync a specific source
        #[arg(long)]
        source: Option<String>,

        /// Max number of sources to sync in parallel
        #[arg(long)]
        parallel: Option<usize>,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Import chat history from a file or directory with auto-detection
    Import {
        /// Path to file or directory to import
        path: PathBuf,

        /// Force a specific adapter (skip auto-detection)
        #[arg(short, long)]
        adapter: Option<String>,

        /// Custom source ID (defaults to adapter name)
        #[arg(long)]
        source_id: Option<String>,

        /// Only show what would be imported (don't write to database)
        #[arg(long)]
        dry_run: bool,
    },

    /// Search across chat history
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "20")]
        limit: i64,

        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,

        /// Search mode (auto, natural, code)
        #[arg(long, value_enum, default_value = "auto")]
        mode: SearchModeArg,

        /// Search scope (local, remote, all)
        #[arg(long, value_enum, default_value = "local")]
        scope: SearchScopeArg,

        /// Remote names to query (default: all enabled)
        #[arg(long)]
        remote: Vec<String>,

        /// Filter by message role (user, assistant, system, tool)
        #[arg(long, short = 'r', value_enum)]
        role: Vec<SearchRoleArg>,

        /// Exclude tool calls and tool results from results
        #[arg(long)]
        no_tools: bool,

        /// Deduplicate similar results (by content hash)
        #[arg(long)]
        dedup: bool,

        /// Include system context (AGENTS.md, etc.) in results
        #[arg(long)]
        include_system: bool,

        /// Only include messages after this date (ISO 8601 or relative: "2d", "1w", "2025-01-15")
        #[arg(long)]
        after: Option<String>,

        /// Only include messages before this date (ISO 8601 or relative)
        #[arg(long)]
        before: Option<String>,

        /// Filter by conversation model (e.g. "claude-sonnet-4")
        #[arg(long)]
        model: Option<String>,

        /// Filter by agent harness (e.g. "pi", "claude")
        #[arg(long)]
        harness_filter: Option<String>,

        /// Filter by conversation tag
        #[arg(long)]
        tag: Option<String>,

        /// Show each session only once with occurrence count
        #[arg(short, long)]
        compact: bool,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Build or refresh the search index
    Index {
        /// Rebuild the index from scratch
        #[arg(long)]
        rebuild: bool,
    },

    /// List conversations
    List {
        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,

        /// Maximum results
        #[arg(short, long, default_value = "50")]
        limit: i64,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Show a conversation
    Show {
        /// Conversation ID
        id: String,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Manage sources
    Source {
        #[command(subcommand)]
        command: SourceCommand,
    },

    /// Manage adapters
    Adapters {
        #[command(subcommand)]
        command: Option<AdapterCommand>,
    },

    /// Manage background service
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },

    /// Scan for chat history sources
    Scan,

    /// Quickstart: scan, add sources, and sync
    Quickstart,

    /// Export conversations to another format
    Export {
        /// Target format (pi, opencode, codex, claude-code, markdown, json)
        #[arg(short, long)]
        format: String,

        /// Conversation IDs to export (comma-separated, or "all" for all)
        #[arg(short, long, default_value = "all")]
        conversations: String,

        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,

        /// Filter by message role (user, assistant, system, tool)
        #[arg(long, short = 'r', value_enum)]
        role: Vec<SearchRoleArg>,

        /// Output path (file for single-output exports, directory for multi-file exports)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Export one file per conversation for markdown/json
        #[arg(long)]
        session_files: bool,

        /// Pretty print JSON output
        #[arg(long)]
        pretty: bool,
    },

    /// Resume a conversation in a coding agent
    ///
    /// Opens a past session in your preferred agent (pi, claude-code, codex, etc.).
    /// If the session is already from the target agent, opens it directly.
    /// Otherwise, converts to the target format and places it in the agent's
    /// native session directory.
    Resume {
        /// Conversation ID (UUID or partial match)
        #[arg(group = "target")]
        id: Option<String>,

        /// Search for a conversation instead of specifying an ID
        #[arg(short, long, group = "target")]
        search: Option<String>,

        /// Target agent to resume in (default: from config)
        #[arg(short, long)]
        agent: Option<String>,

        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,

        /// Only show conversations after this date/time (natural language: "yesterday", "2 days ago", "2026-03-01")
        #[arg(long)]
        after: Option<String>,

        /// Only show conversations before this date/time (natural language)
        #[arg(long)]
        before: Option<String>,

        /// Maximum results when searching
        #[arg(short, long, default_value = "20")]
        limit: i64,

        /// Show what would happen without writing or launching
        #[arg(long)]
        dry_run: bool,

        /// Interactive picker using fzf
        #[arg(short, long)]
        pick: bool,
    },

    /// Show database statistics
    Stats,

    /// Deduplicate conversations in the database
    Dedup {
        /// Only show what would be deleted (don't actually delete)
        #[arg(long)]
        dry_run: bool,

        /// Filter by source
        #[arg(long)]
        source: Option<String>,
    },

    /// Integrate with mmry
    Mmry {
        #[command(subcommand)]
        command: MmryCommand,
    },

    /// Manage remote hosts for syncing history
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },

    /// Manage web-app automation
    Web {
        #[command(subcommand)]
        command: WebCommand,
    },

    /// Show or manage configuration
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
    },

    /// Rebuild a source from scratch (purge → import → [dedup] → index).
    ///
    /// Use this when a source's database state has drifted from its on-disk
    /// JSONL files (duplicate replays, partial imports, schema regressions).
    /// The default uses bulk-reseed mode for throughput; pass `--no-bulk` to
    /// keep indexes online.
    ///
    /// Dedup is *off* by default for reseed: with stable, content-addressable
    /// message ids in place (trx-hjjw.4), re-imports are naturally idempotent
    /// via ON CONFLICT and the conversation-local dedup heuristic is not
    /// needed. Pass `--dedup` to opt in for legacy cleanups.
    Reseed {
        /// Source ID to reseed (required — reseed never touches everything).
        #[arg(long)]
        source: String,
        /// Run conversation-local dedup after import (legacy cleanup).
        #[arg(long)]
        dedup: bool,
        /// Skip the post-import index rebuild.
        #[arg(long)]
        no_index: bool,
        /// Disable bulk reseed mode (keeps indexes online).
        #[arg(long)]
        no_bulk: bool,
        /// Don't actually purge / import, just print the plan.
        #[arg(long)]
        dry_run: bool,
        /// Also drop the source row itself when purging.
        #[arg(long)]
        drop_source: bool,
    },

    /// Verify that a source's DB state matches its on-disk artifacts.
    ///
    /// For Pi (and any adapter that exposes a stable per-conversation
    /// idempotency key) this re-parses the JSONL files, compares conversation
    /// counts and message counts to the database, and reports the drift.
    /// Pass `--repair` to run a `reseed` for any source that has drifted.
    Verify {
        /// Source ID to verify (defaults to all enabled sources).
        #[arg(long)]
        source: Option<String>,
        /// Reseed any drifted source automatically.
        #[arg(long)]
        repair: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Show current configuration
    Show,

    /// Show config file path
    Path,

    /// Open config in editor
    Edit,
}

#[derive(Debug, Subcommand)]
enum WebCommand {
    /// Install Playwright browsers for web automation
    Install {
        /// Browser to install (chromium, firefox, webkit)
        #[arg(long, default_value = "chromium")]
        browser: String,
    },

    /// Login to a web provider and store session state
    Login {
        /// Provider name (chatgpt, claude, gemini)
        provider: String,

        /// Run in headful mode
        #[arg(long)]
        headful: bool,

        /// Browser to use (chromium, firefox, webkit)
        #[arg(long)]
        browser: Option<String>,
    },

    /// Sync web providers and import chats
    Sync {
        /// Provider name (chatgpt, claude, gemini)
        #[arg(long)]
        provider: Option<String>,

        /// Run in headful mode
        #[arg(long)]
        headful: bool,

        /// Browser to use (chromium, firefox, webkit)
        #[arg(long)]
        browser: Option<String>,
    },

    /// Show web login and sync status
    Status,
}

#[derive(Debug, Subcommand)]
enum RemoteCommand {
    /// List configured remotes
    List,

    /// Add a remote host
    Add {
        /// Unique name for this remote
        name: String,

        /// SSH host (e.g., "user@hostname" or SSH config alias)
        host: String,

        /// Path to hstry database on remote
        #[arg(long)]
        database_path: Option<String>,

        /// SSH port (default: 22)
        #[arg(short, long)]
        port: Option<u16>,

        /// Path to SSH identity file
        #[arg(short, long)]
        identity_file: Option<String>,
    },

    /// Remove a remote host
    Remove {
        /// Remote name
        name: String,
    },

    /// Test connection to a remote
    Test {
        /// Remote name
        name: String,
    },

    /// Fetch remote database to local cache
    Fetch {
        /// Remote name (fetches all enabled remotes if not specified)
        #[arg(short, long)]
        remote: Option<String>,
    },

    /// Sync history with remote (fetch + merge)
    Sync {
        /// Remote name (syncs all enabled remotes if not specified)
        #[arg(short, long)]
        remote: Option<String>,

        /// Sync direction
        #[arg(short, long, value_enum, default_value = "pull")]
        direction: SyncDirectionArg,
    },

    /// Show remote cache status
    Status,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SyncDirectionArg {
    /// Pull from remote to local
    Pull,
    /// Push from local to remote
    Push,
    /// Bidirectional merge
    Bidirectional,
}

impl From<SyncDirectionArg> for hstry_core::remote::SyncDirection {
    fn from(value: SyncDirectionArg) -> Self {
        match value {
            SyncDirectionArg::Pull => hstry_core::remote::SyncDirection::Pull,
            SyncDirectionArg::Push => hstry_core::remote::SyncDirection::Push,
            SyncDirectionArg::Bidirectional => hstry_core::remote::SyncDirection::Bidirectional,
        }
    }
}

#[derive(Debug, Subcommand)]
enum SourceCommand {
    /// Add a new source
    Add {
        /// Path to source data
        path: PathBuf,

        /// Adapter to use (auto-detect if not specified)
        #[arg(long)]
        adapter: Option<String>,

        /// Custom source ID
        #[arg(long)]
        id: Option<String>,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// List configured sources
    List,

    /// Remove a source
    Remove {
        /// Source ID
        id: String,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Clean up duplicate sources (same adapter/path with different IDs)
    Cleanup {
        /// Remove duplicate sources automatically
        #[arg(long)]
        auto_remove: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterCommand {
    /// List available adapters
    List,

    /// Add an adapter directory to the config
    Add {
        /// Path to the adapter directory
        path: PathBuf,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Enable an adapter for imports
    Enable {
        /// Adapter name
        name: String,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Disable an adapter for imports
    Disable {
        /// Adapter name
        name: String,

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
    },

    /// Update/download adapters from configured repositories
    Update {
        /// Specific adapter to update (updates all if not specified)
        #[arg(short, long)]
        adapter: Option<String>,

        /// Only update from specific repo
        #[arg(short, long)]
        repo: Option<String>,

        /// Force update even if already up to date
        #[arg(short, long)]
        force: bool,
    },

    /// Manage adapter repositories
    Repo {
        #[command(subcommand)]
        command: AdapterRepoCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AdapterRepoCommand {
    /// List configured adapter repositories
    List,

    /// Add a git repository (GitHub, GitLab, Gitea, self-hosted, etc.)
    AddGit {
        /// Repository name (e.g., "community")
        name: String,

        /// Git repository URL (HTTPS or SSH)
        url: String,

        /// Branch, tag, or commit to use
        #[arg(short = 'r', long, default_value = "main")]
        git_ref: String,

        /// Path within repo where adapters are located
        #[arg(short, long, default_value = "adapters")]
        path: String,
    },

    /// Add an archive URL (tarball or zip)
    AddArchive {
        /// Repository name
        name: String,

        /// URL to the archive (.tar.gz, .zip, .tgz)
        url: String,

        /// Path within archive where adapters are located
        #[arg(short, long, default_value = "adapters")]
        path: String,
    },

    /// Add a local filesystem path
    AddLocal {
        /// Repository name
        name: String,

        /// Path to adapters directory
        path: PathBuf,
    },

    /// Remove an adapter repository
    Remove {
        /// Repository name
        name: String,
    },

    /// Enable an adapter repository
    Enable {
        /// Repository name
        name: String,
    },

    /// Disable an adapter repository
    Disable {
        /// Repository name
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    /// Enable the service in config
    Enable,

    /// Disable the service in config
    Disable,

    /// Start the background service
    Start,

    /// Run the service in the foreground
    Run,

    /// Restart the background service
    Restart,

    /// Stop the background service
    Stop,

    /// Show service status
    Status,
}

#[derive(Debug, Subcommand)]
enum MmryCommand {
    /// Extract memories into mmry
    Extract {
        /// mmry store name
        #[arg(long, default_value = "hstry")]
        store: String,

        /// Path to mmry binary
        #[arg(long, default_value = "mmry")]
        mmry_bin: String,

        /// mmry config file path
        #[arg(long, value_name = "PATH")]
        mmry_config: Option<PathBuf>,

        /// Filter by source
        #[arg(long)]
        source: Option<String>,

        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,

        /// Only include conversations created after this RFC3339 timestamp
        #[arg(long)]
        after: Option<String>,

        /// Limit number of conversations
        #[arg(long)]
        limit: Option<i64>,

        /// Include only these message roles (defaults: user, assistant)
        #[arg(long, value_enum)]
        role: Vec<MmryRoleArg>,

        /// Override memory type for all entries
        #[arg(long, value_enum)]
        memory_type: Option<MmryMemoryTypeArg>,

        /// Print payload instead of invoking mmry
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum MmryRoleArg {
    User,
    Assistant,
    System,
    Tool,
    Other,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum MmryMemoryTypeArg {
    Episodic,
    Semantic,
    Procedural,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let level = match cli.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // Load config
    let config_path = cli.config.unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    match cli.command {
        Command::Search {
            query,
            limit,
            source,
            workspace,
            mode,
            scope,
            remote,
            role,
            no_tools,
            dedup,
            include_system,
            after,
            before,
            model,
            harness_filter,
            tag,
            compact,
            input,
        } => {
            let input = read_input::<SearchInput>(input)?;
            let query = input.as_ref().map_or(query, |v| v.query.clone());
            let limit = input.as_ref().and_then(|v| v.limit).unwrap_or(limit);
            let source = input.as_ref().and_then(|v| v.source.clone()).or(source);
            let workspace = input
                .as_ref()
                .and_then(|v| v.workspace.clone())
                .or(workspace);
            let mode = input.as_ref().and_then(|v| v.mode).unwrap_or(mode);
            let scope = input.as_ref().and_then(|v| v.scope).unwrap_or(scope);
            let remotes = input
                .as_ref()
                .and_then(|v| v.remotes.clone())
                .unwrap_or(remote);
            cmd_search_fast(
                &config,
                &query,
                limit,
                source,
                workspace,
                mode,
                scope,
                remotes,
                role,
                no_tools,
                dedup,
                include_system,
                after,
                before,
                model,
                harness_filter,
                tag,
                compact,
                cli.json,
            )
            .await
        }
        Command::Sync {
            source,
            parallel,
            input,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            let input = read_input::<SyncInput>(input)?;
            let source = input.as_ref().and_then(|v| v.source.clone()).or(source);
            let parallel = input.and_then(|v| v.parallel).or(parallel);
            cmd_sync(&db, &runner, &config, source, parallel, cli.json).await
        }
        Command::Import {
            path,
            adapter,
            source_id,
            dry_run,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_import(
                &db, &runner, &config, path, adapter, source_id, dry_run, cli.json,
            )
            .await
        }
        Command::Index { rebuild } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_index(&config, &db, rebuild, cli.json).await
        }
        Command::List {
            source,
            workspace,
            limit,
            input,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let input = read_input::<ListInput>(input)?;
            let source = input.as_ref().and_then(|v| v.source.clone()).or(source);
            let workspace = input
                .as_ref()
                .and_then(|v| v.workspace.clone())
                .or(workspace);
            let limit = input.as_ref().and_then(|v| v.limit).unwrap_or(limit);
            cmd_list(&db, source, workspace, limit, cli.json).await
        }
        Command::Show { id, input } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let input = read_input::<ShowInput>(input)?;
            let id = input.as_ref().map_or(id, |v| v.id.clone());
            cmd_show(&db, &id, cli.json).await
        }
        Command::Source { command } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_source(&db, &runner, command, cli.json).await
        }
        Command::Adapters { command } => {
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_adapters(&runner, &config, &config_path, command, cli.json)
        }
        Command::Service { command } => match command {
            ServiceCommand::Status => {
                let status = service::get_service_status(&config_path)?;
                if cli.json {
                    emit_json(JsonResponse {
                        ok: true,
                        result: Some(status),
                        error: None,
                    })
                } else {
                    service::cmd_service(&config_path, ServiceCommand::Status).await
                }
            }
            ServiceCommand::Run => service::cmd_service(&config_path, ServiceCommand::Run).await,
            other => {
                service::cmd_service(&config_path, other).await?;
                if cli.json {
                    let status = service::get_service_status(&config_path)?;
                    emit_json(JsonResponse {
                        ok: true,
                        result: Some(status),
                        error: None,
                    })
                } else {
                    Ok(())
                }
            }
        },
        Command::Scan => {
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_scan(&runner, &config, cli.json).await
        }
        Command::Quickstart => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_quickstart(&db, &runner, &config, &config_path, cli.json).await
        }
        Command::Export {
            format,
            conversations,
            source,
            workspace,
            role,
            output,
            session_files,
            pretty,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_export(
                &db,
                &runner,
                &format,
                &conversations,
                source,
                workspace,
                role,
                output,
                session_files,
                pretty,
                cli.json,
            )
            .await
        }
        Command::Resume {
            id,
            search,
            agent,
            source,
            workspace,
            after,
            before,
            limit,
            dry_run,
            pick,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_resume(
                &db, &runner, &config, id, search, agent, source, workspace, after, before, limit,
                dry_run, pick, cli.json,
            )
            .await
        }
        Command::Stats => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_stats(&db, cli.json).await
        }
        Command::Dedup { dry_run, source } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_dedup(&db, dry_run, source, cli.json).await
        }
        Command::Mmry { command } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_mmry(&db, command, cli.json).await
        }
        Command::Remote { command } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_remote(&db, &config, &config_path, command, cli.json).await
        }
        Command::Web { command } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            cmd_web(&db, &config, &config_path, command, cli.json).await
        }
        Command::Config { command } => cmd_config(&config, &config_path, command, cli.json),
        Command::Reseed {
            source,
            dedup,
            no_index,
            no_bulk,
            dry_run,
            drop_source,
        } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_reseed(
                &db,
                &runner,
                &source,
                dedup,
                !no_index,
                !no_bulk,
                dry_run,
                drop_source,
                cli.json,
            )
            .await
        }
        Command::Verify { source, repair } => {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, &config);
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            cmd_verify(&db, &runner, source, repair, cli.json).await
        }
    }
}

/// Ensure sources from config file are in the database.
async fn ensure_config_sources(db: &Database, config: &Config) -> Result<()> {
    for source in &config.sources {
        let existing = db.get_source(&source.id).await?;
        let expanded_path = hstry_core::Config::expand_path(&source.path);
        let normalized_path = expanded_path
            .to_string_lossy()
            .trim_end_matches('/')
            .to_string();
        let entry = match existing {
            Some(mut entry) => {
                let mut reset = false;
                if entry.adapter != source.adapter {
                    entry.adapter.clone_from(&source.adapter);
                    reset = true;
                }
                let existing_normalized = entry
                    .path
                    .as_deref()
                    .map(|p| p.trim_end_matches('/').to_string())
                    .unwrap_or_default();
                if existing_normalized != normalized_path {
                    entry.path = Some(normalized_path);
                    reset = true;
                }
                if reset {
                    entry.last_sync_at = None;
                    if let serde_json::Value::Object(mut config) = entry.config.clone() {
                        config.remove("cursor");
                        entry.config = serde_json::Value::Object(config);
                    }
                }
                entry
            }
            None => Source {
                id: source.id.clone(),
                adapter: source.adapter.clone(),
                path: Some(normalized_path),
                last_sync_at: None,
                config: serde_json::Value::Object(serde_json::Map::default()),
            },
        };
        db.upsert_source(&entry).await?;
    }
    Ok(())
}

fn default_sync_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get().min(4))
        .unwrap_or(4)
}

async fn sync_sources(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    source_filter: Option<String>,
    parallel: Option<usize>,
    print: bool,
) -> Result<Vec<sync::SyncStats>> {
    adapter_manifest::validate_adapter_manifest(&config.adapter_paths)?;

    // Ensure sources from config are in the database
    ensure_config_sources(db, config).await?;

    let sources = db.list_sources().await?;

    if sources.is_empty() {
        if print {
            println!("No sources configured. Use 'hstry source add <path>' to add a source.");
        }
        return Ok(Vec::new());
    }

    let mut sources_to_sync = Vec::new();
    for source in sources {
        if let Some(ref filter) = source_filter
            && &source.id != filter
        {
            continue;
        }

        if !config.adapter_enabled(&source.adapter) {
            if print {
                println!(
                    "Syncing {id} ({adapter})...",
                    id = source.id,
                    adapter = source.adapter
                );
                println!("  Adapter disabled in config, skipping");
            }
            continue;
        }

        sources_to_sync.push(source);
    }

    if sources_to_sync.is_empty() {
        return Ok(Vec::new());
    }

    let parallelism = parallel.unwrap_or_else(default_sync_parallelism).max(1);
    let parallelism = parallelism.min(sources_to_sync.len().max(1));
    let stats = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    stream::iter(sources_to_sync)
        .for_each_concurrent(parallelism, |mut source| {
            let stats = Arc::clone(&stats);
            async move {
                if print {
                    println!(
                        "Syncing {id} ({adapter})...",
                        id = source.id,
                        adapter = source.adapter
                    );
                }

                if source.last_sync_at.is_some() {
                    match db.count_source_data(&source.id).await {
                        Ok((conv_count, _)) => {
                            if conv_count == 0 {
                                if print {
                                    println!("  No existing conversations; resetting sync cursor");
                                }
                                source.last_sync_at = None;
                                if let serde_json::Value::Object(mut config) = source.config.clone()
                                {
                                    config.remove("cursor");
                                    source.config = serde_json::Value::Object(config);
                                }
                            }
                        }
                        Err(err) => {
                            if print {
                                eprintln!("  Error: {err}");
                            }
                            return;
                        }
                    }
                }

                match sync::sync_source(db, runner, &source).await {
                    Ok(result) => {
                        if print {
                            if result.conversations > 0 {
                                println!(
                                    "  Synced {count} conversations",
                                    count = result.conversations
                                );
                            } else {
                                println!("  No new conversations");
                            }
                        }
                        let mut stats = stats.lock().await;
                        stats.push(result);
                    }
                    Err(err) => {
                        if print {
                            eprintln!("  Error: {err}");
                        }
                    }
                }
            }
        })
        .await;

    Ok(stats.lock().await.clone())
}

async fn cmd_sync(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    source_filter: Option<String>,
    parallel: Option<usize>,
    json: bool,
) -> Result<()> {
    let stats = sync_sources(db, runner, config, source_filter, parallel, !json).await?;

    if json {
        let total_sources = stats.len();
        let total_conversations = stats.iter().map(|s| s.conversations).sum();
        let total_messages = stats.iter().map(|s| s.messages).sum();
        return emit_json(JsonResponse {
            ok: true,
            result: Some(SyncSummary {
                sources: stats,
                total_sources,
                total_conversations,
                total_messages,
            }),
            error: None,
        });
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct ImportResult {
    adapter: String,
    confidence: f32,
    source_id: String,
    conversations: usize,
    messages: usize,
    dry_run: bool,
}

#[derive(Debug, Serialize)]
struct DetectionResult {
    adapter: String,
    confidence: f32,
}

#[expect(clippy::too_many_arguments)]
async fn cmd_import(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    path: PathBuf,
    adapter: Option<String>,
    source_id: Option<String>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    let expanded = Config::expand_path(&path_str);

    if !expanded.exists() {
        if json {
            return emit_json(JsonResponse::<()> {
                ok: false,
                result: None,
                error: Some(format!("Path not found: {path}", path = expanded.display())),
            });
        }
        anyhow::bail!("Path not found: {path}", path = expanded.display());
    }

    // Detect or use specified adapter
    let (adapter_name, confidence) = if let Some(name) = adapter {
        // Verify adapter exists
        if runner.find_adapter(&name).is_none() {
            if json {
                return emit_json(JsonResponse::<()> {
                    ok: false,
                    result: None,
                    error: Some(format!("Adapter '{name}' not found")),
                });
            }
            anyhow::bail!("Adapter '{name}' not found");
        }
        (name, 1.0f32)
    } else {
        // Auto-detect adapter
        if !json {
            println!("Detecting format for {path}...", path = expanded.display());
        }

        let mut best_match: Option<(String, f32)> = None;
        let mut all_matches: Vec<DetectionResult> = Vec::new();

        for adapter_name in runner.list_adapters() {
            if !config.adapter_enabled(&adapter_name) {
                continue;
            }

            if let Some(adapter_path) = runner.find_adapter(&adapter_name)
                && let Ok(Some(conf)) = runner
                    .detect(&adapter_path, &expanded.to_string_lossy())
                    .await
                && conf > 0.3
            {
                all_matches.push(DetectionResult {
                    adapter: adapter_name.clone(),
                    confidence: conf,
                });

                if best_match
                    .as_ref()
                    .is_none_or(|(_, best_conf)| conf > *best_conf)
                {
                    best_match = Some((adapter_name, conf));
                }
            }
        }

        // Sort by confidence descending
        all_matches.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if !json && all_matches.len() > 1 {
            println!("Detected formats:");
            for m in &all_matches {
                println!(
                    "  {adapter} ({confidence:.0}%)",
                    adapter = m.adapter,
                    confidence = m.confidence * 100.0
                );
            }
        }

        if let Some((name, conf)) = best_match {
            if !json {
                println!(
                    "Using adapter: {name} (confidence: {confidence:.0}%)",
                    confidence = conf * 100.0
                );
            }
            (name, conf)
        } else {
            if json {
                return emit_json(JsonResponse::<()> {
                    ok: false,
                    result: None,
                    error: Some("Could not detect format. Use --adapter to specify.".to_string()),
                });
            }
            anyhow::bail!(
                "Could not detect format for {path}. Use --adapter to specify.",
                path = expanded.display()
            );
        }
    };

    let Some(adapter_path) = runner.find_adapter(&adapter_name) else {
        anyhow::bail!("Adapter '{adapter_name}' not found");
    };
    let source_id = source_id.unwrap_or_else(|| format!("import-{adapter_name}"));

    // Parse conversations
    let parse_opts = hstry_runtime::runner::ParseOptions {
        since: None,
        limit: None,
        include_tools: true,
        include_attachments: true,
        cursor: None,
        batch_size: None,
    };

    let conversations = runner
        .parse(&adapter_path, &expanded.to_string_lossy(), parse_opts)
        .await?;

    if conversations.is_empty() {
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(ImportResult {
                    adapter: adapter_name,
                    confidence,
                    source_id,
                    conversations: 0,
                    messages: 0,
                    dry_run,
                }),
                error: None,
            });
        }
        println!("No conversations found.");
        return Ok(());
    }

    let conv_count = conversations.len();
    let msg_count: usize = conversations.iter().map(|c| c.messages.len()).sum();

    if dry_run {
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(ImportResult {
                    adapter: adapter_name,
                    confidence,
                    source_id,
                    conversations: conv_count,
                    messages: msg_count,
                    dry_run: true,
                }),
                error: None,
            });
        }
        println!("Dry run: would import {conv_count} conversations ({msg_count} messages)");
        for conv in &conversations {
            let title = conv.title.as_deref().unwrap_or("Untitled");
            let msg_cnt = conv.messages.len();
            println!("  - {title} ({msg_cnt} messages)");
        }
        return Ok(());
    }

    // Ensure source exists
    let source = hstry_core::models::Source {
        id: source_id.clone(),
        adapter: adapter_name.clone(),
        path: Some(expanded.to_string_lossy().to_string()),
        last_sync_at: None,
        config: serde_json::json!({}),
    };
    db.upsert_source(&source).await?;

    // Import conversations
    if !json {
        println!("Importing {conv_count} conversations...");
    }

    let mut imported_convs = 0usize;
    let mut imported_msgs = 0usize;

    for conv in conversations {
        let mut conv_id = uuid::Uuid::new_v4();
        if let Some(external_id) = conv.external_id.as_deref()
            && let Some(existing) = db.get_conversation_id(&source_id, external_id).await?
        {
            conv_id = existing;
        }

        let hstry_conv = hstry_core::models::Conversation {
            id: conv_id,
            source_id: source_id.clone(),
            external_id: conv.external_id,
            readable_id: conv.readable_id,
            platform_id: None,
            title: conv.title,
            created_at: chrono::DateTime::from_timestamp_millis(conv.created_at)
                .unwrap_or_default()
                .with_timezone(&chrono::Utc),
            updated_at: conv.updated_at.and_then(|ts| {
                chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&chrono::Utc))
            }),
            model: conv.model,
            provider: conv.provider,
            workspace: conv.workspace,
            tokens_in: conv.tokens_in,
            tokens_out: conv.tokens_out,
            cost_usd: conv.cost_usd,
            metadata: conv
                .metadata
                .map(|m| serde_json::to_value(m).unwrap_or_default())
                .unwrap_or_default(),
            harness: None,
            version: 0,
            message_count: 0,
            parent_conversation_id: None,
            parent_message_idx: conv.parent_message_idx,
            fork_type: conv.fork_type,
        };

        db.upsert_conversation(&hstry_conv).await?;

        for (idx, msg) in conv.messages.iter().enumerate() {
            let Ok(idx) = i32::try_from(idx) else {
                continue;
            };
            let parts_json = msg.parts.clone().unwrap_or_else(|| serde_json::json!([]));
            let role_str = msg.role.as_str();
            // Stable, content-addressable id (trx-hjjw.4) so re-imports of
            // the same source produce idempotent rows.
            let stable_id = hstry_core::stable_message_id(
                &source_id,
                hstry_conv.external_id.as_deref(),
                idx,
                role_str,
                &msg.content,
                None,
            );
            let hstry_msg = hstry_core::models::Message {
                id: stable_id,
                conversation_id: hstry_conv.id,
                idx,
                role: hstry_core::models::MessageRole::from(role_str),
                content: msg.content.clone(),
                parts_json,
                created_at: msg.created_at.and_then(|ts| {
                    chrono::DateTime::from_timestamp_millis(ts)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
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
            db.insert_message(&hstry_msg).await?;
            imported_msgs += 1;
        }

        imported_convs += 1;
    }

    // Update source last_sync_at
    let mut updated_source = source;
    updated_source.last_sync_at = Some(chrono::Utc::now());
    db.upsert_source(&updated_source).await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(ImportResult {
                adapter: adapter_name,
                confidence,
                source_id,
                conversations: imported_convs,
                messages: imported_msgs,
                dry_run: false,
            }),
            error: None,
        });
    }

    println!(
        "Imported {imported_convs} conversations ({imported_msgs} messages) into source '{source_id}'"
    );
    Ok(())
}

#[expect(clippy::too_many_arguments)]
async fn cmd_search_fast(
    config: &Config,
    query: &str,
    limit: i64,
    source: Option<String>,
    workspace: Option<String>,
    mode: SearchModeArg,
    scope: SearchScopeArg,
    remotes: Vec<String>,
    roles: Vec<SearchRoleArg>,
    no_tools: bool,
    dedup: bool,
    include_system: bool,
    after: Option<String>,
    before: Option<String>,
    model: Option<String>,
    harness_filter: Option<String>,
    tag: Option<String>,
    compact: bool,
    json: bool,
) -> Result<()> {
    // Parse date strings into DateTime<Utc>
    let after_dt = after.as_deref().map(parse_date_filter).transpose()?;
    let before_dt = before.as_deref().map(parse_date_filter).transpose()?;

    // Push single role filter to DB level for efficiency
    let db_role = if roles.len() == 1 {
        Some(roles[0].to_string())
    } else {
        None
    };

    // Request more results than needed if we're filtering, to ensure we get enough after filtering
    let has_filters = !include_system || !roles.is_empty() || no_tools || dedup;
    let fetch_limit = if has_filters { limit * 4 } else { limit };
    let opts = hstry_core::db::SearchOptions {
        source_id: source,
        workspace,
        limit: Some(fetch_limit),
        offset: None,
        mode: mode.into(),
        after: after_dt,
        before: before_dt,
        role: db_role,
        model,
        harness: harness_filter,
        tag,
    };
    let mut messages = Vec::new();

    if scope != SearchScopeArg::Remote {
        let service_expected = std::env::var("HSTRY_NO_SERVICE").is_err()
            && config.service.enabled
            && config.service.search_api;

        let local = if service_expected {
            if let Some(results) = hstry_core::service::try_service_search(query, &opts).await? {
                results
            } else {
                anyhow::bail!(
                    "Search service unavailable. Run `hstry service start` or set HSTRY_NO_SERVICE=1 to use local search."
                );
            }
        } else if let Some(results) = try_api_search(query, &opts, mode).await? {
            results
        } else {
            let db = Database::open(&config.database).await?;
            apply_storage_config(&db, config);
            db.search(query, opts.clone()).await?
        };
        messages.extend(local);
    }

    if scope != SearchScopeArg::Local {
        let remote_list = if remotes.is_empty() {
            config.remotes.clone()
        } else {
            config
                .remotes
                .iter()
                .filter(|remote| remotes.contains(&remote.name))
                .cloned()
                .collect()
        };

        let remote_hits = hstry_core::remote::search_remotes(&remote_list, query, &opts).await?;
        messages.extend(remote_hits);
    }

    messages.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Filter out system context (AGENTS.md, etc.) unless explicitly requested
    if !include_system {
        messages.retain(|hit| !is_system_context(&hit.content));
    }

    // Filter out tool messages if requested
    if no_tools {
        messages.retain(|hit| hit.role != MessageRole::Tool);
    }

    // Filter by role if specified
    if !roles.is_empty() {
        messages.retain(|hit| {
            roles.iter().any(|r| match r {
                SearchRoleArg::User => hit.role == MessageRole::User,
                SearchRoleArg::Assistant => hit.role == MessageRole::Assistant,
                SearchRoleArg::System => hit.role == MessageRole::System,
                SearchRoleArg::Tool => hit.role == MessageRole::Tool,
            })
        });
    }

    // Deduplicate by external_id (real session identifier) if requested
    if dedup {
        let mut seen = std::collections::HashSet::new();
        messages.retain(|hit| {
            // Use external_id if available, otherwise conversation_id
            let key = hit
                .external_id
                .clone()
                .unwrap_or_else(|| hit.conversation_id.to_string());
            seen.insert(key)
        });
    }

    // Group by external_id if compact mode is enabled
    if compact {
        use std::collections::BTreeMap;

        // Group by a unique key: prefer external_id, fall back to conversation_id
        // This ensures sessions that exist in multiple sources are grouped together
        let mut grouped: BTreeMap<String, (usize, hstry_core::models::SearchHit)> = BTreeMap::new();

        for hit in &messages {
            // Use external_id if available, otherwise conversation_id
            let key = hit
                .external_id
                .clone()
                .unwrap_or_else(|| hit.conversation_id.to_string());

            let entry = grouped.entry(key).or_insert((0, hit.clone()));
            entry.0 += 1; // increment occurrence count
            // Keep the hit with the highest score
            if hit.score > entry.1.score {
                entry.1 = hit.clone();
            }
        }

        // Convert back to vector with occurrence counts set
        messages = grouped
            .into_values()
            .map(|(count, mut hit)| {
                hit.occurrences = Some(count as i32);
                hit
            })
            .collect();

        // Re-sort by score after grouping
        messages.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Apply the original limit after filtering
    #[expect(clippy::cast_sign_loss)]
    messages.truncate(limit as usize);

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(messages),
            error: None,
        });
    }

    if compact {
        pretty::print_search_results_compact(&messages);
    } else {
        pretty::print_search_results(&messages);
    }
    Ok(())
}

/// Detect if content is system context (AGENTS.md, etc.) that should be hidden by default.
fn is_system_context(content: &str) -> bool {
    // Strong markers - if any of these are present, it's system context
    let strong_markers = [
        "# AGENTS.md",
        "# Agent Configuration",
        "<available_skills>",
        "Guidance for coding agents",
        "<SYSTEM_PROMPT>",
        "</SYSTEM_PROMPT>",
    ];

    for marker in &strong_markers {
        if content.contains(marker) {
            return true;
        }
    }

    // Check for AGENTS.md file path pattern
    if content.contains("AGENTS.md") && content.contains("instructions") {
        return true;
    }

    false
}

#[derive(Serialize)]
struct SearchApiQuery<'a> {
    query: &'a str,
    limit: Option<i64>,
    offset: Option<i64>,
    source: Option<&'a str>,
    workspace: Option<&'a str>,
    mode: SearchModeArg,
}

async fn try_api_search(
    query: &str,
    opts: &hstry_core::db::SearchOptions,
    mode: SearchModeArg,
) -> Result<Option<Vec<SearchHit>>> {
    if std::env::var("HSTRY_NO_API").is_ok() {
        return Ok(None);
    }

    let api_url =
        std::env::var("HSTRY_API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let url = format!("{base}/search", base = api_url.trim_end_matches('/'));

    let query_params = SearchApiQuery {
        query,
        limit: opts.limit,
        offset: opts.offset,
        source: opts.source_id.as_deref(),
        workspace: opts.workspace.as_deref(),
        mode,
    };

    let client = reqwest::Client::new();
    let Ok(response) = client.get(url).query(&query_params).send().await else {
        return Ok(None);
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    let body = response.text().await?;
    match serde_json::from_str::<Vec<SearchHit>>(&body) {
        Ok(results) => Ok(Some(results)),
        Err(_) => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum SearchModeArg {
    Auto,
    Natural,
    Code,
}

impl From<SearchModeArg> for hstry_core::db::SearchMode {
    fn from(value: SearchModeArg) -> Self {
        match value {
            SearchModeArg::Auto => hstry_core::db::SearchMode::Auto,
            SearchModeArg::Natural => hstry_core::db::SearchMode::NaturalLanguage,
            SearchModeArg::Code => hstry_core::db::SearchMode::Code,
        }
    }
}

#[derive(
    Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize, PartialEq, Eq,
)]
#[serde(rename_all = "lowercase")]
enum SearchScopeArg {
    Local,
    Remote,
    All,
}

#[derive(
    Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize, PartialEq, Eq,
)]
#[serde(rename_all = "lowercase")]
enum SearchRoleArg {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for SearchRoleArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchRoleArg::User => write!(f, "user"),
            SearchRoleArg::Assistant => write!(f, "assistant"),
            SearchRoleArg::System => write!(f, "system"),
            SearchRoleArg::Tool => write!(f, "tool"),
        }
    }
}

async fn cmd_list(
    db: &Database,
    source: Option<String>,
    workspace: Option<String>,
    limit: i64,
    json: bool,
) -> Result<()> {
    let workspace = workspace.map(|value| format!("%{value}%"));
    let opts = hstry_core::db::ListConversationsOptions {
        source_id: source,
        workspace,
        after: None,
        before: None,
        limit: Some(limit),
    };

    if json {
        let conversations = db.list_conversations(opts).await?;
        return emit_json(JsonResponse {
            ok: true,
            result: Some(conversations),
            error: None,
        });
    }

    let previews = db.list_conversation_previews(opts).await?;
    let display = previews
        .into_iter()
        .map(|preview| {
            let title = display_title_for_list(
                preview.conversation.title.as_deref(),
                preview.first_user_message.as_deref(),
            );
            pretty::ConversationDisplay {
                id: preview.conversation.id,
                source_id: preview.conversation.source_id,
                workspace: preview.conversation.workspace,
                created_at: preview.conversation.created_at,
                title,
            }
        })
        .collect::<Vec<_>>();

    pretty::print_conversations(&display);

    Ok(())
}

async fn cmd_show(db: &Database, id: &str, json: bool) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(id)?;
    let conv = db
        .get_conversation(uuid)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

    let messages = db.get_messages(uuid).await?;
    if json {
        let details = hstry_core::models::ConversationWithMessages {
            conversation: conv,
            messages: messages
                .into_iter()
                .map(|message| hstry_core::models::MessageWithExtras {
                    message,
                    tool_calls: Vec::new(),
                    attachments: Vec::new(),
                })
                .collect(),
        };
        return emit_json(JsonResponse {
            ok: true,
            result: Some(details),
            error: None,
        });
    }

    let title = conv.title.as_deref().unwrap_or("(untitled)");
    println!("Title: {title}");
    println!("Created: {created}", created = conv.created_at);
    println!("Source: {source}", source = conv.source_id);
    if let Some(ws) = &conv.workspace {
        println!("Workspace: {ws}");
    }
    println!();

    for msg in messages {
        println!("--- {role} ---", role = msg.role);
        println!("{content}", content = msg.content);
        println!();
    }

    Ok(())
}

async fn cmd_source(
    db: &Database,
    runner: &AdapterRunner,
    command: SourceCommand,
    json: bool,
) -> Result<()> {
    match command {
        SourceCommand::Add {
            path,
            adapter,
            id,
            input,
        } => {
            let input = read_input::<SourceAddInput>(input)?;
            let path_str = input
                .as_ref()
                .map_or_else(|| path.to_string_lossy().to_string(), |v| v.path.clone());
            let (input_adapter, input_id) = input
                .as_ref()
                .map_or((None, None), |v| (v.adapter.clone(), v.id.clone()));
            let adapter = input_adapter.or(adapter);
            let id = input_id.or(id);

            // Auto-detect adapter if not specified
            let adapter_name = if let Some(a) = adapter {
                a
            } else {
                let mut best_adapter = None;
                let mut best_confidence = 0.0f32;

                for adapter_name in runner.list_adapters() {
                    if let Some(adapter_path) = runner.find_adapter(&adapter_name)
                        && let Ok(Some(confidence)) = runner.detect(&adapter_path, &path_str).await
                        && confidence > best_confidence
                    {
                        best_confidence = confidence;
                        best_adapter = Some(adapter_name);
                    }
                }

                best_adapter.ok_or_else(|| {
                    anyhow::anyhow!("Could not auto-detect adapter for path: {path_str}")
                })?
            };

            let source_id = id.unwrap_or_else(|| {
                let uuid = uuid::Uuid::new_v4().to_string();
                let short = uuid.split('-').next().unwrap_or(uuid.as_str());
                format!("{adapter_name}-{short}")
            });

            let source = hstry_core::models::Source {
                id: source_id.clone(),
                adapter: adapter_name.clone(),
                path: Some(path_str),
                last_sync_at: None,
                config: serde_json::Value::Object(serde_json::Map::default()),
            };

            db.upsert_source(&source).await?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(source),
                    error: None,
                });
            }
            println!("Added source: {source_id} ({adapter_name})");
        }
        SourceCommand::List => {
            let sources = db.list_sources().await?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(sources),
                    error: None,
                });
            }
            if sources.is_empty() {
                println!("No sources configured.");
            } else {
                for source in sources {
                    let path = source.path.as_deref().unwrap_or("-");
                    println!(
                        "{id} | {adapter} | {path}",
                        id = source.id,
                        adapter = source.adapter
                    );
                }
            }
        }
        SourceCommand::Remove { id, input } => {
            let input = read_input::<SourceRemoveInput>(input)?;
            let id = input.as_ref().map_or(id, |v| v.id.clone());
            db.remove_source(&id).await?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({ "id": id })),
                    error: None,
                });
            }
            println!("Removed source: {id}");
        }
        SourceCommand::Cleanup { auto_remove } => {
            let sources = db.list_sources().await?;

            // Group by (adapter, path_normalized)
            use std::collections::HashMap;
            let mut groups: HashMap<(String, String), Vec<hstry_core::models::Source>> =
                HashMap::new();

            for source in &sources {
                let path_normalized = source
                    .path
                    .as_deref()
                    .map(|p| p.trim_end_matches('/'))
                    .unwrap_or("")
                    .to_lowercase();

                groups
                    .entry((source.adapter.clone(), path_normalized))
                    .or_default()
                    .push(source.clone());
            }

            // Find duplicates
            let mut to_remove: Vec<String> = Vec::new();
            let mut total_duplicates = 0usize;

            for ((adapter, path), mut group) in groups {
                if group.len() > 1 {
                    total_duplicates += group.len() - 1;
                    if !json {
                        println!(
                            "Found {n} duplicate sources for {adapter}:{path}",
                            n = group.len(),
                            adapter = adapter,
                            path = path
                        );
                    }

                    // Sort: keep the shortest/most canonical ID (e.g., "pi" over "pi-558c036f")
                    group.sort_by(|a, b| {
                        // Prefer non-generated IDs (shorter, no dashes)
                        let a_score = if a.id.contains('-') { 2 } else { 0 };
                        let b_score = if b.id.contains('-') { 2 } else { 0 };
                        a_score.cmp(&b_score).then(a.id.len().cmp(&b.id.len()))
                    });

                    if !json {
                        println!("  Keeping: {id}", id = group[0].id);
                    }

                    // Keep first, mark rest for removal
                    for source in group.iter().skip(1) {
                        to_remove.push(source.id.clone());
                        if !json {
                            println!(
                                "  Would remove: {id} (path: {path})",
                                id = source.id,
                                path = source.path.as_deref().unwrap_or("-")
                            );
                        }
                    }
                }
            }

            if total_duplicates == 0 {
                if !json {
                    println!("No duplicate sources found.");
                }
                return Ok(());
            }

            if auto_remove {
                if !json {
                    println!(
                        "\nRemoving {count} duplicate sources...",
                        count = to_remove.len()
                    );
                }

                let mut conversations_removed = 0i64;
                let mut messages_removed = 0i64;

                for source_id in &to_remove {
                    // Get counts before removing
                    let (conv_count, msg_count) =
                        db.count_source_data(source_id).await.unwrap_or((0, 0));

                    conversations_removed += conv_count;
                    messages_removed += msg_count;

                    db.remove_source(source_id).await?;
                }

                if !json {
                    println!(
                        "Removed {sources} sources, {convs} conversations, {msgs} messages",
                        sources = to_remove.len(),
                        convs = conversations_removed,
                        msgs = messages_removed
                    );
                } else {
                    return emit_json(JsonResponse {
                        ok: true,
                        result: Some(serde_json::json!({
                            "removed_sources": to_remove.len(),
                            "removed_conversations": conversations_removed,
                            "removed_messages": messages_removed,
                            "source_ids": to_remove,
                        })),
                        error: None,
                    });
                }
            } else if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({
                        "duplicate_count": total_duplicates,
                        "source_ids": to_remove,
                    })),
                    error: None,
                });
            }
        }
    }
    Ok(())
}

fn cmd_adapters(
    runner: &AdapterRunner,
    config: &Config,
    config_path: &Path,
    command: Option<AdapterCommand>,
    json: bool,
) -> Result<()> {
    let mut config = config.clone();
    match command.unwrap_or(AdapterCommand::List) {
        AdapterCommand::List => {
            let adapters = runner.list_adapters();
            let statuses: Vec<AdapterStatus> = adapters
                .into_iter()
                .map(|adapter| AdapterStatus {
                    enabled: config.adapter_enabled(&adapter),
                    name: adapter,
                })
                .collect();
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(statuses),
                    error: None,
                });
            }
            if statuses.is_empty() {
                println!("No adapters found.");
            } else {
                println!("Available adapters:");
                for adapter in statuses {
                    let status = if adapter.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    println!("  {name} ({status})", name = adapter.name);
                }
            }
        }
        AdapterCommand::Add { path, input } => {
            let input = read_input::<AdapterAddInput>(input)?;
            let path = input
                .as_ref()
                .map(|v| PathBuf::from(&v.path))
                .unwrap_or(path);
            let expanded = Config::expand_path(&path.to_string_lossy());
            if !config.adapter_paths.contains(&expanded) {
                config.adapter_paths.push(expanded);
                config.save_to_path(config_path)?;
                if json {
                    return emit_json(JsonResponse {
                        ok: true,
                        result: Some(serde_json::json!({
                            "adapter_paths": config.adapter_paths,
                        })),
                        error: None,
                    });
                }
                println!("Added adapter path to config.");
            } else {
                if json {
                    return emit_json(JsonResponse {
                        ok: true,
                        result: Some(serde_json::json!({
                            "adapter_paths": config.adapter_paths,
                        })),
                        error: None,
                    });
                }
                println!("Adapter path already present in config.");
            }
        }
        AdapterCommand::Enable { name, input } => {
            let input = read_input::<AdapterToggleInput>(input)?;
            let name = input.as_ref().map_or(name, |v| v.name.clone());
            upsert_adapter_config(&mut config, &name, true);
            config.save_to_path(config_path)?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(AdapterStatus {
                        name,
                        enabled: true,
                    }),
                    error: None,
                });
            }
            println!("Enabled adapter: {name}");
        }
        AdapterCommand::Disable { name, input } => {
            let input = read_input::<AdapterToggleInput>(input)?;
            let name = input.as_ref().map_or(name, |v| v.name.clone());
            upsert_adapter_config(&mut config, &name, false);
            config.save_to_path(config_path)?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(AdapterStatus {
                        name,
                        enabled: false,
                    }),
                    error: None,
                });
            }
            println!("Disabled adapter: {name}");
        }
        AdapterCommand::Update {
            adapter,
            repo,
            force,
        } => {
            let expected_ref = format!("v{}", env!("CARGO_PKG_VERSION"));

            let mut repos_to_update: Vec<_> = config
                .adapter_repos
                .iter()
                .filter(|r| r.enabled)
                .filter(|r| repo.as_ref().is_none_or(|name| &r.name == name))
                .cloned()
                .collect();

            if repos_to_update.is_empty() {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some("No matching enabled repositories found".to_string()),
                    });
                }
                println!("No matching enabled repositories found.");
                return Ok(());
            }

            let mut config_changed = false;
            for repo in &mut repos_to_update {
                if let AdapterRepoSource::Git { url, git_ref, .. } = &mut repo.source
                    && url == hstry_core::config::DEFAULT_ADAPTER_REPO
                    && git_ref == "main"
                {
                    *git_ref = expected_ref.clone();
                    config_changed = true;
                }
            }

            if config_changed {
                if let Some(repo_override) = repo.as_ref() {
                    config
                        .adapter_repos
                        .iter_mut()
                        .filter(|r| &r.name == repo_override)
                        .for_each(|r| {
                            if let AdapterRepoSource::Git { url, git_ref, .. } = &mut r.source
                                && url == hstry_core::config::DEFAULT_ADAPTER_REPO
                                && git_ref == "main"
                            {
                                *git_ref = expected_ref.clone();
                            }
                        });
                } else {
                    for r in &mut config.adapter_repos {
                        if let AdapterRepoSource::Git { url, git_ref, .. } = &mut r.source
                            && url == hstry_core::config::DEFAULT_ADAPTER_REPO
                            && git_ref == "main"
                        {
                            *git_ref = expected_ref.clone();
                        }
                    }
                }
                config.save_to_path(config_path)?;
            }

            let adapter_root = adapter_root_dir(&config)?;
            std::fs::create_dir_all(&adapter_root)?;

            let mut updated_repos = Vec::new();

            for repo in &repos_to_update {
                let repo_result =
                    update_repo_adapters(repo, &adapter_root, adapter.as_deref(), force)?;
                updated_repos.push(repo_result);
            }

            adapter_manifest::validate_adapter_manifest(&config.adapter_paths)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({
                        "adapter_root": adapter_root,
                        "repos": updated_repos,
                    })),
                    error: None,
                });
            }

            println!("Updated adapters in {}", adapter_root.display());
            for repo_result in &updated_repos {
                println!(
                    "  {name}: {count} adapters",
                    name = repo_result.name,
                    count = repo_result.adapters.len()
                );
            }
        }
        AdapterCommand::Repo { command } => {
            cmd_adapter_repo(&mut config, config_path, command, json)?;
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct RepoUpdateResult {
    name: String,
    adapters: Vec<String>,
    source: String,
}

fn adapter_root_dir(config: &Config) -> Result<PathBuf> {
    if let Some(path) = config.adapter_paths.first() {
        return Ok(path.clone());
    }

    let config_dir = Config::default_config_path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve config directory"))?
        .to_path_buf();
    Ok(config_dir.join("adapters"))
}

fn update_repo_adapters(
    repo: &AdapterRepo,
    adapter_root: &Path,
    filter: Option<&str>,
    force: bool,
) -> Result<RepoUpdateResult> {
    match &repo.source {
        AdapterRepoSource::Git { url, git_ref, path } => {
            let temp_dir = tempfile::tempdir()?;
            let target = temp_dir.path();

            let mut cmd = ProcessCommand::new("git");
            cmd.arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--branch")
                .arg(git_ref)
                .arg(url)
                .arg(target);

            let output = cmd.output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("Failed to clone adapters repo: {stderr}");
            }

            let src_root = target.join(path);
            let source_label = format!("git:{url}@{git_ref}");
            let adapters = copy_adapters_from(&src_root, adapter_root, filter, force)?;

            Ok(RepoUpdateResult {
                name: repo.name.clone(),
                adapters,
                source: source_label,
            })
        }
        AdapterRepoSource::Local { path } => {
            let src_root = PathBuf::from(path);
            let source_label = format!("local:{path}");
            let adapters = copy_adapters_from(&src_root, adapter_root, filter, force)?;

            Ok(RepoUpdateResult {
                name: repo.name.clone(),
                adapters,
                source: source_label,
            })
        }
        AdapterRepoSource::Archive { url, .. } => {
            anyhow::bail!("Archive adapter repositories are not supported yet: {url}");
        }
    }
}

fn copy_adapters_from(
    src_root: &Path,
    dest_root: &Path,
    filter: Option<&str>,
    force: bool,
) -> Result<Vec<String>> {
    let mut adapters = Vec::new();

    if !src_root.exists() {
        anyhow::bail!("Adapter source path not found: {}", src_root.display());
    }

    let manifest_path = src_root.join(".hstry-adapters.json");
    if !manifest_path.exists() {
        anyhow::bail!(
            "Adapter manifest missing at {}. Ensure the repo matches the hstry version.",
            manifest_path.display()
        );
    }

    let mut items = Vec::new();
    if let Some(adapter) = filter {
        items.push(adapter.to_string());
    } else {
        for entry in std::fs::read_dir(src_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                items.push(name.to_string());
            }
        }
    }

    let mut entries_to_copy = Vec::new();
    if !items.iter().any(|item| item == "types") {
        entries_to_copy.push("types".to_string());
    }
    entries_to_copy.extend(items);

    for entry_name in entries_to_copy {
        let src_path = src_root.join(&entry_name);
        if !src_path.exists() {
            if filter.is_some() && !force {
                anyhow::bail!("Adapter not found: {entry_name}");
            }
            continue;
        }

        let dest_path = dest_root.join(&entry_name);
        if dest_path.exists() {
            std::fs::remove_dir_all(&dest_path)?;
        }
        copy_dir_recursive(&src_path, &dest_path)?;

        if entry_name != "types" {
            adapters.push(entry_name);
        }
    }

    let dest_manifest = dest_root.join(".hstry-adapters.json");
    let manifest = adapter_manifest::AdapterManifest {
        hstry_version: adapter_manifest::expected_hstry_version(),
        protocol_version: adapter_manifest::ADAPTER_PROTOCOL_VERSION.to_string(),
    };
    std::fs::write(&dest_manifest, serde_json::to_string_pretty(&manifest)?)?;

    Ok(adapters)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;

    for entry in walkdir::WalkDir::new(src) {
        let entry = entry?;
        let path = entry.path();
        let rel_path = path.strip_prefix(src)?;
        let target_path = dest.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &target_path)?;
        }
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct QuickstartSummary {
    sources_added: Vec<serde_json::Value>,
    sources_skipped: usize,
    sync: SyncSummary,
}

#[derive(Debug, Serialize)]
struct WebInstallResult {
    browser: String,
    command: String,
}

#[derive(Debug, Serialize)]
struct WebLoginResult {
    provider: String,
    storage_state: String,
}

#[derive(Debug, Serialize)]
struct WebSyncResult {
    provider: String,
    export_path: String,
    sources: SyncSummary,
}

#[derive(Debug, Serialize)]
struct WebStatusEntry {
    provider: String,
    logged_in: bool,
    last_sync: Option<String>,
    export_path: Option<String>,
}

async fn cmd_web(
    db: &Database,
    config: &Config,
    config_path: &Path,
    command: WebCommand,
    json: bool,
) -> Result<()> {
    match command {
        WebCommand::Install { browser } => cmd_web_install(&browser, json),
        WebCommand::Login {
            provider,
            headful,
            browser,
        } => cmd_web_login(&provider, headful, browser.as_deref(), json),
        WebCommand::Sync {
            provider,
            headful,
            browser,
        } => {
            cmd_web_sync(
                db,
                config,
                config_path,
                provider.as_deref(),
                headful,
                browser.as_deref(),
                json,
            )
            .await
        }
        WebCommand::Status => cmd_web_status(json),
    }
}

fn cmd_web_install(browser: &str, json: bool) -> Result<()> {
    let browser = browser.to_lowercase();
    let supported = ["chromium", "firefox", "webkit"];
    if !supported.contains(&browser.as_str()) {
        anyhow::bail!("Unsupported browser: {browser}");
    }

    let web_dir = web_runner_dir()?;
    ensure_web_runner(&web_dir)?;

    let bun_path = which::which("bun").map_err(|_| {
        anyhow::anyhow!("Bun is required to install Playwright. Install bun and retry.")
    })?;

    let mut install_cmd = ProcessCommand::new(bun_path);
    install_cmd.arg("install").current_dir(&web_dir);
    let output = install_cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Playwright install failed: {stderr}");
    }

    let (command, mut cmd) = if which::which("playwright").is_ok() {
        let mut cmd = ProcessCommand::new("playwright");
        cmd.arg("install").arg(&browser);
        ("playwright install".to_string(), cmd)
    } else if which::which("bunx").is_ok() {
        let mut cmd = ProcessCommand::new("bunx");
        cmd.arg("playwright").arg("install").arg(&browser);
        ("bunx playwright install".to_string(), cmd)
    } else if which::which("npx").is_ok() {
        let mut cmd = ProcessCommand::new("npx");
        cmd.arg("playwright").arg("install").arg(&browser);
        ("npx playwright install".to_string(), cmd)
    } else {
        anyhow::bail!("Playwright not found. Install bun or node, then retry.");
    };

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Playwright install failed: {stderr}");
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(WebInstallResult {
                browser: browser.to_string(),
                command,
            }),
            error: None,
        });
    }

    println!("Playwright installed for {browser}.");
    Ok(())
}

fn cmd_web_login(provider: &str, headful: bool, browser: Option<&str>, json: bool) -> Result<()> {
    let provider = normalize_provider(provider)?;
    let web_dir = web_runner_dir()?;
    ensure_web_runner(&web_dir)?;

    let storage_state = web_sessions_dir()?.join(format!("{provider}.json"));
    let headful = if headful {
        true
    } else {
        if !json {
            println!("Login requires a visible browser. Launching headful session...");
        }
        true
    };

    let args = build_web_args("login", &provider, headful, browser, &storage_state, None);

    run_web_runner(&web_dir, &args)?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(WebLoginResult {
                provider,
                storage_state: storage_state.to_string_lossy().to_string(),
            }),
            error: None,
        });
    }

    println!("Logged in to {provider}.");
    Ok(())
}

async fn cmd_web_sync(
    db: &Database,
    config: &Config,
    config_path: &Path,
    provider: Option<&str>,
    headful: bool,
    browser: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut config = config.clone();
    let providers = normalize_provider_list(provider)?;

    let web_dir = web_runner_dir()?;
    ensure_web_runner(&web_dir)?;

    let mut results = Vec::new();

    let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
        anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
    })?;
    let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

    for provider in providers {
        let storage_state = web_sessions_dir()?.join(format!("{provider}.json"));
        if !storage_state.exists() {
            anyhow::bail!(
                "No login session found for {provider}. Run 'hstry web login {provider}'."
            );
        }

        let export_dir = web_exports_dir()?.join(&provider);
        let export_path = export_dir.join("conversations.json");

        let args = build_web_args(
            "sync",
            &provider,
            headful,
            browser,
            &storage_state,
            Some(&export_path),
        );
        run_web_runner(&web_dir, &args)?;

        let adapter = match provider.as_str() {
            "chatgpt" => "chatgpt",
            "claude" => "claude-web",
            "gemini" => "gemini",
            _ => "chatgpt",
        };

        let source_id = format!("web-{provider}");
        if !config.sources.iter().any(|s| s.id == source_id) {
            config.sources.push(hstry_core::config::SourceConfig {
                id: source_id.clone(),
                adapter: adapter.to_string(),
                path: export_path.to_string_lossy().to_string(),
                auto_sync: true,
            });
            config.save_to_path(config_path)?;
        }

        ensure_config_sources(db, &config).await?;
        let stats =
            sync_sources(db, &runner, &config, Some(source_id.clone()), None, !json).await?;

        let summary = SyncSummary {
            total_sources: stats.len(),
            total_conversations: stats.iter().map(|s| s.conversations).sum(),
            total_messages: stats.iter().map(|s| s.messages).sum(),
            sources: stats,
        };

        results.push(WebSyncResult {
            provider: provider.to_string(),
            export_path: export_path.to_string_lossy().to_string(),
            sources: summary,
        });
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(results),
            error: None,
        });
    }

    for result in &results {
        println!(
            "Synced {provider} to {path}.",
            provider = result.provider,
            path = result.export_path
        );
    }

    Ok(())
}

fn cmd_web_status(json: bool) -> Result<()> {
    let providers = normalize_provider_list(None)?;
    let mut statuses = Vec::new();

    for provider in providers {
        let storage_state = web_sessions_dir()?.join(format!("{provider}.json"));
        let export_path = web_exports_dir()?
            .join(&provider)
            .join("conversations.json");
        let logged_in = storage_state.exists();
        let last_sync = if export_path.exists() {
            std::fs::metadata(&export_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339())
        } else {
            None
        };

        statuses.push(WebStatusEntry {
            provider,
            logged_in,
            last_sync,
            export_path: export_path
                .exists()
                .then(|| export_path.to_string_lossy().to_string()),
        });
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(statuses),
            error: None,
        });
    }

    for status in &statuses {
        let login_status = if status.logged_in {
            "logged in"
        } else {
            "not logged in"
        };
        println!("{provider}: {login_status}", provider = status.provider);
        if let Some(last_sync) = &status.last_sync {
            println!("  Last sync: {last_sync}");
        }
    }

    Ok(())
}

fn web_runner_dir() -> Result<PathBuf> {
    let config_dir = xdg_config_dir();
    Ok(config_dir.join("hstry").join("web"))
}

fn web_sessions_dir() -> Result<PathBuf> {
    Ok(xdg_data_dir().join("hstry").join("web-sessions"))
}

fn web_exports_dir() -> Result<PathBuf> {
    Ok(xdg_data_dir().join("hstry").join("web-exports"))
}

fn ensure_web_runner(web_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(web_dir)?;

    let version_path = web_dir.join(".hstry-web-version");
    let expected_version = env!("CARGO_PKG_VERSION");
    let current_version = std::fs::read_to_string(&version_path)
        .ok()
        .map(|value| value.trim().to_string());

    if current_version.as_deref() != Some(expected_version) {
        let node_modules = web_dir.join("node_modules");
        if node_modules.exists() {
            std::fs::remove_dir_all(node_modules)?;
        }
    }

    let script_path = web_dir.join("web-runner.ts");
    let package_path = web_dir.join("package.json");

    std::fs::write(script_path, include_str!("../assets/web-runner.ts"))?;
    std::fs::write(package_path, include_str!("../assets/web-package.json"))?;
    std::fs::write(version_path, expected_version)?;

    Ok(())
}

fn build_web_args(
    command: &str,
    provider: &str,
    headful: bool,
    browser: Option<&str>,
    storage_state: &Path,
    output: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "web-runner.ts".to_string(),
        command.to_string(),
        "--provider".to_string(),
        provider.to_string(),
        "--storage-state".to_string(),
        storage_state.to_string_lossy().to_string(),
    ];

    if headful {
        args.push("--headful".to_string());
    }

    if let Some(browser) = browser {
        args.push("--browser".to_string());
        args.push(browser.to_string());
    }

    if let Some(output) = output {
        args.push("--output".to_string());
        args.push(output.to_string_lossy().to_string());
    }

    args
}

fn run_web_runner(web_dir: &Path, args: &[String]) -> Result<()> {
    let bun_path = which::which("bun")
        .map_err(|_| anyhow::anyhow!("Bun is required to run web automation."))?;

    let output = ProcessCommand::new(bun_path)
        .args(args)
        .current_dir(web_dir)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Web runner failed: {stderr}");
    }

    Ok(())
}

fn normalize_provider(provider: &str) -> Result<String> {
    let provider = provider.to_lowercase();
    match provider.as_str() {
        "chatgpt" | "claude" | "gemini" => Ok(provider),
        _ => anyhow::bail!("Unsupported provider: {provider}"),
    }
}

fn normalize_provider_list(provider: Option<&str>) -> Result<Vec<String>> {
    if let Some(provider) = provider {
        return Ok(vec![normalize_provider(provider)?]);
    }

    Ok(vec![
        "chatgpt".to_string(),
        "claude".to_string(),
        "gemini".to_string(),
    ])
}

fn xdg_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    if cfg!(unix) {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".config"))
    } else {
        dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

fn xdg_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    if cfg!(unix) {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".local").join("share"))
    } else {
        dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

async fn cmd_quickstart(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    config_path: &Path,
    json: bool,
) -> Result<()> {
    let mut config = config.clone();

    if adapter_manifest::validate_adapter_manifest(&config.adapter_paths).is_err() {
        ensure_adapter_updates(&mut config, config_path, json)?;
    }

    let hits = scan_hits(runner, &config).await?;
    if hits.is_empty() {
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(QuickstartSummary {
                    sources_added: Vec::new(),
                    sources_skipped: 0,
                    sync: SyncSummary {
                        sources: Vec::new(),
                        total_sources: 0,
                        total_conversations: 0,
                        total_messages: 0,
                    },
                }),
                error: None,
            });
        }
        println!("No sources detected.");
        return Ok(());
    }

    let mut sources_added = Vec::new();
    let mut sources_skipped = 0usize;

    for hit in &hits {
        if config
            .sources
            .iter()
            .any(|source| source.adapter == hit.adapter && source.path == hit.path)
        {
            sources_skipped += 1;
            continue;
        }

        if let Ok(Some(_)) = db.get_source_by_adapter_path(&hit.adapter, &hit.path).await {
            sources_skipped += 1;
            continue;
        }

        let source_id = generate_source_id(&config, &hit.adapter);
        config.sources.push(hstry_core::config::SourceConfig {
            id: source_id.clone(),
            adapter: hit.adapter.clone(),
            path: hit.path.clone(),
            auto_sync: true,
        });

        sources_added.push(serde_json::json!({
            "id": source_id,
            "adapter": hit.adapter,
            "path": hit.path,
        }));
    }

    if !sources_added.is_empty() {
        config.save_to_path(config_path)?;
    }

    ensure_config_sources(db, &config).await?;

    let stats = sync_sources(db, runner, &config, None, None, !json).await?;
    let sync_summary = SyncSummary {
        total_sources: stats.len(),
        total_conversations: stats.iter().map(|s| s.conversations).sum(),
        total_messages: stats.iter().map(|s| s.messages).sum(),
        sources: stats,
    };

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(QuickstartSummary {
                sources_added,
                sources_skipped,
                sync: sync_summary,
            }),
            error: None,
        });
    }

    println!(
        "Quickstart added {} sources (skipped {}).",
        sources_added.len(),
        sources_skipped
    );
    Ok(())
}

fn generate_source_id(config: &Config, adapter: &str) -> String {
    let uuid = uuid::Uuid::new_v4().to_string();
    let short = uuid.split('-').next().unwrap_or(uuid.as_str());
    let mut candidate = format!("{adapter}-{short}");

    let mut idx = 1u32;
    while config.sources.iter().any(|s| s.id == candidate) {
        candidate = format!("{adapter}-{short}-{idx}");
        idx += 1;
    }

    candidate
}

fn ensure_adapter_updates(config: &mut Config, config_path: &Path, json: bool) -> Result<()> {
    let expected_ref = format!("v{}", env!("CARGO_PKG_VERSION"));
    let mut repos_to_update: Vec<_> = config
        .adapter_repos
        .iter()
        .filter(|r| r.enabled)
        .cloned()
        .collect();

    if repos_to_update.is_empty() {
        anyhow::bail!("No enabled adapter repositories. Run 'hstry adapters repo add-git'.");
    }

    let mut config_changed = false;
    for repo in &mut repos_to_update {
        if let AdapterRepoSource::Git { url, git_ref, .. } = &mut repo.source
            && url == hstry_core::config::DEFAULT_ADAPTER_REPO
            && git_ref == "main"
        {
            *git_ref = expected_ref.clone();
            config_changed = true;
        }
    }

    if config_changed {
        for r in &mut config.adapter_repos {
            if let AdapterRepoSource::Git { url, git_ref, .. } = &mut r.source
                && url == hstry_core::config::DEFAULT_ADAPTER_REPO
                && git_ref == "main"
            {
                *git_ref = expected_ref.clone();
            }
        }
        config.save_to_path(config_path)?;
    }

    let adapter_root = adapter_root_dir(config)?;
    std::fs::create_dir_all(&adapter_root)?;

    let mut updated_repos = Vec::new();
    for repo in &repos_to_update {
        let repo_result = update_repo_adapters(repo, &adapter_root, None, false)?;
        updated_repos.push(repo_result);
    }

    adapter_manifest::validate_adapter_manifest(&config.adapter_paths)?;

    if !json {
        println!("Updated adapters in {}", adapter_root.display());
        for repo_result in &updated_repos {
            println!(
                "  {name}: {count} adapters",
                name = repo_result.name,
                count = repo_result.adapters.len()
            );
        }
    }

    Ok(())
}

fn cmd_adapter_repo(
    config: &mut Config,
    config_path: &Path,
    command: AdapterRepoCommand,
    json: bool,
) -> Result<()> {
    match command {
        AdapterRepoCommand::List => {
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(&config.adapter_repos),
                    error: None,
                });
            }
            if config.adapter_repos.is_empty() {
                println!("No adapter repositories configured.");
            } else {
                println!("Adapter repositories:");
                for repo in &config.adapter_repos {
                    let status = if repo.enabled { "enabled" } else { "disabled" };
                    let source_info = match &repo.source {
                        AdapterRepoSource::Git { url, git_ref, path } => {
                            format!("git {url} ({git_ref}) path={path}")
                        }
                        AdapterRepoSource::Archive { url, path } => {
                            format!("archive {url} path={path}")
                        }
                        AdapterRepoSource::Local { path } => format!("local {path}"),
                    };
                    println!("  {name} ({status}) - {source_info}", name = repo.name);
                }
            }
        }
        AdapterRepoCommand::AddGit {
            name,
            url,
            git_ref,
            path,
        } => {
            // Check if repo with this name already exists
            if config.adapter_repos.iter().any(|r| r.name == name) {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' already exists")),
                    });
                }
                anyhow::bail!("Repository '{name}' already exists");
            }

            let repo = AdapterRepo {
                name: name.clone(),
                source: AdapterRepoSource::Git { url, git_ref, path },
                enabled: true,
            };
            config.adapter_repos.push(repo.clone());
            config.save_to_path(config_path)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(repo),
                    error: None,
                });
            }
            println!("Added git repository: {name}");
        }
        AdapterRepoCommand::AddArchive { name, url, path } => {
            if config.adapter_repos.iter().any(|r| r.name == name) {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' already exists")),
                    });
                }
                anyhow::bail!("Repository '{name}' already exists");
            }

            let repo = AdapterRepo {
                name: name.clone(),
                source: AdapterRepoSource::Archive { url, path },
                enabled: true,
            };
            config.adapter_repos.push(repo.clone());
            config.save_to_path(config_path)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(repo),
                    error: None,
                });
            }
            println!("Added archive repository: {name}");
        }
        AdapterRepoCommand::AddLocal { name, path } => {
            if config.adapter_repos.iter().any(|r| r.name == name) {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' already exists")),
                    });
                }
                anyhow::bail!("Repository '{name}' already exists");
            }

            let repo = AdapterRepo {
                name: name.clone(),
                source: AdapterRepoSource::Local {
                    path: path.to_string_lossy().to_string(),
                },
                enabled: true,
            };
            config.adapter_repos.push(repo.clone());
            config.save_to_path(config_path)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(repo),
                    error: None,
                });
            }
            println!("Added local repository: {name}");
        }
        AdapterRepoCommand::Remove { name } => {
            let original_len = config.adapter_repos.len();
            config.adapter_repos.retain(|r| r.name != name);

            if config.adapter_repos.len() == original_len {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' not found")),
                    });
                }
                anyhow::bail!("Repository '{name}' not found");
            }

            config.save_to_path(config_path)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({ "removed": name })),
                    error: None,
                });
            }
            println!("Removed repository: {name}");
        }
        AdapterRepoCommand::Enable { name } => {
            let repo = config.adapter_repos.iter_mut().find(|r| r.name == name);
            if let Some(repo) = repo {
                repo.enabled = true;
                config.save_to_path(config_path)?;

                if json {
                    return emit_json(JsonResponse {
                        ok: true,
                        result: Some(serde_json::json!({ "name": name, "enabled": true })),
                        error: None,
                    });
                }
                println!("Enabled repository: {name}");
            } else {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' not found")),
                    });
                }
                anyhow::bail!("Repository '{name}' not found");
            }
        }
        AdapterRepoCommand::Disable { name } => {
            let repo = config.adapter_repos.iter_mut().find(|r| r.name == name);
            if let Some(repo) = repo {
                repo.enabled = false;
                config.save_to_path(config_path)?;

                if json {
                    return emit_json(JsonResponse {
                        ok: true,
                        result: Some(serde_json::json!({ "name": name, "enabled": false })),
                        error: None,
                    });
                }
                println!("Disabled repository: {name}");
            } else {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Repository '{name}' not found")),
                    });
                }
                anyhow::bail!("Repository '{name}' not found");
            }
        }
    }

    Ok(())
}

async fn scan_hits(runner: &AdapterRunner, config: &Config) -> Result<Vec<ScanHit>> {
    let mut hits = Vec::new();
    for adapter_name in runner.list_adapters() {
        if !config.adapter_enabled(&adapter_name) {
            continue;
        }
        if let Some(adapter_path) = runner.find_adapter(&adapter_name)
            && let Ok(info) = runner.get_info(&adapter_path).await
        {
            for default_path in &info.default_paths {
                let expanded = hstry_core::Config::expand_path(default_path);
                if expanded.exists()
                    && let Ok(Some(confidence)) = runner
                        .detect(&adapter_path, &expanded.to_string_lossy())
                        .await
                    && confidence > 0.5
                {
                    hits.push(ScanHit {
                        adapter: adapter_name.clone(),
                        display_name: info.display_name.clone(),
                        path: expanded.to_string_lossy().to_string(),
                        confidence,
                    });
                }
            }
        }
    }

    Ok(hits)
}

async fn cmd_scan(runner: &AdapterRunner, config: &Config, json: bool) -> Result<()> {
    if !json {
        println!("Scanning for chat history sources...\n");
    }

    let hits = scan_hits(runner, config).await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(hits),
            error: None,
        });
    }

    for hit in hits {
        println!(
            "  {} {} (confidence: {:.0}%)",
            hit.display_name,
            hit.path,
            hit.confidence * 100.0
        );
    }

    Ok(())
}

fn upsert_adapter_config(config: &mut Config, name: &str, enabled: bool) {
    if let Some(entry) = config.adapters.iter_mut().find(|entry| entry.name == name) {
        entry.enabled = enabled;
    } else {
        config.adapters.push(hstry_core::config::AdapterConfig {
            name: name.to_string(),
            enabled,
        });
    }
}

fn read_input<T: DeserializeOwned>(input: Option<PathBuf>) -> Result<Option<T>> {
    let Some(path) = input else {
        return Ok(None);
    };
    let mut buf = String::new();
    if path.as_os_str() == "-" {
        std::io::stdin().read_to_string(&mut buf)?;
    } else {
        let mut file = std::fs::File::open(path)?;
        file.read_to_string(&mut buf)?;
    }
    let value = serde_json::from_str(&buf)?;
    Ok(Some(value))
}

fn emit_json<T: serde::Serialize>(value: T) -> Result<()> {
    let pretty = serde_json::to_string_pretty(&value)?;
    println!("{pretty}");
    Ok(())
}

/// Truncate a title for display, cleaning up whitespace and newlines.
fn truncate_title(title: &str, max_len: usize) -> String {
    // Replace newlines and collapse whitespace
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.chars().count() <= max_len {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

fn display_title_for_list(title: Option<&str>, first_user: Option<&str>) -> String {
    let title = title.unwrap_or("");
    let first_user = first_user.unwrap_or("");

    if (title.is_empty() || is_system_context(title)) && !first_user.trim().is_empty() {
        return first_user.to_string();
    }

    if title.trim().is_empty() {
        "(untitled)".to_string()
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn read_input_reads_json_file() {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(file, "{{\"id\":\"conv-1\"}}").expect("write");
        let value: Option<ShowInput> = read_input(Some(file.path().to_path_buf())).expect("read");
        let value = value.expect("value");
        assert_eq!(value.id, "conv-1");
    }

    #[test]
    fn is_system_context_detects_agents_md() {
        // Should detect AGENTS.md markers
        assert!(is_system_context(
            "# AGENTS.md\n\nGuidance for coding agents"
        ));
        assert!(is_system_context(
            "Some text\n<available_skills>\n</available_skills>"
        ));
        assert!(is_system_context(
            "# Agent Configuration\n\nSome instructions"
        ));
        assert!(is_system_context("AGENTS.md instructions for the agent"));

        // Should NOT detect normal content
        assert!(!is_system_context("Can you help me with this code?"));
        assert!(!is_system_context("The agent ran the command successfully"));
        assert!(!is_system_context("Check the AGENTS.md file")); // just filename mention
    }
}

#[expect(clippy::too_many_arguments)]
async fn cmd_export(
    db: &Database,
    runner: &AdapterRunner,
    format: &str,
    conversations_arg: &str,
    source_filter: Option<String>,
    workspace_filter: Option<String>,
    role_filter: Vec<SearchRoleArg>,
    output: Option<PathBuf>,
    session_files: bool,
    pretty: bool,
    json_output: bool,
) -> Result<()> {
    use hstry_core::db::ListConversationsOptions;
    use std::fs;

    // Find the adapter for the target format
    // For universal formats (markdown, json), use any available adapter
    let adapter_path = if format == "markdown" || format == "json" {
        // Try to use the first available adapter that supports export
        let adapters = runner.list_adapters();
        adapters
            .into_iter()
            .find_map(|name| runner.find_adapter(&name))
            .ok_or_else(|| anyhow::anyhow!("No adapters available for export"))?
    } else {
        runner
            .find_adapter(format)
            .ok_or_else(|| anyhow::anyhow!("No adapter found for format '{format}'"))?
    };

    // Load conversations from database
    // Apply fuzzy matching for workspace filter (wrap with % for SQL LIKE)
    let workspace_filter = workspace_filter.map(|value| format!("%{value}%"));
    let conversations = if conversations_arg == "all" {
        db.list_conversations(ListConversationsOptions {
            source_id: source_filter.clone(),
            workspace: workspace_filter.clone(),
            after: None,
            before: None,
            limit: None,
        })
        .await?
    } else {
        let mut convs = Vec::new();
        for id in conversations_arg.split(',') {
            let id = id.trim();
            if let Ok(uuid) = uuid::Uuid::parse_str(id)
                && let Some(conv) = db.get_conversation(uuid).await?
            {
                convs.push(conv);
            }
        }
        convs
    };

    if conversations.is_empty() {
        if json_output {
            return emit_json(JsonResponse::<()> {
                ok: true,
                result: None,
                error: Some("No conversations found".to_string()),
            });
        }
        println!("No conversations found");
        return Ok(());
    }

    // Convert to export format
    let mut export_convs = Vec::new();
    for conv in &conversations {
        let messages = db.get_messages(conv.id).await?;
        let parsed_messages: Vec<ParsedMessage> = messages
            .into_iter()
            .filter(|m| {
                // Filter by role if specified
                if role_filter.is_empty() {
                    return true;
                }
                role_filter.iter().any(|r| match r {
                    SearchRoleArg::User => m.role == MessageRole::User,
                    SearchRoleArg::Assistant => m.role == MessageRole::Assistant,
                    SearchRoleArg::System => m.role == MessageRole::System,
                    SearchRoleArg::Tool => m.role == MessageRole::Tool,
                })
            })
            .map(|m| ParsedMessage {
                role: m.role.to_string(),
                content: m.content,
                created_at: m.created_at.map(|dt| dt.timestamp_millis()),
                model: m.model,
                tokens: m.tokens,
                cost_usd: m.cost_usd,
                parts: Some(m.parts_json),
                tool_calls: None, // TODO: load from tool_calls table
                metadata: Some(m.metadata),
            })
            .collect();

        export_convs.push(ExportConversation {
            external_id: conv.external_id.clone(),
            readable_id: conv.readable_id.clone(),
            title: conv.title.clone(),
            created_at: conv.created_at.timestamp_millis(),
            updated_at: conv.updated_at.map(|dt| dt.timestamp_millis()),
            model: conv.model.clone(),
            provider: conv.provider.clone(),
            workspace: conv.workspace.clone(),
            tokens_in: conv.tokens_in,
            tokens_out: conv.tokens_out,
            cost_usd: conv.cost_usd,
            messages: parsed_messages,
            metadata: Some(conv.metadata.clone()),
            version: Some(u64::try_from(conv.version).unwrap_or(0)),
            message_count: Some(u32::try_from(conv.message_count).unwrap_or(0)),
        });
    }

    if !json_output && session_files && (format == "markdown" || format == "json") {
        let output_dir = output.unwrap_or_else(|| PathBuf::from("."));
        fs::create_dir_all(&output_dir)?;

        let mut written = 0usize;
        for (index, conv) in export_convs.iter().enumerate() {
            let single_result = runner
                .export(
                    &adapter_path,
                    vec![conv.clone()],
                    ExportOptions {
                        format: format.to_string(),
                        pretty: Some(pretty),
                        include_tools: Some(true),
                        include_attachments: Some(true),
                    },
                )
                .await?;

            if let Some(content) = &single_result.content {
                let filename = build_session_export_filename(conv, index, format);
                fs::write(output_dir.join(filename), content)?;
                written += 1;
                continue;
            }

            if let Some(files) = &single_result.files {
                for file in files {
                    let file_path = output_dir.join(&file.path);
                    if let Some(parent) = file_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&file_path, &file.content)?;
                    written += 1;
                }
            }
        }

        println!(
            "Exported {} conversations to {} files in {}",
            conversations.len(),
            written,
            output_dir.display()
        );
        return Ok(());
    }

    let opts = ExportOptions {
        format: format.to_string(),
        pretty: Some(pretty),
        include_tools: Some(true),
        include_attachments: Some(true),
    };

    let result = runner.export(&adapter_path, export_convs, opts).await?;

    if json_output {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(&result),
            error: None,
        });
    }

    // Handle output
    if let Some(content) = &result.content {
        if let Some(output_path) = output {
            fs::write(&output_path, content)?;
            println!(
                "Exported {} conversations to {}",
                conversations.len(),
                output_path.display()
            );
        } else {
            println!("{content}");
        }
    } else if let Some(files) = &result.files {
        let output_dir = output.unwrap_or_else(|| PathBuf::from("."));
        for file in files {
            let file_path = output_dir.join(&file.path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&file_path, &file.content)?;
        }
        println!(
            "Exported {} conversations to {} files in {}",
            conversations.len(),
            files.len(),
            output_dir.display()
        );
    }

    Ok(())
}

fn build_session_export_filename(conv: &ExportConversation, index: usize, format: &str) -> String {
    let ext = match format {
        "markdown" => "md",
        "json" => "json",
        _ => "txt",
    };

    let stem = conv
        .readable_id
        .as_deref()
        .or(conv.external_id.as_deref())
        .or(conv.title.as_deref())
        .map(sanitize_filename_segment)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("conversation-{:03}", index + 1));

    format!("{:03}_{stem}.{ext}", index + 1)
}

fn sanitize_filename_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_sep = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('-');
            last_was_sep = true;
        }
    }

    out.trim_matches('-').to_string()
}

/// Parse a natural-language or ISO date string into a `DateTime<Utc>`.
///
/// Supports:
/// - ISO dates: "2026-03-01", "2026-03-01T10:00:00Z"
/// - Relative: "today", "yesterday", "N days ago", "N weeks ago", "N months ago"
/// - Named: "last week", "last month"
fn parse_date_filter(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    let lower = s.trim().to_lowercase();

    // Handle relative dates that dateparser doesn't support
    let now = chrono::Utc::now();
    if lower == "today" {
        return Ok(now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    if lower == "yesterday" {
        return Ok((now - chrono::Duration::days(1))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc());
    }
    if lower == "last week" {
        return Ok((now - chrono::Duration::weeks(1))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc());
    }
    if lower == "last month" {
        return Ok((now - chrono::Duration::days(30))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc());
    }

    // "N days/weeks/months ago"
    if let Some(rest) = lower.strip_suffix(" ago") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() == 2
            && let Ok(n) = parts[0].parse::<i64>()
        {
            let duration = match parts[1].trim_end_matches('s') {
                "day" => Some(chrono::Duration::days(n)),
                "week" => Some(chrono::Duration::weeks(n)),
                "month" => Some(chrono::Duration::days(n * 30)),
                "hour" => Some(chrono::Duration::hours(n)),
                _ => None,
            };
            if let Some(d) = duration {
                return Ok(now - d);
            }
        }
    }

    // Fall back to dateparser for ISO dates and other formats
    dateparser::parse(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .map_err(|e| anyhow::anyhow!("Could not parse date '{s}': {e}"))
}

/// Run interactive fzf picker to select a conversation
#[allow(clippy::too_many_arguments)]
async fn run_fzf_picker(
    db: &Database,
    config: &Config,
    source_filter: Option<String>,
    workspace_filter: Option<String>,
    after: Option<chrono::DateTime<chrono::Utc>>,
    before: Option<chrono::DateTime<chrono::Utc>>,
    limit: i64,
    agent_override: Option<String>,
    json_output: bool,
) -> Result<()> {
    use hstry_core::db::ListConversationsOptions;
    use std::process::{Command, Stdio};

    // Determine agent if specified (for preview)
    let agent_name = agent_override
        .as_deref()
        .unwrap_or(&config.resume.default_agent);

    // Fetch conversations with filters
    let workspace_filter_like = workspace_filter.as_ref().map(|v| format!("%{v}%"));
    let conversations = db
        .list_conversation_summaries(ListConversationsOptions {
            source_id: source_filter,
            workspace: workspace_filter_like,
            after,
            before,
            limit: Some(limit),
        })
        .await?;

    if conversations.is_empty() {
        anyhow::bail!("No conversations found. Try adjusting filters.");
    }

    // Build fzf input lines
    let mut lines: Vec<String> = Vec::new();
    let mut id_map: std::collections::HashMap<String, uuid::Uuid> =
        std::collections::HashMap::new();

    for cs in &conversations {
        let title = cs
            .conversation
            .title
            .as_deref()
            .or(cs.first_user_message.as_deref())
            .unwrap_or("(untitled)");
        let source = &cs.conversation.source_id;
        let date = cs.conversation.created_at.format("%Y-%m-%d %H:%M");
        let workspace = cs.conversation.workspace.as_deref().unwrap_or("");
        let ws_short = workspace
            .strip_prefix("/Users/")
            .and_then(|s| s.split_once('/').map(|(_, rest)| format!("~/{rest}")))
            .unwrap_or_else(|| workspace.to_string());
        let id_short = cs.conversation.id.to_string()[..8].to_string();

        // Format: "[source] date  workspace  title  (id)"
        let line = format!(
            "[{}] {}  {}  {}  ({})",
            source, date, ws_short, title, id_short
        );
        id_map.insert(line.clone(), cs.conversation.id);
        lines.push(line);
    }

    // Write to temp file for fzf
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(format!("hstry_picker_{}", std::process::id()));
    std::fs::write(&temp_file, lines.join("\n"))?;

    // Open temp file for stdin redirection
    let temp_file_handle = std::fs::File::open(&temp_file)?;

    // Build fzf command with preview - read from stdin
    let fzf_output = Command::new("fzf")
        .args([
            "--height=80%",
            "--reverse",
            "--inline-info",
            "--bind=ctrl-z:ignore",
            "--preview-window=down:60%",
            r#"--preview=echo {} | grep -oP '\([0-9a-f]{8}\)' | tr -d '()' | xargs -I {} hstry show {} 2>/dev/null | head -20"#,
            "--prompt=Resume conversation> ",
        ])
        .stdin(Stdio::from(temp_file_handle))
        .stdout(Stdio::piped())
        .output()?;

    // Clean up temp file
    let _ = std::fs::remove_file(&temp_file);

    if !fzf_output.status.success() {
        // User cancelled (ESC or ctrl-c)
        return Ok(());
    }

    // Parse selected line
    let selected = String::from_utf8_lossy(&fzf_output.stdout);
    let selected = selected.trim();

    if selected.is_empty() {
        return Ok(());
    }

    // Extract ID and resume
    let selected_id = id_map
        .get(selected)
        .ok_or_else(|| anyhow::anyhow!("Could not find selected conversation"))?;

    // Now call cmd_resume with the selected ID
    // We need to re-resolve but since we have the ID, we can use resolve_conversation_by_id
    let conversation = db
        .get_conversation(*selected_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

    // Get agent config (same logic as cmd_resume)
    let agent_config = config
        .resume
        .agents
        .get(agent_name)
        .ok_or_else(|| {
            let available: Vec<_> = config.resume.agents.keys().collect();
            anyhow::anyhow!(
                "No resume configuration for agent '{agent_name}'. Available: {available:?}"
            )
        })?
        .clone();

    // Get source
    let source = db
        .get_source(&conversation.source_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Source '{}' not found", conversation.source_id))?;

    let source_adapter = &source.adapter;
    let is_same_agent = source_adapter == &agent_config.format;

    // Output what would happen
    if json_output {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(serde_json::json!({
                "conversation_id": conversation.id,
                "title": conversation.title,
                "source": conversation.source_id,
                "source_adapter": source_adapter,
                "target_agent": agent_name,
                "target_format": agent_config.format,
                "is_same_agent": is_same_agent,
                "action": if is_same_agent { "open_directly" } else { "convert" },
            })),
            error: None,
        });
    }

    println!(
        "Selected: {} ({})",
        conversation.title.as_deref().unwrap_or("(untitled)"),
        conversation.id
    );
    println!("Source: {} ({})", source_adapter, conversation.source_id);
    println!("Target agent: {} ({})", agent_name, agent_config.format);

    if is_same_agent {
        println!("Action: Open directly in {}", agent_name);
    } else {
        println!("Action: Convert to {} format", agent_config.format);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_resume(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    id: Option<String>,
    search_query: Option<String>,
    agent_override: Option<String>,
    source_filter: Option<String>,
    workspace_filter: Option<String>,
    after_str: Option<String>,
    before_str: Option<String>,
    limit: i64,
    dry_run: bool,
    pick: bool,
    json_output: bool,
) -> Result<()> {
    use hstry_core::db::ListConversationsOptions;

    let agent_name = agent_override
        .as_deref()
        .unwrap_or(&config.resume.default_agent);

    let agent_config = config
        .resume
        .agents
        .get(agent_name)
        .ok_or_else(|| {
            let available: Vec<_> = config.resume.agents.keys().collect();
            anyhow::anyhow!(
                "No resume configuration for agent '{agent_name}'. Available: {available:?}\n\
                 Add [resume.agents.{agent_name}] to your config.toml"
            )
        })?
        .clone();

    // Parse date filters
    let after = after_str.as_deref().map(parse_date_filter).transpose()?;
    let before = before_str.as_deref().map(parse_date_filter).transpose()?;

    // Interactive fzf picker mode
    if pick {
        return run_fzf_picker(
            db,
            config,
            source_filter,
            workspace_filter,
            after,
            before,
            limit,
            agent_override,
            json_output,
        )
        .await;
    }

    // Step 1: Resolve the conversation
    let conversation = if let Some(ref id_str) = id {
        // Direct ID lookup
        resolve_conversation_by_id(db, id_str).await?
    } else if let Some(ref query) = search_query {
        // Search and pick
        let workspace_filter_like = workspace_filter.as_ref().map(|v| format!("%{v}%"));
        let conversations = db
            .list_conversation_summaries(ListConversationsOptions {
                source_id: source_filter.clone(),
                workspace: workspace_filter_like.clone(),
                after,
                before,
                limit: Some(limit),
            })
            .await?;

        // Filter by search query (fuzzy match on title + first message)
        let query_lower = query.to_lowercase();
        let mut matches: Vec<_> = conversations
            .into_iter()
            .filter(|cs| {
                let title_match = cs
                    .conversation
                    .title
                    .as_ref()
                    .is_some_and(|t| t.to_lowercase().contains(&query_lower));
                let msg_match = cs
                    .first_user_message
                    .as_ref()
                    .is_some_and(|m| m.to_lowercase().contains(&query_lower));
                let workspace_match = cs
                    .conversation
                    .workspace
                    .as_ref()
                    .is_some_and(|w| w.to_lowercase().contains(&query_lower));
                title_match || msg_match || workspace_match
            })
            .collect();

        if matches.is_empty() {
            let search_opts = hstry_core::db::SearchOptions {
                source_id: source_filter.clone(),
                workspace: workspace_filter.clone(),
                limit: Some(limit),
                ..Default::default()
            };
            let hits = db.search(query, search_opts).await?;
            if hits.is_empty() {
                anyhow::bail!("No conversations found matching '{query}'");
            }
            // Use the top hit's conversation
            let top_hit = &hits[0];
            db.get_conversation(top_hit.conversation_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Conversation not found in database"))?
        } else if matches.len() == 1 {
            matches.remove(0).conversation
        } else {
            // Multiple matches: show numbered list for user to pick
            if json_output {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(
                        &matches
                            .iter()
                            .enumerate()
                            .map(|(i, cs)| {
                                serde_json::json!({
                                    "index": i + 1,
                                    "id": cs.conversation.id,
                                    "title": cs.conversation.title,
                                    "source": cs.conversation.source_id,
                                    "workspace": cs.conversation.workspace,
                                    "created_at": cs.conversation.created_at,
                                    "messages": cs.message_count,
                                })
                            })
                            .collect::<Vec<_>>(),
                    ),
                    error: None,
                });
            }

            eprintln!(
                "Found {} conversations matching '{query}':\n",
                matches.len()
            );
            for (i, cs) in matches.iter().enumerate() {
                let title = cs
                    .conversation
                    .title
                    .as_deref()
                    .or(cs.first_user_message.as_deref())
                    .unwrap_or("(untitled)");
                let truncated = if title.len() > 80 {
                    format!("{}...", &title[..77])
                } else {
                    title.to_string()
                };
                let source = &cs.conversation.source_id;
                let date = cs.conversation.created_at.format("%Y-%m-%d %H:%M");
                let workspace = cs.conversation.workspace.as_deref().unwrap_or("");
                let ws_short = workspace
                    .strip_prefix("/Users/")
                    .and_then(|s| s.split_once('/').map(|(_, rest)| format!("~/{rest}")))
                    .unwrap_or_else(|| workspace.to_string());

                eprintln!("  {i:>3}) [{source}] {date}  {ws_short}", i = i + 1);
                eprintln!("       {truncated}");
            }
            eprintln!();

            // Read user selection
            eprint!("Select (1-{}): ", matches.len());
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let choice: usize = input
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid selection"))?;
            if choice == 0 || choice > matches.len() {
                anyhow::bail!("Selection out of range");
            }
            matches.remove(choice - 1).conversation
        }
    } else {
        // No ID and no search: list recent and pick
        let workspace_filter_like = workspace_filter.as_ref().map(|v| format!("%{v}%"));
        let conversations = db
            .list_conversation_summaries(ListConversationsOptions {
                source_id: source_filter.clone(),
                workspace: workspace_filter_like,
                after,
                before,
                limit: Some(limit),
            })
            .await?;

        if conversations.is_empty() {
            anyhow::bail!("No conversations found. Try adjusting filters.");
        }

        if json_output {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(
                    &conversations
                        .iter()
                        .enumerate()
                        .map(|(i, cs)| {
                            serde_json::json!({
                                "index": i + 1,
                                "id": cs.conversation.id,
                                "title": cs.conversation.title,
                                "source": cs.conversation.source_id,
                                "workspace": cs.conversation.workspace,
                                "created_at": cs.conversation.created_at,
                                "messages": cs.message_count,
                            })
                        })
                        .collect::<Vec<_>>(),
                ),
                error: None,
            });
        }

        eprintln!("Recent conversations:\n");
        for (i, cs) in conversations.iter().enumerate() {
            let title = cs
                .conversation
                .title
                .as_deref()
                .or(cs.first_user_message.as_deref())
                .unwrap_or("(untitled)");
            let truncated = if title.len() > 80 {
                format!("{}...", &title[..77])
            } else {
                title.to_string()
            };
            let source = &cs.conversation.source_id;
            let date = cs.conversation.created_at.format("%Y-%m-%d %H:%M");
            let workspace = cs.conversation.workspace.as_deref().unwrap_or("");
            let ws_short = workspace
                .strip_prefix("/Users/")
                .and_then(|s| s.split_once('/').map(|(_, rest)| format!("~/{rest}")))
                .unwrap_or_else(|| workspace.to_string());

            eprintln!("  {i:>3}) [{source}] {date}  {ws_short}", i = i + 1);
            eprintln!("       {truncated}");
        }
        eprintln!();

        eprint!("Select (1-{}): ", conversations.len());
        std::io::stderr().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let choice: usize = input
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid selection"))?;
        if choice == 0 || choice > conversations.len() {
            anyhow::bail!("Selection out of range");
        }
        conversations[choice - 1].conversation.clone()
    };

    // Step 2: Determine source adapter
    let source = db
        .get_source(&conversation.source_id)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("Source '{}' not found in database", conversation.source_id)
        })?;
    let source_adapter = &source.adapter;

    // Step 3: Check for same-agent fast path
    let original_file = conversation
        .metadata
        .get("file")
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let is_same_agent = source_adapter == &agent_config.format;
    let original_exists = original_file.as_ref().is_some_and(|p| p.exists());

    if is_same_agent && original_exists {
        let session_path = original_file.as_ref().unwrap();
        let workspace = conversation.workspace.as_deref().unwrap_or(".");

        if dry_run {
            if json_output {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(&serde_json::json!({
                        "action": "direct_resume",
                        "agent": agent_name,
                        "session_path": session_path,
                        "workspace": workspace,
                        "conversation_id": conversation.id,
                        "title": conversation.title,
                    })),
                    error: None,
                });
            }
            println!("Would resume directly (same agent, original file exists):");
            println!("  Agent:    {agent_name}");
            println!("  Session:  {}", session_path.display());
            println!("  Workspace: {workspace}");
            let cmd = build_resume_command(&agent_config, session_path, &conversation);
            println!("  Command:  {cmd}");
            return Ok(());
        }

        if !json_output {
            eprintln!("Resuming {} session directly in {workspace}", agent_name);
        }

        let cmd = build_resume_command(&agent_config, session_path, &conversation);
        launch_agent(&cmd, workspace, json_output)?;
        return Ok(());
    }

    // Step 4: Convert and place
    let adapter_path = runner.find_adapter(&agent_config.format).ok_or_else(|| {
        anyhow::anyhow!(
            "No adapter found for format '{}'. Is it installed and enabled?",
            agent_config.format
        )
    })?;

    // Load messages and build export conversation
    let messages = db.get_messages(conversation.id).await?;
    let parsed_messages: Vec<ParsedMessage> = messages
        .into_iter()
        .map(|m| ParsedMessage {
            role: m.role.to_string(),
            content: m.content,
            created_at: m.created_at.map(|dt| dt.timestamp_millis()),
            model: m.model,
            tokens: m.tokens,
            cost_usd: m.cost_usd,
            parts: Some(m.parts_json),
            tool_calls: None,
            metadata: Some(m.metadata),
        })
        .collect();

    let export_conv = ExportConversation {
        external_id: conversation.external_id.clone(),
        readable_id: conversation.readable_id.clone(),
        title: conversation.title.clone(),
        created_at: conversation.created_at.timestamp_millis(),
        updated_at: conversation.updated_at.map(|dt| dt.timestamp_millis()),
        model: conversation.model.clone(),
        provider: conversation.provider.clone(),
        workspace: conversation.workspace.clone(),
        tokens_in: conversation.tokens_in,
        tokens_out: conversation.tokens_out,
        cost_usd: conversation.cost_usd,
        messages: parsed_messages,
        metadata: Some(conversation.metadata.clone()),
        version: Some(u64::try_from(conversation.version).unwrap_or(0)),
        message_count: Some(u32::try_from(conversation.message_count).unwrap_or(0)),
    };

    let export_opts = ExportOptions {
        format: agent_config.format.clone(),
        pretty: Some(false),
        include_tools: Some(true),
        include_attachments: Some(true),
    };

    let result = runner
        .export(&adapter_path, vec![export_conv], export_opts)
        .await?;

    // Step 5: Place the exported file(s) in the agent's native session directory
    let session_dir = Config::expand_path(&agent_config.session_dir);
    let placed_paths = place_exported_session(&result, &session_dir, &conversation, dry_run)?;

    if placed_paths.is_empty() {
        anyhow::bail!("Export produced no files to place");
    }

    let primary_path = &placed_paths[0];
    let workspace = conversation.workspace.as_deref().unwrap_or(".");

    if dry_run {
        if json_output {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(&serde_json::json!({
                    "action": "convert_and_resume",
                    "agent": agent_name,
                    "source_adapter": source_adapter,
                    "target_format": agent_config.format,
                    "placed_files": placed_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    "workspace": workspace,
                    "conversation_id": conversation.id,
                    "title": conversation.title,
                })),
                error: None,
            });
        }
        println!("Would convert and resume:");
        println!("  Agent:    {agent_name}");
        println!("  Source:   {source_adapter}");
        println!("  Format:   {}", agent_config.format);
        for p in &placed_paths {
            println!("  Placed:   {}", p.display());
        }
        println!("  Workspace: {workspace}");
        let cmd = build_resume_command(&agent_config, primary_path, &conversation);
        println!("  Command:  {cmd}");
        return Ok(());
    }

    if !json_output {
        eprintln!(
            "Converted {source_adapter} -> {} ({} file(s))",
            agent_config.format,
            placed_paths.len()
        );
    }

    let cmd = build_resume_command(&agent_config, primary_path, &conversation);
    launch_agent(&cmd, workspace, json_output)?;
    Ok(())
}

/// Resolve a conversation by UUID, partial UUID, or external_id.
async fn resolve_conversation_by_id(db: &Database, id_str: &str) -> Result<Conversation> {
    // Try full UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(id_str)
        && let Some(conv) = db.get_conversation(uuid).await?
    {
        return Ok(conv);
    }

    // Try partial UUID match or external_id match
    let all = db
        .list_conversations(hstry_core::db::ListConversationsOptions {
            limit: Some(500),
            ..Default::default()
        })
        .await?;

    let matches: Vec<_> = all
        .into_iter()
        .filter(|c| {
            let id_match = c.id.to_string().starts_with(id_str);
            let ext_match = c
                .external_id
                .as_ref()
                .is_some_and(|e| e.starts_with(id_str) || e == id_str);
            id_match || ext_match
        })
        .collect();

    match matches.len() {
        0 => anyhow::bail!("No conversation found matching '{id_str}'"),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => anyhow::bail!(
            "Ambiguous ID '{id_str}': matched {n} conversations. Use a longer prefix."
        ),
    }
}

/// Build the launch command string by replacing placeholders.
fn build_resume_command(
    agent_config: &hstry_core::config::AgentResumeConfig,
    session_path: &Path,
    conversation: &Conversation,
) -> String {
    let id_string = conversation.id.to_string();
    let session_id = conversation.external_id.as_deref().unwrap_or(&id_string);

    agent_config
        .command
        .replace("{session_path}", &session_path.display().to_string())
        .replace("{session_id}", session_id)
        .replace(
            "{workspace}",
            conversation.workspace.as_deref().unwrap_or("."),
        )
}

/// Place exported session files into the agent's native session directory.
///
/// Adapters may include a `root` prefix in their file paths (e.g., `sessions/...`).
/// Since `session_dir` already points to the target directory, we strip the root prefix.
fn place_exported_session(
    result: &hstry_runtime::ExportResult,
    session_dir: &Path,
    conversation: &Conversation,
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    use std::fs;

    // Extract root prefix from metadata (e.g., "sessions/", "project/")
    let root_prefix = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("root"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut placed = Vec::new();

    if let Some(ref content) = result.content {
        // Single-file export (e.g., JSON, markdown)
        let id_string = conversation.id.to_string();
        let session_id = conversation.external_id.as_deref().unwrap_or(&id_string);
        let ext = match result.format.as_str() {
            "markdown" => "md",
            "json" => "json",
            _ => "jsonl",
        };
        let filename = format!("{session_id}.{ext}");
        let target = session_dir.join(&filename);

        if dry_run {
            placed.push(target);
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, content)?;
            placed.push(target);
        }
    }

    if let Some(ref files) = result.files {
        for file in files {
            // Strip the root prefix from the file path
            let relative = file.path.strip_prefix(root_prefix).unwrap_or(&file.path);
            let target = session_dir.join(relative);

            if dry_run {
                placed.push(target);
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&target, &file.content)?;
                placed.push(target);
            }
        }
    }

    Ok(placed)
}

/// Launch an agent process in the given workspace directory.
fn launch_agent(command_str: &str, workspace: &str, json_output: bool) -> Result<()> {
    let parts: Vec<&str> = command_str.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("Empty resume command");
    }

    let program = parts[0];
    let args = &parts[1..];

    if json_output {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(&serde_json::json!({
                "launched": true,
                "command": command_str,
                "workspace": workspace,
            })),
            error: None,
        });
    }

    eprintln!("Launching: {command_str}");
    eprintln!("Workspace: {workspace}");

    let status = ProcessCommand::new(program)
        .args(args)
        .current_dir(workspace)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("Failed to launch '{program}': {e}"))?;

    if !status.success() {
        anyhow::bail!("Agent exited with status: {status}");
    }

    Ok(())
}

async fn cmd_index(_config: &Config, db: &Database, rebuild: bool, json: bool) -> Result<()> {
    let total = if rebuild {
        db.rebuild_search_fts().await?
    } else {
        0
    };

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(serde_json::json!({
                "indexed": total,
                "rebuild": rebuild,
                "backend": "sqlite-fts5",
            })),
            error: None,
        });
    }

    if rebuild {
        println!("Rebuilt SQLite FTS search index ({total} messages).");
    } else {
        println!("Search uses SQLite FTS5 and stays up to date via triggers.");
    }

    Ok(())
}

async fn cmd_stats(db: &Database, json: bool) -> Result<()> {
    let sources = db.list_sources().await?;
    let conv_count = db.count_conversations().await?;
    let msg_count = db.count_messages().await?;
    let sources_count = i64::try_from(sources.len()).unwrap_or(i64::MAX);
    let per_source = db.get_source_stats().await?;
    let activity = db.get_activity_stats(30).await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(StatsSummary {
                sources: sources_count,
                conversations: conv_count,
                messages: msg_count,
                per_source,
                activity,
            }),
            error: None,
        });
    }

    // Header
    println!("\x1b[1mDatabase Statistics\x1b[0m");
    println!();

    // Totals
    println!("\x1b[1;34mTotals\x1b[0m");
    println!("  Sources:       {sources_count}");
    println!("  Conversations: {conv_count}");
    println!("  Messages:      {msg_count}");
    println!();

    // Activity
    println!("\x1b[1;34mActivity\x1b[0m");
    println!("  Today:      {:>6} conversations", activity.today);
    println!("  This week:  {:>6} conversations", activity.week);
    println!("  This month: {:>6} conversations", activity.month);
    println!();

    // Per-source stats
    if !per_source.is_empty() {
        println!("\x1b[1;34mPer Source\x1b[0m");
        println!(
            "  {:<15} {:<12} {:>8} {:>10} {:>12}",
            "SOURCE", "ADAPTER", "CONVS", "MSGS", "LAST SYNC"
        );
        println!("  {}", "-".repeat(60));
        for stats in &per_source {
            let last_sync = stats
                .last_sync_at
                .map(pretty::relative_time_short)
                .unwrap_or_else(|| "never".to_string());
            println!(
                "  {:<15} {:<12} {:>8} {:>10} {:>12}",
                truncate_title(&stats.source_id, 15),
                truncate_title(&stats.adapter, 12),
                stats.conversations,
                stats.messages,
                last_sync
            );
        }
        println!();
    }

    // Date range
    let oldest = per_source.iter().filter_map(|s| s.oldest).min();
    let newest = per_source.iter().filter_map(|s| s.newest).max();
    if let (Some(oldest), Some(newest)) = (oldest, newest) {
        println!("\x1b[1;34mDate Range\x1b[0m");
        println!("  Oldest: {}", oldest.format("%Y-%m-%d"));
        println!("  Newest: {}", newest.format("%Y-%m-%d"));
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct DedupResult {
    duplicates_found: usize,
    conversations_removed: usize,
    messages_removed: usize,
    dry_run: bool,
}

async fn cmd_dedup(
    db: &Database,
    dry_run: bool,
    source_filter: Option<String>,
    json: bool,
) -> Result<()> {
    use std::collections::HashMap;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let opts = hstry_core::db::ListConversationsOptions {
        source_id: source_filter,
        workspace: None,
        after: None,
        before: None,
        limit: None,
    };

    let conversations = db.list_conversations(opts).await?;

    if !json {
        println!(
            "Scanning {} conversations for duplicates...",
            conversations.len()
        );
    }

    // Group conversations by a hash of their full content
    let mut groups: HashMap<u64, Vec<Conversation>> = HashMap::new();

    for conv in conversations {
        let messages = db.get_messages(conv.id).await?;

        // Hash all message content for accurate dedup
        let mut hasher = DefaultHasher::new();
        conv.source_id.hash(&mut hasher);
        for msg in &messages {
            msg.role.to_string().hash(&mut hasher);
            msg.content.hash(&mut hasher);
        }
        let hash = hasher.finish();

        groups.entry(hash).or_default().push(conv);
    }

    // Find groups with duplicates
    let mut duplicates_found = 0usize;
    let mut to_remove: Vec<uuid::Uuid> = Vec::new();

    for (_key, mut convs) in groups {
        if convs.len() > 1 {
            duplicates_found += convs.len() - 1;
            // Sort by updated_at descending, keep the most recent
            convs.sort_by(|a, b| {
                let a_time = a.updated_at.unwrap_or(a.created_at);
                let b_time = b.updated_at.unwrap_or(b.created_at);
                b_time.cmp(&a_time)
            });
            // Keep first (most recent), mark rest for removal
            for conv in convs.into_iter().skip(1) {
                to_remove.push(conv.id);
            }
        }
    }

    if !json && !to_remove.is_empty() {
        println!("Found {} duplicate conversations", duplicates_found);
    }

    // Count messages that will be removed (use lightweight count query)
    let mut messages_removed = 0usize;
    for conv_id in &to_remove {
        let count = db.count_messages_for_conversation(*conv_id).await?;
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        {
            messages_removed += count as usize;
        }
    }

    if !dry_run && !to_remove.is_empty() {
        // Batch delete all duplicates in a single transaction
        db.delete_conversations_batch(&to_remove).await?;
    }

    let result = DedupResult {
        duplicates_found,
        conversations_removed: to_remove.len(),
        messages_removed,
        dry_run,
    };

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(result),
            error: None,
        });
    }

    if to_remove.is_empty() {
        println!("No duplicates found.");
    } else if dry_run {
        println!(
            "Would remove {} conversations ({} messages)",
            result.conversations_removed, result.messages_removed
        );
        println!("Run without --dry-run to actually remove them.");
    } else {
        println!(
            "Removed {} duplicate conversations ({} messages)",
            result.conversations_removed, result.messages_removed
        );
    }

    Ok(())
}

// =============================================================================
// Reseed / Verify (trx-hjjw)
// =============================================================================

#[derive(Debug, Serialize)]
struct ReseedResult {
    source: String,
    purged_conversations: i64,
    purged_messages: i64,
    imported_conversations: usize,
    imported_messages: usize,
    deduped_messages: i64,
    indexed_messages: usize,
    bulk_mode: bool,
    dry_run: bool,
}

#[allow(clippy::too_many_arguments)]
async fn cmd_reseed(
    db: &Database,
    runner: &AdapterRunner,
    source_id: &str,
    do_dedup: bool,
    do_index: bool,
    bulk_mode: bool,
    dry_run: bool,
    drop_source: bool,
    json: bool,
) -> Result<()> {
    let Some(source) = db.get_source(source_id).await? else {
        if json {
            return emit_json(JsonResponse::<()> {
                ok: false,
                result: None,
                error: Some(format!("Source '{source_id}' not found")),
            });
        }
        anyhow::bail!("Source '{source_id}' not found");
    };

    if !json {
        println!(
            "Reseeding source '{source_id}' ({adapter})",
            adapter = source.adapter
        );
        if dry_run {
            println!("  (dry run — nothing will be modified)");
        }
    }

    if dry_run {
        let (convs, msgs) = db.count_source_data(source_id).await?;
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(ReseedResult {
                    source: source_id.to_string(),
                    purged_conversations: convs,
                    purged_messages: msgs,
                    imported_conversations: 0,
                    imported_messages: 0,
                    deduped_messages: 0,
                    indexed_messages: 0,
                    bulk_mode,
                    dry_run: true,
                }),
                error: None,
            });
        }
        println!("  Would purge {convs} conversations / {msgs} messages");
        println!("  Would re-import from {:?}", source.path);
        if do_dedup {
            println!("  Would run conversation-local dedup");
        }
        if do_index {
            println!("  Would rebuild search index for source");
        }
        return Ok(());
    }

    // Pre-purge counts inform the result.
    let (pre_convs, pre_msgs) = db.count_source_data(source_id).await?;

    // IMPORTANT: purge BEFORE begin_bulk_reseed(). begin_bulk_reseed drops
    // idx_messages_conv_idx to speed up inserts, but that also makes the
    // cascading DELETEs below table-scan. Deleting first keeps the index
    // online for the purge and only drops it for the re-import.
    let purge = db.purge_source(source_id, drop_source).await?;
    if !json {
        println!(
            "  Purged {} conversations / {} messages / {} events",
            purge.conversations, purge.messages, purge.message_events
        );
    }

    // Re-create the source if it was dropped. Otherwise, we still need a
    // fresh copy with last_sync_at=None and cursor cleared — leaving them set
    // makes the adapter filter on "only rows since last time" and skip every
    // session in the source, which is the exact bug we are trying to recover
    // from (trx-hjjw rationale).
    let mut reimport_source = source.clone();
    reimport_source.last_sync_at = None;
    if let serde_json::Value::Object(mut map) = reimport_source.config {
        map.remove("cursor");
        map.remove("file_fingerprint");
        map.remove("watermark_at_ms");
        reimport_source.config = serde_json::Value::Object(map);
    }
    db.upsert_source(&reimport_source).await?;

    if bulk_mode {
        db.begin_bulk_reseed().await?;
    }

    // Re-import via the standard sync path. We deliberately reuse sync_source
    // (rather than cmd_import) because it understands cursor / batched
    // streaming for the Pi adapter. A progress spinner shows running totals
    // so the operator has immediate feedback on a multi-minute reseed.
    use indicatif::{ProgressBar, ProgressStyle};
    let pb: Option<ProgressBar> = if json {
        None
    } else {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner()),
        );
        bar.enable_steady_tick(std::time::Duration::from_millis(120));
        bar.set_message("Importing...");
        Some(bar)
    };

    let stats = if let Some(bar) = pb.as_ref() {
        let cb_box: Box<dyn Fn(usize, usize) + Send + Sync> =
            Box::new(move |convs: usize, msgs: usize| {
                bar.set_message(format!(
                    "Importing... {convs} conversations / {msgs} messages"
                ));
            });
        let cb_ref: sync::ProgressCallback<'_> = cb_box.as_ref();
        sync::sync_source_with_progress(db, runner, &reimport_source, Some(cb_ref)).await?
    } else {
        sync::sync_source_with_progress(db, runner, &reimport_source, None).await?
    };
    if let Some(bar) = pb {
        bar.finish_with_message(format!(
            "Imported {} conversations / {} messages",
            stats.conversations, stats.messages
        ));
    }

    let mut deduped = 0i64;
    if do_dedup {
        let pb = if json {
            None
        } else {
            let bar = indicatif::ProgressBar::new_spinner();
            bar.set_style(
                indicatif::ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
            );
            bar.enable_steady_tick(std::time::Duration::from_millis(120));
            bar.set_message("Deduplicating turns...");
            Some(bar)
        };
        deduped = db
            .dedup_messages_for_source(Some(source_id), 5, false)
            .await?;
        if let Some(bar) = pb {
            bar.finish_with_message(format!("Dedup removed {deduped} duplicate turns"));
        }
    }

    if bulk_mode {
        db.end_bulk_reseed().await?;
    }

    let mut indexed = 0usize;
    if do_index {
        let pb = if json {
            None
        } else {
            let bar = indicatif::ProgressBar::new_spinner();
            bar.set_style(
                indicatif::ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner()),
            );
            bar.enable_steady_tick(std::time::Duration::from_millis(120));
            bar.set_message("Rebuilding SQLite FTS search index...");
            Some(bar)
        };
        indexed = db.rebuild_search_fts().await?;
        if let Some(bar) = pb {
            bar.finish_with_message(format!("Indexed {indexed} messages"));
        }
    }

    let _ = pre_convs;
    let _ = pre_msgs;
    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(ReseedResult {
                source: source_id.to_string(),
                purged_conversations: purge.conversations,
                purged_messages: purge.messages,
                imported_conversations: stats.conversations,
                imported_messages: stats.messages,
                deduped_messages: deduped,
                indexed_messages: indexed,
                bulk_mode,
                dry_run: false,
            }),
            error: None,
        });
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct VerifyDrift {
    source: String,
    db_conversations: i64,
    db_messages: i64,
    on_disk_conversations: usize,
    on_disk_messages: usize,
    drifted: bool,
    repaired: bool,
}

#[derive(Debug, Serialize)]
struct VerifyResult {
    sources: Vec<VerifyDrift>,
    total_drifted: usize,
    total_repaired: usize,
}

async fn cmd_verify(
    db: &Database,
    runner: &AdapterRunner,
    source_filter: Option<String>,
    repair: bool,
    json: bool,
) -> Result<()> {
    let mut targets: Vec<Source> = Vec::new();
    if let Some(id) = source_filter.clone() {
        if let Some(src) = db.get_source(&id).await? {
            targets.push(src);
        }
    } else {
        for src in db.list_sources().await? {
            targets.push(src);
        }
    }

    let mut report = VerifyResult {
        sources: Vec::new(),
        total_drifted: 0,
        total_repaired: 0,
    };

    for source in &targets {
        let Some(adapter_path) = runner.find_adapter(&source.adapter) else {
            continue;
        };
        let Some(path) = source.path.as_ref() else {
            continue;
        };

        let parsed = match runner
            .parse(
                &adapter_path,
                path,
                hstry_runtime::runner::ParseOptions {
                    since: None,
                    limit: None,
                    include_tools: true,
                    include_attachments: false,
                    cursor: None,
                    batch_size: None,
                },
            )
            .await
        {
            Ok(c) => c,
            Err(err) => {
                if !json {
                    eprintln!("  {} ({}): parse error: {err}", source.id, source.adapter);
                }
                continue;
            }
        };
        let on_disk_convs = parsed.len();
        let on_disk_msgs: usize = parsed.iter().map(|c| c.messages.len()).sum();

        let (db_convs, db_msgs) = db.count_source_data(&source.id).await?;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let drifted = on_disk_convs as i64 != db_convs || on_disk_msgs as i64 != db_msgs;

        let mut repaired = false;
        if drifted && repair {
            // Reseed the drifted source. Use the same defaults as `cmd_reseed`.
            cmd_reseed(db, runner, &source.id, true, true, true, false, false, true)
                .await
                .ok();
            repaired = true;
            report.total_repaired += 1;
        }
        if drifted {
            report.total_drifted += 1;
        }

        if !json {
            let marker = if drifted { "DRIFT" } else { "OK" };
            println!(
                "  [{marker}] {id}: db={db_convs}c/{db_msgs}m disk={on_disk_convs}c/{on_disk_msgs}m{}",
                if repaired { " (repaired)" } else { "" },
                id = source.id
            );
        }

        report.sources.push(VerifyDrift {
            source: source.id.clone(),
            db_conversations: db_convs,
            db_messages: db_msgs,
            on_disk_conversations: on_disk_convs,
            on_disk_messages: on_disk_msgs,
            drifted,
            repaired,
        });
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(report),
            error: None,
        });
    }
    Ok(())
}

// =============================================================================
// Config Commands
// =============================================================================

fn cmd_config(
    config: &Config,
    config_path: &Path,
    command: Option<ConfigCommand>,
    json: bool,
) -> Result<()> {
    match command.unwrap_or(ConfigCommand::Show) {
        ConfigCommand::Show => {
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(config),
                    error: None,
                });
            }

            // Pretty print the config as TOML
            let toml_str = toml::to_string_pretty(config)
                .map_err(|e| anyhow::anyhow!("Failed to serialize config: {e}"))?;
            println!("{toml_str}");
        }

        ConfigCommand::Path => {
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({
                        "path": config_path,
                        "exists": config_path.exists(),
                    })),
                    error: None,
                });
            }

            let config_path_display = config_path.display();
            println!("{config_path_display}");
        }

        ConfigCommand::Edit => {
            // Ensure config file exists
            if !config_path.exists() {
                config.save_to_path(config_path)?;
            }

            // Get editor from EDITOR or VISUAL env var, fallback to common editors
            let editor = std::env::var("EDITOR")
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| {
                    // Try common editors
                    for editor in &["nano", "vim", "vi", "notepad"] {
                        if which::which(editor).is_ok() {
                            return editor.to_string();
                        }
                    }
                    "nano".to_string()
                });

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({
                        "editor": editor,
                        "path": config_path,
                    })),
                    error: None,
                });
            }

            let status = std::process::Command::new(&editor)
                .arg(config_path)
                .status()?;

            if !status.success() {
                anyhow::bail!("Editor exited with non-zero status");
            }
        }
    }

    Ok(())
}

// =============================================================================
// Remote Commands
// =============================================================================

#[derive(Debug, serde::Serialize)]
struct RemoteStatus {
    name: String,
    host: String,
    enabled: bool,
    database_path: Option<String>,
    cached: bool,
    cache_path: Option<String>,
    cache_size_bytes: Option<u64>,
    cache_modified: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct RemoteFetchSummary {
    remotes: Vec<hstry_core::remote::FetchResult>,
    total_bytes: u64,
}

#[derive(Debug, serde::Serialize)]
struct RemoteSyncSummary {
    results: Vec<hstry_core::remote::SyncResult>,
    total_conversations_added: usize,
    total_conversations_updated: usize,
    total_messages_added: usize,
}

async fn cmd_remote(
    db: &Database,
    config: &Config,
    config_path: &Path,
    command: RemoteCommand,
    json: bool,
) -> Result<()> {
    use hstry_core::config::RemoteConfig;
    use hstry_core::remote;

    match command {
        RemoteCommand::List => {
            let remotes: Vec<RemoteStatus> = config
                .remotes
                .iter()
                .map(|r| {
                    let cache_path = remote::cached_db_path(&r.name);
                    let (cached, cache_size, cache_modified) = if cache_path.exists() {
                        let meta = std::fs::metadata(&cache_path).ok();
                        let size = meta.as_ref().map(std::fs::Metadata::len);
                        let modified = meta.and_then(|m| m.modified().ok()).map(|t| {
                            chrono::DateTime::<chrono::Utc>::from(t)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        });
                        (true, size, modified)
                    } else {
                        (false, None, None)
                    };

                    RemoteStatus {
                        name: r.name.clone(),
                        host: r.host.clone(),
                        enabled: r.enabled,
                        database_path: r.database_path.clone(),
                        cached,
                        cache_path: if cached {
                            Some(cache_path.to_string_lossy().to_string())
                        } else {
                            None
                        },
                        cache_size_bytes: cache_size,
                        cache_modified,
                    }
                })
                .collect();

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(&remotes),
                    error: None,
                });
            }

            if remotes.is_empty() {
                println!("No remotes configured.");
                println!("Add one with: hstry remote add <name> <host>");
            } else {
                println!("Configured remotes:");
                for r in &remotes {
                    let status = if r.enabled { "enabled" } else { "disabled" };
                    let cache_info = if r.cached {
                        format!(
                            " (cached: {size})",
                            size = r.cache_size_bytes.map(format_bytes).unwrap_or_default()
                        )
                    } else {
                        " (not cached)".to_string()
                    };
                    println!(
                        "  {name} ({status}) - {host}{cache_info}",
                        name = r.name,
                        host = r.host
                    );
                }
            }
        }

        RemoteCommand::Add {
            name,
            host,
            database_path,
            port,
            identity_file,
        } => {
            // Check if remote with this name already exists
            if config.remotes.iter().any(|r| r.name == name) {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Remote '{name}' already exists")),
                    });
                }
                anyhow::bail!("Remote '{name}' already exists");
            }

            let remote = RemoteConfig {
                name: name.clone(),
                host: host.clone(),
                database_path,
                port,
                identity_file,
                enabled: true,
            };

            let mut config = config.clone();
            config.remotes.push(remote.clone());
            config.save_to_path(config_path)?;

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(remote),
                    error: None,
                });
            }
            println!("Added remote: {name} ({host})");
        }

        RemoteCommand::Remove { name } => {
            let mut config = config.clone();
            let original_len = config.remotes.len();
            config.remotes.retain(|r| r.name != name);

            if config.remotes.len() == original_len {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some(format!("Remote '{name}' not found")),
                    });
                }
                anyhow::bail!("Remote '{name}' not found");
            }

            config.save_to_path(config_path)?;

            // Also remove cached database
            let cache_path = remote::cached_db_path(&name);
            if cache_path.exists() {
                std::fs::remove_file(&cache_path)?;
            }

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({ "removed": name })),
                    error: None,
                });
            }
            println!("Removed remote: {name}");
        }

        RemoteCommand::Test { name } => {
            let remote_config = config
                .remotes
                .iter()
                .find(|r| r.name == name)
                .ok_or_else(|| anyhow::anyhow!("Remote '{name}' not found"))?;

            let transport = remote::SshTransport::from_config(remote_config);

            match transport.test_connection() {
                Ok(()) => {
                    if json {
                        return emit_json(JsonResponse {
                            ok: true,
                            result: Some(serde_json::json!({
                                "name": name,
                                "status": "connected"
                            })),
                            error: None,
                        });
                    }
                    println!("✓ Connection to '{name}' successful");
                }
                Err(e) => {
                    if json {
                        return emit_json(JsonResponse::<()> {
                            ok: false,
                            result: None,
                            error: Some(e.to_string()),
                        });
                    }
                    anyhow::bail!("Connection failed: {e}");
                }
            }
        }

        RemoteCommand::Fetch {
            remote: remote_name,
        } => {
            let remotes_to_fetch: Vec<_> = if let Some(ref name) = remote_name {
                config.remotes.iter().filter(|r| &r.name == name).collect()
            } else {
                config.remotes.iter().filter(|r| r.enabled).collect()
            };

            if remotes_to_fetch.is_empty() {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some("No matching remotes found".to_string()),
                    });
                }
                println!("No matching remotes found.");
                return Ok(());
            }

            let mut results = Vec::new();
            let mut total_bytes = 0u64;

            for remote_config in remotes_to_fetch {
                if !json {
                    let name = &remote_config.name;
                    println!("Fetching from {name}...");
                }

                match remote::fetch_remote(remote_config) {
                    Ok(result) => {
                        if !json {
                            let size = format_bytes(result.bytes_transferred);
                            let path = result.local_cache_path.display();
                            println!("  Fetched {size} to {path}");
                        }
                        total_bytes += result.bytes_transferred;
                        results.push(result);
                    }
                    Err(e) => {
                        if !json {
                            eprintln!("  Error: {e}");
                        }
                    }
                }
            }

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(RemoteFetchSummary {
                        remotes: results,
                        total_bytes,
                    }),
                    error: None,
                });
            }
        }

        RemoteCommand::Sync {
            remote: remote_name,
            direction,
        } => {
            let remotes_to_sync: Vec<_> = if let Some(ref name) = remote_name {
                config.remotes.iter().filter(|r| &r.name == name).collect()
            } else {
                config.remotes.iter().filter(|r| r.enabled).collect()
            };

            if remotes_to_sync.is_empty() {
                if json {
                    return emit_json(JsonResponse::<()> {
                        ok: false,
                        result: None,
                        error: Some("No matching remotes found".to_string()),
                    });
                }
                println!("No matching remotes found.");
                return Ok(());
            }

            let direction: hstry_core::remote::SyncDirection = direction.into();
            let mut results = Vec::new();
            let mut total_convs_added = 0usize;
            let mut total_convs_updated = 0usize;
            let mut total_msgs_added = 0usize;

            for remote_config in remotes_to_sync {
                if !json {
                    let name = &remote_config.name;
                    println!("Syncing with {name} ({direction})...");
                }

                let result = match direction {
                    hstry_core::remote::SyncDirection::Pull => {
                        remote::sync_from_remote(db, remote_config)
                            .await
                            .map(|(_, sync)| sync)
                    }
                    hstry_core::remote::SyncDirection::Push => {
                        remote::sync_to_remote(&config.database, remote_config).await
                    }
                    hstry_core::remote::SyncDirection::Bidirectional => {
                        // Pull first, then push
                        let pull_result = remote::sync_from_remote(db, remote_config).await;
                        match pull_result {
                            Ok((_, mut sync)) => {
                                if let Ok(push_sync) =
                                    remote::sync_to_remote(&config.database, remote_config).await
                                {
                                    sync.conversations_added += push_sync.conversations_added;
                                    sync.conversations_updated += push_sync.conversations_updated;
                                    sync.messages_added += push_sync.messages_added;
                                    sync.direction =
                                        hstry_core::remote::SyncDirection::Bidirectional;
                                }
                                Ok(sync)
                            }
                            Err(e) => Err(e),
                        }
                    }
                };

                match result {
                    Ok(sync_result) => {
                        if !json {
                            let added = sync_result.conversations_added;
                            let updated = sync_result.conversations_updated;
                            let messages = sync_result.messages_added;
                            println!(
                                "  Added {added} conversations, updated {updated}, {messages} messages"
                            );
                        }
                        total_convs_added += sync_result.conversations_added;
                        total_convs_updated += sync_result.conversations_updated;
                        total_msgs_added += sync_result.messages_added;
                        results.push(sync_result);
                    }
                    Err(e) => {
                        if !json {
                            eprintln!("  Error: {e}");
                        }
                    }
                }
            }

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(RemoteSyncSummary {
                        results,
                        total_conversations_added: total_convs_added,
                        total_conversations_updated: total_convs_updated,
                        total_messages_added: total_msgs_added,
                    }),
                    error: None,
                });
            }
        }

        RemoteCommand::Status => {
            let cache_dir = remote::remote_cache_dir();
            let mut statuses: Vec<RemoteStatus> = Vec::new();

            for remote in &config.remotes {
                let cache_path = remote::cached_db_path(&remote.name);
                let (cached, cache_size, cache_modified) = if cache_path.exists() {
                    let meta = std::fs::metadata(&cache_path).ok();
                    let size = meta.as_ref().map(std::fs::Metadata::len);
                    let modified = meta.and_then(|m| m.modified().ok()).map(|t| {
                        chrono::DateTime::<chrono::Utc>::from(t)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    });
                    (true, size, modified)
                } else {
                    (false, None, None)
                };

                statuses.push(RemoteStatus {
                    name: remote.name.clone(),
                    host: remote.host.clone(),
                    enabled: remote.enabled,
                    database_path: remote.database_path.clone(),
                    cached,
                    cache_path: if cached {
                        Some(cache_path.to_string_lossy().to_string())
                    } else {
                        None
                    },
                    cache_size_bytes: cache_size,
                    cache_modified,
                });
            }

            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({
                        "cache_directory": cache_dir,
                        "remotes": statuses,
                    })),
                    error: None,
                });
            }

            let cache_dir_display = cache_dir.display();
            println!("Remote cache directory: {cache_dir_display}");
            if statuses.is_empty() {
                println!("No remotes configured.");
            } else {
                println!();
                for s in &statuses {
                    let status = if s.enabled { "enabled" } else { "disabled" };
                    let name = &s.name;
                    let host = &s.host;
                    println!("{name} ({status}):");
                    println!("  Host: {host}");
                    if let Some(ref path) = s.database_path {
                        println!("  Remote DB: {path}");
                    }
                    if s.cached {
                        let cache_path = s.cache_path.as_deref().unwrap_or("-");
                        let cache_size = s.cache_size_bytes.map(format_bytes).unwrap_or_default();
                        println!("  Cached: {cache_path} ({cache_size})");
                        if let Some(ref modified) = s.cache_modified {
                            println!("  Last fetched: {modified}");
                        }
                    } else {
                        println!("  Cached: no");
                    }
                    println!();
                }
            }
        }
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    #[expect(clippy::cast_precision_loss)]
    let bytes_f = bytes as f64;
    #[expect(clippy::cast_precision_loss)]
    let kb_f = KB as f64;
    #[expect(clippy::cast_precision_loss)]
    let mb_f = MB as f64;
    #[expect(clippy::cast_precision_loss)]
    let gb_f = GB as f64;

    if bytes >= GB {
        format!("{:.2} GB", bytes_f / gb_f)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes_f / mb_f)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes_f / kb_f)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Debug, serde::Serialize)]
struct MmryMemory {
    content: String,
    #[serde(rename = "memory_type")]
    memory_type: String,
    category: String,
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    importance: Option<i32>,
    metadata: serde_json::Value,
}

#[derive(Debug, serde::Serialize)]
struct MmryExtractSummary {
    conversations: usize,
    messages: usize,
    memories: usize,
    store: String,
    mmry_bin: String,
}

#[derive(Debug, serde::Serialize)]
struct MmryExtractResult {
    summary: MmryExtractSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Vec<MmryMemory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mmry_stdout: Option<String>,
}

async fn cmd_mmry(db: &Database, command: MmryCommand, json: bool) -> Result<()> {
    match command {
        MmryCommand::Extract {
            store,
            mmry_bin,
            mmry_config,
            source,
            workspace,
            after,
            limit,
            role,
            memory_type,
            dry_run,
        } => {
            cmd_mmry_extract(
                db,
                &store,
                &mmry_bin,
                mmry_config.as_deref(),
                source,
                workspace,
                after.as_deref(),
                limit,
                role,
                memory_type,
                dry_run,
                json,
            )
            .await
        }
    }
}

#[expect(clippy::too_many_arguments)]
async fn cmd_mmry_extract(
    db: &Database,
    store: &str,
    mmry_bin: &str,
    mmry_config: Option<&Path>,
    source: Option<String>,
    workspace: Option<String>,
    after: Option<&str>,
    limit: Option<i64>,
    roles: Vec<MmryRoleArg>,
    memory_type: Option<MmryMemoryTypeArg>,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let sources = db.list_sources().await?;
    let source_map: HashMap<String, Source> =
        sources.into_iter().map(|s| (s.id.clone(), s)).collect();

    let after = match after {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map_err(|err| anyhow::anyhow!("Invalid --after timestamp: {err}"))?
                .with_timezone(&chrono::Utc),
        ),
        None => None,
    };

    let convs = db
        .list_conversations(hstry_core::db::ListConversationsOptions {
            source_id: source.clone(),
            workspace: workspace.clone(),
            after,
            before: None,
            limit,
        })
        .await?;

    let active_roles = if roles.is_empty() {
        vec![MmryRoleArg::User, MmryRoleArg::Assistant]
    } else {
        roles
    };

    let memory_type = memory_type.unwrap_or(MmryMemoryTypeArg::Episodic);
    let memory_type_str = match memory_type {
        MmryMemoryTypeArg::Episodic => "episodic",
        MmryMemoryTypeArg::Semantic => "semantic",
        MmryMemoryTypeArg::Procedural => "procedural",
    };

    let mut memories = Vec::new();
    let mut message_count = 0usize;

    for conv in &convs {
        let messages = db.get_messages(conv.id).await?;
        for msg in messages {
            if !role_allowed(&active_roles, &msg.role) {
                continue;
            }
            // Skip system context (AGENTS.md, etc.) - not useful as memories
            if is_system_context(&msg.content) {
                continue;
            }
            message_count += 1;
            let source = source_map.get(&conv.source_id);
            let category = conv
                .workspace
                .clone()
                .unwrap_or_else(|| "hstry".to_string());
            let mut tags = vec![
                "hstry".to_string(),
                format!("role:{role}", role = msg.role),
                format!("source:{source}", source = conv.source_id),
            ];
            if let Some(adapter) = source.map(|s| s.adapter.as_str()) {
                tags.push(format!("adapter:{adapter}"));
            }
            if let Some(workspace) = conv.workspace.as_deref() {
                tags.push(format!("workspace:{workspace}"));
            }

            let metadata = build_mmry_metadata(conv, &msg, source);

            memories.push(MmryMemory {
                content: msg.content,
                memory_type: memory_type_str.to_string(),
                category,
                tags,
                importance: None,
                metadata,
            });
        }
    }

    let summary = MmryExtractSummary {
        conversations: convs.len(),
        messages: message_count,
        memories: memories.len(),
        store: store.to_string(),
        mmry_bin: mmry_bin.to_string(),
    };

    if memories.is_empty() {
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(MmryExtractResult {
                    summary,
                    payload: if dry_run { Some(memories) } else { None },
                    mmry_stdout: None,
                }),
                error: None,
            });
        }
        println!("No messages matched the filters.");
        return Ok(());
    }

    if dry_run {
        if json {
            return emit_json(JsonResponse {
                ok: true,
                result: Some(MmryExtractResult {
                    summary,
                    payload: Some(memories),
                    mmry_stdout: None,
                }),
                error: None,
            });
        }
        let pretty = serde_json::to_string_pretty(&memories)?;
        println!("{pretty}");
        return Ok(());
    }

    let mmry_output = run_mmry_add(mmry_bin, mmry_config, store, &memories)?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(MmryExtractResult {
                summary,
                payload: None,
                mmry_stdout: mmry_output,
            }),
            error: None,
        });
    }

    println!(
        "Sent {} memories from {} conversations to mmry store '{}'.",
        summary.memories, summary.conversations, store
    );

    Ok(())
}

fn role_allowed(roles: &[MmryRoleArg], role: &MessageRole) -> bool {
    roles.iter().any(|candidate| {
        matches!(
            (candidate, role),
            (MmryRoleArg::User, MessageRole::User)
                | (MmryRoleArg::Assistant, MessageRole::Assistant)
                | (MmryRoleArg::System, MessageRole::System)
                | (MmryRoleArg::Tool, MessageRole::Tool)
                | (MmryRoleArg::Other, MessageRole::Other)
        )
    })
}

fn build_mmry_metadata(
    conv: &Conversation,
    msg: &Message,
    source: Option<&Source>,
) -> serde_json::Value {
    let mut inner = serde_json::Map::new();
    inner.insert(
        "conversation_id".to_string(),
        serde_json::Value::String(conv.id.to_string()),
    );
    inner.insert(
        "message_id".to_string(),
        serde_json::Value::String(msg.id.to_string()),
    );
    inner.insert(
        "message_index".to_string(),
        serde_json::Value::Number(serde_json::Number::from(i64::from(msg.idx))),
    );
    inner.insert(
        "role".to_string(),
        serde_json::Value::String(msg.role.to_string()),
    );
    inner.insert(
        "source_id".to_string(),
        serde_json::Value::String(conv.source_id.clone()),
    );
    if let Some(source) = source {
        inner.insert(
            "adapter".to_string(),
            serde_json::Value::String(source.adapter.clone()),
        );
        if let Some(path) = source.path.as_ref() {
            inner.insert(
                "source_path".to_string(),
                serde_json::Value::String(path.clone()),
            );
        }
    }
    if let Some(external_id) = conv.external_id.as_ref() {
        inner.insert(
            "external_id".to_string(),
            serde_json::Value::String(external_id.clone()),
        );
    }
    if let Some(title) = conv.title.as_ref() {
        inner.insert(
            "title".to_string(),
            serde_json::Value::String(title.clone()),
        );
    }
    if let Some(workspace) = conv.workspace.as_ref() {
        inner.insert(
            "workspace".to_string(),
            serde_json::Value::String(workspace.clone()),
        );
    }
    inner.insert(
        "conversation_created_at".to_string(),
        serde_json::Value::String(conv.created_at.to_rfc3339()),
    );
    if let Some(updated) = conv.updated_at.as_ref() {
        inner.insert(
            "conversation_updated_at".to_string(),
            serde_json::Value::String(updated.to_rfc3339()),
        );
    }
    if let Some(created) = msg.created_at.as_ref() {
        inner.insert(
            "message_created_at".to_string(),
            serde_json::Value::String(created.to_rfc3339()),
        );
    }
    if let Some(model) = msg.model.as_ref() {
        inner.insert(
            "message_model".to_string(),
            serde_json::Value::String(model.clone()),
        );
    }
    if let Some(tokens) = msg.tokens {
        inner.insert(
            "message_tokens".to_string(),
            serde_json::Value::Number(serde_json::Number::from(tokens)),
        );
    }
    if let Some(cost) = msg.cost_usd
        && let Some(value) = serde_json::Number::from_f64(cost)
    {
        inner.insert(
            "message_cost_usd".to_string(),
            serde_json::Value::Number(value),
        );
    }

    let mut metadata = serde_json::Map::new();
    metadata.insert("hstry".to_string(), serde_json::Value::Object(inner));
    serde_json::Value::Object(metadata)
}

fn run_mmry_add(
    mmry_bin: &str,
    mmry_config: Option<&Path>,
    store: &str,
    memories: &[MmryMemory],
) -> Result<Option<String>> {
    let mut command = ProcessCommand::new(mmry_bin);
    command.arg("add").arg("-").arg("--store").arg(store);
    if let Some(config_path) = mmry_config {
        command.arg("--config").arg(config_path);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("mmry stdin missing"))?;
        let payload = serde_json::to_vec(memories)?;
        stdin.write_all(&payload)?;
    }
    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("mmry add failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        Ok(None)
    } else {
        Ok(Some(stdout))
    }
}
