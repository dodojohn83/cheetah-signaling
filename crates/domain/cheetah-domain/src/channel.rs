//! Channel aggregate and value objects.

use crate::device::validate_metadata;
use crate::{DomainError, DomainEvent};
use cheetah_signal_types::{ChannelId, Clock, DeviceId, Revision, TenantId, UtcTimestamp};
use std::collections::BTreeMap;

/// Maximum number of stream profiles per channel.
const MAX_STREAM_PROFILES: usize = 16;
/// Maximum byte length of a channel display name.
const MAX_CHANNEL_NAME_BYTES: usize = 1024;
/// Maximum byte length of stream profile string fields.
const MAX_STREAM_PROFILE_FIELD_BYTES: usize = 256;

/// Channel kind.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChannelKind {
    /// Unknown kind.
    #[default]
    Unknown,
    /// Video channel.
    Video,
    /// Audio channel.
    Audio,
    /// PTZ channel.
    Ptz,
    /// Organization/department.
    Organization,
    /// Event channel.
    Event,
    /// IO channel.
    Io,
    /// Composite channel.
    Composite,
}

impl std::fmt::Display for ChannelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Ptz => "ptz",
            Self::Organization => "organization",
            Self::Event => "event",
            Self::Io => "io",
            Self::Composite => "composite",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ChannelKind {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let kind = match s.to_lowercase().as_str() {
            "video" => Self::Video,
            "audio" => Self::Audio,
            "ptz" => Self::Ptz,
            "organization" => Self::Organization,
            "event" => Self::Event,
            "io" => Self::Io,
            "composite" => Self::Composite,
            _ => Self::Unknown,
        };
        Ok(kind)
    }
}

/// Channel status.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChannelStatus {
    /// Unknown status.
    #[default]
    Unknown,
    /// Channel is online.
    Online,
    /// Channel is offline.
    Offline,
    /// Channel has a fault.
    Fault,
}

impl std::fmt::Display for ChannelStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Fault => "fault",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ChannelStatus {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let status = match s.to_lowercase().as_str() {
            "online" => Self::Online,
            "offline" => Self::Offline,
            "fault" => Self::Fault,
            _ => Self::Unknown,
        };
        Ok(status)
    }
}

/// Stream profile for a channel.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StreamProfile {
    /// Video encoding such as h264, h265.
    pub encoding: String,
    /// Resolution such as 1920x1080.
    pub resolution: String,
    /// Frame rate in fps.
    pub frame_rate: u32,
    /// Bitrate in kbps.
    pub bitrate: u32,
}

impl StreamProfile {
    /// Creates a new stream profile.
    pub fn new(
        encoding: impl Into<String>,
        resolution: impl Into<String>,
        frame_rate: u32,
        bitrate: u32,
    ) -> crate::Result<Self> {
        let profile = Self {
            encoding: encoding.into(),
            resolution: resolution.into(),
            frame_rate,
            bitrate,
        };
        profile.validate()?;
        Ok(profile)
    }

    /// Validates the stream profile.
    pub fn validate(&self) -> crate::Result<()> {
        if self.encoding.is_empty() || self.encoding.len() > MAX_STREAM_PROFILE_FIELD_BYTES {
            return Err(DomainError::invalid_argument(
                "encoding must not be empty and must not exceed 256 bytes",
            ));
        }
        if self.resolution.is_empty() || self.resolution.len() > MAX_STREAM_PROFILE_FIELD_BYTES {
            return Err(DomainError::invalid_argument(
                "resolution must not be empty and must not exceed 256 bytes",
            ));
        }
        if self.frame_rate == 0 {
            return Err(DomainError::invalid_argument("frame_rate must be > 0"));
        }
        if self.bitrate == 0 {
            return Err(DomainError::invalid_argument("bitrate must be > 0"));
        }
        Ok(())
    }
}

/// PTZ capabilities of a channel.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PtzCapabilities {
    /// Pan support.
    pub pan: bool,
    /// Tilt support.
    pub tilt: bool,
    /// Zoom support.
    pub zoom: bool,
    /// Preset support.
    pub preset: bool,
    /// Focus support.
    pub focus: bool,
    /// Iris support.
    pub iris: bool,
}

