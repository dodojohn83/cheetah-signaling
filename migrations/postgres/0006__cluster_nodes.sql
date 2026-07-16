CREATE TABLE IF NOT EXISTS cluster_nodes (
    node_id UUID NOT NULL PRIMARY KEY,
    instance_id UUID NOT NULL,
    zone TEXT NOT NULL,
    version TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    draining BOOLEAN NOT NULL DEFAULT FALSE,
    contract_versions JSONB NOT NULL,
    capacity JSONB NOT NULL,
    load JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cluster_nodes_lease
    ON cluster_nodes (lease_until);

CREATE INDEX IF NOT EXISTS idx_cluster_nodes_zone
    ON cluster_nodes (zone);
