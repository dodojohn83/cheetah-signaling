CREATE TABLE IF NOT EXISTS cluster_nodes (
    node_id BLOB NOT NULL PRIMARY KEY,
    instance_id BLOB NOT NULL,
    zone TEXT NOT NULL,
    version TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    lease_until INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    draining INTEGER NOT NULL DEFAULT 0,
    contract_versions TEXT NOT NULL,
    capacity TEXT NOT NULL,
    load TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cluster_nodes_lease
    ON cluster_nodes (lease_until);

CREATE INDEX IF NOT EXISTS idx_cluster_nodes_zone
    ON cluster_nodes (zone);
