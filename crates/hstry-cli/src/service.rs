//! Background service for watching and syncing sources.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use tokio_stream::wrappers::TcpListenerStream;
#[cfg(unix)]
use tokio_stream::wrappers::UnixListenerStream;
use walkdir::WalkDir;

use crate::ServiceCommand;
use crate::sync;
use hstry_core::config::ServiceTransport;
use hstry_core::models::Source;
use hstry_core::search_tantivy::SearchIndex;
use hstry_core::service::{
    ReadService, ReadServiceServer, SearchService, SearchServiceServer, WriteService,
    WriteServiceServer, conversation_from_proto, conversation_summary_to_proto,
    conversation_to_proto, hit_to_proto, message_from_proto, message_to_proto,
    search_request_to_opts,
};
use hstry_core::{Config, Database, search_tantivy};
use hstry_runtime::{AdapterRunner, Runtime};

const DETECT_THRESHOLD: f32 = 0.5;

#[derive(Clone)]
struct ServerState {
    db: Arc<Database>,
    index: Arc<SearchIndex>,
}

#[tonic::async_trait]
impl SearchService for ServerState {
    async fn search(
        &self,
        request: tonic::Request<hstry_core::service::proto::SearchRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::SearchResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();
        let opts = search_request_to_opts(&request);
        let query = request.query.clone();

        let hits = match self.index.search(&query, &opts) {
            Ok(hits) => hits,
            Err(_) => self
                .db
                .search(&query, opts)
                .await
                .map_err(|e| tonic::Status::internal(format!("Search failed: {e}")))?,
        };

        let response = hstry_core::service::proto::SearchResponse {
            hits: hits.iter().map(hit_to_proto).collect(),
        };
        Ok(tonic::Response::new(response))
    }
}

#[tonic::async_trait]
impl WriteService for ServerState {
    async fn write_conversation(
        &self,
        request: tonic::Request<hstry_core::service::proto::WriteConversationRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::WriteConversationResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();
        let Some(conv_proto) = request.conversation else {
            return Err(tonic::Status::invalid_argument("conversation is required"));
        };

        // Convert and upsert conversation
        let conv = conversation_from_proto(conv_proto);
        self.db
            .upsert_conversation(&conv)
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to upsert conversation: {e}")))?;

        // Get the actual conversation ID (may differ if it was an update)
        let conv_id = if let Some(ref external_id) = conv.external_id {
            self.db
                .get_conversation_id(&conv.source_id, external_id)
                .await
                .map_err(|e| {
                    tonic::Status::internal(format!("Failed to get conversation ID: {e}"))
                })?
                .unwrap_or(conv.id)
        } else {
            conv.id
        };

        // Insert messages
        let mut messages_written = 0i32;
        for msg_proto in request.messages {
            let msg = message_from_proto(msg_proto, conv_id);
            self.db
                .insert_message(&msg)
                .await
                .map_err(|e| tonic::Status::internal(format!("Failed to insert message: {e}")))?;
            messages_written += 1;
        }

        Ok(tonic::Response::new(
            hstry_core::service::proto::WriteConversationResponse {
                conversation_id: conv_id.to_string(),
                messages_written,
            },
        ))
    }

    async fn append_messages(
        &self,
        request: tonic::Request<hstry_core::service::proto::AppendMessagesRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::AppendMessagesResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();

        // Find existing conversation by source_id + external_id
        let conv_id = self
            .db
            .get_conversation_id(&request.source_id, &request.external_id)
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to find conversation: {e}")))?
            .ok_or_else(|| tonic::Status::not_found("Conversation not found"))?;

        // Insert messages
        let mut messages_written = 0i32;
        for msg_proto in request.messages {
            let msg = message_from_proto(msg_proto, conv_id);
            self.db
                .insert_message(&msg)
                .await
                .map_err(|e| tonic::Status::internal(format!("Failed to insert message: {e}")))?;
            messages_written += 1;
        }

        // Update conversation updated_at if provided
        if let Some(updated_at_ms) = request.updated_at_ms {
            if let Some(updated_at) = chrono::DateTime::from_timestamp_millis(updated_at_ms) {
                let _ = self
                    .db
                    .update_conversation_updated_at(conv_id, updated_at.with_timezone(&chrono::Utc))
                    .await;
            }
        }

        Ok(tonic::Response::new(
            hstry_core::service::proto::AppendMessagesResponse {
                conversation_id: conv_id.to_string(),
                messages_written,
            },
        ))
    }

