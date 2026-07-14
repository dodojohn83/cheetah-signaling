//! NATS subject helpers for tenant-scoped command and event streams.

use cheetah_signal_types::{NodeId, TenantId};

const COMMAND_PREFIX: &str = "sig.v1.command";
const EVENT_PREFIX: &str = "sig.v1.event";

/// Returns a stable bucket key for a tenant used to partition streams.
///
/// The bucket is the two-character hex encoding of the first byte of the
/// tenant UUID. This keeps stream subjects bounded while still spreading load.
pub fn tenant_bucket(tenant_id: &TenantId) -> String {
    let bytes = tenant_id.as_bytes();
    format!("{:02x}", bytes[0])
}

/// Returns the subject for a command addressed to a specific owner node.
pub fn command_subject(tenant_id: TenantId, owner_node_id: NodeId) -> String {
    format!(
        "{}.{}.{}",
        COMMAND_PREFIX,
        tenant_bucket(&tenant_id),
        owner_node_id
    )
}

/// Returns the subject for a tenant-scoped event of the given type.
pub fn event_subject(tenant_id: TenantId, event_type: &str) -> String {
    format!("{}.{}.{}", EVENT_PREFIX, tenant_bucket(&tenant_id), event_type)
}
