//! Mapping from old-system records to Cheetah domain aggregates.

use crate::error::MigrationError;
use crate::model::{EntityType, OldRecord};
use cheetah_domain::{
    Channel, ChannelKind, ChannelStatus, Device, DeviceKind, DomainEvent, Protocol, PtzCapabilities,
};
use cheetah_signal_types::{
    ChannelId, Clock, CorrelationId, DeviceId, Event, EventId, MessageId, NodeId, ProtocolIdentity,
    ResourceId, ResourceKind, ResourceRef, TenantId,
};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// Map from (tenant_id, external_id) to the protocol used by the parent device.
pub(crate) type ParentProtocols = HashMap<(String, String), Protocol>;

/// Namespace used for deterministic v5 UUIDs derived from old-system identifiers.
const MIGRATION_NAMESPACE: Uuid = Uuid::from_bytes([
    0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f, 0x70, 0x81, 0x92, 0xa3, 0xb4, 0xc5, 0xd6, 0xe7, 0xf8, 0x09,
]);

/// A domain object produced by a mapper, plus any action items for the operator.
#[derive(Debug)]
pub struct MappedEntity {
    /// The resulting domain aggregate, if any.
    pub entity: MappedAggregate,
    /// Human-readable action items (e.g. missing credentials).
    pub actions: Vec<String>,
    /// Outbox events produced alongside the aggregate.
    pub events: Vec<Event<DomainEvent>>,
}

/// Domain aggregate variants produced by mappers.
#[derive(Debug)]
pub enum MappedAggregate {
    /// Device aggregate.
    Device(Device),
    /// Channel aggregate.
    Channel(Channel),
    /// Tenant placeholder (no aggregate, only action items).
    Tenant,
    /// Secret reference placeholder (no aggregate).
    SecretReference,
    /// GB28181 platform mapped as a device with `Platform` kind.
    Gb28181Platform(Device),
    /// ONVIF endpoint mapped as a device with `Camera` kind.
    OnvifEndpoint(Device),
    /// Unknown or skipped entity.
    Skipped,
}

/// Maps a source record into a domain aggregate.
pub fn map_record(
    clock: &dyn Clock,
    record: &OldRecord,
    parent_protocols: &ParentProtocols,
) -> Result<MappedEntity, MigrationError> {
    match record.entity_type {
        EntityType::Tenant => Ok(MappedEntity {
            entity: MappedAggregate::Tenant,
            actions: Vec::new(),
            events: Vec::new(),
        }),
        EntityType::Device | EntityType::Gb28181Platform | EntityType::OnvifEndpoint => {
            map_device_like(clock, record)
        }
        EntityType::Channel => map_channel(clock, record, parent_protocols),
        EntityType::SecretReference => Ok(MappedEntity {
            entity: MappedAggregate::SecretReference,
            actions: Vec::new(),
            events: Vec::new(),
        }),
        EntityType::Unknown => Ok(MappedEntity {
            entity: MappedAggregate::Skipped,
            actions: Vec::new(),
            events: Vec::new(),
        }),
    }
}

fn map_device_like(clock: &dyn Clock, record: &OldRecord) -> Result<MappedEntity, MigrationError> {
    if record.external_id.is_empty() {
        return Err(MigrationError::InvalidRecord {
            row: 0,
            message: "device-like record missing external_id".to_string(),
        });
    }
    if record.tenant_id.is_empty() {
        return Err(MigrationError::InvalidRecord {
            row: 0,
            message: "device-like record missing tenant_id".to_string(),
        });
    }

    let tenant_id = stable_tenant_id(&record.tenant_id);
    let protocol = parse_protocol(&record.protocol)?;
    let device_id = stable_device_id(&record.tenant_id, &record.external_id, protocol);
    let external_id = ProtocolIdentity::new(&record.external_id)?;
    let kind = parse_device_kind(&record.kind, record.entity_type);
    let metadata = extract_metadata(record);

    let (device, domain_event) = Device::new(
        clock,
        tenant_id,
        device_id,
        protocol,
        external_id,
        record.authority.clone(),
        record.name.clone(),
        kind,
        Vec::new(),
        metadata,
    )?;

    let event = event_for(
        clock,
        tenant_id,
        ResourceKind::Device,
        ResourceId::Device(device_id),
        domain_event,
        0,
    );

    let actions = if record.has_secret() {
        vec![format!(
            "device {} has secrets that must be re-entered: {}",
            record.external_id,
            record.secret_field_names().join(", ")
        )]
    } else {
        Vec::new()
    };

    let aggregate = match record.entity_type {
        EntityType::Gb28181Platform => MappedAggregate::Gb28181Platform(device),
        EntityType::OnvifEndpoint => MappedAggregate::OnvifEndpoint(device),
        _ => MappedAggregate::Device(device),
    };

    Ok(MappedEntity {
        entity: aggregate,
        actions,
        events: vec![event],
    })
}

