-- Add platform_id column to conversations for orchestrator session ID mapping.
-- external_id stores the agent-native session ID (e.g., Pi session ID).
-- platform_id stores the orchestrating platform's session ID (e.g., "octo-xxx").
-- Lookups can use either column.

ALTER TABLE conversations ADD COLUMN platform_id TEXT;

CREATE INDEX IF NOT EXISTS idx_conv_platform_id ON conversations(platform_id);
