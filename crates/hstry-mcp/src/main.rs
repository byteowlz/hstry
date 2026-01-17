use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser};
use rmcp::{
    ServerHandler, ServiceExt,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool,
    transport::io::stdio,
};

use rust_core::{AppConfig, AppPaths};

fn main() {
    if let Err(err) = try_main() {
        let _ = writeln!(io::stderr(), "{err:?}");
        std::process::exit(1);
    }
}

#[tokio::main]
async fn try_main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::discover(cli.common.config)?;
    let config = AppConfig::load(&paths, false)?;

    let server = McpServer::new(config);
    let transport = stdio();

    server
        .serve(transport)
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {}", e))?;

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

#[derive(Clone)]
struct McpServer {
    config: AppConfig,
}

impl McpServer {
    fn new(config: AppConfig) -> Self {
        Self { config }
    }
}

#[tool(tool_box)]
impl McpServer {
    /// Get the current configuration profile
    #[tool(description = "Returns the current configuration profile name")]
    async fn get_profile(&self) -> String {
        self.config.profile.clone()
    }

    /// Echo a message back
    #[tool(description = "Echoes the provided message back")]
    async fn echo(&self, #[tool(param)] message: String) -> String {
        format!("Echo: {message}")
    }

    /// Get runtime configuration
    #[tool(description = "Returns the runtime configuration including parallelism and timeout")]
    async fn get_runtime_config(&self) -> String {
        serde_json::to_string_pretty(&self.config.runtime).unwrap_or_else(|_| "{}".to_string())
    }
}

#[tool(tool_box)]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "hstry-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some("MCP server for rust-workspace template".to_string()),
        }
    }
}
