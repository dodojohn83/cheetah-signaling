//! Parent device resolution helpers used during channel import.

use crate::error::MigrationError;
use crate::mappers::{ParentProtocols, parse_protocol};
use crate::model::{EntityType, OldRecord};
use cheetah_domain::{DeviceRepository, Protocol};
use cheetah_signal_types::{DeviceId, ProtocolIdentity, TenantId};

/// Builds a map from old-system `(tenant, external_id)` to the protocol of
/// device-like records. This is used to infer the parent protocol for channel
/// records when the parent protocol is not explicitly recorded on the channel.
pub(crate) fn build_parent_protocols(records: &[OldRecord]) -> ParentProtocols {
    let mut map = ParentProtocols::new();
    for record in records {
        if record.validate().is_err() {
            continue;
        }
        if matches!(
            record.entity_type,
            EntityType::Device | EntityType::Gb28181Platform | EntityType::OnvifEndpoint
        ) && let Ok(protocol) = parse_protocol(&record.protocol)
        {
            map.insert(
                (record.tenant_id.clone(), record.external_id.clone()),
                protocol,
            );
        }
    }
    map
}

/// Caches lookups from `(tenant, protocol, external_id)` to the actual
/// persisted device id so a parent device referenced by many channels is
/// only queried once per import run.
pub(crate) struct ParentDeviceResolver {
    cache: std::collections::HashMap<(TenantId, Protocol, String), Option<DeviceId>>,
}

impl ParentDeviceResolver {
    pub(crate) fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    pub(crate) async fn resolve(
        &mut self,
        repo: &mut dyn DeviceRepository,
        tenant_id: TenantId,
        protocol: Protocol,
        external_id: &ProtocolIdentity,
    ) -> Result<Option<DeviceId>, MigrationError> {
        let key = (tenant_id, protocol, external_id.as_str().to_string());
        if let Some(cached) = self.cache.get(&key) {
            return Ok(*cached);
        }

        let device = repo
            .get_by_external_id(tenant_id, protocol, external_id.clone())
            .await?;
        let id = device.map(|d| d.device_id());
        self.cache.insert(key, id);
        Ok(id)
    }
}
