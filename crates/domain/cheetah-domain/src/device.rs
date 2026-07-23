//! Device aggregate, value objects and lifecycle.

use crate::{DomainError, DomainEvent};
use cheetah_signal_types::{
    Clock, DeviceId, OwnerEpoch, ProtocolIdentity, Revision, TenantId, UtcTimestamp,
};
use std::collections::{BTreeMap, HashSet};

/// Maximum number of capabilities per device.
pub(crate) const MAX_CAPABILITIES: usize = 256;
/// Maximum length of a capability key in bytes.
pub(crate) const MAX_CAPABILITY_KEY_BYTES: usize = 128;
/// Maximum length of a string capability value in bytes.
pub(crate) const MAX_CAPABILITY_STRING_VALUE_BYTES: usize = 1024;
/// Maximum number of items in a string list capability value.
pub(crate) const MAX_STRING_LIST_ITEMS: usize = 64;
/// Maximum length of a string list item in bytes.
pub(crate) const MAX_STRING_LIST_ITEM_BYTES: usize = 256;
/// Maximum number of metadata entries.
pub(crate) const MAX_METADATA_KEYS: usize = 64;
/// Maximum length of a metadata key in bytes.
pub(crate) const MAX_METADATA_KEY_BYTES: usize = 128;
/// Maximum length of a metadata value in bytes.
pub(crate) const MAX_METADATA_VALUE_BYTES: usize = 4096;
/// Maximum length of a device display name in bytes.
pub(crate) const MAX_DEVICE_NAME_BYTES: usize = 1024;
/// Maximum length of a device authority in bytes.
pub(crate) const MAX_DEVICE_AUTHORITY_BYTES: usize = 256;
/// Maximum length of an online/offline reason in bytes.
pub(crate) const MAX_DEVICE_REASON_BYTES: usize = 1024;

/// Device kind.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeviceKind {
    /// Unknown device kind.
    #[default]
    Unknown,
    /// Network camera.
    Camera,
    /// Network video recorder.
    Nvr,
    /// Digital video recorder.
    Dvr,
    /// Encoder.
    Encoder,
    /// IoT device.
    Iot,
    /// Platform/gateway.
    Platform,
}

impl std::fmt::Display for DeviceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Camera => "camera",
            Self::Nvr => "nvr",
            Self::Dvr => "dvr",
            Self::Encoder => "encoder",
            Self::Iot => "iot",
            Self::Platform => "platform",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for DeviceKind {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("camera") {
            Ok(Self::Camera)
        } else if s.eq_ignore_ascii_case("nvr") {
            Ok(Self::Nvr)
        } else if s.eq_ignore_ascii_case("dvr") {
            Ok(Self::Dvr)
        } else if s.eq_ignore_ascii_case("encoder") {
            Ok(Self::Encoder)
        } else if s.eq_ignore_ascii_case("iot") {
            Ok(Self::Iot)
        } else if s.eq_ignore_ascii_case("platform") {
            Ok(Self::Platform)
        } else {
            let display = crate::truncate_for_error(s);
            Err(DomainError::invalid_argument(format!(
                "unknown device kind: {display}"
            )))
        }
    }
}

/// Protocol used by a device to register.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Protocol {
    /// Unknown protocol.
    #[default]
    Unknown,
    /// GB/T 28181.
    Gb28181,
    /// ONVIF.
    Onvif,
    /// Plugin adapter.
    Plugin,
    /// MQTT.
    Mqtt,
    /// JT808.
    Jt808,
    /// ISUP.
    Isup,
    /// HomeKit.
    Homekit,
    /// Matter.
    Matter,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Gb28181 => "gb28181",
            Self::Onvif => "onvif",
            Self::Plugin => "plugin",
            Self::Mqtt => "mqtt",
            Self::Jt808 => "jt808",
            Self::Isup => "isup",
            Self::Homekit => "homekit",
            Self::Matter => "matter",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for Protocol {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("gb28181") {
            Ok(Self::Gb28181)
        } else if s.eq_ignore_ascii_case("onvif") {
            Ok(Self::Onvif)
        } else if s.eq_ignore_ascii_case("plugin") {
            Ok(Self::Plugin)
        } else if s.eq_ignore_ascii_case("mqtt") {
            Ok(Self::Mqtt)
        } else if s.eq_ignore_ascii_case("jt808") {
            Ok(Self::Jt808)
        } else if s.eq_ignore_ascii_case("isup") {
            Ok(Self::Isup)
        } else if s.eq_ignore_ascii_case("homekit") {
            Ok(Self::Homekit)
        } else if s.eq_ignore_ascii_case("matter") {
            Ok(Self::Matter)
        } else {
            let display = crate::truncate_for_error(s);
            Err(DomainError::invalid_argument(format!(
                "unknown protocol: {display}"
            )))
        }
    }
}

