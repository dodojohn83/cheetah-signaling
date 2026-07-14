-- Adds result/status columns to the idempotent inbox table.
ALTER TABLE processed_messages
    ADD COLUMN idempotency_key TEXT;

ALTER TABLE processed_messages
    ADD COLUMN status TEXT NOT NULL DEFAULT 'completed';

ALTER TABLE processed_messages
    ADD COLUMN result_payload TEXT;

ALTER TABLE processed_messages
    ADD COLUMN expires_at TEXT;

CREATE INDEX IF NOT EXISTS idx_processed_messages_expires
    ON processed_messages (tenant_id, expires_at);
