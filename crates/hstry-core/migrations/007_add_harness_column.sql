-- Add harness column to conversations and messages.
-- Tracks which agent harness produced the conversation/message (e.g., "pi", "claude").
-- Conversation-level is the default; message-level overrides when harness changes mid-session.
-- Nullable for backwards compatibility (existing data has no harness recorded).

ALTER TABLE conversations ADD COLUMN harness TEXT;
ALTER TABLE messages ADD COLUMN harness TEXT;