fn map_channel(
    clock: &dyn Clock,
    record: &OldRecord,
    parent_protocols: &ParentProtocols,
) -> Result<MappedEntity, MigrationError> {
    if record.external_id.is_empty() {
        return Err(MigrationError::InvalidRecord {
            row: 0,
            message: "channel record missing external_id".to_string(),
        });
    }
    if record.parent_device_id.is_empty() {
        return Err(MigrationError::InvalidRecord {
            row: 0,
            message: format!("channel {} missing parent_device_id", record.external_id),
        });
    }
    if record.tenant_id.is_empty() {
        return Err(MigrationError::InvalidRecord {
            row: 0,
            message: format!("channel {} missing tenant_id", record.external_id),
        });
    }

    let tenant_id = stable_tenant_id(&record.tenant_id);
    let parent_protocol = parent_protocols
        .get(&(record.tenant_id.clone(), record.parent_device_id.clone()))
        .copied()
        .unwrap_or_else(|| parse_protocol(&record.protocol).unwrap_or(Protocol::Gb28181));
    let device_id = stable_device_id(&record.tenant_id, &record.parent_device_id, parent_protocol);
    let channel_id = stable_channel_id(
        &record.tenant_id,
        &record.parent_device_id,
        &record.external_id,
        parent_protocol,
    );
    let kind = parse_channel_kind(&record.channel_kind);
    let metadata = extract_metadata(record);

    let (channel, domain_event) = Channel::new(
        clock,
        tenant_id,
        device_id,
        channel_id,
        kind,
        record.name.clone(),
        record.enabled,
        Some(ChannelStatus::Offline),
        Vec::new(),
        PtzCapabilities::default(),
        metadata,
    )?;

    let event = event_for(
        clock,
        tenant_id,
        ResourceKind::Channel,
        ResourceId::Channel(channel_id),
        domain_event,
        0,
    );

    let actions = if record.has_secret() {
        vec![format!(
            "channel {} has secrets that must be re-entered: {}",
            record.external_id,
            record.secret_field_names().join(", ")
        )]
    } else {
        Vec::new()
    };

    Ok(MappedEntity {
        entity: MappedAggregate::Channel(channel),
        actions,
        events: vec![event],
    })
}

/// Wraps a domain event into an outbox [`Event`] envelope.
pub(crate) fn event_for(
    clock: &dyn Clock,
    tenant_id: TenantId,
    kind: ResourceKind,
    id: ResourceId,
    payload: DomainEvent,
    aggregate_sequence: u64,
) -> Event<DomainEvent> {
    Event {
        event_id: EventId::generate(),
        tenant_id,
        aggregate_ref: ResourceRef {
            tenant_id,
            kind,
            id,
        },
        aggregate_sequence,
        occurred_at: clock.now_wall(),
        correlation_id: CorrelationId::generate(),
        causation_id: MessageId::generate(),
        source: NodeId::from_uuid(Uuid::nil()),
        payload,
    }
}

/// Derives a deterministic `TenantId` from the old-system tenant name.
fn stable_tenant_id(name: &str) -> TenantId {
    TenantId::from_uuid(Uuid::new_v5(&MIGRATION_NAMESPACE, name.as_bytes()))
}

fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Gb28181 => "gb28181",
        Protocol::Onvif => "onvif",
        Protocol::Mqtt => "mqtt",
        Protocol::Jt808 => "jt808",
        Protocol::Isup => "isup",
        Protocol::Plugin => "plugin",
        Protocol::Unknown => "unknown",
        _ => "unknown",
    }
}

/// Derives a deterministic `DeviceId` from tenant + protocol + external identity.
fn stable_device_id(tenant_id: &str, external_id: &str, protocol: Protocol) -> DeviceId {
    let input = format!("{tenant_id}:{external_id}:{}", protocol_name(protocol));
    DeviceId::from_uuid(Uuid::new_v5(&MIGRATION_NAMESPACE, input.as_bytes()))
}

