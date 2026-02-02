//! Database schema for hstry.

/// SQL schema for the schema migrations tracking table.
/// The full database schema is managed via migrations in the migrations/ directory.
pub const SCHEMA: &str = r#"
-- Schema migration tracking table
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);
"#;
