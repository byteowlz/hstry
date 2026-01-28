//! Error types for hstry-core

use thiserror::Error;

/// Core library error type.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Adapter error: {0}")]
    Adapter(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Remote error: {0}")]
    Remote(String),

    #[error("{0}")]
    Other(String),
}

/// Result type alias using Error.
pub type Result<T> = std::result::Result<T, Error>;
