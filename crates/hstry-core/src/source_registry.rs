//! Source-registration invariant enforcement (trx-gzfh).
//!
//! Every code path that creates a `sources` row routes through
//! [`validate_new_source`]. The validator refuses paths that would violate
//! the five rules captured in `trx-gzfh`:
//!
//! 1. Duplicate path. A path equal to an existing source's path (any
//!    adapter).
//! 2. Sub-path of an existing source. A directory that is a strict
//!    descendant of an existing source's path.
//! 3. Super-path of an existing source. A directory that is a strict
//!    ancestor of an existing source's path.
//! 4. Individual file paths. A source's path must be a directory.
//! 5. Cross-harness territory. A source for adapter `X` cannot live under
//!    another adapter's canonical root.
//!
//! Canonical roots are the `defaultPaths` array returned from each
//! adapter's `info()` method. The CLI layer resolves them via
//! `AdapterRunner::get_info` and passes them in as data, so this module
//! stays runtime-agnostic.

use crate::models::Source;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Map from adapter name to its canonical root paths (expanded, absolute).
pub type CanonicalRoots = HashMap<String, Vec<PathBuf>>;

/// Reasons a source-registration request can be refused.
#[derive(Debug, Error, Clone)]
pub enum RegistrationError {
    #[error("source path must be a directory, not a file: {path}")]
    NotADirectory { path: String },

    #[error(
        "path '{path}' is not inside a canonical root for adapter '{adapter}'. Allowed roots: {roots:?}"
    )]
    NotInCanonicalRoot {
        path: String,
        adapter: String,
        roots: Vec<String>,
    },

    #[error(
        "path '{path}' is inside canonical territory of adapter '{conflicting_adapter}' (root: {conflicting_root}); each harness owns its tree"
    )]
    CrossHarnessTerritory {
        path: String,
        conflicting_adapter: String,
        conflicting_root: String,
    },

    #[error(
        "path '{path}' is already registered as source '{existing_source_id}' (adapter '{existing_adapter}')"
    )]
    DuplicatePath {
        path: String,
        existing_source_id: String,
        existing_adapter: String,
    },

    #[error(
        "path '{path}' is inside existing source '{parent_source_id}' ({parent_path}); use the parent root instead"
    )]
    SubPathOf {
        path: String,
        parent_source_id: String,
        parent_path: String,
    },

    #[error(
        "path '{path}' is a parent of existing source '{child_source_id}' ({child_path}); remove the child source first if you want to replace it"
    )]
    SuperPathOf {
        path: String,
        child_source_id: String,
        child_path: String,
    },
}

