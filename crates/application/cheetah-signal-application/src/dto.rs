//! Data transfer objects used by application services.

use cheetah_domain::{
    Capability, CapabilityValue, Channel, ChannelKind, ChannelStatus, CommandPayload, Connectivity,
    Device, DeviceKind, DeviceLifecycle, MediaBinding, MediaBindingError, MediaBindingState,
    MediaControl, MediaPurpose, MediaSession, MediaSessionDesiredState, MediaSessionError,
    MediaSessionState, Operation, OperationError, OperationResult, OperationStatus, Protocol,
    PtzCapabilities, PtzDirection, StreamProfile,
};
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId,
    OperationId, OwnerEpoch, ResourceRef, Revision, TenantId, UtcTimestamp,
};
use std::collections::BTreeMap;

/// Request to register or update a device.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RegisterDeviceRequest {
    /// Protocol used by the device.
    pub protocol: String,
    /// External identity of the device.
    pub external_id: String,
    /// Authority that issued the external identity.
    pub authority: Option<String>,
    /// Human readable name.
    pub name: String,
    /// Kind of device.
    pub kind: String,
    /// Capabilities of the device.
    pub capabilities: Option<Vec<CapabilityDto>>,
    /// Metadata of the device.
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Request to replace the channel catalog for a device.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ReplaceChannelCatalogRequest {
    /// Channels in the catalog.
    pub channels: Vec<ChannelDescriptor>,
}

/// Request to update device capabilities.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct UpdateDeviceCapabilitiesRequest {
    /// Capabilities of the device.
    pub capabilities: Option<Vec<CapabilityDto>>,
    /// Metadata of the device.
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Request to mark a device as online.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct MarkDeviceOnlineRequest {
    /// Reason for the online change.
    pub reason: Option<String>,
}

/// Request to mark a device as offline.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct MarkDeviceOfflineRequest {
    /// Reason for the offline change.
    pub reason: String,
}

/// Request to retire a device.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RetireDeviceRequest {}

/// Request to start a live media session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StartLiveRequest {
    /// Device that provides the channel.
    pub device_id: String,
    /// Channel to stream.
    pub channel_id: String,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Optional RFC 3339 deadline.
    pub deadline: Option<String>,
}

/// Request to stop a live media session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StopLiveRequest {
    /// Media session to stop.
    pub media_session_id: String,
    /// Idempotency key for the stop operation.
    pub idempotency_key: String,
}

/// Request to start a playback session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StartPlaybackRequest {
    /// Device that provides the channel.
    pub device_id: String,
    /// Channel to playback.
    pub channel_id: String,
    /// Start of the playback window.
    pub start_time: String,
    /// End of the playback window.
    pub end_time: String,
    /// Playback speed scale.
    pub scale: f64,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Optional RFC 3339 deadline.
    pub deadline: Option<String>,
}

/// Request to start a two-way talk session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StartTalkRequest {
    /// Device that provides the channel.
    pub device_id: String,
    /// Channel to talk through.
    pub channel_id: String,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Optional RFC 3339 deadline.
    pub deadline: Option<String>,
}

/// Request to start a one-way voice broadcast to a device.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StartBroadcastRequest {
    /// Device that receives the broadcast.
    pub device_id: String,
    /// Channel to broadcast to.
    pub channel_id: String,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Optional RFC 3339 deadline.
    pub deadline: Option<String>,
}

/// Request to control an active playback session.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ControlPlaybackRequest {
    /// Media session to control.
    pub media_session_id: String,
    /// Playback control command.
    pub command: MediaControlDto,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
}

/// Request to submit a new operation.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SubmitOperationRequest {
    /// Device that the operation targets.
    pub device_id: DeviceId,
    /// Target resource of the operation.
    pub target: ResourceRef,
    /// Command payload.
    pub payload: CommandPayload,
    /// Idempotency key for the operation.
    pub idempotency_key: String,
    /// Optional deadline for the operation.
    pub deadline: Option<Deadline>,
    /// Expected owner epoch for fencing.
    pub expected_owner_epoch: OwnerEpoch,
}