    async fn upload_attachment(
        &self,
        request: tonic::Request<hstry_core::service::proto::UploadAttachmentRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::UploadAttachmentResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();

        let attachment_id = uuid::Uuid::new_v4().to_string();
        let message_id = uuid::Uuid::parse_str(&request.message_id)
            .map_err(|_| tonic::Status::invalid_argument("Invalid message_id"))?;

        self.db
            .insert_attachment(
                &attachment_id,
                message_id,
                &request.mime_type,
                request.filename.as_deref(),
                &request.data,
            )
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to insert attachment: {e}")))?;

        Ok(tonic::Response::new(
            hstry_core::service::proto::UploadAttachmentResponse { attachment_id },
        ))
    }
}

#[tonic::async_trait]
impl ReadService for ServerState {
    async fn get_conversation(
        &self,
        request: tonic::Request<hstry_core::service::proto::GetConversationRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::GetConversationResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();

        let conv = self
            .db
            .get_conversation_by_reference(
                if request.source_id.is_empty() {
                    None
                } else {
                    Some(request.source_id.as_str())
                },
                if request.external_id.is_empty() {
                    None
                } else {
                    Some(request.external_id.as_str())
                },
                if request.readable_id.is_empty() {
                    None
                } else {
                    Some(request.readable_id.as_str())
                },
                if request.conversation_id.is_empty() {
                    None
                } else {
                    Some(request.conversation_id.as_str())
                },
                if request.workspace.is_empty() {
                    None
                } else {
                    Some(request.workspace.as_str())
                },
            )
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to load conversation: {e}")))?;

        let response = hstry_core::service::proto::GetConversationResponse {
            conversation: conv.as_ref().map(conversation_to_proto),
        };
        Ok(tonic::Response::new(response))
    }

    async fn get_messages(
        &self,
        request: tonic::Request<hstry_core::service::proto::GetMessagesRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::GetMessagesResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();

        let conv = self
            .db
            .get_conversation_by_reference(
                if request.source_id.is_empty() {
                    None
                } else {
                    Some(request.source_id.as_str())
                },
                if request.external_id.is_empty() {
                    None
                } else {
                    Some(request.external_id.as_str())
                },
                if request.readable_id.is_empty() {
                    None
                } else {
                    Some(request.readable_id.as_str())
                },
                if request.conversation_id.is_empty() {
                    None
                } else {
                    Some(request.conversation_id.as_str())
                },
                if request.workspace.is_empty() {
                    None
                } else {
                    Some(request.workspace.as_str())
                },
            )
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to resolve conversation: {e}")))?;

        let Some(conv) = conv else {
            return Ok(tonic::Response::new(
                hstry_core::service::proto::GetMessagesResponse {
                    conversation_id: String::new(),
                    messages: Vec::new(),
                },
            ));
        };

        let mut messages = self
            .db
            .get_messages(conv.id)
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to load messages: {e}")))?;

        if request.limit > 0 && messages.len() > request.limit as usize {
            let start = messages.len() - request.limit as usize;
            messages = messages.split_off(start);
        }

        let response = hstry_core::service::proto::GetMessagesResponse {
            conversation_id: conv.id.to_string(),
            messages: messages.iter().map(message_to_proto).collect(),
        };
        Ok(tonic::Response::new(response))
    }

    async fn list_conversations(
        &self,
        request: tonic::Request<hstry_core::service::proto::ListConversationsRequest>,
    ) -> std::result::Result<
        tonic::Response<hstry_core::service::proto::ListConversationsResponse>,
        tonic::Status,
    > {
        let request = request.into_inner();
        let summaries = self
            .db
            .list_conversation_summaries(hstry_core::db::ListConversationsOptions {
                source_id: if request.source_id.is_empty() {
                    None
                } else {
                    Some(request.source_id)
                },
                workspace: if request.workspace.is_empty() {
                    None
                } else {
                    Some(request.workspace)
                },
                after: None,
                limit: if request.limit > 0 {
                    Some(request.limit)
                } else {
                    None
                },
            })
            .await
            .map_err(|e| tonic::Status::internal(format!("Failed to list conversations: {e}")))?;

        let response = hstry_core::service::proto::ListConversationsResponse {
            conversations: summaries
                .iter()
                .map(|summary| {
                    conversation_summary_to_proto(
                        &summary.conversation,
                        summary.message_count,
                        summary.first_user_message.clone(),
                    )
                })
                .collect(),
        };

        Ok(tonic::Response::new(response))
    }
}

