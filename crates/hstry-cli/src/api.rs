//! `hstry api` — manage the hstry-api HTTP ingest server.
//!
//! Mirrors `hstry service`: enable/disable toggles a config flag, and
//! start/stop run the `hstry-api` binary as a detached background process
//! (no launchd/plist, so it does not trip endpoint-security behavioral
//! protection). `run` runs it in the foreground for debugging.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use clap::Subcommand;
use hstry_core::Config;

use crate::service::{is_process_running, terminate_process, xdg_state_dir};

#[derive(Debug, Subcommand)]
pub enum ApiCommand {
    /// Enable the API in config
    Enable,

    /// Disable the API in config
    Disable,

    /// Start the background HTTP API
    Start,

    /// Run the HTTP API in the foreground
    Run,

    /// Restart the background HTTP API
    Restart,

    /// Stop the background HTTP API
    Stop,

    /// Show the API status
    Status,
}

/// Info about the running API for `status` / `--json`.
#[derive(Debug, serde::Serialize)]
pub struct ApiStatus {
    pub enabled: bool,
    pub running: bool,
    pub pid: Option<u32>,
}

pub async fn cmd_api(config_path: &Path, command: ApiCommand) -> Result<()> {
    match command {
        ApiCommand::Enable => {
            let mut config = Config::ensure_at(config_path)?;
            config.api.enabled = true;
            config.save_to_path(config_path)?;
            println!("API enabled in config.");
        }
        ApiCommand::Disable => {
            let mut config = Config::ensure_at(config_path)?;
            config.api.enabled = false;
            config.save_to_path(config_path)?;
            stop_api()?;
            println!("API disabled in config.");
        }
        ApiCommand::Start => start_api(config_path)?,
        ApiCommand::Run => run_api(config_path)?,
        ApiCommand::Restart => {
            stop_api()?;
            start_api(config_path)?;
        }
        ApiCommand::Stop => stop_api()?,
        ApiCommand::Status => {
            let status = get_api_status(config_path)?;
            let enabled = if status.enabled {
                "enabled"
            } else {
                "disabled"
            };
            let running = if status.running { "running" } else { "stopped" };
            match status.pid {
                Some(pid) => println!("API {enabled}, {running} (pid {pid})."),
                None => println!("API {enabled}, {running}."),
            }
        }
    }
    Ok(())
}

fn start_api(config_path: &Path) -> Result<()> {
    let config = Config::ensure_at(config_path)?;
    if !config.api.enabled {
        anyhow::bail!("API is disabled in config. Run `hstry api enable` first.");
    }

    if let Some(pid) = read_api_pid()? {
        if is_process_running(pid) {
            anyhow::bail!("API already running with pid {pid}");
        }
        let _ = std::fs::remove_file(api_pid_file_path());
    }

    let exe = api_binary()?;
    let log_file = open_api_log()?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--config")
        .arg(config_path)
        .arg("--port")
        .arg(config.api.port.to_string());
    if !config.api.token.is_empty() {
        cmd.arg("--token").arg(&config.api.token);
    }
    cmd.stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    let child = cmd.spawn().context("Failed to start API process")?;
    let pid = child.id();
    write_api_pid(pid)?;
    println!("API started (pid {pid}).");
    Ok(())
}

fn run_api(config_path: &Path) -> Result<()> {
    let config = Config::ensure_at(config_path)?;
    let exe = api_binary()?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--config")
        .arg(config_path)
        .arg("--port")
        .arg(config.api.port.to_string());
    if !config.api.token.is_empty() {
        cmd.arg("--token").arg(&config.api.token);
    }
    let status = cmd.status().context("Failed to run API process")?;
    if !status.success() {
        anyhow::bail!("API exited with {status}");
    }
    Ok(())
}

fn stop_api() -> Result<()> {
    let Some(pid) = read_api_pid()? else {
        println!("API not running.");
        return Ok(());
    };

    if is_process_running(pid) {
        terminate_process(pid)?;
        println!("Sent termination signal to API (pid {pid}).");
    } else {
        println!("API not running.");
    }

    let _ = std::fs::remove_file(api_pid_file_path());
    Ok(())
}

pub fn get_api_status(config_path: &Path) -> Result<ApiStatus> {
    let config = Config::ensure_at(config_path)?;
    let pid = read_api_pid()?;
    let running = pid.is_some_and(is_process_running);
    Ok(ApiStatus {
        enabled: config.api.enabled,
        running,
        pid: if running { pid } else { None },
    })
}

/// Locate the `hstry-api` binary: prefer one installed next to the `hstry`
/// CLI (same bin dir), otherwise fall back to `hstry-api` on PATH.
fn api_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join("hstry-api");
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    Ok(PathBuf::from("hstry-api"))
}

fn api_state_dir() -> PathBuf {
    xdg_state_dir().join("hstry")
}

fn api_pid_file_path() -> PathBuf {
    api_state_dir().join("api.pid")
}

fn api_log_file_path() -> PathBuf {
    api_state_dir().join("api.log")
}

fn open_api_log() -> Result<File> {
    let dir = api_state_dir();
    std::fs::create_dir_all(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(api_log_file_path())?;
    Ok(file)
}

fn read_api_pid() -> Result<Option<u32>> {
    let path = api_pid_file_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)?;
    Ok(contents.trim().parse::<u32>().ok())
}

fn write_api_pid(pid: u32) -> Result<()> {
    let dir = api_state_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(api_pid_file_path(), pid.to_string())?;
    Ok(())
}