/// Result of registering or updating a device.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RegisterDeviceResult {
    /// The resulting device.
    pub device: DeviceDto,
    /// Whether the device was newly created rather than updated.
    pub created: bool,
}

/// Device response DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct DeviceDto {
    /// Tenant of the device.
    pub tenant_id: TenantId,
    /// Device identifier.
    pub device_id: DeviceId,
    /// Protocol of the device.
    pub protocol: Protocol,
    /// External identity of the device.
    pub external_id: String,
    /// Authority that issued the external identity.
    pub authority: String,
    /// Human readable name.
    pub name: String,
    /// Kind of device.
    pub kind: DeviceKind,
    /// Lifecycle state.
    pub lifecycle: DeviceLifecycle,
    /// Connectivity state.
    pub connectivity: Connectivity,
    /// Owner epoch.
    pub owner_epoch: OwnerEpoch,
    /// Capabilities.
    pub capabilities: Vec<Capability>,
    /// Metadata.
    pub metadata: BTreeMap<String, String>,
    /// Creation timestamp.
    pub created_at: UtcTimestamp,
    /// Last update timestamp.
    pub updated_at: UtcTimestamp,
    /// Revision.
    pub revision: Revision,
}

/// Channel response DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ChannelDto {
    /// Tenant of the channel.
    pub tenant_id: TenantId,
    /// Device identifier.
    pub device_id: DeviceId,
    /// Channel identifier.
    pub channel_id: ChannelId,
    /// Kind of channel.
    pub kind: ChannelKind,
    /// Human readable name.
    pub name: String,
    /// Whether the channel is enabled.
    pub enabled: bool,
    /// Status of the channel.
    pub status: ChannelStatus,
    /// Stream profiles.
    pub stream_profiles: Vec<StreamProfile>,
    /// PTZ capabilities.
    pub ptz_capabilities: PtzCapabilities,
    /// Metadata.
    pub metadata: BTreeMap<String, String>,
    /// Creation timestamp.
    pub created_at: UtcTimestamp,
    /// Last update timestamp.
    pub updated_at: UtcTimestamp,
    /// Revision.
    pub revision: Revision,
}

/// Operation response DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct OperationDto {
    /// Operation identifier.
    pub operation_id: OperationId,
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Device identifier.
    pub device_id: DeviceId,
    /// Human readable kind of the command.
    pub kind: String,
    /// Target resource.
    pub target: ResourceRef,
    /// Current status.
    pub status: OperationStatus,
    /// Result if terminal.
    pub result: Option<OperationResult>,
    /// Error if terminal failure.
    pub error: Option<OperationError>,
    /// Idempotency key.
    pub idempotency_key: String,
    /// Deadline for the operation.
    pub deadline: Option<Deadline>,
    /// Expected owner epoch for fencing.
    pub expected_owner_epoch: OwnerEpoch,
    /// Creation timestamp.
    pub created_at: UtcTimestamp,
    /// Last update timestamp.
    pub updated_at: UtcTimestamp,
    /// Revision.
    pub revision: Revision,
}

/// Media session response DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MediaSessionDto {
    /// Media session identifier.
    pub media_session_id: MediaSessionId,
    /// Operation that created the session.
    pub operation_id: OperationId,
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Device identifier.
    pub device_id: DeviceId,
    /// Channel identifier.
    pub channel_id: ChannelId,
    /// Purpose of the session.
    pub purpose: MediaPurpose,
    /// Desired state.
    pub desired_state: MediaSessionDesiredState,
    /// Current state.
    pub state: MediaSessionState,
    /// Error if failed.
    pub error: Option<MediaSessionError>,
    /// Owner epoch.
    pub owner_epoch: OwnerEpoch,
    /// Idempotency key.
    pub idempotency_key: String,
    /// Creation timestamp.
    pub created_at: UtcTimestamp,
    /// Last update timestamp.
    pub updated_at: UtcTimestamp,
    /// Session generation for binding fencing.
    pub generation: u64,
    /// Playback window start, if a playback session.
    pub playback_start_time: Option<UtcTimestamp>,
    /// Playback window end, if a playback session.
    pub playback_end_time: Option<UtcTimestamp>,
    /// Playback scale, if a playback session.
    pub playback_scale: Option<f64>,
    /// Revision.
    pub revision: Revision,
}