pub async fn cmd_service(config_path: &Path, command: ServiceCommand) -> Result<()> {
    match command {
        ServiceCommand::Enable => {
            let mut config = Config::ensure_at(config_path)?;
            config.service.enabled = true;
            config.save_to_path(config_path)?;
            println!("Service enabled in config.");
        }
        ServiceCommand::Disable => {
            let mut config = Config::ensure_at(config_path)?;
            config.service.enabled = false;
            config.save_to_path(config_path)?;
            stop_service()?;
            println!("Service disabled in config.");
        }
        ServiceCommand::Start => {
            start_service(config_path)?;
        }
        ServiceCommand::Run => {
            run_service(config_path).await?;
        }
        ServiceCommand::Restart => {
            stop_service()?;
            start_service(config_path)?;
        }
        ServiceCommand::Stop => {
            stop_service()?;
        }
        ServiceCommand::Status => {
            let status = get_service_status(config_path)?;
            let enabled = if status.enabled {
                "enabled"
            } else {
                "disabled"
            };
            let running = if status.running { "running" } else { "stopped" };
            if let Some(pid) = status.pid {
                println!("Service {enabled}, {running} (pid {pid}).");
            } else {
                println!("Service {enabled}, {running}.");
            }
        }
    }

    Ok(())
}

fn start_service(config_path: &Path) -> Result<()> {
    let config = Config::ensure_at(config_path)?;
    if !config.service.enabled {
        anyhow::bail!("Service is disabled in config. Run `hstry service enable` first.");
    }

    if let Some(pid) = read_pid_file().unwrap_or(None) {
        if is_process_running(pid) {
            anyhow::bail!("Service already running with pid {pid}");
        }
        let _ = std::fs::remove_file(pid_file_path());
    }

    let exe = std::env::current_exe().context("Failed to locate current executable")?;
    let log_file = open_log_file()?;
    let mut cmd = Command::new(exe);
    cmd.arg("--config")
        .arg(config_path)
        .arg("service")
        .arg("run")
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let child = cmd.spawn().context("Failed to start service process")?;
    let pid = child.id();
    write_pid_file(pid)?;
    println!("Service started (pid {pid}).");
    Ok(())
}

fn stop_service() -> Result<()> {
    let Some(pid) = read_pid_file()? else {
        println!("Service not running.");
        return Ok(());
    };

    if is_process_running(pid) {
        if let Ok(pid_i32) = i32::try_from(pid) {
            unsafe {
                libc::kill(pid_i32, libc::SIGTERM);
            }
            println!("Sent SIGTERM to service (pid {pid}).");
        } else {
            println!("Service not running.");
        }
    } else {
        println!("Service not running.");
    }

    let _ = std::fs::remove_file(pid_file_path());
    Ok(())
}

fn is_process_running(pid: u32) -> bool {
    i32::try_from(pid)
        .map(|pid_i32| unsafe { libc::kill(pid_i32, 0) == 0 })
        .unwrap_or(false)
}

pub fn get_service_status(config_path: &Path) -> Result<ServiceStatus> {
    let config = Config::ensure_at(config_path)?;
    let pid = read_pid_file()?;
    let running = pid.is_some_and(is_process_running);
    Ok(ServiceStatus {
        enabled: config.service.enabled,
        running,
        pid: if running { pid } else { None },
    })
}

#[derive(Debug, serde::Serialize)]
pub struct ServiceStatus {
    pub enabled: bool,
    pub running: bool,
    pub pid: Option<u32>,
}

fn service_state_dir() -> PathBuf {
    let state_dir = xdg_state_dir();
    state_dir.join("hstry")
}

/// Get XDG-compliant state directory.
/// Checks `$XDG_STATE_HOME` first, then falls back to `~/.local/state`.
fn xdg_state_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    // Fallback: ~/.local/state on Unix, platform default elsewhere
    if cfg!(unix) {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |h| h.join(".local").join("state"))
    } else {
        dirs::state_dir()
            .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")))
    }
}

