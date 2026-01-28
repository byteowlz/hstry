//! Adapter runner - executes TypeScript adapters via JS runtime.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::process::Command as AsyncCommand;

/// JavaScript runtime to use for adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Bun,
    Deno,
    Node,
}

impl Runtime {
    /// Get the binary name for this runtime.
    pub fn binary(&self) -> &'static str {
        match self {
            Runtime::Bun => "bun",
            Runtime::Deno => "deno",
            Runtime::Node => "node",
        }
    }

    /// Get the run flag for this runtime.
    pub fn run_args(&self) -> Vec<&'static str> {
        match self {
            Runtime::Bun => vec!["run"],
            Runtime::Deno => vec!["run", "--allow-read", "--allow-env"],
            Runtime::Node => vec!["--experimental-strip-types"],
        }
    }

    /// Detect the best available runtime.
    pub fn detect() -> Option<Self> {
        // Try bun first (fastest)
        if Command::new("bun").arg("--version").output().is_ok() {
            return Some(Runtime::Bun);
        }
        // Then deno (good TS support)
        if Command::new("deno").arg("--version").output().is_ok() {
            return Some(Runtime::Deno);
        }
        // Finally node (most common)
        if Command::new("node").arg("--version").output().is_ok() {
            return Some(Runtime::Node);
        }
        None
    }

    /// Parse runtime from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bun" => Some(Runtime::Bun),
            "deno" => Some(Runtime::Deno),
            "node" => Some(Runtime::Node),
            "auto" => Self::detect(),
            _ => None,
        }
    }
}

impl std::str::FromStr for Runtime {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Runtime::parse(s).ok_or(())
    }
}

/// Adapter metadata returned from adapters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo {
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub default_paths: Vec<String>,
}

/// Request sent to adapter.
#[derive(Debug, Serialize)]
#[serde(tag = "method", content = "params")]
pub enum AdapterRequest {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "detect")]
    Detect { path: String },
    #[serde(rename = "parse")]
    Parse { path: String, opts: ParseOptions },
    #[serde(rename = "parseStream")]
    ParseStream { path: String, opts: ParseOptions },
    #[serde(rename = "export")]
    Export {
        conversations: Vec<ExportConversation>,
        opts: ExportOptions,
    },
}

/// Parse options sent to adapter.
#[derive(Debug, Default, Serialize)]
pub struct ParseOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_tools: bool,
    #[serde(default)]
    pub include_attachments: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<usize>,
}

/// Export options sent to adapter.
#[derive(Debug, Serialize)]
pub struct ExportOptions {
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pretty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tools: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_attachments: Option<bool>,
}

/// Parsed conversation from TS adapter (matches TS types)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedConversation {
    pub external_id: Option<String>,
    pub readable_id: Option<String>,
    pub title: Option<String>,
    pub created_at: i64, // Unix ms
    pub updated_at: Option<i64>,
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
    pub messages: Vec<ParsedMessage>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Parsed batch response from TS adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseStreamResult {
    pub conversations: Vec<ParsedConversation>,
    #[serde(default)]
    pub cursor: Option<serde_json::Value>,
    #[serde(default)]
    pub done: Option<bool>,
}

