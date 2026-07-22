//! Immutable command value object and typed payloads.

use crate::{DomainError, channel::PresetAction, media_session::MediaPurpose};
use cheetah_signal_types::{
    ChannelId, CorrelationId, Deadline, DeviceId, IdGenerator, MediaSessionId, MessageId, NodeId,
    OperationId, OwnerEpoch, Principal, ResourceRef, TenantId, UtcTimestamp,
};

/// Scope used to deduplicate operations and media sessions.
#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct IdempotencyScope {
    /// Tenant of the request.
    pub tenant_id: TenantId,
    /// Principal identifier that originated the request.
    pub principal_id: String,
    /// Target resource of the command.
    pub target: ResourceRef,
    /// Caller supplied idempotency key.
    pub idempotency_key: String,
}

impl IdempotencyScope {
    /// Creates a new idempotency scope.
    ///
    /// The `idempotency_key` must be non-empty.
    pub fn new(
        tenant_id: TenantId,
        principal_id: impl Into<String>,
        target: ResourceRef,
        idempotency_key: impl Into<String>,
    ) -> crate::Result<Self> {
        let idempotency_key = idempotency_key.into();
        if idempotency_key.is_empty() {
            return Err(DomainError::invalid_argument(
                "idempotency_key must not be empty",
            ));
        }
        Ok(Self {
            tenant_id,
            principal_id: principal_id.into(),
            target,
            idempotency_key,
        })
    }
}

/// An immutable typed instruction dispatched to an owner, protocol driver or plugin.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Command {
    command_id: MessageId,
    message_id: MessageId,
    operation_id: OperationId,
    /// Optional saga step identifier, used to correlate dispatches with
    /// `OperationStep` records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    step_id: Option<MessageId>,
    tenant_id: TenantId,
    device_id: DeviceId,
    target: ResourceRef,
    payload: CommandPayload,
    idempotency_key: String,
    idempotency_scope: IdempotencyScope,
    deadline: Option<Deadline>,
    expected_owner_epoch: OwnerEpoch,
    requested_by: Principal,
    correlation_id: CorrelationId,
    causation_id: MessageId,
    traceparent: Option<String>,
    tracestate: Option<String>,
}

impl Command {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id_generator: &dyn IdGenerator,
        operation_id: OperationId,
        step_id: Option<MessageId>,
        tenant_id: TenantId,
        device_id: DeviceId,
        idempotency_scope: IdempotencyScope,
        target: ResourceRef,
        payload: CommandPayload,
        deadline: Option<Deadline>,
        expected_owner_epoch: OwnerEpoch,
        requested_by: Principal,
        correlation_id: CorrelationId,
        causation_id: MessageId,
        traceparent: Option<String>,
        tracestate: Option<String>,
    ) -> Self {
        Self {
            command_id: id_generator.generate_message_id(),
            message_id: id_generator.generate_message_id(),
            operation_id,
            step_id,
            tenant_id,
            device_id,
            idempotency_key: idempotency_scope.idempotency_key.clone(),
            idempotency_scope,
            target,
            payload,
            deadline,
            expected_owner_epoch,
            requested_by,
            correlation_id,
            causation_id,
            traceparent,
            tracestate,
        }
    }

    /// Returns the command id.
    pub fn command_id(&self) -> MessageId {
        self.command_id
    }

    /// Returns the envelope message id.
    pub fn message_id(&self) -> MessageId {
        self.message_id
    }

    /// Returns the owning operation id.
    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Returns the saga step id, if the command belongs to a specific step.
    pub fn step_id(&self) -> Option<MessageId> {
        self.step_id
    }

    /// Returns the tenant id.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Returns the device id that the command is addressed to.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Returns the target resource.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Returns the typed payload.
    pub fn payload(&self) -> &CommandPayload {
        &self.payload
    }

    /// Returns the idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Returns the idempotency scope.
    pub fn idempotency_scope(&self) -> &IdempotencyScope {
        &self.idempotency_scope
    }

    /// Returns the deadline for the command.
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Returns the expected owner epoch for fencing.
    pub fn expected_owner_epoch(&self) -> OwnerEpoch {
        self.expected_owner_epoch
    }

    /// Returns the principal that requested the command.
    pub fn requested_by(&self) -> &Principal {
        &self.requested_by
    }

    /// Returns the correlation id.
    pub fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    /// Returns the causation id.
    pub fn causation_id(&self) -> MessageId {
        self.causation_id
    }

    /// Returns the W3C trace parent.
    pub fn traceparent(&self) -> Option<&str> {
        self.traceparent.as_deref()
    }

    /// Returns the W3C trace state.
    pub fn tracestate(&self) -> Option<&str> {
        self.tracestate.as_deref()
    }

    /// Returns the human-readable kind of the command.
    pub fn kind(&self) -> &'static str {
        self.payload.kind()
    }
}