/// Derives a deterministic `ChannelId` from tenant + protocol + device + channel identity.
fn stable_channel_id(
    tenant_id: &str,
    device_external_id: &str,
    channel_external_id: &str,
    protocol: Protocol,
) -> ChannelId {
    let input = format!(
        "{tenant_id}:{device_external_id}:{channel_external_id}:{}",
        protocol_name(protocol)
    );
    ChannelId::from_uuid(Uuid::new_v5(&MIGRATION_NAMESPACE, input.as_bytes()))
}

pub(crate) fn parse_protocol(value: &str) -> Result<Protocol, MigrationError> {
    match value.to_lowercase().as_str() {
        "gb28181" => Ok(Protocol::Gb28181),
        "onvif" => Ok(Protocol::Onvif),
        "mqtt" => Ok(Protocol::Mqtt),
        "jt808" => Ok(Protocol::Jt808),
        "isup" => Ok(Protocol::Isup),
        "" => Ok(Protocol::Gb28181),
        _ => Err(MigrationError::InvalidRecord {
            row: 0,
            message: format!("unknown protocol: {value}"),
        }),
    }
}

fn parse_device_kind(value: &str, entity_type: EntityType) -> DeviceKind {
    let kind = match value.to_lowercase().as_str() {
        "camera" => DeviceKind::Camera,
        "nvr" => DeviceKind::Nvr,
        "dvr" => DeviceKind::Dvr,
        "encoder" => DeviceKind::Encoder,
        "iot" => DeviceKind::Iot,
        "platform" => DeviceKind::Platform,
        _ => DeviceKind::Unknown,
    };
    if kind == DeviceKind::Unknown {
        match entity_type {
            EntityType::Gb28181Platform => DeviceKind::Platform,
            EntityType::OnvifEndpoint => DeviceKind::Camera,
            _ => DeviceKind::Camera,
        }
    } else {
        kind
    }
}

fn parse_channel_kind(value: &str) -> ChannelKind {
    match value.to_lowercase().as_str() {
        "video" => ChannelKind::Video,
        "audio" => ChannelKind::Audio,
        "ptz" => ChannelKind::Ptz,
        "organization" => ChannelKind::Organization,
        "event" => ChannelKind::Event,
        "io" => ChannelKind::Io,
        _ => ChannelKind::Video,
    }
}

fn extract_metadata(record: &OldRecord) -> BTreeMap<String, String> {
    let secret_names: std::collections::HashSet<&str> =
        record.secret_field_names().into_iter().collect();
    record
        .metadata
        .iter()
        .filter(|(k, _)| !is_core_field(k) && !secret_names.contains(k.as_str()))
        .map(|(k, v)| {
            let s = v.as_str().map_or_else(|| v.to_string(), String::from);
            (k.clone(), s)
        })
        .collect()
}

