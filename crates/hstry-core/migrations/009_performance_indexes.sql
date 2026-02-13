-- Performance indexes for bulk operations (sync, dedup, delete)
--
-- The composite index idx_msg_conv (conversation_id, idx) exists but SQLite
-- cannot use it efficiently for bare conversation_id lookups in DELETE/COUNT
-- queries. A dedicated single-column index on conversation_id dramatically
-- speeds up cascade-style deletes and conversation-scoped aggregations.

CREATE INDEX IF NOT EXISTS idx_msg_conversation_id ON messages(conversation_id);

-- Index on message_events.conversation_id for fast cascade deletes and lookups
CREATE INDEX IF NOT EXISTS idx_msg_events_conv ON message_events(conversation_id);

-- Index on conversation_snapshots already has PK on conversation_id (no change needed)

-- Index on conversation_summary_cache already has PK on conversation_id (no change needed)