/// Device lifecycle.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeviceLifecycle {
    /// Device is being provisioned.
    #[default]
    Provisioning,
    /// Device is active and may be used.
    Active,
    /// Device is suspended.
    Suspended,
    /// Device is retired and immutable.
    Retired,
}

impl std::fmt::Display for DeviceLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Retired => "retired",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for DeviceLifecycle {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("provisioning") {
            Ok(Self::Provisioning)
        } else if s.eq_ignore_ascii_case("active") {
            Ok(Self::Active)
        } else if s.eq_ignore_ascii_case("suspended") {
            Ok(Self::Suspended)
        } else if s.eq_ignore_ascii_case("retired") {
            Ok(Self::Retired)
        } else {
            let display = crate::truncate_for_error(s);
            Err(DomainError::invalid_argument(format!(
                "unknown lifecycle: {display}"
            )))
        }
    }
}

/// Device connectivity.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum Connectivity {
    /// Unknown connectivity.
    #[default]
    Unknown,
    /// Device is online.
    Online,
    /// Device is offline.
    Offline {
        /// When the device went offline.
        since: UtcTimestamp,
        /// Reason for being offline.
        reason: String,
    },
}

/// A device capability with a typed value.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Capability {
    key: String,
    value: CapabilityValue,
}

impl Capability {
    /// Creates a new capability after validating the key and value.
    pub fn new(key: impl Into<String>, value: CapabilityValue) -> crate::Result<Self> {
        let key = key.into();
        value.validate()?;
        if key.is_empty() || key.len() > MAX_CAPABILITY_KEY_BYTES {
            return Err(DomainError::invalid_argument("invalid capability key"));
        }
        Ok(Self { key, value })
    }

    /// Returns the capability key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the typed value.
    pub fn value(&self) -> &CapabilityValue {
        &self.value
    }
}

/// Typed capability value.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CapabilityValue {
    /// String value.
    String(String),
    /// List of strings.
    StringList(Vec<String>),
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Int(i64),
}

impl CapabilityValue {
    /// Creates a string value.
    pub fn new_string(value: impl Into<String>) -> crate::Result<Self> {
        let value = value.into();
        if value.len() > MAX_CAPABILITY_STRING_VALUE_BYTES {
            return Err(DomainError::invalid_argument(
                "capability string value too long",
            ));
        }
        Ok(Self::String(value))
    }

    /// Creates a string list value.
    pub fn new_string_list(values: Vec<String>) -> crate::Result<Self> {
        if values.len() > MAX_STRING_LIST_ITEMS {
            return Err(DomainError::invalid_argument(
                "capability string list too long",
            ));
        }
        for value in &values {
            if value.len() > MAX_STRING_LIST_ITEM_BYTES {
                return Err(DomainError::invalid_argument(
                    "capability string list item too long",
                ));
            }
        }
        Ok(Self::StringList(values))
    }

    /// Creates a boolean value.
    pub fn new_bool(value: bool) -> Self {
        Self::Bool(value)
    }

    /// Creates an integer value.
    pub fn new_int(value: i64) -> Self {
        Self::Int(value)
    }