/// Parsed message from TS adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMessage {
    pub role: String,
    pub content: String,
    pub created_at: Option<i64>,
    pub model: Option<String>,
    pub tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub parts: Option<serde_json::Value>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ParsedToolCall>>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Parsed tool call from TS adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedToolCall {
    pub tool_name: String,
    pub input: Option<serde_json::Value>,
    pub output: Option<String>,
    pub status: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Export conversation input (matches TS types)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportConversation {
    pub external_id: Option<String>,
    pub readable_id: Option<String>,
    pub title: Option<String>,
    pub created_at: i64, // Unix ms
    pub updated_at: Option<i64>,
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost_usd: Option<f64>,
    pub messages: Vec<ParsedMessage>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Export file entry (for multi-file formats).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFile {
    pub path: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
}

/// Export result from adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<ExportFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response from adapter.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AdapterResponse {
    Info(AdapterInfo),
    Detect(Option<f32>),
    Parse(Vec<ParsedConversation>),
    ParseStream(ParseStreamResult),
    Export(ExportResult),
    Error { error: String },
}

/// Runner for TypeScript adapters.
pub struct AdapterRunner {
    runtime: Runtime,
    adapter_paths: Vec<PathBuf>,
}

impl AdapterRunner {
    /// Create a new adapter runner.
    pub fn new(runtime: Runtime, adapter_paths: Vec<PathBuf>) -> Self {
        Self {
            runtime,
            adapter_paths,
        }
    }

    /// Find an adapter by name.
    pub fn find_adapter(&self, name: &str) -> Option<PathBuf> {
        for base_path in &self.adapter_paths {
            let adapter_dir = base_path.join(name);
            let adapter_file = adapter_dir.join("adapter.ts");
            if adapter_file.exists() {
                return Some(adapter_file);
            }
        }
        None
    }

    /// List available adapters.
    pub fn list_adapters(&self) -> Vec<String> {
        let mut adapters = Vec::new();
        for base_path in &self.adapter_paths {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let adapter_file = path.join("adapter.ts");
                        if adapter_file.exists()
                            && let Some(name) = path.file_name().and_then(|n| n.to_str())
                        {
                            adapters.push(name.to_string());
                        }
                    }
                }
            }
        }
        adapters.sort();
        adapters.dedup();
        adapters
    }

    /// Call an adapter method.
    pub async fn call(
        &self,
        adapter_path: &Path,
        request: AdapterRequest,
    ) -> anyhow::Result<AdapterResponse> {
        use tokio::io::AsyncWriteExt;

        let request_json = serde_json::to_string(&request)?;

        let mut args = self
            .runtime
            .run_args()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        args.push(adapter_path.display().to_string());

        // Use stdin for large requests (> 100KB) to avoid env var size limits
        let use_stdin = request_json.len() > 100_000;

        let mut cmd = AsyncCommand::new(self.runtime.binary());
        cmd.args(&args);

        if use_stdin {
            cmd.env("HSTRY_REQUEST_STDIN", "1");
            cmd.stdin(std::process::Stdio::piped());
        } else {
            cmd.env("HSTRY_REQUEST", &request_json);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;

        if use_stdin && let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(request_json.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Adapter failed: {stderr}");
        }

        let stdout = String::from_utf8(output.stdout)?;
        let response: AdapterResponse = serde_json::from_str(&stdout)?;

        Ok(response)
    }

    /// Get adapter info.
    pub async fn get_info(&self, adapter_path: &Path) -> anyhow::Result<AdapterInfo> {
        match self.call(adapter_path, AdapterRequest::Info).await? {
            AdapterResponse::Info(info) => Ok(info),
            AdapterResponse::Error { error } => anyhow::bail!("Adapter error: {error}"),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    /// Detect if path contains data for adapter.
    pub async fn detect(&self, adapter_path: &Path, path: &str) -> anyhow::Result<Option<f32>> {
        match self
            .call(
                adapter_path,
                AdapterRequest::Detect {
                    path: path.to_string(),
                },
            )
            .await?
        {
            AdapterResponse::Detect(confidence) => Ok(confidence),
            AdapterResponse::Error { error } => anyhow::bail!("Adapter error: {error}"),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    /// Parse conversations from path.
    pub async fn parse(
        &self,
        adapter_path: &Path,
        path: &str,
        opts: ParseOptions,
    ) -> anyhow::Result<Vec<ParsedConversation>> {
        match self
            .call(
                adapter_path,
                AdapterRequest::Parse {
                    path: path.to_string(),
                    opts,
                },
            )
            .await?
        {
            AdapterResponse::Parse(conversations) => Ok(conversations),
            AdapterResponse::Error { error } => anyhow::bail!("Adapter error: {error}"),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    /// Parse conversations in batches with cursor/backpressure if supported.
    pub async fn parse_stream(
        &self,
        adapter_path: &Path,
        path: &str,
        opts: ParseOptions,
    ) -> anyhow::Result<Option<ParseStreamResult>> {
        match self
            .call(
                adapter_path,
                AdapterRequest::ParseStream {
                    path: path.to_string(),
                    opts,
                },
            )
            .await?
        {
            AdapterResponse::ParseStream(result) => Ok(Some(result)),
            AdapterResponse::Error { error }
                if error.contains("parseStream")
                    || error.contains("Unknown method")
                    || error.contains("does not support") =>
            {
                Ok(None)
            }
            AdapterResponse::Error { error } => anyhow::bail!("Adapter error: {error}"),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }

    /// Export conversations to a format.
    pub async fn export(
        &self,
        adapter_path: &Path,
        conversations: Vec<ExportConversation>,
        opts: ExportOptions,
    ) -> anyhow::Result<ExportResult> {
        match self
            .call(
                adapter_path,
                AdapterRequest::Export {
                    conversations,
                    opts,
                },
            )
            .await?
        {
            AdapterResponse::Export(result) => Ok(result),
            AdapterResponse::Error { error } => anyhow::bail!("Adapter error: {error}"),
            _ => anyhow::bail!("Unexpected response type"),
        }
    }
}
