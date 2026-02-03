CREATE TABLE IF NOT EXISTS conversation_summary_cache (
    conversation_id TEXT PRIMARY KEY,
    message_count INTEGER NOT NULL,
    first_user_message TEXT,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_conv_summary_updated ON conversation_summary_cache(updated_at DESC);

INSERT INTO conversation_summary_cache (conversation_id, message_count, first_user_message, updated_at)
SELECT
    c.id,
    COUNT(m.id) AS message_count,
    (
        SELECT content
        FROM messages m2
        WHERE m2.conversation_id = c.id
          AND m2.role = 'user'
        ORDER BY m2.idx ASC
        LIMIT 1
    ) AS first_user_message,
    CAST(strftime('%s', 'now') AS INTEGER)
FROM conversations c
LEFT JOIN messages m ON m.conversation_id = c.id
GROUP BY c.id
ON CONFLICT(conversation_id) DO UPDATE SET
    message_count = excluded.message_count,
    first_user_message = excluded.first_user_message,
    updated_at = excluded.updated_at;