/// Media binding response DTO.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MediaBindingDto {
    /// Media binding identifier.
    pub media_binding_id: MediaBindingId,
    /// Media session identifier.
    pub media_session_id: MediaSessionId,
    /// Channel identifier.
    pub channel_id: ChannelId,
    /// Media node identifier.
    pub media_node_id: NodeId,
    /// Media node instance epoch.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Current state.
    pub state: MediaBindingState,
    /// Error if failed.
    pub error: Option<MediaBindingError>,
    /// Owner epoch.
    pub owner_epoch: OwnerEpoch,
    /// Creation timestamp.
    pub created_at: UtcTimestamp,
    /// Last update timestamp.
    pub updated_at: UtcTimestamp,
    /// Revision.
    pub revision: Revision,
}

/// Channel descriptor for catalog replacement.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ChannelDescriptor {
    /// Optional channel identifier. If omitted, a new channel is created.
    pub id: Option<String>,
    /// Human readable name.
    pub name: String,
    /// Kind of channel.
    pub kind: String,
    /// Whether the channel is enabled.
    pub enabled: bool,
    /// Status of the channel.
    pub status: Option<String>,
    /// Stream profiles.
    pub stream_profiles: Vec<StreamProfileDto>,
    /// PTZ capabilities.
    pub ptz_capabilities: Option<PtzCapabilitiesDto>,
    /// Metadata.
    pub metadata: Option<BTreeMap<String, String>>,
}

/// Capability DTO.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CapabilityDto {
    /// Capability key.
    pub key: String,
    /// Capability value.
    pub value: CapabilityValueDto,
}

/// Capability value DTO.
#[derive(Clone, Debug, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum CapabilityValueDto {
    /// A string value.
    String(String),
    /// A list of strings.
    StringList(Vec<String>),
    /// An integer value.
    Integer(i64),
    /// A boolean value.
    Boolean(bool),
}

/// Stream profile DTO.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct StreamProfileDto {
    /// Encoding, e.g. "h264".
    pub encoding: String,
    /// Resolution, e.g. "1920x1080".
    pub resolution: String,
    /// Frame rate in fps.
    pub frame_rate: u32,
    /// Bitrate in kbps.
    pub bitrate: u32,
}

/// PTZ capabilities DTO.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct PtzCapabilitiesDto {
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

/// Playback control DTO.
#[derive(Clone, Debug, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum MediaControlDto {
    /// Resume or start playback.
    Play,
    /// Pause playback.
    Pause,
    /// Stop playback.
    Stop,
    /// Seek to a relative offset in milliseconds.
    Seek {
        /// Offset in milliseconds.
        offset_ms: i64,
    },
    /// Change playback speed.
    Scale {
        /// Speed multiplier.
        value: f64,
    },
}

/// PTZ direction DTO.
#[derive(Clone, Debug, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum PtzDirectionDto {
    /// Stop movement.
    Stop,
    /// Up.
    Up,
    /// Down.
    Down,
    /// Left.
    Left,
    /// Right.
    Right,
    /// Up and left.
    UpLeft,
    /// Up and right.
    UpRight,
    /// Down and left.
    DownLeft,
    /// Down and right.
    DownRight,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
}

