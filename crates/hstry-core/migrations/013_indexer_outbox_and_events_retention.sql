-- Indexer outbox + message_events retention plumbing.
--
-- Two related concerns are addressed here:
--
-- 1. **Indexer outbox** (trx-z42c.5/.6): a durable, ordered queue of pending
--    indexing work. Producers (message upserts) enqueue rows; a dedicated
--    worker (`indexer_outbox` job loop) drains them with at-least-once
--    semantics. The table is intentionally tiny so it can be polled cheaply.
--
-- 2. **message_events maintenance** (trx-jtxf, trx-aa3m): we need fast lookups
--    by created_at to enforce retention (delete rows older than N days), and
--    by (conversation_id, created_at) to keep only the newest K events per
--    conversation. Both are covered by the new index below.
--
-- Both tables/indexes use `IF NOT EXISTS` so re-running on a partially
-- migrated database is a no-op.

CREATE TABLE IF NOT EXISTS indexer_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL,
    message_id TEXT,
    op TEXT NOT NULL DEFAULT 'upsert',  -- 'upsert' | 'delete' | 'rebuild'
    enqueued_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_indexer_outbox_enqueued_at
    ON indexer_outbox(enqueued_at);

CREATE INDEX IF NOT EXISTS idx_indexer_outbox_conv
    ON indexer_outbox(conversation_id);

-- Speed up retention sweeps and per-conversation compaction.
CREATE INDEX IF NOT EXISTS idx_msg_events_created_at
    ON message_events(created_at);

CREATE INDEX IF NOT EXISTS idx_msg_events_conv_created_at
    ON message_events(conversation_id, created_at DESC);
