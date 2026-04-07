//! hstry-core: Universal AI chat history database
//!
//! This crate provides the core functionality for storing, searching, and
//! managing chat history from multiple AI sources (ChatGPT, Claude, Gemini,
//! OpenCode, Cursor, etc.)

pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod parts;
pub mod paths;
pub mod remote;
pub mod schema;
pub mod search_tantivy;
pub mod service;

pub use config::Config;
pub use db::Database;
pub use error::Error;
pub use error::Result;

/// Application name used for config directories and paths.
pub const APP_NAME: &str = "hstry";

/// Stable UUID v5 namespace for hstry message ids.
///
/// Generated once with `Uuid::new_v4()`; never change this constant or you
/// will invalidate every previously stored idempotency key.
pub const HSTRY_MSG_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x4f3a_2c1b_7d9e_4f60_8a2c_1b3d_4e5f_6071);

/// Derive a stable, content-addressable id for a message inside a given
/// conversation. Used by importers (Pi today, others over time) to make
/// re-imports idempotent: replaying the same JSONL produces the same row id
/// and is deduped by SQLite's `ON CONFLICT` clauses.
///
/// `client_id` (when present) takes priority because it is the harness's own
/// stable identifier; otherwise we hash a stable tuple over the source,
/// conversation, position, and content. (trx-hjjw.4)
pub fn stable_message_id(
    source_id: &str,
    conversation_external_id: Option<&str>,
    idx: i32,
    role: &str,
    content: &str,
    client_id: Option<&str>,
) -> uuid::Uuid {
    if let Some(cid) = client_id
        && !cid.is_empty()
    {
        return uuid::Uuid::new_v5(
            &HSTRY_MSG_NAMESPACE,
            format!("client:{source_id}:{cid}").as_bytes(),
        );
    }
    let conv = conversation_external_id.unwrap_or("");
    // Use a short prefix of the content to avoid pathological 100KB hashes
    // while still distinguishing edits.
    let content_prefix = if content.len() > 4096 {
        &content[..4096]
    } else {
        content
    };
    let key = format!("hash:{source_id}:{conv}:{idx}:{role}:{content_prefix}");
    uuid::Uuid::new_v5(&HSTRY_MSG_NAMESPACE, key.as_bytes())
}

/// Returns the environment variable prefix for this application.
pub fn env_prefix() -> String {
    "HSTRY".to_string()
}
