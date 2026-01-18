//! Background service for watching and syncing sources.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use walkdir::WalkDir;

use crate::ServiceCommand;
use crate::sync;
use hstry_core::{Config, Database, models::Source};
use hstry_runtime::{AdapterRunner, Runtime};

const DETECT_THRESHOLD: f32 = 0.5;

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
            let _ = stop_service()?;
            println!("Service disabled in config.");
        }
        ServiceCommand::Start => {
            start_service(config_path)?;
        }
        ServiceCommand::Run => {
            run_service(config_path).await?;
        }
        ServiceCommand::Restart => {
            let _ = stop_service()?;
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
        } else {
            let _ = std::fs::remove_file(pid_file_path());
        }
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
    write_pid_file(child.id())?;
    println!("Service started (pid {}).", child.id());
    Ok(())
}

fn stop_service() -> Result<()> {
    let pid = match read_pid_file()? {
        Some(pid) => pid,
        None => {
            println!("Service not running.");
            return Ok(());
        }
    };

    if is_process_running(pid) {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        println!("Sent SIGTERM to service (pid {}).", pid);
    } else {
        println!("Service not running.");
    }

    let _ = std::fs::remove_file(pid_file_path());
    Ok(())
}

fn is_process_running(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn get_service_status(config_path: &Path) -> Result<ServiceStatus> {
    let config = Config::ensure_at(config_path)?;
    let pid = read_pid_file()?;
    let running = pid.map(is_process_running).unwrap_or(false);
    Ok(ServiceStatus {
        enabled: config.service.enabled,
        running,
        pid: if running { pid } else { None },
    })
}

pub struct ServiceStatus {
    pub enabled: bool,
    pub running: bool,
    pub pid: Option<u32>,
}

fn service_state_dir() -> PathBuf {
    let state_dir = dirs::state_dir()
        .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")));
    state_dir.join("hstry")
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

    Ok(())
}

struct ServiceState {
    config_path: PathBuf,
    config: Config,
    config_mtime: Option<SystemTime>,
    db: Database,
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

        let db = Database::open(&config.database).await?;
        let runtime = Runtime::from_str(&config.js_runtime).ok_or_else(|| {
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
        let runtime = Runtime::from_str(&config.js_runtime).ok_or_else(|| {
            anyhow::anyhow!("No JavaScript runtime found. Install bun, deno, or node.")
        })?;

        let db = Database::open(&config.database).await?;
        let runner = AdapterRunner::new(runtime, config.adapter_paths.clone());

        self.config = config;
        self.config_mtime = mtime;
        self.db = db;
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
        for path in watch_paths.iter() {
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
            self.db.upsert_source(&entry).await?;
        }
        Ok(())
    }

    async fn discover_default_sources(&self) -> Result<()> {
        for adapter_name in self.runner.list_adapters() {
            if !self.enabled_adapters.contains(&adapter_name) {
                continue;
            }

            let adapter_path = match self.runner.find_adapter(&adapter_name) {
                Some(path) => path,
                None => continue,
            };
            let info = match self.runner.get_info(&adapter_path).await {
                Ok(info) => info,
                Err(_) => continue,
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
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
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
            let adapter_path = match self.runner.find_adapter(&adapter_name) {
                Some(path) => path,
                None => continue,
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

        if let Some((adapter_name, adapter_path)) = best_adapter {
            if best_confidence >= DETECT_THRESHOLD {
                self.detect_and_upsert(&adapter_name, &adapter_path, &path)
                    .await?;
            }
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

        if self
            .db
            .get_source_by_adapter_path(adapter_name, &path_str)
            .await?
            .is_some()
        {
            return Ok(());
        }

        let source_id = format!(
            "{}-{}",
            adapter_name,
            uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
        );

        let source = Source {
            id: source_id.clone(),
            adapter: adapter_name.to_string(),
            path: Some(path_str),
            last_sync_at: None,
            config: serde_json::Value::Object(Default::default()),
        };
        self.db.upsert_source(&source).await?;
        println!("Discovered source: {} ({})", source_id, adapter_name);
        Ok(())
    }

    async fn sync_existing_sources(&self) -> Result<()> {
        let sources = self.db.list_sources().await?;
        for source in sources {
            if !self.enabled_adapters.contains(&source.adapter) {
                continue;
            }
            if let Some(auto_sync) = self.auto_sync_by_id.get(&source.id) {
                if !auto_sync {
                    continue;
                }
            }

            println!("Syncing {} ({})...", source.id, source.adapter);
            if let Err(err) = sync::sync_source(&self.db, &self.runner, &source).await {
                eprintln!("  Error: {}", err);
            }
        }
        Ok(())
    }
}

fn build_watcher(event_tx: mpsc::Sender<PathBuf>) -> Result<RecommendedWatcher> {
    let watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                if let Some(path) = event.paths.get(0) {
                    let _ = event_tx.blocking_send(path.clone());
                }
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
        if let Some(adapter_path) = runner.find_adapter(&adapter_name) {
            if let Ok(info) = runner.get_info(&adapter_path).await {
                for default_path in &info.default_paths {
                    let expanded = Config::expand_path(default_path);
                    if expanded.exists() {
                        paths.push(expanded);
                    }
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
    match path.extension().and_then(|s| s.to_str()) {
        Some("json") | Some("jsonl") => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_env<F: FnOnce()>(f: F) {
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
