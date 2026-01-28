#![allow(clippy::print_stdout, clippy::print_stderr)]
//! hstry CLI - Universal AI chat history

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::Result;
use clap::{Parser, Subcommand};
use hstry_core::config::{AdapterRepo, AdapterRepoSource};
use hstry_core::models::{Conversation, Message, MessageRole, SearchHit, Source};
use hstry_core::{Config, Database};
use hstry_runtime::{AdapterRunner, ExportConversation, ExportOptions, ParsedMessage, Runtime};
use serde::{Serialize, de::DeserializeOwned};

mod pretty;
mod service;
mod sync;

#[derive(Debug, serde::Deserialize)]
struct SyncInput {
    source: Option<String>,
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

        /// Output directory (for multi-file formats like pi, opencode)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Pretty print JSON output
        #[arg(long)]
        pretty: bool,
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

    /// Show or manage configuration
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommand>,
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
        0 => tracing::Level::INFO,
        1 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
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
                cli.json,
            )
            .await
        }
        Command::Sync { source, input } => {
            let db = Database::open(&config.database).await?;
            let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
                anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
            })?;
            let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());
            let input = read_input::<SyncInput>(input)?;
            let source = input.and_then(|v| v.source).or(source);
            cmd_sync(&db, &runner, &config, source, cli.json).await
        }
        Command::Import {
            path,
            adapter,
            source_id,
            dry_run,
        } => {
            let db = Database::open(&config.database).await?;
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
            cmd_index(&config, &db, rebuild, cli.json).await
        }
        Command::List {
            source,
            workspace,
            limit,
            input,
        } => {
            let db = Database::open(&config.database).await?;
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
            let input = read_input::<ShowInput>(input)?;
            let id = input.as_ref().map_or(id, |v| v.id.clone());
            cmd_show(&db, &id, cli.json).await
        }
        Command::Source { command } => {
            let db = Database::open(&config.database).await?;
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
        Command::Export {
            format,
            conversations,
            source,
            workspace,
            output,
            pretty,
        } => {
            let db = Database::open(&config.database).await?;
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
                output,
                pretty,
                cli.json,
            )
            .await
        }
        Command::Stats => {
            let db = Database::open(&config.database).await?;
            cmd_stats(&db, cli.json).await
        }
        Command::Dedup { dry_run, source } => {
            let db = Database::open(&config.database).await?;
            cmd_dedup(&db, dry_run, source, cli.json).await
        }
        Command::Mmry { command } => {
            let db = Database::open(&config.database).await?;
            cmd_mmry(&db, command, cli.json).await
        }
        Command::Remote { command } => {
            let db = Database::open(&config.database).await?;
            cmd_remote(&db, &config, &config_path, command, cli.json).await
        }
        Command::Config { command } => cmd_config(&config, &config_path, command, cli.json),
    }
}

/// Ensure sources from config file are in the database.
async fn ensure_config_sources(db: &Database, config: &Config) -> Result<()> {
    for source in &config.sources {
        let existing = db.get_source(&source.id).await?;
        let entry = match existing {
            Some(mut entry) => {
                entry.adapter.clone_from(&source.adapter);
                entry.path = Some(source.path.clone());
                entry
            }
            None => Source {
                id: source.id.clone(),
                adapter: source.adapter.clone(),
                path: Some(source.path.clone()),
                last_sync_at: None,
                config: serde_json::Value::Object(serde_json::Map::default()),
            },
        };
        db.upsert_source(&entry).await?;
    }
    Ok(())
}

