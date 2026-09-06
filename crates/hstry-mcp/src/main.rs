use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::io::stdio,
};

use hstry_core::Config;

#[cfg(test)]
mod recall_tests;

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli
        .common
        .config
        .unwrap_or_else(Config::default_config_path);
    let config = Config::ensure_at(&config_path)?;

    let db = hstry_core::Database::open(&config.database).await?;
    let server = McpServer::new(config, db);
    let transport = stdio();

    server
        .serve(transport)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))?;

    Ok(())
}

#[derive(Debug, Parser)]
#[command(author, version, about = "MCP server for rust-workspace")]
struct Cli {
    #[command(flatten)]
    common: CommonOpts,
}

#[derive(Debug, Clone, Args)]
struct CommonOpts {
    /// Override the config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct EchoRequest {
    #[schemars(description = "The message to echo back")]
    message: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchRequest {
    query: String,
    /// auto (default), needle, exact (literal), regex, natural, code, recent
    mode: Option<String>,
    source: Option<String>,
    workspace: Option<String>,
    role: Option<String>,
    model: Option<String>,
    harness: Option<String>,
    tag: Option<String>,
    after: Option<String>,
    before: Option<String>,
    remote: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    max_chars: Option<usize>,
    snippet_chars: Option<usize>,
    raw: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ExpandRequest {
    conversation_id: String,
    message_idx: Option<i32>,
    message_id: Option<String>,
    before: Option<usize>,
    after: Option<usize>,
    expand_interactions: Option<bool>,
    field: Option<String>,
    /// Offset into field records, distinct from the character offset.
    page_offset: Option<usize>,
    limit: Option<usize>,
    version: Option<i64>,
    /// Character offset within the anchored message (use match_position from search).
    offset: Option<usize>,
    max_chars: Option<usize>,
    remote: Option<String>,
}

#[derive(Clone)]
struct McpServer {
    config: Config,
    db: std::sync::Arc<hstry_core::Database>,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    fn new(config: Config, db: hstry_core::Database) -> Self {
        Self {
            config,
            db: std::sync::Arc::new(db),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl McpServer {
    #[tool(
        description = "Find relevant evidence across all message roles. Default 3000-char envelope with snippets, message anchors, attempted modes and snapshot completeness. Exact is literal; auto/needle report regex/FTS fallback. Use expand for a matching passage."
    )]
    async fn search(&self, Parameters(req): Parameters<SearchRequest>) -> String {
        let result: anyhow::Result<serde_json::Value> = async {
            use hstry_core::{
                db::{SearchMode, SearchOptions},
                recall::{Budget, project},
            };
            let mode = match req.mode.as_deref().unwrap_or("auto") {
                "auto" => SearchMode::Auto,
                "needle" => SearchMode::Needle,
                "exact" => SearchMode::Exact,
                "regex" => SearchMode::Regex,
                "natural" => SearchMode::NaturalLanguage,
                "code" => SearchMode::Code,
                "recent" => SearchMode::Recent,
                _ => anyhow::bail!("Invalid search mode"),
            };
            let opts = SearchOptions {
                mode,
                limit: req.limit,
                offset: req.offset,
                source_id: req.source,
                workspace: req.workspace,
                role: req.role,
                model: req.model,
                harness: req.harness,
                tag: req.tag,
                after: req.after.map(|s| s.parse()).transpose()?,
                before: req.before.map(|s| s.parse()).transpose()?,
            };
            let mut report = if let Some(name) = req.remote {
                let remote = self
                    .config
                    .remotes
                    .iter()
                    .find(|r| r.name == name && r.enabled)
                    .ok_or_else(|| anyhow::anyhow!("Unknown or disabled remote"))?;
                hstry_core::remote::search_remote(remote, &req.query, &opts).await?
            } else {
                self.db.search_report(&req.query, opts).await?
            };
            report.available_remotes = self
                .config
                .remotes
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.name.clone())
                .collect();
            Ok(project(
                &report,
                Budget {
                    total: req.max_chars.unwrap_or(3000),
                    snippet: req.snippet_chars.unwrap_or(300),
                },
                req.raw.unwrap_or(false),
            )?)
        }
        .await;
        match result {
            Ok(v) => v.to_string(),
            Err(e) => {
                serde_json::json!({"ok":false,"error":hstry_core::recall::clip(&e.to_string(),300)})
                    .to_string()
            }
        }
    }

    #[tool(
        description = "Read evidence with a serialized budget locally or on a named remote. Pass message_idx or message_id for anchor-first context, or omit for chronological pages. Each record has a field and next_offset_chars: continue that field with offset and field. next_offset advances field-record pages via page_offset. Pass version during pagination to detect changes. expand_interactions includes only directly linked tool calls/results."
    )]
    async fn expand(&self, Parameters(req): Parameters<ExpandRequest>) -> String {
        let result: anyhow::Result<serde_json::Value> = async {
            let options = hstry_core::read::ReadOptions {
                message_idx: req.message_idx,
                message_id: req.message_id.as_deref().map(str::parse).transpose()?,
                before: req.before.unwrap_or(0),
                after: req.after.unwrap_or(0),
                expand_interactions: req.expand_interactions.unwrap_or(false),
                offset: req.page_offset.unwrap_or(0),
                limit: req.limit.unwrap_or(50),
                max_chars: req.max_chars.unwrap_or(3000),
                field: req.field.or_else(|| req.offset.map(|_| "content".into())),
                offset_chars: req.offset.unwrap_or(0),
                version: req.version,
                machine: None,
            };
            let page = if let Some(name) = &req.remote {
                let peer = self
                    .config
                    .remotes
                    .iter()
                    .find(|r| r.name == *name && r.enabled)
                    .ok_or_else(|| anyhow::anyhow!("Unknown remote"))?;
                hstry_core::remote::read_remote(peer, &req.conversation_id, &options).await?
            } else {
                self.db
                    .read_page(req.conversation_id.parse()?, options)
                    .await?
            };
            Ok(serde_json::json!({"ok":true,"result":page}))
        }
        .await;
        match result {
            Ok(v) => v.to_string(),
            Err(e) => {
                serde_json::json!({"ok":false,"error":hstry_core::recall::clip(&e.to_string(),300)})
                    .to_string()
            }
        }
    }

    /// Get the current configuration profile
    #[tool(description = "Returns the active configuration profile name")]
    async fn get_profile(&self) -> String {
        tokio::task::yield_now().await;
        "default".to_string()
    }

    /// Echo a message back
    #[tool(description = "Echoes the provided message back")]
    async fn echo(&self, Parameters(req): Parameters<EchoRequest>) -> String {
        tokio::task::yield_now().await;
        format!("Echo: {}", req.message)
    }

    /// Get service configuration
    #[tool(description = "Returns the service configuration (enabled and poll interval)")]
    async fn get_runtime_config(&self) -> String {
        tokio::task::yield_now().await;
        serde_json::to_string_pretty(&self.config.service).unwrap_or_else(|_| "{}".to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some("MCP server for hstry - Universal AI chat history".to_string()),
            ..Default::default()
        }
    }
}