fn is_core_field(key: &str) -> bool {
    matches!(
        key,
        "entity_type"
            | "tenant_id"
            | "external_id"
            | "name"
            | "protocol"
            | "kind"
            | "authority"
            | "parent_device_id"
            | "channel_kind"
            | "enabled"
            | "secret_fields"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use std::collections::BTreeMap;

    fn record(entity_type: EntityType, external_id: &str, tenant_id: &str) -> OldRecord {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "location".to_string(),
            serde_json::Value::String("hallway".to_string()),
        );
        OldRecord {
            entity_type,
            tenant_id: tenant_id.to_string(),
            external_id: external_id.to_string(),
            name: "Test".to_string(),
            protocol: "gb28181".to_string(),
            kind: "camera".to_string(),
            authority: "192.0.2.1:5060".to_string(),
            parent_device_id: String::new(),
            channel_kind: String::new(),
            enabled: true,
            metadata,
            secret_fields: String::new(),
        }
    }

    #[test]
    fn map_device_creates_deterministic_id() -> Result<(), MigrationError> {
        let clock = SystemClock::new();
        let mut r = record(EntityType::Device, "cam-01", "tenant-a");
        r.parent_device_id = "cam-01".to_string();
        let entity = map_record(&clock, &r, &ParentProtocols::new())?;
        match entity.entity {
            MappedAggregate::Device(d) => {
                assert_eq!(d.external_id().as_str(), "cam-01");
                assert_eq!(d.name(), "Test");
                assert_eq!(d.kind(), DeviceKind::Camera);
                assert_eq!(
                    d.metadata().get("location").map(String::as_str),
                    Some("hallway")
                );
                // Stable id means the same input always maps to the same UUID.
                let id = d.device_id();
                let entity2 = map_record(&clock, &r, &ParentProtocols::new())?;
                match entity2.entity {
                    MappedAggregate::Device(d2) => assert_eq!(d2.device_id(), id),
                    _ => panic!("expected device"),
                }
            }
            _ => panic!("expected device aggregate"),
        }
        Ok(())
    }

    #[test]
    fn map_device_with_secret_emits_action() -> Result<(), MigrationError> {
        let clock = SystemClock::new();
        let mut r = record(EntityType::Device, "cam-02", "tenant-a");
        r.secret_fields = "password,api_key".to_string();
        let entity = map_record(&clock, &r, &ParentProtocols::new())?;
        assert!(!entity.actions.is_empty());
        assert!(entity.actions[0].contains("password"));
        assert!(entity.actions[0].contains("api_key"));
        Ok(())
    }

    #[test]
    fn map_device_with_secret_excludes_secret_from_metadata() -> Result<(), MigrationError> {
        let clock = SystemClock::new();
        let mut r = record(EntityType::Device, "cam-02", "tenant-a");
        r.parent_device_id = "cam-02".to_string();
        r.secret_fields = "password,api_key".to_string();
        r.metadata.insert(
            "password".to_string(),
            serde_json::Value::String("hunter2".to_string()),
        );
        r.metadata.insert(
            "api_key".to_string(),
            serde_json::Value::String("abc".to_string()),
        );
        r.metadata.insert(
            "note".to_string(),
            serde_json::Value::String("keep me".to_string()),
        );

        let entity = map_record(&clock, &r, &ParentProtocols::new())?;
        match entity.entity {
            MappedAggregate::Device(d) => {
                assert!(d.metadata().get("password").is_none());
                assert!(d.metadata().get("api_key").is_none());
                assert_eq!(
                    d.metadata().get("note").map(String::as_str),
                    Some("keep me")
                );
            }
            _ => panic!("expected device aggregate"),
        }
        Ok(())
    }

    #[test]
    fn map_channel_requires_parent() {
        let clock = SystemClock::new();
        let mut r = record(EntityType::Channel, "ch-01", "tenant-a");
        r.parent_device_id = String::new();
        assert!(map_record(&clock, &r, &ParentProtocols::new()).is_err());
    }

    #[test]
    fn map_channel_with_secret_emits_action_and_excludes_secret() -> Result<(), MigrationError> {
        let clock = SystemClock::new();
        let mut r = record(EntityType::Channel, "ch-02", "tenant-a");
        r.parent_device_id = "cam-01".to_string();
        r.secret_fields = "stream_key".to_string();
        r.metadata.insert(
            "stream_key".to_string(),
            serde_json::Value::String("topsecret".to_string()),
        );

        let entity = map_record(&clock, &r, &ParentProtocols::new())?;
        match entity.entity {
            MappedAggregate::Channel(c) => {
                assert!(c.metadata().get("stream_key").is_none());
            }
            _ => panic!("expected channel aggregate"),
        }
        assert!(!entity.actions.is_empty());
        assert!(entity.actions[0].contains("stream_key"));
        Ok(())
    }

    #[test]
    fn map_channel_uses_parent_device_protocol() -> Result<(), MigrationError> {
        let clock = SystemClock::new();
        let mut device = record(EntityType::Device, "cam-01", "tenant-a");
        device.protocol = "onvif".to_string();
        device.kind = "camera".to_string();

        let mut parent_protocols = ParentProtocols::new();
        let device_entity = map_record(&clock, &device, &parent_protocols)?;
        let parent_device_id = match device_entity.entity {
            MappedAggregate::Device(d) => d.device_id(),
            _ => panic!("expected device aggregate"),
        };
        parent_protocols.insert(
            ("tenant-a".to_string(), "cam-01".to_string()),
            Protocol::Onvif,
        );

        let mut channel = record(EntityType::Channel, "ch-01", "tenant-a");
        channel.parent_device_id = "cam-01".to_string();
        channel.protocol = "".to_string();

        let channel_entity = map_record(&clock, &channel, &parent_protocols)?;
        match channel_entity.entity {
            MappedAggregate::Channel(c) => assert_eq!(c.device_id(), parent_device_id),
            _ => panic!("expected channel aggregate"),
        }
        Ok(())
    }
}
