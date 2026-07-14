CREATE TABLE IF NOT EXISTS webhook_configs (
    tenant_id BLOB NOT NULL,
    webhook_id BLOB NOT NULL PRIMARY KEY,
    url TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    event_types TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    revision INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    data TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_webhook_configs_tenant
    ON webhook_configs (tenant_id, enabled);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    tenant_id BLOB NOT NULL,
    delivery_id BLOB NOT NULL PRIMARY KEY,
    webhook_id BLOB NOT NULL,
    event_id BLOB NOT NULL,
    status TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL,
    data TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_pending
    ON webhook_deliveries (tenant_id, status, next_attempt_at)
    WHERE status IN ('pending', 'failed');

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_webhook
    ON webhook_deliveries (tenant_id, webhook_id, status);
