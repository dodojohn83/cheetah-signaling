CREATE TABLE IF NOT EXISTS protocol_sessions (
    protocol_session_id BLOB NOT NULL PRIMARY KEY,
    tenant_id BLOB NOT NULL,
    device_id BLOB NOT NULL,
    protocol TEXT NOT NULL,
    protocol_identity TEXT NOT NULL,
    presence TEXT NOT NULL DEFAULT 'unknown',
    expiry_at INTEGER NOT NULL,
    owner_epoch INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    data TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_protocol_sessions_device
    ON protocol_sessions(tenant_id, protocol, device_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_protocol_sessions_identity
    ON protocol_sessions(tenant_id, protocol, protocol_identity);
CREATE INDEX IF NOT EXISTS idx_protocol_sessions_expiry
    ON protocol_sessions(expiry_at, protocol_session_id);
CREATE INDEX IF NOT EXISTS idx_protocol_sessions_updated
    ON protocol_sessions(updated_at, protocol_session_id);
