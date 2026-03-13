-- Add monotonic version counter and denormalized message_count to conversations.
-- version: bumped on every mutation (message insert/update, conversation update).
-- message_count: maintained atomically alongside version.

ALTER TABLE conversations ADD COLUMN version INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN message_count INTEGER NOT NULL DEFAULT 0;

-- Backfill message_count from actual message rows.
UPDATE conversations SET message_count = (
    SELECT COUNT(*) FROM messages m WHERE m.conversation_id = conversations.id
);

-- Backfill version = 1 for any conversation that already has messages.
UPDATE conversations SET version = 1 WHERE message_count > 0;
