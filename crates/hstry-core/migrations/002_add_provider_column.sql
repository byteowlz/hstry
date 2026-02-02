-- Add provider column to conversations table
-- This was missing from initial schema
ALTER TABLE conversations ADD COLUMN provider TEXT;
