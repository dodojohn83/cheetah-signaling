CREATE TABLE IF NOT EXISTS tenants (
    tenant_id UUID NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS devices (
    tenant_id UUID NOT NULL,
    device_id UUID NOT NULL PRIMARY KEY,
    protocol TEXT NOT NULL,
    external_id TEXT NOT NULL,
    authority TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    lifecycle TEXT NOT NULL,
    connectivity_kind TEXT NOT NULL,
    owner_epoch BIGINT NOT NULL DEFAULT 0,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE (tenant_id, device_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_external_id
    ON devices (tenant_id, protocol, external_id)
    WHERE deleted = FALSE;

CREATE INDEX IF NOT EXISTS idx_devices_tenant_lifecycle
    ON devices (tenant_id, lifecycle)
    WHERE deleted = FALSE;

CREATE INDEX IF NOT EXISTS idx_devices_tenant_connectivity
    ON devices (tenant_id, connectivity_kind)
    WHERE deleted = FALSE;

CREATE INDEX IF NOT EXISTS idx_devices_tenant_deleted
    ON devices (tenant_id, deleted);

CREATE TABLE IF NOT EXISTS device_endpoints (
    tenant_id UUID NOT NULL,
    device_id UUID NOT NULL,
    endpoint_id UUID NOT NULL,
    transport TEXT NOT NULL,
    address TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, device_id, endpoint_id)
);

CREATE TABLE IF NOT EXISTS device_capabilities (
    tenant_id UUID NOT NULL,
    device_id UUID NOT NULL,
    capability_key TEXT NOT NULL,
    capability_value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, device_id, capability_key)
);

CREATE TABLE IF NOT EXISTS channels (
    tenant_id UUID NOT NULL,
    device_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    status TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, device_id, channel_id)
);

CREATE INDEX IF NOT EXISTS idx_channels_device
    ON channels (tenant_id, device_id, deleted);

CREATE TABLE IF NOT EXISTS operations (
    tenant_id UUID NOT NULL,
    operation_id UUID NOT NULL PRIMARY KEY,
    device_id UUID NOT NULL,
    principal_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    status TEXT NOT NULL,
    result_type TEXT,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deadline TIMESTAMPTZ,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_operations_idempotency
    ON operations (tenant_id, principal_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_operations_status_deadline
    ON operations (tenant_id, status, deadline)
    WHERE status IN ('pending', 'running');

CREATE TABLE IF NOT EXISTS operation_steps (
    tenant_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    attempt INTEGER NOT NULL,
    owner_epoch BIGINT NOT NULL,
    status TEXT NOT NULL,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    data JSONB NOT NULL,
    PRIMARY KEY (tenant_id, operation_id, attempt)
);

CREATE TABLE IF NOT EXISTS media_sessions (
    tenant_id UUID NOT NULL,
    media_session_id UUID NOT NULL PRIMARY KEY,
    device_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    operation_id UUID NOT NULL,
    principal_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    purpose TEXT NOT NULL,
    state TEXT NOT NULL,
    desired_state TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deadline TIMESTAMPTZ,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_media_sessions_idempotency
    ON media_sessions (tenant_id, principal_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_media_sessions_state_deadline
    ON media_sessions (tenant_id, state, deadline)
    WHERE state NOT IN ('closed', 'failed');

CREATE TABLE IF NOT EXISTS media_bindings (
    tenant_id UUID NOT NULL,
    media_binding_id UUID NOT NULL PRIMARY KEY,
    media_session_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    media_node_id UUID NOT NULL,
    owner_epoch BIGINT NOT NULL DEFAULT 0,
    state TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_media_bindings_session
    ON media_bindings (tenant_id, media_session_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_media_bindings_active_session
    ON media_bindings (tenant_id, media_session_id)
    WHERE state NOT IN ('released', 'failed');

CREATE TABLE IF NOT EXISTS device_owners (
    tenant_id UUID NOT NULL,
    device_id UUID NOT NULL PRIMARY KEY,
    owner_node_id UUID NOT NULL,
    owner_epoch BIGINT NOT NULL DEFAULT 0,
    expires_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (tenant_id, device_id) REFERENCES devices (tenant_id, device_id)
);

CREATE INDEX IF NOT EXISTS idx_device_owners_expires
    ON device_owners (expires_at);

CREATE TABLE IF NOT EXISTS outbox_events (
    event_id UUID NOT NULL PRIMARY KEY,
    tenant_id UUID NOT NULL,
    aggregate_ref JSONB NOT NULL,
    aggregate_sequence BIGINT NOT NULL,
    payload JSONB NOT NULL,
    published BOOLEAN NOT NULL DEFAULT FALSE,
    occurred_at TIMESTAMPTZ NOT NULL,
    correlation_id UUID NOT NULL,
    causation_id UUID NOT NULL,
    source UUID NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON outbox_events (tenant_id, published, occurred_at)
    WHERE published = FALSE;

CREATE TABLE IF NOT EXISTS processed_messages (
    tenant_id UUID NOT NULL,
    message_id UUID NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, message_id)
);

CREATE TABLE IF NOT EXISTS plugin_instances (
    tenant_id UUID NOT NULL,
    plugin_id UUID NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    data JSONB NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (tenant_id, plugin_id)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID NOT NULL PRIMARY KEY,
    tenant_id UUID NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    target_id UUID NOT NULL,
    result TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    data JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_tenant
    ON audit_logs (tenant_id, occurred_at);
