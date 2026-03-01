use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const ADAPTER_PROTOCOL_VERSION: &str = "1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterManifest {
    pub hstry_version: String,
    pub protocol_version: String,
}

pub fn expected_hstry_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn validate_adapter_manifest(adapter_paths: &[PathBuf]) -> Result<AdapterManifest> {
    if std::env::var("HSTRY_ALLOW_UNPINNED_ADAPTERS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        return Ok(AdapterManifest {
            hstry_version: expected_hstry_version(),
            protocol_version: ADAPTER_PROTOCOL_VERSION.to_string(),
        });
    }

    let mut manifests = Vec::new();
    for base_path in adapter_paths {
        if let Some(manifest) = read_manifest(base_path)? {
            manifests.push(manifest);
        }
    }

    if manifests.is_empty() {
        anyhow::bail!(
            "Adapter manifest not found. Run 'hstry adapters update' to install version-pinned adapters."
        );
    }

    let expected_version_value = expected_hstry_version();
    let expected_version = normalize_version(&expected_version_value);
    let expected_protocol = ADAPTER_PROTOCOL_VERSION;

    for manifest in manifests {
        let manifest_version = normalize_version(&manifest.hstry_version);
        if manifest_version != expected_version {
            anyhow::bail!(
                "Adapter version mismatch (expected hstry {}, found {}). Run 'hstry adapters update'.",
                expected_version,
                manifest_version
            );
        }
        if manifest.protocol_version != expected_protocol {
            anyhow::bail!(
                "Adapter protocol mismatch (expected {}, found {}). Run 'hstry adapters update'.",
                expected_protocol,
                manifest.protocol_version
            );
        }
    }

    Ok(AdapterManifest {
        hstry_version: expected_hstry_version(),
        protocol_version: expected_protocol.to_string(),
    })
}

fn read_manifest(adapter_path: &Path) -> Result<Option<AdapterManifest>> {
    let manifest_path = adapter_path.join(".hstry-adapters.json");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&manifest_path)?;
    let manifest: AdapterManifest = serde_json::from_str(&content)?;
    Ok(Some(manifest))
}

fn normalize_version(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}
