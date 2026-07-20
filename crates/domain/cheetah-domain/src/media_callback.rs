//! Types for media-node callbacks into the signaling control plane.

use cheetah_signal_types::{
    ChannelId, DeviceId, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId,
    OperationId, OwnerEpoch, Revision,
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
    /// Owner epoch of the device/session. Optional because older media nodes
    /// may emit events without an owner_epoch.
    pub owner_epoch: Option<OwnerEpoch>,
    /// Message / request identifier of the original command.
    pub message_id: String,
    /// Revision of the binding at the time the command was issued. Optional
    /// because older media nodes may emit events without revision fields.
    pub binding_revision: Option<Revision>,
    /// Revision of the session at the time the command was issued. Optional
    /// because older media nodes may emit events without revision fields.
    pub session_revision: Option<Revision>,
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
