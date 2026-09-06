//! Deterministic launch arguments and non-destructive export placement.
use anyhow::{Context, Result, bail};
use hstry_runtime::ExportResult;
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn fresh_session_id(format: &str) -> String {
    let id = uuid::Uuid::new_v4();
    if format == "opencode" {
        format!("ses_{}", id.simple())
    } else {
        id.to_string()
    }
}

pub fn arguments(
    template: &str,
    session_path: &Path,
    session_id: &str,
    workspace: &str,
) -> Result<Vec<String>> {
    // Parse template quoting BEFORE substitution; a path/ID can never create arguments.
    let tokens = shlex::split(template).context("Invalid quoting in resume command")?;
    if tokens.is_empty() {
        bail!("Empty resume command");
    }
    if tokens
        .iter()
        .any(|s| matches!(s.as_str(), "|" | "||" | "&&" | ";" | ">" | "<"))
    {
        bail!("Resume commands are argument templates, not shell programs");
    }
    let session_path = session_path
        .to_str()
        .context("Session path is not valid UTF-8 for a structured launch plan")?;
    Ok(tokens
        .into_iter()
        .map(|s| expand(&s, session_path, session_id, workspace))
        .collect())
}

fn expand(template: &str, path: &str, id: &str, workspace: &str) -> String {
    let mut rest = template;
    let mut out = String::new();
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find('}') else {
            break;
        };
        let token = &rest[..=end];
        out.push_str(match token {
            "{session_path}" => path,
            "{session_id}" => id,
            "{workspace}" => workspace,
            _ => token,
        });
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn place(
    result: &ExportResult,
    root: &Path,
    session_id: &str,
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    let prefix = result
        .metadata
        .as_ref()
        .and_then(|m| m["root"].as_str())
        .unwrap_or("");
    // OpenCode config names its data root; project/ is part of the legacy layout.
    let prefix = if result.format == "opencode" {
        ""
    } else {
        prefix
    };
    let mut files = Vec::new();
    if let Some(text) = &result.content {
        let ext = match result.format.as_str() {
            "json" => "json",
            "markdown" => "md",
            _ => "jsonl",
        };
        files.push((format!("{session_id}.{ext}"), text.as_str()));
    }
    if let Some(exports) = &result.files {
        for file in exports {
            if file
                .encoding
                .as_deref()
                .is_some_and(|e| e != "utf8" && e != "utf-8")
            {
                bail!("Unsupported resume export encoding");
            }
            files.push((
                file.path
                    .strip_prefix(prefix)
                    .unwrap_or(&file.path)
                    .to_string(),
                file.content.as_str(),
            ));
        }
    }
    if files.is_empty() {
        bail!("Adapter produced no resume files");
    }
    let mut seen = HashSet::new();
    let mut targets = Vec::new();
    for (name, text) in files {
        let normalized = name.replace('\\', "/");
        if normalized
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == ".." || c.contains(':'))
        {
            bail!("Unsafe export path: {name}");
        }
        let target = root.join(normalized);
        if !seen.insert(target.clone()) {
            bail!("Duplicate export path");
        }
        for ancestor in target.ancestors() {
            if let Ok(meta) = fs::symlink_metadata(ancestor)
                && meta.file_type().is_symlink()
            {
                bail!("Refusing symlink in resume destination");
            }
            if ancestor == root {
                break;
            }
        }
        if target.exists() {
            bail!(
                "Refusing to overwrite an existing session: {}",
                target.display()
            );
        }
        targets.push((target, text));
    }
    if dry_run {
        return Ok(targets.into_iter().map(|(p, _)| p).collect());
    }
    let mut created = Vec::new();
    for (target, text) in targets {
        let write: Result<()> = (|| {
            fs::create_dir_all(target.parent().context("Missing destination parent")?)?;
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&target)?;
            created.push(target.clone());
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(error) = write {
            for file in &created {
                let _ = fs::remove_file(file);
            }
            return Err(error);
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_foreign_ids_cannot_create_arguments() -> Result<()> {
        assert_eq!(
            arguments(
                "agent --session {session_path} --name '{session_id}'",
                Path::new("/a directory/it's a file"),
                "x; touch injected",
                "/work"
            )?,
            vec![
                "agent",
                "--session",
                "/a directory/it's a file",
                "--name",
                "x; touch injected"
            ]
        );
        assert!(arguments("agent && other", Path::new("x"), "x", ".").is_err());
        assert_eq!(
            arguments(
                "agent {session_path} {session_id}",
                Path::new("{session_id}/file"),
                "{workspace}",
                "injected"
            )?,
            vec!["agent", "{session_id}/file", "{workspace}"]
        );
        Ok(())
    }
    #[test]
    fn opencode_keeps_its_project_root_and_native_id_prefix() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let export: ExportResult = serde_json::from_value(
            serde_json::json!({"format":"opencode","metadata":{"root":"project/"},"files":[{"path":"project/global/storage/session/info/ses_fixture.json","content":"{}"}]}),
        )?;
        assert_eq!(
            place(&export, dir.path(), "unused", true)?,
            vec![
                dir.path()
                    .join("project/global/storage/session/info/ses_fixture.json")
            ]
        );
        assert!(fresh_session_id("opencode").starts_with("ses_"));
        Ok(())
    }
    #[test]
    fn placement_is_dry_run_safe_and_never_overwrites_or_escapes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut result: ExportResult = serde_json::from_value(
            serde_json::json!({"format":"pi","files":[{"path":"nested/session.jsonl","content":"fixture"}]}),
        )?;
        let paths = place(&result, dir.path(), "new-id", true)?;
        assert!(!paths[0].exists());
        assert_eq!(place(&result, dir.path(), "new-id", false)?, paths);
        assert!(place(&result, dir.path(), "new-id", false).is_err());
        for path in [
            "../outside",
            "/absolute",
            "C:\\outside",
            "nested\\..\\outside",
        ] {
            result
                .files
                .as_mut()
                .context("Missing fixture export files")?[0]
                .path = path.into();
            assert!(place(&result, dir.path(), "new-id", false).is_err());
        }
        Ok(())
    }
}
