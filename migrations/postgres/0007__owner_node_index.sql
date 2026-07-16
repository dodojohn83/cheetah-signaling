CREATE INDEX IF NOT EXISTS idx_device_owners_owner_node
    ON device_owners (owner_node_id, updated_at, device_id);