    /// Returns the string value if any.
    pub fn as_string(&self) -> Option<&str> {
        if let Self::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Returns the string list value if any.
    pub fn as_string_list(&self) -> Option<&[String]> {
        if let Self::StringList(list) = self {
            Some(list)
        } else {
            None
        }
    }

    /// Returns the boolean value if any.
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    /// Returns the integer value if any.
    pub fn as_int(&self) -> Option<i64> {
        if let Self::Int(i) = self {
            Some(*i)
        } else {
            None
        }
    }
}

/// Device aggregate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Device {
    tenant_id: TenantId,
    device_id: DeviceId,
    protocol: Protocol,
    external_id: ProtocolIdentity,
    authority: String,
    name: String,
    kind: DeviceKind,
    lifecycle: DeviceLifecycle,
    connectivity: Connectivity,
    owner_epoch: OwnerEpoch,
    capabilities: Vec<Capability>,
    metadata: BTreeMap<String, String>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl Device {
    /// Creates a new device.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: &dyn Clock,
        tenant_id: TenantId,
        device_id: DeviceId,
        protocol: Protocol,
        external_id: ProtocolIdentity,
        authority: impl Into<String>,
        name: impl Into<String>,
        kind: DeviceKind,
        capabilities: Vec<Capability>,
        metadata: BTreeMap<String, String>,
    ) -> crate::Result<(Self, DomainEvent)> {
        if device_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("device_id must not be nil"));
        }
        if protocol == Protocol::Unknown {
            return Err(DomainError::invalid_argument("protocol must be known"));
        }
        if external_id.as_str().is_empty() {
            return Err(DomainError::invalid_argument(
                "external_id must not be empty",
            ));
        }
        let name = name.into();
        if name.is_empty() || name.len() > MAX_DEVICE_NAME_BYTES {
            return Err(DomainError::invalid_argument(
                "name must not be empty and must not exceed 1024 bytes",
            ));
        }
        let authority = authority.into();
        if authority.len() > MAX_DEVICE_AUTHORITY_BYTES {
            return Err(DomainError::invalid_argument(
                "authority must not exceed 256 bytes",
            ));
        }
        if kind == DeviceKind::Unknown {
            return Err(DomainError::invalid_argument("device kind must be known"));
        }
        validate_capabilities(&capabilities)?;
        validate_metadata(&metadata)?;

        let now = clock.now_wall();
        let device = Self {
            tenant_id,
            device_id,
            protocol,
            external_id,
            authority,
            name,
            kind,
            lifecycle: DeviceLifecycle::Provisioning,
            connectivity: Connectivity::Unknown,
            owner_epoch: OwnerEpoch::default(),
            capabilities,
            metadata,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        };
        let event = DomainEvent::DeviceRegistered {
            tenant_id,
            device_id,
            protocol,
            external_id: device.external_id.as_str().to_string(),
            authority: device.authority.clone(),
            name: device.name.clone(),
            kind,
            capabilities: device.capabilities.clone(),
            metadata: device.metadata.clone(),
            lifecycle: device.lifecycle,
            connectivity: device.connectivity.clone(),
            owner_epoch: device.owner_epoch,
            created_at: device.created_at,
        };
        Ok((device, event))
    }

    /// Updates mutable fields.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        clock: &dyn Clock,
        name: Option<String>,
        kind: Option<DeviceKind>,
        protocol: Option<Protocol>,
        external_id: Option<ProtocolIdentity>,
        authority: Option<String>,
        capabilities: Option<Vec<Capability>>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        if let Some(name) = name {
            if name.is_empty() || name.len() > MAX_DEVICE_NAME_BYTES {
                return Err(DomainError::invalid_argument(
                    "name must not be empty and must not exceed 1024 bytes",
                ));
            }
            self.name = name;
        }
        if let Some(kind) = kind {
            if kind == DeviceKind::Unknown {
                return Err(DomainError::invalid_argument("device kind must be known"));
            }
            self.kind = kind;
        }
        if let Some(protocol) = protocol {
            if protocol == Protocol::Unknown {
                return Err(DomainError::invalid_argument("protocol must be known"));
            }
            self.protocol = protocol;
        }
        if let Some(external_id) = external_id {
            if external_id.as_str().is_empty() {
                return Err(DomainError::invalid_argument(
                    "external_id must not be empty",
                ));
            }
            self.external_id = external_id;
        }
        if let Some(authority) = authority {
            if authority.len() > MAX_DEVICE_AUTHORITY_BYTES {
                return Err(DomainError::invalid_argument(
                    "authority must not exceed 256 bytes",
                ));
            }
            self.authority = authority;
        }
        if let Some(capabilities) = capabilities {
            validate_capabilities(&capabilities)?;
            self.capabilities = capabilities;
        }
        if let Some(metadata) = metadata {
            validate_metadata(&metadata)?;
            self.metadata = metadata;
        }
        self.bump(clock);
        Ok(self.updated_event())
    }

    /// Marks the device as online.
    pub fn mark_online(
        &mut self,
        clock: &dyn Clock,
        reason: Option<String>,
    ) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        if let Some(ref r) = reason
            && r.len() > MAX_DEVICE_REASON_BYTES
        {
            return Err(DomainError::invalid_argument(
                "reason must not exceed 1024 bytes",
            ));
        }
        if self.lifecycle == DeviceLifecycle::Provisioning {
            self.lifecycle = DeviceLifecycle::Active;
        }
        self.connectivity = Connectivity::Online;
        self.bump(clock);
        Ok(DomainEvent::DeviceOnlineChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            connectivity: self.connectivity.clone(),
            lifecycle: self.lifecycle,
            reason,
        })
    }

    /// Marks the device as offline.
    pub fn mark_offline(
        &mut self,
        clock: &dyn Clock,
        reason: String,
    ) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        if reason.len() > MAX_DEVICE_REASON_BYTES {
            return Err(DomainError::invalid_argument(
                "reason must not exceed 1024 bytes",
            ));
        }
        self.connectivity = Connectivity::Offline {
            since: clock.now_wall(),
            reason,
        };
        self.bump(clock);
        Ok(DomainEvent::DeviceOnlineChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            connectivity: self.connectivity.clone(),
            lifecycle: self.lifecycle,
            reason: self.connectivity.offline_reason().map(ToString::to_string),
        })
    }

    /// Suspends the device.
    pub fn suspend(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        let previous = self.lifecycle;
        self.lifecycle = DeviceLifecycle::Suspended;
        if self.connectivity == Connectivity::Online {
            self.connectivity = Connectivity::Offline {
                since: clock.now_wall(),
                reason: "suspended".to_string(),
            };
        }
        self.bump(clock);
        Ok(DomainEvent::DeviceLifecycleChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            previous_lifecycle: previous,
            lifecycle: self.lifecycle,
            connectivity: self.connectivity.clone(),
        })
    }

    /// Activates a suspended device.
    pub fn activate(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        let previous = self.lifecycle;
        self.lifecycle = DeviceLifecycle::Active;
        self.bump(clock);
        Ok(DomainEvent::DeviceLifecycleChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            previous_lifecycle: previous,
            lifecycle: self.lifecycle,
            connectivity: self.connectivity.clone(),
        })
    }

    /// Retires the device.
    pub fn retire(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        let previous = self.lifecycle;
        self.lifecycle = DeviceLifecycle::Retired;
        self.connectivity = Connectivity::Offline {
            since: clock.now_wall(),
            reason: "retired".to_string(),
        };
        self.bump(clock);
        Ok(DomainEvent::DeviceLifecycleChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            previous_lifecycle: previous,
            lifecycle: self.lifecycle,
            connectivity: self.connectivity.clone(),
        })
    }

    /// Replaces the device capabilities.
    pub fn replace_capabilities(
        &mut self,
        clock: &dyn Clock,
        capabilities: Vec<Capability>,
    ) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        validate_capabilities(&capabilities)?;
        self.capabilities = capabilities;
        self.bump(clock);
        Ok(self.updated_event())
    }

    /// Replaces the device metadata.
    pub fn replace_metadata(
        &mut self,
        clock: &dyn Clock,
        metadata: BTreeMap<String, String>,
    ) -> crate::Result<DomainEvent> {
        self.guard_not_retired()?;
        validate_metadata(&metadata)?;
        self.metadata = metadata;
        self.bump(clock);
        Ok(self.updated_event())
    }

    /// Bumps revision and updated_at without changing domain fields.
    pub fn touch(&mut self, clock: &dyn Clock) -> DomainEvent {
        self.bump(clock);
        self.updated_event()
    }

    fn guard_not_retired(&self) -> crate::Result<()> {
        if self.lifecycle == DeviceLifecycle::Retired {
            return Err(DomainError::invalid_transition(
                "Device", "Retired", "mutating",
            ));
        }
        Ok(())
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    fn updated_event(&self) -> DomainEvent {
        DomainEvent::DeviceUpdated {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            name: self.name.clone(),
            kind: self.kind,
            protocol: self.protocol,
            external_id: self.external_id.as_str().to_string(),
            authority: self.authority.clone(),
            capabilities: self.capabilities.clone(),
            metadata: self.metadata.clone(),
            lifecycle: self.lifecycle,
            connectivity: self.connectivity.clone(),
            owner_epoch: self.owner_epoch,
            updated_at: self.updated_at,
        }
    }

    /// Tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Device identifier.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Protocol.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// External protocol identity.
    pub fn external_id(&self) -> &ProtocolIdentity {
        &self.external_id
    }

    /// Authority of the protocol identity.
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Device kind.
    pub fn kind(&self) -> DeviceKind {
        self.kind
    }

    /// Lifecycle.
    pub fn lifecycle(&self) -> DeviceLifecycle {
        self.lifecycle
    }

    /// Connectivity.
    pub fn connectivity(&self) -> Connectivity {
        self.connectivity.clone()
    }

    /// Owner epoch.
    pub fn owner_epoch(&self) -> OwnerEpoch {
        self.owner_epoch
    }

    /// Capabilities.
    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    /// Metadata.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Creation timestamp.
    pub fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }

    /// Last update timestamp.
    pub fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }

    /// Revision for optimistic concurrency.
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

