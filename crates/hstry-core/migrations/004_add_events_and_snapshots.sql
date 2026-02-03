-- Add message event log and conversation snapshots for fast reads

CREATE TABLE IF NOT EXISTS message_events (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    payload_json JSON NOT NULL,
    created_at INTEGER,
    metadata JSON DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_msg_events_conv_idx ON message_events(conversation_id, idx);

CREATE TABLE IF NOT EXISTS conversation_snapshots (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    message_count INTEGER NOT NULL,
    payload_json JSON NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conv_snap_updated ON conversation_snapshots(updated_at DESC);
