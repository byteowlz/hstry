-- Add sender attribution and per-message provider to messages.
-- Both columns are nullable for backwards compatibility.
-- Existing messages keep NULL sender (implied by role) and NULL provider
-- (falls back to conversation-level provider).

ALTER TABLE messages ADD COLUMN sender_json TEXT;
ALTER TABLE messages ADD COLUMN provider TEXT;