fn pid_file_path() -> PathBuf {
    service_state_dir().join("service.pid")
}

fn log_file_path() -> PathBuf {
    service_state_dir().join("service.log")
}

fn open_log_file() -> Result<File> {
    let dir = service_state_dir();
    std::fs::create_dir_all(&dir)?;
    let path = log_file_path();
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    Ok(file)
}

fn read_pid_file() -> Result<Option<u32>> {
    let path = pid_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)?;
    let pid = contents.trim().parse::<u32>().ok();
    Ok(pid)
}

fn write_pid_file(pid: u32) -> Result<()> {
    let dir = service_state_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(pid_file_path(), pid.to_string())?;
    Ok(())
}

async fn run_service(config_path: &Path) -> Result<()> {
    let mut state = ServiceState::load(config_path).await?;
    let server_handle = if state.config.service.search_api {
        Some(
            start_search_server(
                state.config.service.transport,
                state.config.service.search_port,
                state.search_index.clone(),
                state.db.clone(),
            )
            .await?,
        )
    } else {
        None
    };

    state.sync_all().await?;

    let mut tick = interval(Duration::from_secs(state.config.service.poll_interval_secs));

    loop {
        tokio::select! {
            _ = tick.tick() => {
                state.sync_all().await?;
            }
            Some(event_path) = state.event_rx.recv() => {
                state.handle_event(event_path).await?;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Service shutting down.");
                break;
            }
        }
    }

    if let Some(handle) = server_handle {
        handle.abort();
    }

    Ok(())
}

async fn start_search_server(
    transport: ServiceTransport,
    port: Option<u16>,
    search_index: Arc<SearchIndex>,
    db: Arc<Database>,
) -> Result<tokio::task::JoinHandle<()>> {
    let server = ServerState {
        db,
        index: search_index,
    };

    match transport {
        ServiceTransport::Tcp => start_tcp_server(port, server).await,
        #[cfg(unix)]
        ServiceTransport::Unix => start_unix_server(server).await,
        #[cfg(not(unix))]
        ServiceTransport::Unix => {
            anyhow::bail!("Unix domain sockets are not supported on this platform")
        }
    }
}

async fn start_tcp_server(
    port: Option<u16>,
    server: ServerState,
) -> Result<tokio::task::JoinHandle<()>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;

    let port_path = hstry_core::paths::service_port_path();
    if let Some(parent) = port_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&port_path, local_addr.port().to_string())?;

    println!("Service listening on {local_addr} (TCP)");

    let handle = tokio::spawn(async move {
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(SearchServiceServer::new(server.clone()))
            .add_service(WriteServiceServer::new(server.clone()))
            .add_service(ReadServiceServer::new(server))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
        {
            eprintln!("Service error: {err}");
        }
    });

    Ok(handle)
}

#[cfg(unix)]
async fn start_unix_server(server: ServerState) -> Result<tokio::task::JoinHandle<()>> {
    let socket_path = hstry_core::paths::service_socket_path();

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Remove stale socket file if it exists
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    let listener = tokio::net::UnixListener::bind(&socket_path)?;

    // Set socket permissions to user-only (0600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&socket_path, perms)?;
    }

    println!(
        "Service listening on {} (Unix socket)",
        socket_path.display()
    );

    let handle = tokio::spawn(async move {
        if let Err(err) = tonic::transport::Server::builder()
            .add_service(SearchServiceServer::new(server.clone()))
            .add_service(WriteServiceServer::new(server.clone()))
            .add_service(ReadServiceServer::new(server))
            .serve_with_incoming(UnixListenerStream::new(listener))
            .await
        {
            eprintln!("Service error: {err}");
        }
    });

    Ok(handle)
}

struct ServiceState {
    config_path: PathBuf,
    config: Config,
    config_mtime: Option<SystemTime>,
    db: Arc<Database>,
    search_index: Arc<SearchIndex>,
    runner: AdapterRunner,
    enabled_adapters: HashSet<String>,
    auto_sync_by_id: HashMap<String, bool>,
    watcher: RecommendedWatcher,
    event_rx: mpsc::Receiver<PathBuf>,
}

