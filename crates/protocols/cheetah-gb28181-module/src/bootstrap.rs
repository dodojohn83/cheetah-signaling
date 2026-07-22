//! GB28181 bootstrap query operation helpers.
//!
//! Generates the `CommandPayload::Query` payloads and stable idempotency keys
//! used to submit automatic Catalog / DeviceInfo / DeviceStatus queries when a
//! GB28181 device comes online. This module stays in the Sans-I/O protocol
//! layer and does not perform database or NATS I/O.

use cheetah_domain::{CommandPayload, QueryCommand, QueryKind};
use cheetah_signal_types::{DeviceId, OwnerEpoch, TenantId};

/// Kinds of query commands automatically issued after a successful REGISTER.
const BOOTSTRAP_QUERY_KINDS: &[QueryKind] = &[
    QueryKind::Catalog,
    QueryKind::DeviceInfo,
    QueryKind::DeviceStatus,
];

/// Returns the ordered set of query kinds to issue when a device comes online.
pub fn bootstrap_query_kinds() -> &'static [QueryKind] {
    BOOTSTRAP_QUERY_KINDS
}

/// Builds a `CommandPayload::Query` for the given bootstrap query kind.
pub fn bootstrap_query_payload(kind: QueryKind) -> CommandPayload {
    CommandPayload::Query {
        query: QueryCommand {
            kind,
            channel_id: None,
            start_time: None,
            end_time: None,
            config_type: None,
            scale: None,
        },
    }
}

/// Builds a stable, scoped idempotency key for a bootstrap query.
///
/// The key embeds tenant, device, owner epoch, registration sequence and query
/// kind so that the same registration does not spawn duplicate operations, while
/// a new owner or a new registration still creates fresh operations.
pub fn bootstrap_idempotency_key(
    tenant_id: TenantId,
    device_id: DeviceId,
    owner_epoch: OwnerEpoch,
    registration_sequence: u64,
    kind: QueryKind,
) -> String {
    format!(
        "gb28181:bootstrap:{tenant}:{device}:{owner_epoch}:{registration_sequence}:{kind}",
        tenant = tenant_id.as_uuid(),
        device = device_id.as_uuid(),
        owner_epoch = owner_epoch.0,
        kind = kind.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_query_payloads_match_kinds() {
        for kind in bootstrap_query_kinds() {
            let payload = bootstrap_query_payload(*kind);
            assert_eq!(
                payload,
                CommandPayload::Query {
                    query: QueryCommand {
                        kind: *kind,
                        channel_id: None,
                        start_time: None,
                        end_time: None,
                        config_type: None,
                        scale: None,
                    },
                }
            );
        }
    }

    #[test]
    fn idempotency_keys_are_stable_and_isolated() {
        let tenant_id = TenantId::generate();
        let device_id = DeviceId::generate();
        let owner_epoch = OwnerEpoch(5);
        let registration_sequence = 7;

        let catalog1 = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence,
            QueryKind::Catalog,
        );
        let catalog2 = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence,
            QueryKind::Catalog,
        );
        assert_eq!(catalog1, catalog2, "idempotency key must be stable");

        let device_info = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence,
            QueryKind::DeviceInfo,
        );
        let device_status = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence,
            QueryKind::DeviceStatus,
        );
        assert_ne!(catalog1, device_info);
        assert_ne!(catalog1, device_status);
        assert_ne!(device_info, device_status);

        let other_device = bootstrap_idempotency_key(
            tenant_id,
            DeviceId::generate(),
            owner_epoch,
            registration_sequence,
            QueryKind::Catalog,
        );
        assert_ne!(catalog1, other_device);

        let other_tenant = bootstrap_idempotency_key(
            TenantId::generate(),
            device_id,
            owner_epoch,
            registration_sequence,
            QueryKind::Catalog,
        );
        assert_ne!(catalog1, other_tenant);

        let other_epoch = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            OwnerEpoch(6),
            registration_sequence,
            QueryKind::Catalog,
        );
        assert_ne!(catalog1, other_epoch);

        let other_sequence = bootstrap_idempotency_key(
            tenant_id,
            device_id,
            owner_epoch,
            registration_sequence + 1,
            QueryKind::Catalog,
        );
        assert_ne!(catalog1, other_sequence);
    }
}
