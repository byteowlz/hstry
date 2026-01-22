//! hstry CLI - Universal AI chat history

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::Result;
use clap::{Parser, Subcommand};
use hstry_core::models::{Conversation, Message, MessageRole, Source};
use hstry_core::{Config, Database};
use hstry_runtime::{AdapterRunner, ExportConversation, ExportOptions, ParsedMessage, Runtime};
use serde::de::DeserializeOwned;

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

        /// Read JSON input from file or "-" for stdin
        #[arg(long)]
        input: Option<PathBuf>,
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

    /// Integrate with mmry
    Mmry {
        #[command(subcommand)]
        command: MmryCommand,
    },
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

    // Open database
    let db = Database::open(&config.database).await?;

    // Detect JS runtime
    let runtime = Runtime::from_str(&config.js_runtime).ok_or_else(|| {
        anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
    })?;

    let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

    match cli.command {
        Command::Sync { source, input } => {
            let input = read_input::<SyncInput>(input)?;
            let source = input.and_then(|v| v.source).or(source);
            cmd_sync(&db, &runner, &config, source, cli.json).await
        }
        Command::Search {
            query,
            limit,
            source,
            workspace,
            mode,
            input,
        } => {
            let input = read_input::<SearchInput>(input)?;
            let query = input.as_ref().map(|v| v.query.clone()).unwrap_or(query);
            let limit = input.as_ref().and_then(|v| v.limit).unwrap_or(limit);
            let source = input.as_ref().and_then(|v| v.source.clone()).or(source);
            let workspace = input
                .as_ref()
                .and_then(|v| v.workspace.clone())
                .or(workspace);
            let mode = input.as_ref().and_then(|v| v.mode).unwrap_or(mode);
            cmd_search(&db, &query, limit, source, workspace, mode, cli.json).await
        }
        Command::List {
            source,
            workspace,
            limit,
            input,
        } => {
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
            let input = read_input::<ShowInput>(input)?;
            let id = input.as_ref().map(|v| v.id.clone()).unwrap_or(id);
            cmd_show(&db, &id, cli.json).await
        }
        Command::Source { command } => cmd_source(&db, &runner, command, cli.json).await,
        Command::Adapters { command } => {
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
        Command::Scan => cmd_scan(&runner, &config, cli.json).await,
        Command::Export {
            format,
            conversations,
            source,
            workspace,
            output,
            pretty,
        } => {
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
        Command::Stats => cmd_stats(&db, cli.json).await,
        Command::Mmry { command } => cmd_mmry(&db, command, cli.json).await,
    }
}

/// Ensure sources from config file are in the database.
async fn ensure_config_sources(db: &Database, config: &Config) -> Result<()> {
    for source in &config.sources {
        let existing = db.get_source(&source.id).await?;
        let entry = match existing {
            Some(mut entry) => {
                entry.adapter = source.adapter.clone();
                entry.path = Some(source.path.clone());
                entry
            }
            None => Source {
                id: source.id.clone(),
                adapter: source.adapter.clone(),
                path: Some(source.path.clone()),
                last_sync_at: None,
                config: serde_json::Value::Object(Default::default()),
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
        if let Some(ref filter) = source_filter {
            if &source.id != filter {
                continue;
            }
        }

        if !config.adapter_enabled(&source.adapter) {
            if !json {
                println!("Syncing {} ({})...", source.id, source.adapter);
                println!("  Adapter disabled in config, skipping");
            }
            continue;
        }

        if !json {
            println!("Syncing {} ({})...", source.id, source.adapter);
        }
        match sync::sync_source(db, runner, &source).await {
            Ok(result) => {
                if !json {
                    if result.conversations > 0 {
                        println!("  Synced {} conversations", result.conversations);
                    } else {
                        println!("  No new conversations");
                    }
                }
                stats.push(result);
            }
            Err(err) => {
                if !json {
                    eprintln!("  Error: {}", err);
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

async fn cmd_search(
    db: &Database,
    query: &str,
    limit: i64,
    source: Option<String>,
    workspace: Option<String>,
    mode: SearchModeArg,
    json: bool,
) -> Result<()> {
    let opts = hstry_core::db::SearchOptions {
        source_id: source,
        workspace,
        limit: Some(limit),
        offset: None,
        mode: mode.into(),
    };
    let messages = db.search(query, opts).await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(messages),
            error: None,
        });
    }

    if messages.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    let count = messages.len();
    for msg in messages {
        // Header line with separator
        println!("{}", "-".repeat(72));

        // Score (negate BM25 so higher = better), role, adapter, and workspace
        let display_score = -msg.score;
        let workspace = msg
            .workspace
            .as_ref()
            .map(|ws| format!(" | WS: {ws}"))
            .unwrap_or_default();
        println!(
            "Score: {:.2} | {} | {}{}",
            display_score,
            msg.role,
            msg.source_adapter,
            workspace
        );

        // Title if available
        if let Some(title) = &msg.title {
            println!("Title: {}", truncate(title, 68));
        }

        // Dates: created and updated
        let created = msg.conv_created_at.format("%Y-%m-%d %H:%M");
        let updated = msg
            .conv_updated_at
            .map(|dt| format!(" | Updated: {}", dt.format("%Y-%m-%d %H:%M")))
            .unwrap_or_default();
        println!("Date: {}{}", created, updated);

        // Snippet with search highlights
        println!("Snippet: {}", truncate(&msg.snippet, 200));
    }
    // Final separator
    if count > 0 {
        println!("{}", "-".repeat(72));
    }

    Ok(())
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
        println!("{} | {} | {}", conv.id, date, title);
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

    println!("Title: {}", conv.title.as_deref().unwrap_or("(untitled)"));
    println!("Created: {}", conv.created_at);
    println!("Source: {}", conv.source_id);
    if let Some(ws) = &conv.workspace {
        println!("Workspace: {}", ws);
    }
    println!();

    for msg in messages {
        println!("--- {} ---", msg.role);
        println!("{}", msg.content);
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
                .map(|v| v.path.clone())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            let (input_adapter, input_id) = input
                .as_ref()
                .map(|v| (v.adapter.clone(), v.id.clone()))
                .unwrap_or((None, None));
            let adapter = input_adapter.or(adapter);
            let id = input_id.or(id);

            // Auto-detect adapter if not specified
            let adapter_name = if let Some(a) = adapter {
                a
            } else {
                let mut best_adapter = None;
                let mut best_confidence = 0.0f32;

                for adapter_name in runner.list_adapters() {
                    if let Some(adapter_path) = runner.find_adapter(&adapter_name) {
                        if let Ok(Some(confidence)) = runner.detect(&adapter_path, &path_str).await
                        {
                            if confidence > best_confidence {
                                best_confidence = confidence;
                                best_adapter = Some(adapter_name);
                            }
                        }
                    }
                }

                best_adapter.ok_or_else(|| {
                    anyhow::anyhow!("Could not auto-detect adapter for path: {}", path_str)
                })?
            };

            let source_id = id.unwrap_or_else(|| {
                format!(
                    "{}-{}",
                    adapter_name,
                    uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
                )
            });

            let source = hstry_core::models::Source {
                id: source_id.clone(),
                adapter: adapter_name.clone(),
                path: Some(path_str),
                last_sync_at: None,
                config: serde_json::Value::Object(Default::default()),
            };

            db.upsert_source(&source).await?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(source),
                    error: None,
                });
            }
            println!("Added source: {} ({})", source_id, adapter_name);
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
                    println!(
                        "{} | {} | {}",
                        source.id,
                        source.adapter,
                        source.path.as_deref().unwrap_or("-")
                    );
                }
            }
        }
        SourceCommand::Remove { id, input } => {
            let input = read_input::<SourceRemoveInput>(input)?;
            let id = input.as_ref().map(|v| v.id.clone()).unwrap_or(id);
            db.remove_source(&id).await?;
            if json {
                return emit_json(JsonResponse {
                    ok: true,
                    result: Some(serde_json::json!({ "id": id })),
                    error: None,
                });
            }
            println!("Removed source: {}", id);
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
                    println!("  {} ({})", adapter.name, status);
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
            let name = input.as_ref().map(|v| v.name.clone()).unwrap_or(name);
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
            println!("Enabled adapter: {}", name);
        }
        AdapterCommand::Disable { name, input } => {
            let input = read_input::<AdapterToggleInput>(input)?;
            let name = input.as_ref().map(|v| v.name.clone()).unwrap_or(name);
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
            println!("Disabled adapter: {}", name);
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
        if let Some(adapter_path) = runner.find_adapter(&adapter_name) {
            if let Ok(info) = runner.get_info(&adapter_path).await {
                for default_path in &info.default_paths {
                    let expanded = hstry_core::Config::expand_path(default_path);
                    if expanded.exists() {
                        if let Ok(Some(confidence)) = runner
                            .detect(&adapter_path, &expanded.to_string_lossy())
                            .await
                        {
                            if confidence > 0.5 {
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
    println!("{}", serde_json::to_string_pretty(&value)?);
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
}

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
            .ok_or_else(|| anyhow::anyhow!("No adapter found for format '{}'", format))?
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
            if let Ok(uuid) = uuid::Uuid::parse_str(id) {
                if let Some(conv) = db.get_conversation(uuid).await? {
                    convs.push(conv);
                }
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
                tool_calls: None, // TODO: load from tool_calls table
                metadata: Some(m.metadata),
            })
            .collect();

        export_convs.push(ExportConversation {
            external_id: conv.external_id.clone(),
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
            println!("Exported {} conversations to {}", conversations.len(), output_path.display());
        } else {
            println!("{}", content);
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

async fn cmd_stats(db: &Database, json: bool) -> Result<()> {
    let sources = db.list_sources().await?;
    let conv_count = db.count_conversations().await?;
    let msg_count = db.count_messages().await?;

    if json {
        return emit_json(JsonResponse {
            ok: true,
            result: Some(StatsSummary {
                sources: sources.len() as i64,
                conversations: conv_count,
                messages: msg_count,
            }),
            error: None,
        });
    }

    println!("Database Statistics");
    println!("-------------------");
    println!("Sources:       {}", sources.len());
    println!("Conversations: {}", conv_count);
    println!("Messages:      {}", msg_count);

    Ok(())
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
            message_count += 1;
            let source = source_map.get(&conv.source_id);
            let category = conv
                .workspace
                .clone()
                .unwrap_or_else(|| "hstry".to_string());
            let mut tags = vec![
                "hstry".to_string(),
                format!("role:{}", msg.role),
                format!("source:{}", conv.source_id),
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
        println!("{}", serde_json::to_string_pretty(&memories)?);
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
    roles.iter().any(|candidate| match (candidate, role) {
        (MmryRoleArg::User, MessageRole::User) => true,
        (MmryRoleArg::Assistant, MessageRole::Assistant) => true,
        (MmryRoleArg::System, MessageRole::System) => true,
        (MmryRoleArg::Tool, MessageRole::Tool) => true,
        (MmryRoleArg::Other, MessageRole::Other) => true,
        _ => false,
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
        serde_json::Value::Number(serde_json::Number::from(msg.idx as i64)),
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
    if let Some(cost) = msg.cost_usd {
        if let Some(value) = serde_json::Number::from_f64(cost) {
            inner.insert(
                "message_cost_usd".to_string(),
                serde_json::Value::Number(value),
            );
        }
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

fn truncate(s: &str, max_len: usize) -> String {
    let s = s.replace('\n', " ");
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}