impl ServiceState {
    async fn load(config_path: &Path) -> Result<Self> {
        let config = Config::ensure_at(config_path)?;
        let config_mtime = config_path.metadata().and_then(|m| m.modified()).ok();

        let db = Arc::new(Database::open(&config.database).await?);
        let index_path = config.search_index_path();
        let search_index = Arc::new(SearchIndex::open(&index_path)?);
        let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
            anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
        })?;
        let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

        let enabled_adapters = enabled_adapters(&config, &runner);
        let auto_sync_by_id = auto_sync_map(&config);

        let (event_tx, event_rx) = mpsc::channel(64);
        let watcher = build_watcher(event_tx)?;

        let mut state = Self {
            config_path: config_path.to_path_buf(),
            config,
            config_mtime,
            db,
            search_index,
            runner,
            enabled_adapters,
            auto_sync_by_id,
            watcher,
            event_rx,
        };

        state.refresh_watches().await?;

        Ok(state)
    }

    async fn reload_config_if_needed(&mut self) -> Result<bool> {
        let mtime = self.config_path.metadata().and_then(|m| m.modified()).ok();
        if mtime == self.config_mtime {
            return Ok(false);
        }

        let config = Config::load_from_path(&self.config_path)?;
        let runtime = Runtime::parse(&config.js_runtime).ok_or_else(|| {
            anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
        })?;

        let db = Arc::new(Database::open(&config.database).await?);
        let index_path = config.search_index_path();
        let search_index = Arc::new(SearchIndex::open(&index_path)?);
        let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

        self.config = config;
        self.config_mtime = mtime;
        self.db = db;
        self.search_index = search_index;
        self.runner = runner;
        self.enabled_adapters = enabled_adapters(&self.config, &self.runner);
        self.auto_sync_by_id = auto_sync_map(&self.config);

        self.refresh_watches().await?;

        Ok(true)
    }

    async fn refresh_watches(&mut self) -> Result<()> {
        let sources = self.db.list_sources().await?;
        let watch_paths = collect_watch_paths(
            &self.config,
            &self.runner,
            &self.enabled_adapters,
            &sources,
            &self.config_path,
        )
        .await;

        self.watcher.unwatch(&self.config_path).ok();
        for path in &watch_paths {
            let _ = self.watcher.unwatch(path);
        }

        for path in watch_paths {
            let mode = if path.is_dir() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            self.watcher.watch(&path, mode)?;
        }

        Ok(())
    }

    async fn handle_event(&mut self, path: PathBuf) -> Result<()> {
        let _ = self.reload_config_if_needed().await?;
        self.discover_from_path(&path).await?;
        self.sync_existing_sources().await?;
        Ok(())
    }

    async fn sync_all(&mut self) -> Result<()> {
        let _ = self.reload_config_if_needed().await?;
        self.ensure_config_sources().await?;
        self.discover_default_sources().await?;
        self.discover_workspaces().await?;
        self.sync_existing_sources().await?;
        Ok(())
    }

    async fn ensure_config_sources(&self) -> Result<()> {
        for source in &self.config.sources {
            let existing = self.db.get_source(&source.id).await?;
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
            self.db.upsert_source(&entry).await?;
        }
        Ok(())
    }

    async fn discover_default_sources(&self) -> Result<()> {
        for adapter_name in self.runner.list_adapters() {
            if !self.enabled_adapters.contains(&adapter_name) {
                continue;
            }

            let Some(adapter_path) = self.runner.find_adapter(&adapter_name) else {
                continue;
            };
            let Ok(info) = self.runner.get_info(&adapter_path).await else {
                continue;
            };

            for default_path in &info.default_paths {
                let expanded = Config::expand_path(default_path);
                if !expanded.exists() {
                    continue;
                }
                self.detect_and_upsert(&adapter_name, &adapter_path, &expanded)
                    .await?;
            }
        }
        Ok(())
    }

    async fn discover_workspaces(&self) -> Result<()> {
        for root in &self.config.workspaces {
            let root = Config::expand_path(root);
            if !root.exists() {
                continue;
            }
            self.discover_workspace_root(&root).await?;
        }
        Ok(())
    }

    async fn discover_workspace_root(&self, root: &Path) -> Result<()> {
        self.discover_from_path(root).await?;

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !should_skip(entry))
        {
            let Ok(entry) = entry else {
                continue;
            };

            let path = entry.path();
            if entry.file_type().is_dir() {
                if is_candidate_dir(path) {
                    self.discover_from_path(path).await?;
                }
            } else if is_candidate_file(path) {
                self.discover_from_path(path).await?;
            }
        }

        Ok(())
    }

    async fn discover_from_path(&self, path: &Path) -> Result<()> {
        let path = path.to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        let mut best_adapter = None;
        let mut best_confidence = 0.0f32;

        for adapter_name in self.runner.list_adapters() {
            if !self.enabled_adapters.contains(&adapter_name) {
                continue;
            }
            let Some(adapter_path) = self.runner.find_adapter(&adapter_name) else {
                continue;
            };
            let confidence = self
                .runner
                .detect(&adapter_path, &path_str)
                .await
                .ok()
                .flatten()
                .unwrap_or(0.0);
            if confidence > best_confidence {
                best_confidence = confidence;
                best_adapter = Some((adapter_name, adapter_path));
            }
        }

        if let Some((adapter_name, adapter_path)) = best_adapter
            && best_confidence >= DETECT_THRESHOLD
        {
            self.detect_and_upsert(&adapter_name, &adapter_path, &path)
                .await?;
        }

        Ok(())
    }

    async fn detect_and_upsert(
        &self,
        adapter_name: &str,
        adapter_path: &Path,
        path: &Path,
    ) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let confidence = self
            .runner
            .detect(adapter_path, &path_str)
            .await?
            .unwrap_or(0.0);

        if confidence < DETECT_THRESHOLD {
            return Ok(());
        }

        // Check if this exact path already exists
        if self
            .db
            .get_source_by_adapter_path(adapter_name, &path_str)
            .await?
            .is_some()
        {
            return Ok(());
        }

        // Check if this path is a child of an existing source for the same adapter
        // This prevents creating sources for individual files when parent directory is already a source
        let sources = self.db.list_sources().await?;
        let normalized_path = std::path::Path::new(&path_str);
        for existing in &sources {
            if existing.adapter != adapter_name {
                continue;
            }
            if let Some(existing_path) = &existing.path {
                if let Ok(existing) = std::path::Path::new(existing_path).canonicalize() {
                    if let Ok(path_canon) = normalized_path.canonicalize() {
                        if path_canon.starts_with(&existing) {
                            return Ok(());
                        }
                    }
                }
            }
        }

        let uuid = uuid::Uuid::new_v4().to_string();
        let short = uuid.split('-').next().unwrap_or(uuid.as_str());
        let source_id = format!("{adapter_name}-{short}");

        let source = Source {
            id: source_id.clone(),
            adapter: adapter_name.to_string(),
            path: Some(path_str),
            last_sync_at: None,
            config: serde_json::Value::Object(serde_json::Map::default()),
        };
        self.db.upsert_source(&source).await?;
        println!("Discovered source: {source_id} ({adapter_name})");
        Ok(())
    }

    async fn sync_existing_sources(&self) -> Result<()> {
        let sources = self.db.list_sources().await?;
        for source in sources {
            if !self.enabled_adapters.contains(&source.adapter) {
                continue;
            }
            if let Some(auto_sync) = self.auto_sync_by_id.get(&source.id)
                && !auto_sync
            {
                continue;
            }

            println!(
                "Syncing {id} ({adapter})...",
                id = source.id,
                adapter = source.adapter
            );
            match sync::sync_source(&self.db, &self.runner, &source).await {
                Ok(result) => {
                    if result.conversations > 0 {
                        println!(
                            "  Synced {count} conversations",
                            count = result.conversations
                        );
                    } else {
                        println!("  No new conversations");
                    }
                }
                Err(err) => {
                    eprintln!("  Error: {err}");
                }
            }
        }
        let index_path = self.config.search_index_path();
        let batch_size = self.config.search.index_batch_size;
        let mut total_indexed = 0usize;
        loop {
            let indexed = search_tantivy::index_new_messages(&self.db, &index_path, batch_size)
                .await
                .context("Indexing new messages for search")?;
            total_indexed += indexed;
            if indexed < batch_size {
                break;
            }
        }
        if total_indexed > 0 {
            println!("Indexed {total_indexed} messages for search.");
        }
        Ok(())
    }
}

