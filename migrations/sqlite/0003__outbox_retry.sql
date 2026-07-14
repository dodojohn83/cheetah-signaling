-- Adds retry and failure columns to the transactional outbox table.
ALTER TABLE outbox_events
    ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE outbox_events
    ADD COLUMN failed INTEGER NOT NULL DEFAULT 0;

ALTER TABLE outbox_events
    ADD COLUMN next_attempt_at TEXT;

ALTER TABLE outbox_events
    ADD COLUMN error TEXT;

DROP INDEX IF EXISTS idx_outbox_pending;

CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON outbox_events (tenant_id, published, failed, next_attempt_at, occurred_at)
    WHERE published = 0 AND failed = 0;
