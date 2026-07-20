CREATE TABLE IF NOT EXISTS media_nodes (
    node_id BLOB PRIMARY KEY,
    instance_id TEXT NOT NULL,
    instance_epoch INTEGER NOT NULL,
    zone TEXT NOT NULL,
    region TEXT NOT NULL,
    network_zones TEXT NOT NULL DEFAULT '[]',
    labels TEXT NOT NULL DEFAULT '{}',
    control_endpoint TEXT NOT NULL,
    media_addresses TEXT NOT NULL DEFAULT '[]',
    capabilities TEXT NOT NULL DEFAULT '[]',
    capacity TEXT NOT NULL DEFAULT '{}',
    load INTEGER NOT NULL DEFAULT 0,
    session_count INTEGER NOT NULL DEFAULT 0,
    draining INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    last_heartbeat_at INTEGER,
    lease_until INTEGER,
    generation INTEGER NOT NULL DEFAULT 1,
    contract_version INTEGER NOT NULL DEFAULT 1,
    revision INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_nodes_lease ON media_nodes(lease_until);
CREATE INDEX IF NOT EXISTS idx_media_nodes_updated ON media_nodes(updated_at, node_id);
