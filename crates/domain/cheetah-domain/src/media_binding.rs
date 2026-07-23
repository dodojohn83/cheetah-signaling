//! Media binding aggregate and lifecycle.

use crate::{DomainError, DomainEvent};
use cheetah_signal_types::{
    ChannelId, Clock, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OwnerEpoch,
    Revision, TenantId, UtcTimestamp,
};

/// Maximum byte length of a media binding error code.
const MAX_MEDIA_BINDING_ERROR_CODE_BYTES: usize = 128;
/// Maximum byte length of a media binding error message.
const MAX_MEDIA_BINDING_ERROR_MESSAGE_BYTES: usize = 2048;

/// State of a media binding.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaBindingState {
    /// Resources are reserved.
    #[default]
    Reserved,
    /// Binding is active.
    Active,
    /// Binding is active but its media-node association needs verification.
    NeedsVerification,
    /// Releasing resources.
    Releasing,
    /// Resources released.
    Released,
    /// Binding failed.
    Failed,
}

impl MediaBindingState {
    /// Whether this state is terminal.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Released | Self::Failed)
    }
}

/// Error attached to a failed media binding.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MediaBindingError {
    code: String,
    message: String,
}

impl MediaBindingError {
    /// Creates a new media binding error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Error for a timeout.
    pub fn timeout() -> Self {
        Self::new("timeout", "media binding timed out")
    }

    /// Error for a media node being unavailable.
    pub fn media_node_unavailable() -> Self {
        Self::new("media_node_unavailable", "media node unavailable")
    }

    /// Error for a resource that was released before activation.
    pub fn released_before_active() -> Self {
        Self::new(
            "released_before_active",
            "binding released before activation",
        )
    }

    /// Error for a not found resource.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    /// Stable code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Validates the error code and message length.
    pub fn validate(&self) -> crate::Result<()> {
        if self.code.len() > MAX_MEDIA_BINDING_ERROR_CODE_BYTES {
            return Err(DomainError::invalid_argument(
                "media binding error code must not exceed 128 bytes",
            ));
        }
        if self.message.len() > MAX_MEDIA_BINDING_ERROR_MESSAGE_BYTES {
            return Err(DomainError::invalid_argument(
                "media binding error message must not exceed 2048 bytes",
            ));
        }
        Ok(())
    }
}