/// Typed payload of a [`Command`].
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum CommandPayload {
    /// Start a live media session.
    StartLive {
        /// Media session being started.
        media_session_id: MediaSessionId,
        /// Channel that provides the stream.
        channel_id: ChannelId,
        /// Target media node for the binding.
        media_node_id: NodeId,
        /// Purpose of the media session.
        purpose: MediaPurpose,
    },
    /// Start a playback session.
    StartPlayback {
        /// Media session being started.
        media_session_id: MediaSessionId,
        /// Channel that provides the stream.
        channel_id: ChannelId,
        /// Target media node for the binding.
        media_node_id: NodeId,
        /// Start of the playback window.
        start_time: UtcTimestamp,
        /// End of the playback window.
        end_time: UtcTimestamp,
        /// Playback speed scale.
        scale: f64,
    },
    /// Start a two-way talk session.
    StartTalk {
        /// Media session being started.
        media_session_id: MediaSessionId,
        /// Channel that provides the stream.
        channel_id: ChannelId,
        /// Target media node for the binding.
        media_node_id: NodeId,
    },
    /// Start a one-way voice broadcast to the device (media sender only).
    StartBroadcast {
        /// Media session being started.
        media_session_id: MediaSessionId,
        /// Channel that receives the broadcast audio.
        channel_id: ChannelId,
        /// Target media node for the binding.
        media_node_id: NodeId,
    },
    /// Stop a media session.
    StopMediaSession {
        /// Media session being stopped.
        media_session_id: MediaSessionId,
    },
    /// Control an active playback session.
    ControlPlayback {
        /// Media session being controlled.
        media_session_id: MediaSessionId,
        /// Playback control command.
        command: MediaControl,
    },
    /// PTZ movement on a channel.
    Ptz {
        /// Channel to control.
        channel_id: ChannelId,
        /// Direction of movement.
        direction: PtzDirection,
        /// Speed factor.
        speed: f64,
    },
    /// Query a device or channel for state/catalog/records.
    Query {
        /// Query to perform.
        query: QueryCommand,
    },
    /// PTZ preset action on a channel.
    Preset {
        /// Preset action to perform.
        preset: PresetCommand,
    },
    /// Device control action such as guard, reboot, or I-frame request.
    DeviceControl {
        /// Control action to perform.
        control: DeviceControlCommand,
    },
}

impl CommandPayload {
    /// Returns the human-readable kind of the payload.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StartLive { .. } => "StartLive",
            Self::StartPlayback { .. } => "StartPlayback",
            Self::StartTalk { .. } => "StartTalk",
            Self::StartBroadcast { .. } => "StartBroadcast",
            Self::StopMediaSession { .. } => "StopMediaSession",
            Self::ControlPlayback { .. } => "ControlPlayback",
            Self::Ptz { .. } => "Ptz",
            Self::Query { .. } => "Query",
            Self::Preset { .. } => "Preset",
            Self::DeviceControl { .. } => "DeviceControl",
        }
    }
}

/// Playback control commands.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum MediaControl {
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

/// Direction for PTZ movement.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum PtzDirection {
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

/// A query command sent to a GB28181 device or channel.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueryCommand {
    /// Kind of query to perform.
    pub kind: QueryKind,
    /// Optional target channel for channel-scoped queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    /// Optional start of a playback/record window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<UtcTimestamp>,
    /// Optional end of a playback/record window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<UtcTimestamp>,
    /// Optional configuration type for `ConfigDownload` queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_type: Option<String>,
    /// Optional playback speed scale for `RecordInfo` queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
}

/// Kinds of query commands.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum QueryKind {
    /// Query device catalog.
    #[default]
    Catalog,
    /// Query device info.
    DeviceInfo,
    /// Query device status.
    DeviceStatus,
    /// Query record info.
    RecordInfo,
    /// Query PTZ presets.
    PresetQuery,
    /// Download device configuration.
    ConfigDownload,
}

impl QueryKind {
    /// Stable snake_case name used for logging and idempotency keys.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::DeviceInfo => "device_info",
            Self::DeviceStatus => "device_status",
            Self::RecordInfo => "record_info",
            Self::PresetQuery => "preset_query",
            Self::ConfigDownload => "config_download",
        }
    }
}

/// A PTZ preset command.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PresetCommand {
    /// Target channel.
    pub channel_id: ChannelId,
    /// Preset action to perform.
    pub action: PresetAction,
    /// Preset identifier.
    pub preset_id: u32,
}

/// A device control command.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeviceControlCommand {
    /// Kind of device control action.
    pub kind: DeviceControlKind,
    /// Optional target channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    /// Boolean parameter for toggle actions such as guard or record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Opaque string parameter for actions that need a value such as a
    /// configuration section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

/// Kinds of device control actions.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DeviceControlKind {
    /// Armed/guard status toggle.
    #[default]
    Guard,
    /// Reset an active alarm.
    AlarmReset,
    /// Manual record toggle.
    Record,
    /// Remote reboot.
    TeleBoot,
    /// Request an I-frame.
    IFrame,
    /// Update a device configuration section.
    DeviceConfig,
}
