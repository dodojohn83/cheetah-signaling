//! Types for media-node callbacks into the signaling control plane.

use cheetah_signal_types::{
    ChannelId, DeviceId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId,
    OperationId, OwnerEpoch, Revision, TenantId, UtcTimestamp,
};

/// A callback event emitted by a media node for a specific binding/session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaNodeCallback {
    /// Media node that emitted the callback.
    pub media_node_id: NodeId,
    /// Instance epoch of the media node.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Media session identifier.
    pub media_session_id: MediaSessionId,
    /// Media binding identifier.
    pub media_binding_id: MediaBindingId,
    /// Operation that triggered the callback. Optional because older media nodes
    /// may emit events without an operation_id.
    pub operation_id: Option<OperationId>,
    /// Owner epoch of the device/session when the command was issued.
    pub owner_epoch: OwnerEpoch,
    /// Message / request identifier of the original command.
    pub message_id: String,
    /// Revision of the binding at the time the command was issued.
    pub binding_revision: Revision,
    /// Revision of the session at the time the command was issued.
    pub session_revision: Revision,
    /// Kind of callback event.
    pub kind: MediaNodeCallbackKind,
}

/// Kind of media-node callback event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaNodeCallbackKind {
    /// The media session has started on the node.
    Started,
    /// The media session has stopped on the node.
    Stopped {
        /// Human-readable reason.
        reason: String,
    },
    /// The media session failed on the node.
    Failed {
        /// Stable error code.
        code: String,
        /// Human-readable error message.
        message: String,
    },
}

/// A media node event as delivered by the client stream.
///
/// The event carries the raw envelope fields needed for deduplication and
/// cursor management, plus an optional parsed callback. If the callback cannot
/// be parsed (unknown event type, missing identifiers, etc.), `callback` is
/// `None` and the consumer can log a diagnostic and advance the cursor without
/// treating the event as a transient failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaNodeEvent {
    /// Tenant that owns the media session.
    pub tenant_id: TenantId,
    /// Event identifier supplied by the media node.
    pub event_id: String,
    /// Correlation identifier from the media node event.
    pub correlation_id: String,
    /// Monotonic sequence number used for cursor and gap detection.
    pub sequence: u64,
    /// Wall-clock time at which the media node reports the event occurred.
    pub occurred_at: Option<UtcTimestamp>,
    /// W3C trace parent, if propagated by the media node.
    pub traceparent: Option<String>,
    /// W3C trace state, if propagated by the media node.
    pub tracestate: Option<String>,
    /// Parsed callback, if the payload was recognized and all required
    /// identifiers could be parsed.
    pub callback: Option<MediaNodeCallback>,
}

/// A media session as reported by a media node for reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaNodeSessionRef {
    /// Media session identifier.
    pub media_session_id: MediaSessionId,
    /// Device identifier, if known to the media node.
    pub device_id: Option<DeviceId>,
    /// Channel identifier, if known to the media node.
    pub channel_id: Option<ChannelId>,
    /// Instance epoch of the media node that reported the session.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
}
