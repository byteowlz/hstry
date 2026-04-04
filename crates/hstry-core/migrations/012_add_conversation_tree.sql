-- Session tree tracking: parent/child relationships with branch point granularity.
--
-- Enables:
--   "Show all child sessions of X"
--   "Show the full session tree with branch points"
--   "What was the context when this child session was created?"
--   Rendering fork trees in the Oqto sidebar
--
-- parent_conversation_id: the hstry conversation UUID of the parent session.
-- parent_message_idx:     the message index in the PARENT where the fork branched off.
--                         NULL means "forked from end" or "relationship unknown".
-- fork_type:              why this child was created.
--                         'fork'    = user explicitly forked at a message
--                         'thread'  = orchestrator dispatched a worker thread
--                         'resume'  = session was resumed/continued
--                         NULL      = unknown/legacy

ALTER TABLE conversations ADD COLUMN parent_conversation_id TEXT;
ALTER TABLE conversations ADD COLUMN parent_message_idx INTEGER;
ALTER TABLE conversations ADD COLUMN fork_type TEXT;

CREATE INDEX IF NOT EXISTS idx_conv_parent ON conversations(parent_conversation_id);
