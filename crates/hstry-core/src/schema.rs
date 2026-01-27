//! Database schema for hstry.

/// SQL schema for creating all tables.
pub const SCHEMA: &str = r#"
-- Sources (where history comes from)
CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    adapter TEXT NOT NULL,
    path TEXT,
    last_sync_at INTEGER,
    config JSON DEFAULT '{}'
);

-- Conversations (normalized from all sources)
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id),
    external_id TEXT,
    readable_id TEXT,
    title TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER,
    model TEXT,
    workspace TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    cost_usd REAL,
    metadata JSON DEFAULT '{}',
    UNIQUE(source_id, external_id)
);

-- Messages
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    parts_json JSON NOT NULL DEFAULT '[]',
    created_at INTEGER,
    model TEXT,
    tokens INTEGER,
    cost_usd REAL,
    metadata JSON DEFAULT '{}',
    UNIQUE(conversation_id, idx)
);

-- Tool calls (for agent interactions)
CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    input JSON,
    output TEXT,
    status TEXT,
    duration_ms INTEGER
);

-- Attachments (files, images, code blocks)
CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    type TEXT NOT NULL,
    name TEXT,
    mime_type TEXT,
    content BLOB,
    path TEXT,
    language TEXT,
    metadata JSON DEFAULT '{}'
);

-- Tags for organization
CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS conversation_tags (
    conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
    tag_id INTEGER REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (conversation_id, tag_id)
);

-- Embeddings (optional, for semantic search via mmry)
CREATE TABLE IF NOT EXISTS message_embeddings (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    embedding BLOB,
    model TEXT,
    created_at INTEGER
);

CREATE TABLE IF NOT EXISTS conversation_embeddings (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    embedding BLOB,
    model TEXT,
    created_at INTEGER
);

-- FTS for fast text search
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,
    content=messages,
    content_rowid=rowid,
    tokenize = 'porter',
    prefix = '2 3 4'
);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_code_fts USING fts5(
    content,
    content=messages,
    content_rowid=rowid,
    tokenize = "unicode61 tokenchars '_./:'",
    prefix = '2 3 4'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content);
    INSERT INTO messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_code_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_code_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_code_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
    VALUES('delete', OLD.rowid, OLD.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_code_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_code_fts(messages_code_fts, rowid, content)
    VALUES('delete', OLD.rowid, OLD.content);
    INSERT INTO messages_code_fts(rowid, content) VALUES (NEW.rowid, NEW.content);
END;

-- Indexes
CREATE INDEX IF NOT EXISTS idx_conv_source ON conversations(source_id);
CREATE INDEX IF NOT EXISTS idx_conv_created ON conversations(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_conv_workspace ON conversations(workspace);
CREATE INDEX IF NOT EXISTS idx_conv_model ON conversations(model);
CREATE INDEX IF NOT EXISTS idx_msg_conv ON messages(conversation_id, idx);
CREATE INDEX IF NOT EXISTS idx_msg_created ON messages(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_msg_role ON messages(role);
CREATE INDEX IF NOT EXISTS idx_tool_msg ON tool_calls(message_id);
CREATE INDEX IF NOT EXISTS idx_attach_msg ON attachments(message_id);
"#;