impl From<&Device> for DeviceDto {
    fn from(device: &Device) -> Self {
        Self {
            tenant_id: device.tenant_id(),
            device_id: device.device_id(),
            protocol: device.protocol(),
            external_id: device.external_id().as_str().to_string(),
            authority: device.authority().to_string(),
            name: device.name().to_string(),
            kind: device.kind(),
            lifecycle: device.lifecycle(),
            connectivity: device.connectivity().clone(),
            owner_epoch: device.owner_epoch(),
            capabilities: device.capabilities().to_vec(),
            metadata: device.metadata().clone(),
            created_at: device.created_at(),
            updated_at: device.updated_at(),
            revision: device.revision(),
        }
    }
}

impl From<&Channel> for ChannelDto {
    fn from(channel: &Channel) -> Self {
        Self {
            tenant_id: channel.tenant_id(),
            device_id: channel.device_id(),
            channel_id: channel.channel_id(),
            kind: channel.kind(),
            name: channel.name().to_string(),
            enabled: channel.enabled(),
            status: channel.status(),
            stream_profiles: channel.stream_profiles().to_vec(),
            ptz_capabilities: channel.ptz_capabilities().clone(),
            metadata: channel.metadata().clone(),
            created_at: channel.created_at(),
            updated_at: channel.updated_at(),
            revision: channel.revision(),
        }
    }
}

impl From<&Operation> for OperationDto {
    fn from(operation: &Operation) -> Self {
        Self {
            operation_id: operation.operation_id(),
            tenant_id: operation.tenant_id(),
            device_id: operation.device_id(),
            kind: operation.command().kind().to_string(),
            target: operation.command().target().clone(),
            status: operation.status(),
            result: operation.result(),
            error: operation.error(),
            idempotency_key: operation.command().idempotency_key().to_string(),
            deadline: operation.deadline(),
            expected_owner_epoch: operation.expected_owner_epoch(),
            created_at: operation.created_at(),
            updated_at: operation.updated_at(),
            revision: operation.revision(),
        }
    }
}

impl From<&MediaSession> for MediaSessionDto {
    fn from(session: &MediaSession) -> Self {
        Self {
            media_session_id: session.media_session_id(),
            operation_id: session.operation_id(),
            tenant_id: session.tenant_id(),
            device_id: session.device_id(),
            channel_id: session.channel_id(),
            purpose: session.purpose(),
            desired_state: session.desired_state(),
            state: session.state(),
            error: session.error(),
            owner_epoch: session.owner_epoch(),
            idempotency_key: session.idempotency_scope().idempotency_key.clone(),
            created_at: session.created_at(),
            updated_at: session.updated_at(),
            generation: session.generation(),
            playback_start_time: session.playback_start_time(),
            playback_end_time: session.playback_end_time(),
            playback_scale: session.playback_scale(),
            revision: session.revision(),
        }
    }
}

impl From<&MediaBinding> for MediaBindingDto {
    fn from(binding: &MediaBinding) -> Self {
        Self {
            media_binding_id: binding.media_binding_id(),
            media_session_id: binding.media_session_id(),
            channel_id: binding.channel_id(),
            media_node_id: binding.media_node_id(),
            media_node_instance_epoch: binding.media_node_instance_epoch(),
            state: binding.state(),
            error: binding.error(),
            owner_epoch: binding.owner_epoch(),
            created_at: binding.created_at(),
            updated_at: binding.updated_at(),
            revision: binding.revision(),
        }
    }
}

impl TryFrom<CapabilityDto> for Capability {
    type Error = cheetah_domain::DomainError;

    fn try_from(dto: CapabilityDto) -> Result<Self, Self::Error> {
        let value = CapabilityValue::try_from(dto.value)?;
        Capability::new(dto.key, value)
    }
}

impl TryFrom<CapabilityValueDto> for CapabilityValue {
    type Error = cheetah_domain::DomainError;

    fn try_from(dto: CapabilityValueDto) -> Result<Self, Self::Error> {
        match dto {
            CapabilityValueDto::String(value) => CapabilityValue::new_string(value),
            CapabilityValueDto::StringList(values) => CapabilityValue::new_string_list(values),
            CapabilityValueDto::Integer(value) => Ok(CapabilityValue::new_int(value)),
            CapabilityValueDto::Boolean(value) => Ok(CapabilityValue::new_bool(value)),
        }
    }
}