pub(crate) fn validate_capabilities(capabilities: &[Capability]) -> crate::Result<()> {
    if capabilities.len() > MAX_CAPABILITIES {
        return Err(DomainError::invalid_argument("too many capabilities"));
    }
    let mut seen = HashSet::new();
    for capability in capabilities {
        if !seen.insert(capability.key().to_string()) {
            return Err(DomainError::invalid_argument("duplicate capability key"));
        }
        if capability.key().is_empty() || capability.key().len() > MAX_CAPABILITY_KEY_BYTES {
            return Err(DomainError::invalid_argument("capability key invalid"));
        }
        capability.value().validate()?;
    }
    Ok(())
}

pub(crate) fn validate_metadata(metadata: &BTreeMap<String, String>) -> crate::Result<()> {
    if metadata.len() > MAX_METADATA_KEYS {
        return Err(DomainError::invalid_argument("too many metadata entries"));
    }
    for (key, value) in metadata {
        if key.is_empty() || key.len() > MAX_METADATA_KEY_BYTES {
            return Err(DomainError::invalid_argument("metadata key invalid"));
        }
        if value.len() > MAX_METADATA_VALUE_BYTES {
            return Err(DomainError::invalid_argument("metadata value too long"));
        }
    }
    Ok(())
}

impl Connectivity {
    /// Returns the offline reason, if any.
    fn offline_reason(&self) -> Option<&str> {
        if let Self::Offline { reason, .. } = self {
            Some(reason)
        } else {
            None
        }
    }
}

impl CapabilityValue {
    /// Validates the value dimensions.
    pub fn validate(&self) -> crate::Result<()> {
        match self {
            Self::String(s) if s.len() > MAX_CAPABILITY_STRING_VALUE_BYTES => Err(
                DomainError::invalid_argument("capability string value too long"),
            ),
            Self::StringList(list) => {
                if list.len() > MAX_STRING_LIST_ITEMS {
                    return Err(DomainError::invalid_argument(
                        "capability string list too long",
                    ));
                }
                for value in list {
                    if value.len() > MAX_STRING_LIST_ITEM_BYTES {
                        return Err(DomainError::invalid_argument(
                            "capability string list item too long",
                        ));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
