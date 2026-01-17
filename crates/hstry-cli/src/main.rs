//! hstry CLI - Universal AI chat history

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use hstry_core::{Config, Database};
use hstry_runtime::{AdapterRunner, Runtime};

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

    /// List available adapters
    Adapters,

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
    let config = if let Some(path) = cli.config {
        Config::load_from_path(&path)?
    } else {
        Config::load()?
    };

    // Open database
    let db = Database::open(&config.database).await?;

    // Detect JS runtime
    let runtime = Runtime::from_str(&config.js_runtime).ok_or_else(|| {
        anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
    })?;

    let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

    match cli.command {
        Command::Sync { source } => cmd_sync(&db, &runner, source).await,
        Command::Search { query, limit } => cmd_search(&db, &query, limit).await,
        Command::List {
            source,
            workspace,
            limit,
        } => cmd_list(&db, source, workspace, limit).await,
        Command::Show { id } => cmd_show(&db, &id).await,
        Command::Source { command } => cmd_source(&db, &runner, command).await,
        Command::Adapters => cmd_adapters(&runner),
        Command::Scan => cmd_scan(&runner).await,
        Command::Stats => cmd_stats(&db).await,
    }
}

async fn cmd_sync(
    db: &Database,
    runner: &AdapterRunner,
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

        let adapter_path = match runner.find_adapter(&source.adapter) {
            Some(p) => p,
            None => {
                eprintln!("  Adapter '{}' not found, skipping", source.adapter);
                continue;
            }
        };

        let path = match &source.path {
            Some(p) => p.clone(),
            None => {
                eprintln!("  No path configured, skipping");
                continue;
            }
        };

        match runner.parse(&adapter_path, &path, Default::default()).await {
            Ok(conversations) => {
                let mut new_count = 0;
                for conv in conversations {
                    let hstry_conv = hstry_core::models::Conversation {
                        id: uuid::Uuid::new_v4(),
                        source_id: source.id.clone(),
                        external_id: conv.external_id,
                        title: conv.title,
                        created_at: chrono::DateTime::from_timestamp_millis(conv.created_at as i64)
                            .unwrap_or_default()
                            .with_timezone(&chrono::Utc),
                        updated_at: conv.updated_at.and_then(|ts| {
                            chrono::DateTime::from_timestamp_millis(ts as i64)
                                .map(|dt| dt.with_timezone(&chrono::Utc))
                        }),
                        model: conv.model,
                        workspace: conv.workspace,
                        tokens_in: conv.tokens_in.map(|t| t as i64),
                        tokens_out: conv.tokens_out.map(|t| t as i64),
                        cost_usd: conv.cost_usd,
                        metadata: conv
                            .metadata
                            .map(|m| serde_json::to_value(m).unwrap_or_default())
                            .unwrap_or_default(),
                    };

                    db.upsert_conversation(&hstry_conv).await?;

                    for (idx, msg) in conv.messages.iter().enumerate() {
                        let hstry_msg = hstry_core::models::Message {
                            id: uuid::Uuid::new_v4(),
                            conversation_id: hstry_conv.id,
                            idx: idx as i32,
                            role: hstry_core::models::MessageRole::from(msg.role.as_str()),
                            content: msg.content.clone(),
                            created_at: msg.created_at.and_then(|ts| {
                                chrono::DateTime::from_timestamp_millis(ts as i64)
                                    .map(|dt| dt.with_timezone(&chrono::Utc))
                            }),
                            model: msg.model.clone(),
                            tokens: msg.tokens.map(|t| t as i64),
                            cost_usd: msg.cost_usd,
                            metadata: serde_json::Value::Object(Default::default()),
                        };
                        db.insert_message(&hstry_msg).await?;
                    }

                    new_count += 1;
                }
                println!("  Synced {} conversations", new_count);
            }
            Err(e) => {
                eprintln!("  Error: {}", e);
            }
        }
    }

    Ok(())
}

async fn cmd_search(db: &Database, query: &str, limit: i64) -> Result<()> {
    let messages = db.search(query, limit).await?;

    if messages.is_empty() {
        println!("No results found.");
        return Ok(());
    }

    for msg in messages {
        println!(
            "[{}] {}: {}",
            msg.conversation_id,
            msg.role,
            truncate(&msg.content, 100)
        );
    }

    Ok(())
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
            println!("Removing source: {}", id);
            // TODO: implement remove
        }
    }
    Ok(())
}

fn cmd_adapters(runner: &AdapterRunner) -> Result<()> {
    let adapters = runner.list_adapters();
    if adapters.is_empty() {
        println!("No adapters found.");
    } else {
        println!("Available adapters:");
        for adapter in adapters {
            println!("  {}", adapter);
        }
    }
    Ok(())
}

async fn cmd_scan(runner: &AdapterRunner) -> Result<()> {
    println!("Scanning for chat history sources...\n");

    for adapter_name in runner.list_adapters() {
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
