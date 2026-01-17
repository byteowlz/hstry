//! hstry-core: Universal AI chat history database
//!
//! This crate provides the core functionality for storing, searching, and
//! managing chat history from multiple AI sources (ChatGPT, Claude, Gemini,
//! OpenCode, Cursor, etc.)

pub mod config;
pub mod db;
pub mod error;
pub mod models;
pub mod schema;

pub use config::Config;
pub use db::Database;
pub use error::Error;
pub use error::Result;

/// Application name used for config directories and paths.
pub const APP_NAME: &str = "hstry";

/// Returns the environment variable prefix for this application.
pub fn env_prefix() -> String {
    "HSTRY".to_string()
}
