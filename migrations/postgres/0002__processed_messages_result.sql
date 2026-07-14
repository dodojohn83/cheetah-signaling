-- Adds result/status columns to the idempotent inbox table.
ALTER TABLE processed_messages
    ADD COLUMN IF NOT EXISTS idempotency_key TEXT;

ALTER TABLE processed_messages
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'completed';

ALTER TABLE processed_messages
    ADD COLUMN IF NOT EXISTS result_payload JSONB;

ALTER TABLE processed_messages
    ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_processed_messages_expires
    ON processed_messages (tenant_id, expires_at);