impl TryFrom<StreamProfileDto> for StreamProfile {
    type Error = cheetah_domain::DomainError;

    fn try_from(dto: StreamProfileDto) -> Result<Self, Self::Error> {
        StreamProfile::new(dto.encoding, dto.resolution, dto.frame_rate, dto.bitrate)
    }
}

impl From<PtzCapabilitiesDto> for PtzCapabilities {
    fn from(dto: PtzCapabilitiesDto) -> Self {
        PtzCapabilities::new(dto.pan, dto.tilt, dto.zoom, dto.preset, dto.focus, dto.iris)
    }
}

impl From<MediaControlDto> for MediaControl {
    fn from(dto: MediaControlDto) -> Self {
        match dto {
            MediaControlDto::Play => MediaControl::Play,
            MediaControlDto::Pause => MediaControl::Pause,
            MediaControlDto::Stop => MediaControl::Stop,
            MediaControlDto::Seek { offset_ms } => MediaControl::Seek { offset_ms },
            MediaControlDto::Scale { value } => MediaControl::Scale { value },
        }
    }
}

impl From<PtzDirectionDto> for PtzDirection {
    fn from(dto: PtzDirectionDto) -> Self {
        match dto {
            PtzDirectionDto::Stop => PtzDirection::Stop,
            PtzDirectionDto::Up => PtzDirection::Up,
            PtzDirectionDto::Down => PtzDirection::Down,
            PtzDirectionDto::Left => PtzDirection::Left,
            PtzDirectionDto::Right => PtzDirection::Right,
            PtzDirectionDto::UpLeft => PtzDirection::UpLeft,
            PtzDirectionDto::UpRight => PtzDirection::UpRight,
            PtzDirectionDto::DownLeft => PtzDirection::DownLeft,
            PtzDirectionDto::DownRight => PtzDirection::DownRight,
            PtzDirectionDto::ZoomIn => PtzDirection::ZoomIn,
            PtzDirectionDto::ZoomOut => PtzDirection::ZoomOut,
        }
    }
}

/// Request to create a webhook configuration.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreateWebhookRequest {
    /// Target URL for deliveries.
    pub url: String,
    /// Secret reference used to sign payloads.
    pub secret_ref: String,
    /// Subscribed event types; empty means all events.
    pub event_types: Vec<String>,
}

/// Request to update a webhook configuration.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct UpdateWebhookRequest {
    /// New target URL.
    pub url: Option<String>,
    /// New secret reference.
    pub secret_ref: Option<String>,
    /// New subscribed event types.
    pub event_types: Option<Vec<String>>,
    /// New enabled flag.
    pub enabled: Option<bool>,
}

/// Request to manually trigger a test delivery.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct TriggerWebhookRequest {
    /// Event type to simulate.
    pub event_type: String,
    /// JSON payload to send.
    pub payload: serde_json::Value,
}

/// Result of a tenant media-session reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReconciliationReport {
    /// Number of media nodes scanned.
    pub nodes_scanned: u64,
    /// Number of active sessions found on media nodes.
    pub sessions_found: u64,
    /// Number of non-terminal bindings for stopped sessions that were released.
    pub missing_released: u64,
    /// Number of active sessions missing on the media node that were marked failed.
    pub missing_failed: u64,
    /// Number of active sessions migrated to a new media node.
    pub migrations_succeeded: u64,
    /// Number of migration attempts that failed.
    pub migrations_failed: u64,
    /// Number of orphan sessions detected but not yet cleaned.
    pub orphans_detected: u64,
}

/// Result of an operation reconciliation pass.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OperationReconciliationReport {
    /// Number of non-terminal operations scanned.
    pub scanned: u64,
    /// Number of operations that were timed out.
    pub timed_out: u64,
}
