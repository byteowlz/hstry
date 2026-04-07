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
    // while still distinguishing edits. Slice on a UTF-8 char boundary so
    // multi-byte glyphs (box drawing, emoji, CJK) don't panic.
    let content_prefix = utf8_prefix(content, 4096);
    let key = format!("hash:{source_id}:{conv}:{idx}:{role}:{content_prefix}");
    uuid::Uuid::new_v5(&HSTRY_MSG_NAMESPACE, key.as_bytes())
}

/// Return a prefix of `s` containing at most `max_bytes` bytes, truncated at
/// the nearest UTF-8 character boundary at or below `max_bytes`.
fn utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn utf8_prefix_handles_multibyte_at_boundary() {
        // 3-byte char '─' (U+2500) repeated; choose a length so 4096 lands
        // mid-character.
        let s: String = std::iter::repeat('─').take(2000).collect();
        // 2000 * 3 = 6000 bytes. max_bytes 4096 lands inside a glyph.
        let p = utf8_prefix(&s, 4096);
        assert!(p.len() <= 4096);
        assert!(s.starts_with(p));
        // No panic on round-trip
        let _ = stable_message_id("pi", Some("c"), 0, "user", &s, None);
    }

    #[test]
    fn utf8_prefix_passthrough_when_short() {
        assert_eq!(utf8_prefix("hello", 4096), "hello");
    }
}

/// Returns the environment variable prefix for this application.
pub fn env_prefix() -> String {
    "HSTRY".to_string()
}
