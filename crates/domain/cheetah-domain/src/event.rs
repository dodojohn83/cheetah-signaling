//! Domain events produced by aggregates.

use crate::Protocol;
use crate::{
    Capability, Command, Connectivity, DeviceKind, DeviceLifecycle, IdempotencyScope,
    MediaBindingState, MediaPurpose, MediaSessionDesiredState, MediaSessionState, OperationResult,
    OperationStatus,
};
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, MediaBindingId, MediaSessionId, NodeId, OperationId, OwnerEpoch,
    TenantId, UtcTimestamp,
};

/// Domain events emitted by aggregates.
///
/// Events are typed value objects. They must be wrapped in [`cheetah_signal_types::Event`]
/// before being stored in the outbox.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum DomainEvent {
    /// A device was registered.
    DeviceRegistered {
        /// Tenant of the device.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Protocol used by the device.
        protocol: Protocol,
        /// External protocol identity.
        external_id: String,
        /// Authority of the protocol identity.
        authority: String,
        /// Display name.
        name: String,
        /// Device kind.
        kind: DeviceKind,
        /// Initial capabilities.
        capabilities: Vec<Capability>,
        /// Initial metadata.
        metadata: std::collections::BTreeMap<String, String>,
        /// Initial lifecycle.
        lifecycle: DeviceLifecycle,
        /// Initial connectivity.
        connectivity: Connectivity,
        /// Owner epoch.
        owner_epoch: OwnerEpoch,
        /// Creation timestamp.
        created_at: UtcTimestamp,
    },
    /// A device was updated.
    DeviceUpdated {
        /// Tenant of the device.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Display name.
        name: String,
        /// Device kind.
        kind: DeviceKind,
        /// Protocol used by the device.
        protocol: Protocol,
        /// External protocol identity.
        external_id: String,
        /// Authority of the protocol identity.
        authority: String,
        /// Capabilities.
        capabilities: Vec<Capability>,
        /// Metadata.
        metadata: std::collections::BTreeMap<String, String>,
        /// Current lifecycle.
        lifecycle: DeviceLifecycle,
        /// Current connectivity.
        connectivity: Connectivity,
        /// Owner epoch.
        owner_epoch: OwnerEpoch,
        /// Update timestamp.
        updated_at: UtcTimestamp,
    },
    /// Device connectivity changed.
    DeviceOnlineChanged {
        /// Tenant of the device.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// New connectivity.
        connectivity: Connectivity,
        /// Current lifecycle.
        lifecycle: DeviceLifecycle,
        /// Optional reason for online/offline change.
        reason: Option<String>,
    },
    /// Device lifecycle changed.
    DeviceLifecycleChanged {
        /// Tenant of the device.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// Previous lifecycle.
        previous_lifecycle: DeviceLifecycle,
        /// New lifecycle.
        lifecycle: DeviceLifecycle,
        /// Current connectivity.
        connectivity: Connectivity,
    },
    /// A channel was created.
    ChannelCreated {
        /// Tenant of the channel.
        tenant_id: TenantId,
        /// Device that owns the channel.
        device_id: DeviceId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Channel kind.
        kind: crate::ChannelKind,
        /// Display name.
        name: String,
        /// Whether the channel is enabled.
        enabled: bool,
        /// Current status.
        status: crate::ChannelStatus,
        /// Stream profiles.
        stream_profiles: Vec<crate::StreamProfile>,
        /// PTZ capabilities.
        ptz_capabilities: crate::PtzCapabilities,
        /// Metadata.
        metadata: std::collections::BTreeMap<String, String>,
        /// Creation timestamp.
        created_at: UtcTimestamp,
    },
    /// A channel was updated.
    ChannelUpdated {
        /// Tenant of the channel.
        tenant_id: TenantId,
        /// Device that owns the channel.
        device_id: DeviceId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Channel kind.
        kind: crate::ChannelKind,
        /// Display name.
        name: String,
        /// Whether the channel is enabled.
        enabled: bool,
        /// Current status.
        status: crate::ChannelStatus,
        /// Stream profiles.
        stream_profiles: Vec<crate::StreamProfile>,
        /// PTZ capabilities.
        ptz_capabilities: crate::PtzCapabilities,
        /// Metadata.
        metadata: std::collections::BTreeMap<String, String>,
        /// Update timestamp.
        updated_at: UtcTimestamp,
    },
    /// Channel status changed.
    ChannelOnlineChanged {
        /// Tenant of the channel.
        tenant_id: TenantId,
        /// Device that owns the channel.
        device_id: DeviceId,
        /// Channel identifier.
        channel_id: ChannelId,
        /// Previous status.
        previous_status: crate::ChannelStatus,
        /// New status.
        status: crate::ChannelStatus,
    },
    /// A channel was removed.
    ChannelRemoved {
        /// Tenant of the channel.
        tenant_id: TenantId,
        /// Device that owns the channel.
        device_id: DeviceId,
        /// Channel identifier.
        channel_id: ChannelId,
    },
    /// An operation was submitted.
    OperationSubmitted {
        /// Operation identifier.
        operation_id: OperationId,
        /// Tenant of the operation.
        tenant_id: TenantId,
        /// Device the operation targets.
        device_id: DeviceId,
        /// Idempotency scope.
        idempotency_scope: Box<IdempotencyScope>,
        /// Command that describes the work.
        command: Box<Command>,
    },
    /// An operation changed state.
    OperationStateChanged {
        /// Operation identifier.
        operation_id: OperationId,
        /// Tenant of the operation.
        tenant_id: TenantId,
        /// Previous status.
        previous_status: OperationStatus,
        /// New status.
        status: OperationStatus,
        /// Result of the operation.
        result: Option<OperationResult>,
        /// Error of the operation.
        error: Option<crate::OperationError>,
    },
    /// A media session was created.
    MediaSessionCreated {
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Tenant of the media session.
        tenant_id: TenantId,
        /// Device that owns the session.
        device_id: DeviceId,
        /// Channel that provides the stream.
        channel_id: ChannelId,
        /// Purpose of the session.
        purpose: MediaPurpose,
        /// Desired state.
        desired_state: MediaSessionDesiredState,
        /// Initial state.
        state: MediaSessionState,
        /// Owner epoch.
        owner_epoch: OwnerEpoch,
        /// Creating operation.
        operation_id: OperationId,
        /// Idempotency scope.
        idempotency_scope: Box<IdempotencyScope>,
        /// Deadline.
        deadline: Option<Deadline>,
        /// Creation timestamp.
        created_at: UtcTimestamp,
    },
    /// A media session changed state.
    MediaSessionStateChanged {
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Tenant of the media session.
        tenant_id: TenantId,
        /// Previous state.
        previous_state: MediaSessionState,
        /// New state.
        state: MediaSessionState,
        /// Desired state.
        desired_state: MediaSessionDesiredState,
        /// Error, if any.
        error: Option<crate::MediaSessionError>,
    },
    /// A media binding was created.
    MediaBindingCreated {
        /// Media binding identifier.
        media_binding_id: MediaBindingId,
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Tenant of the binding.
        tenant_id: TenantId,
        /// Channel that provides the stream.
        channel_id: ChannelId,
        /// Media node id.
        media_node_id: NodeId,
        /// Owner epoch.
        owner_epoch: OwnerEpoch,
        /// Initial state.
        state: MediaBindingState,
        /// Creation timestamp.
        created_at: UtcTimestamp,
    },
    /// A media binding changed state.
    MediaBindingStateChanged {
        /// Media binding identifier.
        media_binding_id: MediaBindingId,
        /// Media session identifier.
        media_session_id: MediaSessionId,
        /// Tenant of the binding.
        tenant_id: TenantId,
        /// Previous state.
        previous_state: MediaBindingState,
        /// New state.
        state: MediaBindingState,
        /// Error, if any.
        error: Option<crate::MediaBindingError>,
    },
    /// Device ownership changed.
    ///
    /// Emitted when a different node wins ownership or an existing owner is
    /// re-confirmed after a lease change. Consumers use this to fence stale
    /// local protocol sessions and trigger recovery.
    OwnerChanged {
        /// Tenant of the device.
        tenant_id: TenantId,
        /// Device identifier.
        device_id: DeviceId,
        /// New owner node identifier.
        node_id: NodeId,
        /// New owner epoch.
        owner_epoch: OwnerEpoch,
        /// Previous owner node identifier, if known.
        previous_node_id: Option<NodeId>,
        /// Previous owner epoch, if known.
        previous_epoch: Option<OwnerEpoch>,
        /// Whether this was a takeover from a different or failed node.
        takeover: bool,
    },
}
