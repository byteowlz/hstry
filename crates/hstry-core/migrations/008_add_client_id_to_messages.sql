-- Add client_id column for optimistic message matching
-- The client_id is a frontend-generated ID that allows matching
-- provisional (optimistic) messages to their server-persisted versions.

ALTER TABLE messages ADD COLUMN client_id TEXT;

-- Index for efficient lookups by client_id within a conversation
CREATE INDEX IF NOT EXISTS idx_msg_client_id ON messages(conversation_id, client_id);
