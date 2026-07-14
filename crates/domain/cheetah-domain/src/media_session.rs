//! Media session aggregate and lifecycle.

use crate::{DomainError, DomainEvent, IdempotencyScope};
use cheetah_signal_types::{
    ChannelId, Clock, Deadline, DeviceId, MediaSessionId, OperationId, OwnerEpoch, Revision,
    TenantId, UtcTimestamp,
};

/// Desired state of a media session.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaSessionDesiredState {
    /// Session should be active.
    #[default]
    Active,
    /// Session should be stopped.
    Stopped,
}

/// State of a media session.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaSessionState {
    /// Session requested.
    #[default]
    Requested,
    /// Allocating media resources.
    Allocating,
    /// Inviting device.
    Inviting,
    /// Active.
    Active,
    /// Stopping.
    Stopping,
    /// Stopped.
    Stopped,
    /// Failed.
    Failed,
}

impl MediaSessionState {
    /// Whether this state is terminal.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

/// Purpose of a media session.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaPurpose {
    /// Unknown purpose.
    #[default]
    Unknown,
    /// Live preview.
    Live,
    /// Playback.
    Playback,
    /// Two-way talk.
    Talk,
}

impl std::fmt::Display for MediaPurpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Live => "live",
            Self::Playback => "playback",
            Self::Talk => "talk",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for MediaPurpose {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let purpose = match s.to_lowercase().as_str() {
            "live" => Self::Live,
            "playback" => Self::Playback,
            "talk" => Self::Talk,
            _ => Self::Unknown,
        };
        if purpose == Self::Unknown {
            return Err(DomainError::invalid_argument(format!(
                "unknown media purpose: {s}"
            )));
        }
        Ok(purpose)
    }
}

/// Error attached to a failed media session.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MediaSessionError {
    code: String,
    message: String,
}

impl MediaSessionError {
    /// Creates a new media session error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Error for a timeout.
    pub fn timeout() -> Self {
        Self::new("timeout", "media session timed out")
    }

    /// Error for a cancellation.
    pub fn cancelled() -> Self {
        Self::new("cancelled", "media session was cancelled")
    }

    /// Error for a media binding failure.
    pub fn binding_failed(message: impl Into<String>) -> Self {
        Self::new("binding_failed", message)
    }

    /// Error for a device being offline.
    pub fn device_offline() -> Self {
        Self::new("device_offline", "device is offline")
    }

    /// Error for an unsupported capability.
    pub fn not_supported(message: impl Into<String>) -> Self {
        Self::new("not_supported", message)
    }

