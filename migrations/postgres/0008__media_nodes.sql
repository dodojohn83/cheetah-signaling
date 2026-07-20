CREATE TABLE IF NOT EXISTS media_nodes (
    node_id UUID PRIMARY KEY,
    instance_id TEXT NOT NULL,
    instance_epoch BIGINT NOT NULL,
    zone TEXT NOT NULL,
    region TEXT NOT NULL,
    network_zones JSONB NOT NULL DEFAULT '[]'::jsonb,
    labels JSONB NOT NULL DEFAULT '{}'::jsonb,
    control_endpoint TEXT NOT NULL,
    media_addresses JSONB NOT NULL DEFAULT '[]'::jsonb,
    capabilities JSONB NOT NULL DEFAULT '[]'::jsonb,
    capacity JSONB NOT NULL DEFAULT '{}'::jsonb,
    load BIGINT NOT NULL DEFAULT 0,
    session_count BIGINT NOT NULL DEFAULT 0,
    draining BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL DEFAULT 'active',
    last_heartbeat_at BIGINT,
    lease_until BIGINT,
    generation BIGINT NOT NULL DEFAULT 1,
    contract_version BIGINT NOT NULL DEFAULT 1,
    revision BIGINT NOT NULL DEFAULT 0,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_nodes_lease ON media_nodes(lease_until);
CREATE INDEX IF NOT EXISTS idx_media_nodes_updated ON media_nodes(updated_at, node_id);