async fn cmd_sync(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    source_filter: Option<String>,
    json: bool,
) -> Result<()> {
    // Ensure sources from config are in the database
    ensure_config_sources(db, config).await?;

    let sources = db.list_sources().await?;

    if sources.is_empty() {
        if json {
            return emit_json(JsonResponse::<SyncSummary> {
                ok: true,
                result: Some(SyncSummary {
                    sources: Vec::new(),
                    total_sources: 0,
                    total_conversations: 0,
                    total_messages: 0,
                }),
                error: None,
            });
        }
        println!("No sources configured. Use 'hstry source add <path>' to add a source.");
        return Ok(());
    }

    let mut stats = Vec::new();
    for source in sources {
        if let Some(ref filter) = source_filter
            && &source.id != filter
        {
            continue;
        }

        if !config.adapter_enabled(&source.adapter) {
            if !json {
                println!(
                    "Syncing {id} ({adapter})...",
                    id = source.id,
                    adapter = source.adapter
                );
                println!("  Adapter disabled in config, skipping");
            }
            continue;
        }

        if !json {
            println!(
                "Syncing {id} ({adapter})...",
                id = source.id,
                adapter = source.adapter
            );
        }
        match sync::sync_source(db, runner, &source).await {
            Ok(result) => {
                if !json {
                    if result.conversations > 0 {
                        println!(
                            "  Synced {count} conversations",
                            count = result.conversations
                        );
                    } else {
                        println!("  No new conversations");
                    }
                }
                stats.push(result);
            }
            Err(err) => {
                if !json {
                    eprintln!("  Error: {err}");
                }
            }
        }
    }

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
            title: conv.title,
            created_at: chrono::DateTime::from_timestamp_millis(conv.created_at)
                .unwrap_or_default()
                .with_timezone(&chrono::Utc),
            updated_at: conv.updated_at.and_then(|ts| {
                chrono::DateTime::from_timestamp_millis(ts).map(|dt| dt.with_timezone(&chrono::Utc))
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
            let Ok(idx) = i32::try_from(idx) else {
                continue;
            };
            let parts_json = msg.parts.clone().unwrap_or_else(|| serde_json::json!([]));
            let hstry_msg = hstry_core::models::Message {
                id: uuid::Uuid::new_v4(),
                conversation_id: hstry_conv.id,
                idx,
                role: hstry_core::models::MessageRole::from(msg.role.as_str()),
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

    println!("Imported {imported_convs} conversations ({imported_msgs} messages)");
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
    json: bool,
) -> Result<()> {
    // Request more results than needed if we're filtering, to ensure we get enough after filtering
    let has_filters = !include_system || !roles.is_empty() || no_tools || dedup;
    let fetch_limit = if has_filters { limit * 4 } else { limit };
    let opts = hstry_core::db::SearchOptions {
        source_id: source,
        workspace,
        limit: Some(fetch_limit),
        offset: None,
        mode: mode.into(),
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
            let index_path = config.search_index_path();
            hstry_core::search_tantivy::search_with_fallback(&db, &index_path, query, &opts).await?
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

    // Deduplicate by content hash if requested
    if dedup {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut seen = std::collections::HashSet::new();
        messages.retain(|hit| {
            let mut hasher = DefaultHasher::new();
            hit.content.hash(&mut hasher);
            let hash = hasher.finish();
            seen.insert(hash)
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

    pretty::print_search_results(&messages);
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

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum SearchRoleArg {
    User,
    Assistant,
    System,
    Tool,
}

async fn cmd_list(
    db: &Database,
    source: Option<String>,
    workspace: Option<String>,
    limit: i64,
    json: bool,
) -> Result<()> {
    let opts = hstry_core::db::ListConversationsOptions {
        source_id: source,
        workspace,
        after: None,
        limit: Some(limit),
    };

    let conversations = db.list_conversations(opts).await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(conversations),
            error: None,
        });
    }

    if conversations.is_empty() {
        println!("No conversations found.");
        return Ok(());
    }

    for conv in conversations {
        let title = conv.title.as_deref().unwrap_or("(untitled)");
        let date = conv.created_at.format("%Y-%m-%d %H:%M");
        println!("{id} | {date} | {title}", id = conv.id);
    }

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
            // TODO: Implement actual download/update logic
            // For now, just report what would be updated
            let repos_to_update: Vec<_> = config
                .adapter_repos
                .iter()
                .filter(|r| r.enabled)
                .filter(|r| repo.as_ref().is_none_or(|name| &r.name == name))
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

            if json {
                #[derive(Serialize)]
                struct UpdateResult {
                    repos: Vec<String>,
                    adapter: Option<String>,
                    force: bool,
                    message: String,
                }
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(UpdateResult {
                        repos: repos_to_update.iter().map(|r| r.name.clone()).collect(),
                        adapter,
                        force,
                        message: "Update functionality not yet implemented".to_string(),
                    }),
                    error: None,
                });
            }

            println!("Would update from repositories:");
            for r in &repos_to_update {
                let source_info = match &r.source {
                    AdapterRepoSource::Git { url, git_ref, .. } => {
                        format!("git: {url} ({git_ref})")
                    }
                    AdapterRepoSource::Archive { url, .. } => format!("archive: {url}"),
                    AdapterRepoSource::Local { path } => format!("local: {path}"),
                };
                println!("  {name} - {source_info}", name = r.name);
            }
            if let Some(adapter_name) = &adapter {
                println!("Filtering for adapter: {adapter_name}");
            }
            if force {
                println!("Force update enabled");
            }
            println!("\nNote: Update functionality not yet implemented.");
        }
        AdapterCommand::Repo { command } => {
            cmd_adapter_repo(&mut config, config_path, command, json)?;
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

async fn cmd_scan(runner: &AdapterRunner, config: &Config, json: bool) -> Result<()> {
    if !json {
        println!("Scanning for chat history sources...\n");
    }

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
                    if json {
                        hits.push(ScanHit {
                            adapter: adapter_name.clone(),
                            display_name: info.display_name.clone(),
                            path: expanded.to_string_lossy().to_string(),
                            confidence,
                        });
                    } else {
                        println!(
                            "  {} {} (confidence: {:.0}%)",
                            info.display_name,
                            expanded.display(),
                            confidence * 100.0
                        );
                    }
                }
            }
        }
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(hits),
            error: None,
        });
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
        assert!(is_system_context("# AGENTS.md\n\nGuidance for coding agents"));
        assert!(is_system_context("Some text\n<available_skills>\n</available_skills>"));
        assert!(is_system_context("# Agent Configuration\n\nSome instructions"));
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
    output: Option<PathBuf>,
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
    let conversations = if conversations_arg == "all" {
        db.list_conversations(ListConversationsOptions {
            source_id: source_filter.clone(),
            workspace: workspace_filter.clone(),
            after: None,
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
            workspace: conv.workspace.clone(),
            tokens_in: conv.tokens_in,
            tokens_out: conv.tokens_out,
            cost_usd: conv.cost_usd,
            messages: parsed_messages,
            metadata: Some(conv.metadata.clone()),
        });
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

async fn cmd_index(config: &Config, db: &Database, rebuild: bool, json: bool) -> Result<()> {
    let index_path = config.search_index_path();
    if rebuild {
        hstry_core::search_tantivy::reset_index(db, &index_path).await?;
    }

    let mut total = 0usize;
    let batch_size = config.search.index_batch_size;
    loop {
        let indexed =
            hstry_core::search_tantivy::index_new_messages(db, &index_path, batch_size).await?;
        total += indexed;
        if indexed < batch_size {
            break;
        }
    }

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(serde_json::json!({
                "indexed": total,
                "rebuild": rebuild,
                "index_path": index_path,
            })),
            error: None,
        });
    }

    if rebuild {
        println!("Rebuilt search index ({total} messages).");
    } else {
        println!("Indexed {total} messages.");
    }

    Ok(())
}

