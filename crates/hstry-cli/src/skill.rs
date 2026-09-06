use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const CONTENT: &str = include_str!("../../../skills/hstry-search/SKILL.md");
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Target {
    Shared,
    Claude,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install without overwriting a different existing skill
    Install {
        #[arg(long, value_enum, default_value = "shared")]
        target: Target,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Inspect freshness/local edits without modifying files
    Status {
        #[arg(long, value_enum, default_value = "shared")]
        target: Target,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Refresh an existing managed copy; --force explicitly replaces local edits
    Update {
        #[arg(long, value_enum, default_value = "shared")]
        target: Target,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
}
#[derive(Serialize, Deserialize)]
struct Baseline {
    version: String,
    content: String,
}
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Status {
    pub state: String,
    pub installed_version: Option<String>,
    pub binary_version: String,
}
fn status(dir: &Path) -> Result<Status> {
    let current = fs::read_to_string(dir.join("SKILL.md"));
    let baseline = fs::read(dir.join(".hstry-managed.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Baseline>(&b).ok());
    let state = match current {
        Ok(ref text) if text == CONTENT => "current",
        Ok(ref text) if baseline.as_ref().is_some_and(|b| b.content == *text) => "outdated",
        Ok(_) => "modified_or_unmanaged",
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(e) => return Err(e.into()),
    };
    Ok(Status {
        state: state.into(),
        installed_version: baseline.map(|b| b.version),
        binary_version: env!("CARGO_PKG_VERSION").into(),
    })
}
fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("Missing skill parent")?)?;
    temp.write_all(content)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn update(dir: &Path, install: bool, force: bool) -> Result<Status> {
    let before = status(dir)?;
    if install && !matches!(before.state.as_str(), "missing" | "current") {
        bail!("Existing skill differs; inspect it before explicit skill update --force");
    }
    if !install && before.state == "missing" {
        bail!("Skill is not installed; use skill install");
    }
    if !install && before.state == "modified_or_unmanaged" && !force {
        bail!("Skill contains local edits or is unmanaged; --force is required to replace it");
    }
    fs::create_dir_all(dir)?;
    atomic_write(&dir.join("SKILL.md"), CONTENT.as_bytes())?;
    atomic_write(
        &dir.join(".hstry-managed.json"),
        &serde_json::to_vec(&Baseline {
            version: env!("CARGO_PKG_VERSION").into(),
            content: CONTENT.into(),
        })?,
    )?;
    status(dir)
}
pub fn warn_if_stale() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    for (target, path) in [
        ("shared", ".agents/skills/hstry-search"),
        ("claude", ".claude/skills/hstry-search"),
    ] {
        if let Ok(s) = status(&home.join(path))
            && matches!(s.state.as_str(), "outdated" | "modified_or_unmanaged")
        {
            eprintln!(
                "hstry skill differs from this binary; inspect with hstry skill status --target {target} (no files changed)"
            );
        }
    }
}

pub fn run(command: Command) -> Result<()> {
    let (target, path) = match &command {
        Command::Install { target, path }
        | Command::Status { target, path }
        | Command::Update { target, path, .. } => (*target, path.clone()),
    };
    let dir = path.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(match target {
                Target::Shared => ".agents/skills/hstry-search",
                Target::Claude => ".claude/skills/hstry-search",
            })
    });
    let result = match command {
        Command::Install { .. } => update(&dir, true, false)?,
        Command::Update { force, .. } => update(&dir, false, force)?,
        Command::Status { .. } => status(&dir)?,
    };
    println!(
        "{}",
        serde_json::json!({"ok":true,"result":result,"path":dir})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clean_older_skill_can_be_updated_without_force() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("SKILL.md"), "old managed instructions")?;
        fs::write(
            dir.path().join(".hstry-managed.json"),
            serde_json::to_vec(&Baseline {
                version: "0.0.1".into(),
                content: "old managed instructions".into(),
            })?,
        )?;
        assert_eq!(
            status(dir.path())?,
            Status {
                state: "outdated".into(),
                installed_version: Some("0.0.1".into()),
                binary_version: env!("CARGO_PKG_VERSION").into()
            }
        );
        assert_eq!(
            update(dir.path(), false, false)?,
            Status {
                state: "current".into(),
                installed_version: Some(env!("CARGO_PKG_VERSION").into()),
                binary_version: env!("CARGO_PKG_VERSION").into()
            }
        );
        Ok(())
    }
    #[test]
    fn install_status_and_update_preserve_local_edits() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let dir = temp.path().join("skill");
        assert_eq!(status(&dir)?.state, "missing");
        assert_eq!(update(&dir, true, false)?.state, "current");
        fs::write(dir.join("SKILL.md"), "local edit")?;
        assert!(update(&dir, true, false).is_err());
        assert!(update(&dir, false, false).is_err());
        assert_eq!(fs::read_to_string(dir.join("SKILL.md"))?, "local edit");
        assert_eq!(update(&dir, false, true)?.state, "current");
        assert!(CONTENT.contains("next_offset_chars"));
        Ok(())
    }
}