    /// Stable code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human readable message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Media session aggregate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MediaSession {
    media_session_id: MediaSessionId,
    tenant_id: TenantId,
    device_id: DeviceId,
    channel_id: ChannelId,
    purpose: MediaPurpose,
    desired_state: MediaSessionDesiredState,
    state: MediaSessionState,
    owner_epoch: OwnerEpoch,
    operation_id: OperationId,
    idempotency_scope: IdempotencyScope,
    deadline: Option<Deadline>,
    error: Option<MediaSessionError>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl MediaSession {
    /// Creates a new media session.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        clock: &dyn Clock,
        media_session_id: MediaSessionId,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
        purpose: MediaPurpose,
        desired_state: MediaSessionDesiredState,
        owner_epoch: OwnerEpoch,
        operation_id: OperationId,
        idempotency_scope: IdempotencyScope,
        deadline: Option<Deadline>,
    ) -> crate::Result<(Self, DomainEvent)> {
        if media_session_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "media_session_id must not be nil",
            ));
        }
        if device_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("device_id must not be nil"));
        }
        if channel_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("channel_id must not be nil"));
        }
        if operation_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "operation_id must not be nil",
            ));
        }
        if purpose == MediaPurpose::Unknown {
            return Err(DomainError::invalid_argument("media purpose must be known"));
        }
        let now = clock.now_wall();
        let session = Self {
            media_session_id,
            tenant_id,
            device_id,
            channel_id,
            purpose,
            desired_state,
            state: MediaSessionState::Requested,
            owner_epoch,
            operation_id,
            idempotency_scope,
            deadline,
            error: None,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        };
        let event = DomainEvent::MediaSessionCreated {
            media_session_id,
            tenant_id,
            device_id,
            channel_id,
            purpose,
            desired_state,
            state: MediaSessionState::Requested,
            owner_epoch,
            operation_id,
            idempotency_scope: Box::new(session.idempotency_scope.clone()),
            deadline,
            created_at: session.created_at,
        };
        Ok((session, event))
    }

    /// Transitions to `Allocating`.
    pub fn allocating(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaSessionState::Allocating, None)
    }

    /// Transitions to `Inviting`.
    pub fn inviting(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaSessionState::Inviting, None)
    }

    /// Transitions to `Active`.
    pub fn active(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaSessionState::Active, None)
    }

    /// Stops the session from `Active` to `Stopping`.
    pub fn stopping(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaSession",
                format!("{:?}", self.state),
            ));
        }
        if self.state != MediaSessionState::Active {
            return Err(DomainError::invalid_transition(
                "MediaSession",
                format!("{:?}", self.state),
                "Stopping",
            ));
        }
        self.desired_state = MediaSessionDesiredState::Stopped;
        self.transition_to(clock, MediaSessionState::Stopping, None)
    }

    /// Stops the session.
    pub fn stop(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaSession",
                format!("{:?}", self.state),
            ));
        }
        self.desired_state = MediaSessionDesiredState::Stopped;
        if self.state == MediaSessionState::Active {
            return self.transition_to(clock, MediaSessionState::Stopping, None);
        }
        self.transition_to(clock, MediaSessionState::Stopped, None)
    }

    /// Transitions to `Stopped` from `Stopping`.
    pub fn stopped(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaSessionState::Stopped, None)
    }

    /// Transitions to `Failed` with an error.
    pub fn failed(
        &mut self,
        error: MediaSessionError,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        self.transition_to(clock, MediaSessionState::Failed, Some(error))
    }

    /// Cancels the session.
    pub fn cancel(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaSession",
                format!("{:?}", self.state),
            ));
        }
        self.desired_state = MediaSessionDesiredState::Stopped;
        self.transition_to(clock, MediaSessionState::Stopped, None)
    }

    /// Times out the session.
    pub fn timeout(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.failed(MediaSessionError::timeout(), clock)
    }

    fn transition_to(
        &mut self,
        clock: &dyn Clock,
        new_state: MediaSessionState,
        error: Option<MediaSessionError>,
    ) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaSession",
                format!("{:?}", self.state),
            ));
        }
        if !Self::allowed_transition(self.state, new_state) {
            return Err(DomainError::invalid_transition(
                "MediaSession",
                format!("{:?}", self.state),
                format!("{:?}", new_state),
            ));
        }
        let previous = self.state;
        self.state = new_state;
        if error.is_some() {
            self.error = error;
        } else if new_state != MediaSessionState::Failed {
            self.error = None;
        }
        self.bump(clock);
        Ok(self.state_changed_event(previous))
    }

    const fn allowed_transition(from: MediaSessionState, to: MediaSessionState) -> bool {
        match from {
            MediaSessionState::Requested => matches!(
                to,
                MediaSessionState::Allocating
                    | MediaSessionState::Stopped
                    | MediaSessionState::Failed
            ),
            MediaSessionState::Allocating => matches!(
                to,
                MediaSessionState::Inviting
                    | MediaSessionState::Stopped
                    | MediaSessionState::Failed
            ),
            MediaSessionState::Inviting => matches!(
                to,
                MediaSessionState::Active | MediaSessionState::Stopped | MediaSessionState::Failed
            ),
            MediaSessionState::Active => matches!(
                to,
                MediaSessionState::Stopping
                    | MediaSessionState::Stopped
                    | MediaSessionState::Failed
            ),
            MediaSessionState::Stopping => {
                matches!(to, MediaSessionState::Stopped | MediaSessionState::Failed)
            }
            _ => false,
        }
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    fn state_changed_event(&self, previous_state: MediaSessionState) -> DomainEvent {
        DomainEvent::MediaSessionStateChanged {
            media_session_id: self.media_session_id,
            tenant_id: self.tenant_id,
            previous_state,
            state: self.state,
            desired_state: self.desired_state,
            error: self.error.clone(),
        }
    }

    /// Media session identifier.
    pub fn media_session_id(&self) -> MediaSessionId {
        self.media_session_id
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

    /// Purpose.
    pub fn purpose(&self) -> MediaPurpose {
        self.purpose
    }

    /// Desired state.
    pub fn desired_state(&self) -> MediaSessionDesiredState {
        self.desired_state
    }

    /// Current state.
    pub fn state(&self) -> MediaSessionState {
        self.state
    }

    /// Owner epoch.
    pub fn owner_epoch(&self) -> OwnerEpoch {
        self.owner_epoch
    }

    /// Owning operation identifier.
    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Idempotency scope.
    pub fn idempotency_scope(&self) -> &IdempotencyScope {
        &self.idempotency_scope
    }

    /// Deadline.
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Error, if failed.
    pub fn error(&self) -> Option<MediaSessionError> {
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

    /// Whether the session is terminal.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}