/// Media binding aggregate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MediaBinding {
    media_binding_id: MediaBindingId,
    media_session_id: MediaSessionId,
    tenant_id: TenantId,
    channel_id: ChannelId,
    media_node_id: NodeId,
    owner_epoch: OwnerEpoch,
    media_node_instance_epoch: MediaNodeInstanceEpoch,
    state: MediaBindingState,
    error: Option<MediaBindingError>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl MediaBinding {
    /// Creates a new media binding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: &dyn Clock,
        media_binding_id: MediaBindingId,
        media_session_id: MediaSessionId,
        tenant_id: TenantId,
        channel_id: ChannelId,
        media_node_id: NodeId,
        owner_epoch: OwnerEpoch,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
    ) -> crate::Result<(Self, DomainEvent)> {
        if media_binding_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "media_binding_id must not be nil",
            ));
        }
        if media_session_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "media_session_id must not be nil",
            ));
        }
        if channel_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("channel_id must not be nil"));
        }
        if media_node_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "media_node_id must not be nil",
            ));
        }
        let now = clock.now_wall();
        let binding = Self {
            media_binding_id,
            media_session_id,
            tenant_id,
            channel_id,
            media_node_id,
            owner_epoch,
            media_node_instance_epoch,
            state: MediaBindingState::Reserved,
            error: None,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        };
        let event = DomainEvent::MediaBindingCreated {
            media_binding_id,
            media_session_id,
            tenant_id,
            channel_id,
            media_node_id,
            owner_epoch,
            state: MediaBindingState::Reserved,
            created_at: binding.created_at,
        };
        Ok((binding, event))
    }

    /// Activates the binding.
    pub fn activate(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::Active, None)
    }

    /// Releases the binding.
    pub fn release(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::Releasing, None)
    }

    /// Marks the binding as released.
    pub fn released(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::Released, None)
    }

    /// Marks the binding as failed.
    pub fn failed(
        &mut self,
        error: MediaBindingError,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::Failed, Some(error))
    }

    /// Times out the binding.
    pub fn timeout(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.failed(MediaBindingError::timeout(), clock)
    }

    /// Marks the binding as needing verification.
    pub fn needs_verification(
        &mut self,
        error: MediaBindingError,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::NeedsVerification, Some(error))
    }

    /// Confirms a binding that was awaiting verification is active again.
    pub fn verified(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaBindingState::Active, None)
    }

    fn transition_to(
        &mut self,
        clock: &dyn Clock,
        new_state: MediaBindingState,
        error: Option<MediaBindingError>,
    ) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaBinding",
                format!("{:?}", self.state),
            ));
        }
        if !Self::allowed_transition(self.state, new_state) {
            return Err(DomainError::invalid_transition(
                "MediaBinding",
                format!("{:?}", self.state),
                format!("{:?}", new_state),
            ));
        }
        if let Some(ref error) = error {
            error.validate()?;
        }
        let previous = self.state;
        self.state = new_state;
        if error.is_some() {
            self.error = error;
        } else if new_state != MediaBindingState::Failed {
            self.error = None;
        }
        self.bump(clock);
        Ok(self.state_changed_event(previous))
    }

    const fn allowed_transition(from: MediaBindingState, to: MediaBindingState) -> bool {
        match from {
            MediaBindingState::Reserved => matches!(
                to,
                MediaBindingState::Active
                    | MediaBindingState::NeedsVerification
                    | MediaBindingState::Releasing
                    | MediaBindingState::Failed
            ),
            MediaBindingState::Active => matches!(
                to,
                MediaBindingState::NeedsVerification
                    | MediaBindingState::Releasing
                    | MediaBindingState::Failed
            ),
            MediaBindingState::NeedsVerification => matches!(
                to,
                MediaBindingState::Active
                    | MediaBindingState::Releasing
                    | MediaBindingState::Failed
            ),
            MediaBindingState::Releasing => {
                matches!(to, MediaBindingState::Released | MediaBindingState::Failed)
            }
            _ => false,
        }
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    fn state_changed_event(&self, previous_state: MediaBindingState) -> DomainEvent {
        DomainEvent::MediaBindingStateChanged {
            media_binding_id: self.media_binding_id,
            media_session_id: self.media_session_id,
            tenant_id: self.tenant_id,
            previous_state,
            state: self.state,
            error: self.error.clone(),
        }
    }

    /// Media binding identifier.
    pub fn media_binding_id(&self) -> MediaBindingId {
        self.media_binding_id
    }

    /// Media session identifier.
    pub fn media_session_id(&self) -> MediaSessionId {
        self.media_session_id
    }

    /// Tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Channel identifier.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// Media node identifier.
    pub fn media_node_id(&self) -> NodeId {
        self.media_node_id
    }

    /// Owner epoch.
    pub fn owner_epoch(&self) -> OwnerEpoch {
        self.owner_epoch
    }

    /// Media node instance epoch.
    pub fn media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch {
        self.media_node_instance_epoch
    }

    /// Current state.
    pub fn state(&self) -> MediaBindingState {
        self.state
    }

    /// Error, if failed.
    pub fn error(&self) -> Option<MediaBindingError> {
        self.error.clone()
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

    /// Whether the binding is terminal.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::IdGenerator;

    #[test]
    fn media_binding_error_rejects_oversized_code_and_message() {
        let error = MediaBindingError::new("x".repeat(129), "msg");
        assert!(matches!(
            error.validate(),
            Err(DomainError::InvalidArgument { .. })
        ));

        let error = MediaBindingError::new("code", "x".repeat(2049));
        assert!(matches!(
            error.validate(),
            Err(DomainError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn media_binding_failed_rejects_oversized_error() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let mut binding = match MediaBinding::new(
            &clock,
            ids.generate_media_binding_id(),
            ids.generate_media_session_id(),
            ids.generate_tenant_id(),
            ids.generate_channel_id(),
            ids.generate_node_id(),
            OwnerEpoch::default(),
            MediaNodeInstanceEpoch::default(),
        ) {
            Ok((b, _)) => b,
            Err(e) => panic!("{e}"),
        };
        let result = binding.failed(MediaBindingError::new("code", "x".repeat(2049)), &clock);
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }
}
