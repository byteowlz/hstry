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

    let server = McpServer::new(config);
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

#[derive(Clone)]
struct McpServer {
    config: Config,
    tool_router: ToolRouter<Self>,
}

impl McpServer {
    fn new(config: Config) -> Self {
        Self {
            config,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl McpServer {
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
