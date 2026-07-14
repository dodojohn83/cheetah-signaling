-- Adds retry and failure columns to the transactional outbox table.
ALTER TABLE outbox_events
    ADD COLUMN IF NOT EXISTS attempts BIGINT NOT NULL DEFAULT 0;

ALTER TABLE outbox_events
    ADD COLUMN IF NOT EXISTS failed BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE outbox_events
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

ALTER TABLE outbox_events
    ADD COLUMN IF NOT EXISTS error TEXT;

DROP INDEX IF EXISTS idx_outbox_pending;

CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON outbox_events (tenant_id, published, failed, next_attempt_at, occurred_at)
    WHERE published = FALSE AND failed = FALSE;