async fn cmd_stats(db: &Database, json: bool) -> Result<()> {
    let sources = db.list_sources().await?;
    let conv_count = db.count_conversations().await?;
    let msg_count = db.count_messages().await?;
    let sources_count = i64::try_from(sources.len()).unwrap_or(i64::MAX);

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(StatsSummary {
                sources: sources_count,
                conversations: conv_count,
                messages: msg_count,
            }),
            error: None,
        });
    }

    println!("Database Statistics");
    println!("-------------------");
    let sources_len = sources.len();
    println!("Sources:       {sources_len}");
    println!("Conversations: {conv_count}");
    println!("Messages:      {msg_count}");

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
        limit: None,
    };

    let conversations = db.list_conversations(opts).await?;

    if !json {
        println!("Scanning {} conversations for duplicates...", conversations.len());
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

    let mut messages_removed = 0usize;

    if !dry_run && !to_remove.is_empty() {
        for conv_id in &to_remove {
            let msg_count = db.get_messages(*conv_id).await?.len();
            messages_removed += msg_count;
            db.delete_conversation(*conv_id).await?;
        }
    } else if dry_run && !to_remove.is_empty() {
        // Count messages that would be removed
        for conv_id in &to_remove {
            let msg_count = db.get_messages(*conv_id).await?.len();
            messages_removed += msg_count;
        }
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


