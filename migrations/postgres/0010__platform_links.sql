CREATE TABLE IF NOT EXISTS platform_links (
    platform_link_id UUID NOT NULL PRIMARY KEY,
    tenant_id UUID NOT NULL,
    direction TEXT NOT NULL,
    local_identity TEXT NOT NULL,
    remote_identity TEXT NOT NULL,
    desired_state TEXT NOT NULL DEFAULT 'unregistered',
    actual_state TEXT NOT NULL DEFAULT 'idle',
    owner_epoch BIGINT NOT NULL DEFAULT 0,
    link_generation BIGINT NOT NULL DEFAULT 0,
    revision BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    data JSONB NOT NULL,
    schema_version BIGINT NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_platform_links_remote
    ON platform_links(tenant_id, direction, remote_identity);
CREATE INDEX IF NOT EXISTS idx_platform_links_updated
    ON platform_links(tenant_id, updated_at, platform_link_id);