impl PtzCapabilities {
    /// Creates a new capability set.
    pub fn new(pan: bool, tilt: bool, zoom: bool, preset: bool, focus: bool, iris: bool) -> Self {
        Self {
            pan,
            tilt,
            zoom,
            preset,
            focus,
            iris,
        }
    }
}

/// Preset actions for PTZ preset commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PresetAction {
    /// Go to a preset.
    Goto,
    /// Set a preset.
    Set,
    /// Delete a preset.
    Delete,
    /// List presets.
    List,
}

/// Channel aggregate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Channel {
    tenant_id: TenantId,
    device_id: DeviceId,
    channel_id: ChannelId,
    kind: ChannelKind,
    name: String,
    enabled: bool,
    status: ChannelStatus,
    stream_profiles: Vec<StreamProfile>,
    ptz_capabilities: PtzCapabilities,
    metadata: BTreeMap<String, String>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl Channel {
    /// Creates a new channel.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: &dyn Clock,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        kind: ChannelKind,
        name: impl Into<String>,
        enabled: bool,
        status: Option<ChannelStatus>,
        stream_profiles: Vec<StreamProfile>,
        ptz_capabilities: PtzCapabilities,
        metadata: BTreeMap<String, String>,
    ) -> crate::Result<(Self, DomainEvent)> {
        if channel_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("channel_id must not be nil"));
        }
        let name = name.into();
        if name.is_empty() || name.len() > MAX_CHANNEL_NAME_BYTES {
            return Err(DomainError::invalid_argument(
                "name must not be empty and must not exceed 1024 bytes",
            ));
        }
        validate_stream_profiles(&stream_profiles)?;
        validate_metadata(&metadata)?;

        let now = clock.now_wall();
        let channel = Self {
            tenant_id,
            device_id,
            channel_id,
            kind,
            name,
            enabled,
            status: status.unwrap_or_default(),
            stream_profiles,
            ptz_capabilities,
            metadata,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        };
        let event = DomainEvent::ChannelCreated {
            tenant_id,
            device_id,
            channel_id,
            kind,
            name: channel.name.clone(),
            enabled,
            status: channel.status,
            stream_profiles: channel.stream_profiles.clone(),
            ptz_capabilities: channel.ptz_capabilities.clone(),
            metadata: channel.metadata.clone(),
            created_at: channel.created_at,
        };
        Ok((channel, event))
    }

    /// Updates mutable fields.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        clock: &dyn Clock,
        kind: Option<ChannelKind>,
        name: Option<String>,
        enabled: Option<bool>,
        status: Option<ChannelStatus>,
        stream_profiles: Option<Vec<StreamProfile>>,
        ptz_capabilities: Option<PtzCapabilities>,
        metadata: Option<BTreeMap<String, String>>,
    ) -> crate::Result<DomainEvent> {
        if let Some(kind) = kind {
            self.kind = kind;
        }
        if let Some(name) = name {
            if name.is_empty() || name.len() > MAX_CHANNEL_NAME_BYTES {
                return Err(DomainError::invalid_argument(
                    "name must not be empty and must not exceed 1024 bytes",
                ));
            }
            self.name = name;
        }
        if let Some(enabled) = enabled {
            self.enabled = enabled;
        }
        if let Some(status) = status {
            self.status = status;
        }
        if let Some(stream_profiles) = stream_profiles {
            validate_stream_profiles(&stream_profiles)?;
            self.stream_profiles = stream_profiles;
        }
        if let Some(ptz_capabilities) = ptz_capabilities {
            self.ptz_capabilities = ptz_capabilities;
        }
        if let Some(metadata) = metadata {
            validate_metadata(&metadata)?;
            self.metadata = metadata;
        }
        self.bump(clock);
        Ok(self.updated_event())
    }

    /// Updates the channel status.
    pub fn update_status(&mut self, clock: &dyn Clock, status: ChannelStatus) -> DomainEvent {
        let previous = self.status;
        self.status = status;
        self.bump(clock);
        DomainEvent::ChannelOnlineChanged {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            channel_id: self.channel_id,
            previous_status: previous,
            status,
        }
    }

    /// Enables the channel.
    pub fn enable(&mut self, clock: &dyn Clock) -> DomainEvent {
        self.enabled = true;
        self.bump(clock);
        self.updated_event()
    }

    /// Disables the channel.
    pub fn disable(&mut self, clock: &dyn Clock) -> DomainEvent {
        self.enabled = false;
        self.bump(clock);
        self.updated_event()
    }

    /// Produces a `ChannelRemoved` event.
    pub fn remove(self) -> DomainEvent {
        DomainEvent::ChannelRemoved {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            channel_id: self.channel_id,
        }
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    fn updated_event(&self) -> DomainEvent {
        DomainEvent::ChannelUpdated {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            channel_id: self.channel_id,
            kind: self.kind,
            name: self.name.clone(),
            enabled: self.enabled,
            status: self.status,
            stream_profiles: self.stream_profiles.clone(),
            ptz_capabilities: self.ptz_capabilities.clone(),
            metadata: self.metadata.clone(),
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

    /// Channel identifier.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// Channel kind.
    pub fn kind(&self) -> ChannelKind {
        self.kind
    }

    /// Display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the channel is enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Current status.
    pub fn status(&self) -> ChannelStatus {
        self.status
    }

    /// Stream profiles.
    pub fn stream_profiles(&self) -> &[StreamProfile] {
        &self.stream_profiles
    }

    /// PTZ capabilities.
    pub fn ptz_capabilities(&self) -> &PtzCapabilities {
        &self.ptz_capabilities
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

    /// Revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }
}

fn validate_stream_profiles(profiles: &[StreamProfile]) -> crate::Result<()> {
    if profiles.len() > MAX_STREAM_PROFILES {
        return Err(DomainError::invalid_argument("too many stream profiles"));
    }
    for profile in profiles {
        profile.validate()?;
    }
    Ok(())
}

/// Maps a GB28181 catalog/record item external id to a stable internal
/// [`ChannelId`].
///
/// The id is deterministic per `(tenant_id, device_external_id,
/// channel_external_id)` so repeated registrations and queries always resolve to
/// the same channel, and it is independent of the internal device id assignment.
pub fn map_gb28181_channel_id(
    tenant_id: TenantId,
    device_external_id: &str,
    channel_external_id: &str,
) -> ChannelId {
    let namespace = uuid::Uuid::NAMESPACE_OID;
    let name = format!(
        "gb28181/{}/{}/{}",
        tenant_id.as_uuid(),
        device_external_id,
        channel_external_id
    );
    ChannelId::from_uuid(uuid::Uuid::new_v5(&namespace, name.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::IdGenerator;

    fn clock_and_ids() -> (
        InMemoryClock,
        InMemoryIdGenerator,
        TenantId,
        DeviceId,
        ChannelId,
    ) {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let tenant_id = ids.generate_tenant_id();
        let device_id = ids.generate_device_id();
        let channel_id = ids.generate_channel_id();
        (clock, ids, tenant_id, device_id, channel_id)
    }

    #[test]
    fn channel_new_rejects_oversized_name() {
        let (clock, _, tenant_id, device_id, channel_id) = clock_and_ids();
        let result = Channel::new(
            &clock,
            tenant_id,
            device_id,
            channel_id,
            ChannelKind::Video,
            "x".repeat(1025),
            true,
            None,
            Vec::new(),
            PtzCapabilities::default(),
            BTreeMap::new(),
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn channel_update_rejects_oversized_name() {
        let (clock, _, tenant_id, device_id, channel_id) = clock_and_ids();
        let (mut channel, _) = match Channel::new(
            &clock,
            tenant_id,
            device_id,
            channel_id,
            ChannelKind::Video,
            "channel-01",
            true,
            None,
            Vec::new(),
            PtzCapabilities::default(),
            BTreeMap::new(),
        ) {
            Ok(v) => v,
            Err(e) => panic!("{e}"),
        };
        let result = channel.update(
            &clock,
            None,
            Some("x".repeat(1025)),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn stream_profile_rejects_oversized_encoding_and_resolution() {
        let result = StreamProfile::new("x".repeat(257), "1080p", 25, 4_000_000);
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));

        let result = StreamProfile::new("h264", "x".repeat(257), 25, 4_000_000);
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }
}