/// Normalize a source path for comparison. Trims trailing slashes; does not
/// touch case (paths are case-sensitive on Linux/macOS; Windows handles its
/// own folding at the filesystem layer).
pub fn normalize_path(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

/// Returns true if `descendant` is a strict descendant of `ancestor` (not
/// equal, but rooted under it).
fn is_strict_descendant(descendant: &Path, ancestor: &Path) -> bool {
    descendant != ancestor && descendant.starts_with(ancestor)
}

/// Validate a prospective source against the five invariant rules.
///
/// On success, returns a fully-constructed [`Source`] ready to hand to
/// [`crate::Database::upsert_source`]. On failure, returns a structured
/// [`RegistrationError`] naming the specific rule that was violated.
///
/// `path_exists_as_dir` is a closure so the validator stays testable
/// without touching the filesystem. In production wire it to
/// `Path::is_dir`; in tests pass a stub.
pub fn validate_new_source<F>(
    adapter_name: &str,
    raw_path: &str,
    source_id: String,
    config: serde_json::Value,
    canonical_roots: &CanonicalRoots,
    existing_sources: &[Source],
    path_exists_as_dir: F,
) -> Result<Source, RegistrationError>
where
    F: Fn(&Path) -> bool,
{
    let normalized = normalize_path(raw_path);
    let candidate = PathBuf::from(&normalized);

    // Rule 4: must be a directory. We do this first because the other
    // rules assume directory-like semantics.
    if !path_exists_as_dir(&candidate) {
        return Err(RegistrationError::NotADirectory { path: normalized });
    }

    // Rule 5 + canonical-root constraint: the path must live inside one
    // of this adapter's canonical roots, AND must NOT live inside any
    // other adapter's canonical root. (Equal-to-root counts as inside.)
    let our_roots = canonical_roots
        .get(adapter_name)
        .cloned()
        .unwrap_or_default();

    let inside_own = our_roots
        .iter()
        .any(|root| candidate == *root || candidate.starts_with(root));
    if !inside_own {
        return Err(RegistrationError::NotInCanonicalRoot {
            path: normalized,
            adapter: adapter_name.to_string(),
            roots: our_roots
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
        });
    }

    // Among canonical roots that contain the candidate, the *most specific*
    // (longest) root wins. If that root belongs to a different adapter,
    // refuse — that's another harness's tree.
    let mut best_owner: Option<(&str, &Path)> = None;
    for (other_adapter, roots) in canonical_roots {
        for root in roots {
            if candidate == *root || candidate.starts_with(root) {
                let take = match best_owner {
                    None => true,
                    Some((_, current)) => {
                        root.as_path().components().count() > current.components().count()
                    }
                };
                if take {
                    best_owner = Some((other_adapter.as_str(), root.as_path()));
                }
            }
        }
    }
    if let Some((owner, root)) = best_owner
        && owner != adapter_name
    {
        return Err(RegistrationError::CrossHarnessTerritory {
            path: normalized,
            conflicting_adapter: owner.to_string(),
            conflicting_root: root.to_string_lossy().to_string(),
        });
    }

    // Rules 1, 2, 3: compare against every existing source.
    for existing in existing_sources {
        let Some(existing_path_raw) = existing.path.as_deref() else {
            continue;
        };
        let existing_normalized = normalize_path(existing_path_raw);
        let existing_path = PathBuf::from(&existing_normalized);

        // Rule 1: duplicate (any adapter).
        if existing_path == candidate {
            // Re-registering the same (adapter, path, id) is idempotent
            // and not an error — that's how update-by-id flows reach
            // here. Only reject if it's a *different* row.
            if existing.id == source_id && existing.adapter == adapter_name {
                continue;
            }
            return Err(RegistrationError::DuplicatePath {
                path: normalized,
                existing_source_id: existing.id.clone(),
                existing_adapter: existing.adapter.clone(),
            });
        }

        // Rule 2: candidate is a sub-path of an existing source.
        if is_strict_descendant(&candidate, &existing_path) {
            return Err(RegistrationError::SubPathOf {
                path: normalized,
                parent_source_id: existing.id.clone(),
                parent_path: existing_normalized,
            });
        }

        // Rule 3: candidate is a super-path of an existing source. Reject
        // by default — operator must remove the child first.
        if is_strict_descendant(&existing_path, &candidate) {
            return Err(RegistrationError::SuperPathOf {
                path: normalized,
                child_source_id: existing.id.clone(),
                child_path: existing_normalized,
            });
        }
    }

    Ok(Source {
        id: source_id,
        adapter: adapter_name.to_string(),
        path: Some(normalized),
        last_sync_at: None,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(adapter: &str, path: &str) -> CanonicalRoots {
        let mut m = HashMap::new();
        m.insert(adapter.to_string(), vec![PathBuf::from(path)]);
        m
    }

    fn merge(a: CanonicalRoots, b: CanonicalRoots) -> CanonicalRoots {
        let mut m = a;
        for (k, v) in b {
            m.entry(k).or_default().extend(v);
        }
        m
    }

    fn existing(id: &str, adapter: &str, path: &str) -> Source {
        Source {
            id: id.to_string(),
            adapter: adapter.to_string(),
            path: Some(path.to_string()),
            last_sync_at: None,
            config: serde_json::json!({}),
        }
    }

    const ALWAYS_DIR: fn(&Path) -> bool = |_| true;
    const NEVER_DIR: fn(&Path) -> bool = |_| false;

    #[test]
    fn accepts_canonical_root_for_owning_adapter() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let s = validate_new_source(
            "claude-code",
            "/home/u/.claude/projects",
            "claude-code".to_string(),
            serde_json::json!({}),
            &roots,
            &[],
            ALWAYS_DIR,
        )
        .expect("canonical root should validate");
        assert_eq!(s.path.as_deref(), Some("/home/u/.claude/projects"));
    }

    #[test]
    fn rejects_file_paths() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let err = validate_new_source(
            "claude-code",
            "/home/u/.claude/projects/some-session.jsonl",
            "claude-code".to_string(),
            serde_json::json!({}),
            &roots,
            &[],
            NEVER_DIR,
        )
        .expect_err("file path must be rejected");
        assert!(matches!(err, RegistrationError::NotADirectory { .. }));
    }

    #[test]
    fn rejects_subpath_of_existing_source_same_adapter() {
        let roots = root("pi", "/home/u/.pi/agent/sessions");
        let existing = vec![existing("pi", "pi", "/home/u/.pi/agent/sessions")];
        let err = validate_new_source(
            "pi",
            "/home/u/.pi/agent/sessions/--home-user-project--",
            "import-pi".to_string(),
            serde_json::json!({}),
            &roots,
            &existing,
            ALWAYS_DIR,
        )
        .expect_err("subpath must be rejected");
        match err {
            RegistrationError::SubPathOf {
                parent_source_id, ..
            } => assert_eq!(parent_source_id, "pi"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_superpath_of_existing_source() {
        let roots = root("pi", "/home/u");
        let existing = vec![existing("pi", "pi", "/home/u/.pi/agent/sessions")];
        let err = validate_new_source(
            "pi",
            "/home/u",
            "pi-broad".to_string(),
            serde_json::json!({}),
            &roots,
            &existing,
            ALWAYS_DIR,
        )
        .expect_err("super-path must be rejected");
        assert!(matches!(err, RegistrationError::SuperPathOf { .. }));
    }

    #[test]
    fn rejects_duplicate_path() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let existing = vec![existing(
            "claude-code",
            "claude-code",
            "/home/u/.claude/projects",
        )];
        let err = validate_new_source(
            "claude-code",
            "/home/u/.claude/projects",
            "claude-code-7acee973".to_string(),
            serde_json::json!({}),
            &roots,
            &existing,
            ALWAYS_DIR,
        )
        .expect_err("duplicate must be rejected");
        assert!(matches!(err, RegistrationError::DuplicatePath { .. }));
    }

    #[test]
    fn allows_idempotent_self_reregistration() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let existing = vec![existing(
            "claude-code",
            "claude-code",
            "/home/u/.claude/projects",
        )];
        validate_new_source(
            "claude-code",
            "/home/u/.claude/projects",
            "claude-code".to_string(),
            serde_json::json!({}),
            &roots,
            &existing,
            ALWAYS_DIR,
        )
        .expect("same id+adapter+path must be idempotent");
    }

    #[test]
    fn rejects_cross_harness_territory() {
        // claude-code attempts to register inside pi's tree
        let roots = merge(
            root("claude-code", "/home/u/.claude/projects"),
            root("pi", "/home/u/.pi/agent/sessions"),
        );
        let err = validate_new_source(
            "claude-code",
            "/home/u/.pi/agent/sessions/--home-user-foo--",
            "claude-code-bad".to_string(),
            serde_json::json!({}),
            &roots,
            &[],
            ALWAYS_DIR,
        )
        .expect_err("cross-harness must be rejected");
        match err {
            RegistrationError::CrossHarnessTerritory {
                conflicting_adapter,
                ..
            } => assert_eq!(conflicting_adapter, "pi"),
            // Also acceptable: the candidate isn't in claude-code's roots
            // (catches earlier). Either of these is the right refusal.
            RegistrationError::NotInCanonicalRoot { .. } => {}
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn rejects_path_outside_any_canonical_root() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let err = validate_new_source(
            "claude-code",
            "/tmp/random",
            "claude-code-rand".to_string(),
            serde_json::json!({}),
            &roots,
            &[],
            ALWAYS_DIR,
        )
        .expect_err("path outside any root must be rejected");
        assert!(matches!(err, RegistrationError::NotInCanonicalRoot { .. }));
    }

    #[test]
    fn trailing_slash_does_not_break_duplicate_detection() {
        let roots = root("claude-code", "/home/u/.claude/projects");
        let existing = vec![existing(
            "claude-code",
            "claude-code",
            "/home/u/.claude/projects",
        )];
        let err = validate_new_source(
            "claude-code",
            "/home/u/.claude/projects/",
            "claude-code-dup".to_string(),
            serde_json::json!({}),
            &roots,
            &existing,
            ALWAYS_DIR,
        )
        .expect_err("trailing-slash duplicate must still be rejected");
        assert!(matches!(err, RegistrationError::DuplicatePath { .. }));
    }
}
