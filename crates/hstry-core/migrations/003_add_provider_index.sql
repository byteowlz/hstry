-- Add index on conversations.provider for filtering by provider
CREATE INDEX IF NOT EXISTS idx_conv_provider ON conversations(provider);