fn build_watcher(event_tx: mpsc::Sender<PathBuf>) -> Result<RecommendedWatcher> {
    let watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res
                && let Some(path) = event.paths.first()
            {
                let _ = event_tx.blocking_send(path.clone());
            }
        },
        notify::Config::default(),
    )?;
    Ok(watcher)
}

async fn collect_watch_paths(
    config: &Config,
    runner: &AdapterRunner,
    enabled_adapters: &HashSet<String>,
    sources: &[Source],
    config_path: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(config_path.to_path_buf());

    for workspace in &config.workspaces {
        let expanded = Config::expand_path(workspace);
        if expanded.exists() {
            paths.push(expanded);
        }
    }

    for adapter_name in runner.list_adapters() {
        if !enabled_adapters.contains(&adapter_name) {
            continue;
        }
        if let Some(adapter_path) = runner.find_adapter(&adapter_name)
            && let Ok(info) = runner.get_info(&adapter_path).await
        {
            for default_path in &info.default_paths {
                let expanded = Config::expand_path(default_path);
                if expanded.exists() {
                    paths.push(expanded);
                }
            }
        }
    }

    for source in sources {
        if let Some(path) = &source.path {
            let expanded = Config::expand_path(path);
            if expanded.exists() {
                paths.push(expanded);
            }
        }
    }

    paths
}

