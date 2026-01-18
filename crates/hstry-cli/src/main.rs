//! hstry CLI - Universal AI chat history

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};
use hstry_core::{Config, Database};
use hstry_runtime::{AdapterRunner, Runtime};

mod service;
mod sync;
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
    },

    /// Show a conversation
    Show {
        /// Conversation ID
        id: String,
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

    /// Show database statistics
    Stats,
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
    },

    /// List configured sources
    List,

    /// Remove a source
    Remove {
        /// Source ID
        id: String,
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
    },

    /// Enable an adapter for imports
    Enable {
        /// Adapter name
        name: String,
    },

    /// Disable an adapter for imports
    Disable {
        /// Adapter name
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
        Command::Sync { source } => cmd_sync(&db, &runner, &config, source).await,
        Command::Search {
            query,
            limit,
            source,
            workspace,
            mode,
        } => cmd_search(&db, &query, limit, source, workspace, mode).await,
        Command::List {
            source,
            workspace,
            limit,
        } => cmd_list(&db, source, workspace, limit).await,
        Command::Show { id } => cmd_show(&db, &id).await,
        Command::Source { command } => cmd_source(&db, &runner, command).await,
        Command::Adapters { command } => cmd_adapters(&runner, &config, &config_path, command),
        Command::Service { command } => service::cmd_service(&config_path, command).await,
        Command::Scan => cmd_scan(&runner, &config).await,
        Command::Stats => cmd_stats(&db).await,
    }
}

async fn cmd_sync(
    db: &Database,
    runner: &AdapterRunner,
    config: &Config,
    source_filter: Option<String>,
) -> Result<()> {
    let sources = db.list_sources().await?;

    if sources.is_empty() {
        println!("No sources configured. Use 'hstry source add <path>' to add a source.");
        return Ok(());
    }

    for source in sources {
        if let Some(ref filter) = source_filter {
            if &source.id != filter {
                continue;
            }
        }

        println!("Syncing {} ({})...", source.id, source.adapter);

        if !config.adapter_enabled(&source.adapter) {
            println!("  Adapter disabled in config, skipping");
            continue;
        }

        if let Err(err) = sync::sync_source(db, runner, &source).await {
            eprintln!("  Error: {}", err);
        }
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
) -> Result<()> {
    let opts = hstry_core::db::SearchOptions {
        source_id: source,
        workspace,
        limit: Some(limit),
        offset: None,
        mode: mode.into(),
    };
    let messages = db.search(query, opts).await?;

    if messages.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    for msg in messages {
        let source_path = msg
            .source_path
            .as_ref()
            .map(|path| format!(" ({path})"))
            .unwrap_or_default();
        let external = msg
            .external_id
            .as_ref()
            .map(|id| format!(" ext:{id}"))
            .unwrap_or_default();
        println!(
            "[{} #{} {} | {}{}{}] {}",
            msg.conversation_id,
            msg.message_idx,
            msg.role,
            msg.source_adapter,
            source_path,
            external,
            truncate(&msg.snippet, 120)
        );
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
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
) -> Result<()> {
    let opts = hstry_core::db::ListConversationsOptions {
        source_id: source,
        workspace,
        after: None,
        limit: Some(limit),
    };

    let conversations = db.list_conversations(opts).await?;

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

async fn cmd_show(db: &Database, id: &str) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(id)?;
    let conv = db
        .get_conversation(uuid)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Conversation not found"))?;

    println!("Title: {}", conv.title.as_deref().unwrap_or("(untitled)"));
    println!("Created: {}", conv.created_at);
    println!("Source: {}", conv.source_id);
    if let Some(ws) = &conv.workspace {
        println!("Workspace: {}", ws);
    }
    println!();

    let messages = db.get_messages(uuid).await?;
    for msg in messages {
        println!("--- {} ---", msg.role);
        println!("{}", msg.content);
        println!();
    }

    Ok(())
}

async fn cmd_source(db: &Database, runner: &AdapterRunner, command: SourceCommand) -> Result<()> {
    match command {
        SourceCommand::Add { path, adapter, id } => {
            let path_str = path.to_string_lossy().to_string();

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
            println!("Added source: {} ({})", source_id, adapter_name);
        }
        SourceCommand::List => {
            let sources = db.list_sources().await?;
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
        SourceCommand::Remove { id } => {
            db.remove_source(&id).await?;
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
) -> Result<()> {
    let mut config = config.clone();
    match command.unwrap_or(AdapterCommand::List) {
        AdapterCommand::List => {
            let adapters = runner.list_adapters();
            if adapters.is_empty() {
                println!("No adapters found.");
            } else {
                println!("Available adapters:");
                for adapter in adapters {
                    let status = if config.adapter_enabled(&adapter) {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    println!("  {} ({})", adapter, status);
                }
            }
        }
        AdapterCommand::Add { path } => {
            let expanded = Config::expand_path(&path.to_string_lossy());
            if !config.adapter_paths.contains(&expanded) {
                config.adapter_paths.push(expanded);
                config.save_to_path(config_path)?;
                println!("Added adapter path to config.");
            } else {
                println!("Adapter path already present in config.");
            }
        }
        AdapterCommand::Enable { name } => {
            upsert_adapter_config(&mut config, &name, true);
            config.save_to_path(config_path)?;
            println!("Enabled adapter: {}", name);
        }
        AdapterCommand::Disable { name } => {
            upsert_adapter_config(&mut config, &name, false);
            config.save_to_path(config_path)?;
            println!("Disabled adapter: {}", name);
        }
    }

    Ok(())
}

async fn cmd_scan(runner: &AdapterRunner, config: &Config) -> Result<()> {
    println!("Scanning for chat history sources...\n");

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

async fn cmd_stats(db: &Database) -> Result<()> {
    let sources = db.list_sources().await?;
    let conv_count = db.count_conversations().await?;
    let msg_count = db.count_messages().await?;

    println!("Database Statistics");
    println!("-------------------");
    println!("Sources:       {}", sources.len());
    println!("Conversations: {}", conv_count);
    println!("Messages:      {}", msg_count);

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    let s = s.replace('\n', " ");
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}