fn enabled_adapters(config: &Config, runner: &AdapterRunner) -> HashSet<String> {
    let mut enabled = HashSet::new();
    for adapter in runner.list_adapters() {
        if config.adapter_enabled(&adapter) {
            enabled.insert(adapter);
        }
    }
    enabled
}

fn auto_sync_map(config: &Config) -> HashMap<String, bool> {
    let mut map = HashMap::new();
    for source in &config.sources {
        map.insert(source.id.clone(), source.auto_sync);
    }
    map
}

fn should_skip(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    matches!(
        name.as_ref(),
        ".git" | ".hg" | ".svn" | "node_modules" | "target" | "dist" | "build" | ".cache"
    )
}

fn is_candidate_dir(path: &Path) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let name = name.to_lowercase();
    name.contains("claude")
        || name.contains("codex")
        || name.contains("gemini")
        || name.contains("opencode")
        || name.contains("aider")
        || name.contains("cursor")
        || name.contains("chatgpt")
        || name.contains("assistant")
}

fn is_candidate_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("json" | "jsonl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn with_temp_env<F: FnOnce()>(f: F) {
        let _guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let state_home = temp_dir.path().join("state");
        let config_home = temp_dir.path().join("config");
        std::fs::create_dir_all(&state_home).expect("state dir");
        std::fs::create_dir_all(&config_home).expect("config dir");

        let prev_state = std::env::var("XDG_STATE_HOME").ok();
        let prev_config = std::env::var("XDG_CONFIG_HOME").ok();

        unsafe {
            std::env::set_var("XDG_STATE_HOME", &state_home);
            std::env::set_var("XDG_CONFIG_HOME", &config_home);
        }

        f();

        if let Some(value) = prev_state {
            unsafe {
                std::env::set_var("XDG_STATE_HOME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("XDG_STATE_HOME");
            }
        }

        if let Some(value) = prev_config {
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", value);
            }
        } else {
            unsafe {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    #[test]
    fn status_reports_stopped_when_no_pid_file() {
        with_temp_env(|| {
            let config_path = Config::default_config_path();
            let mut config = Config::ensure_at(&config_path).expect("config");
            config.service.enabled = true;
            config.save_to_path(&config_path).expect("save");

            let status = get_service_status(&config_path).expect("status");
            assert!(status.enabled);
            assert!(!status.running);
            assert!(status.pid.is_none());
        });
    }

    #[test]
    fn status_reports_running_when_pid_alive() {
        with_temp_env(|| {
            let config_path = Config::default_config_path();
            let mut config = Config::ensure_at(&config_path).expect("config");
            config.service.enabled = true;
            config.save_to_path(&config_path).expect("save");

            let pid = std::process::id();
            std::fs::create_dir_all(service_state_dir()).expect("state dir");
            std::fs::write(pid_file_path(), pid.to_string()).expect("pid");

            let status = get_service_status(&config_path).expect("status");
            assert!(status.enabled);
            assert!(status.running);
            assert_eq!(status.pid, Some(pid));
        });
    }
}
