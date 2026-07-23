//! Media session aggregate and lifecycle.

use crate::{DomainError, DomainEvent, IdempotencyScope};
use cheetah_signal_types::{
    ChannelId, Clock, Deadline, DeviceId, MediaSessionId, OperationId, OwnerEpoch, Revision,
    TenantId, UtcTimestamp,
};

/// Maximum byte length of a media session error code.
const MAX_MEDIA_SESSION_ERROR_CODE_BYTES: usize = 128;
/// Maximum byte length of a media session error message.
const MAX_MEDIA_SESSION_ERROR_MESSAGE_BYTES: usize = 2048;

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

impl std::fmt::Display for MediaSessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Requested => "requested",
            Self::Allocating => "allocating",
            Self::Inviting => "inviting",
            Self::Active => "active",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for MediaSessionState {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let state = if s.eq_ignore_ascii_case("requested") {
            Self::Requested
        } else if s.eq_ignore_ascii_case("allocating") {
            Self::Allocating
        } else if s.eq_ignore_ascii_case("inviting") {
            Self::Inviting
        } else if s.eq_ignore_ascii_case("active") {
            Self::Active
        } else if s.eq_ignore_ascii_case("stopping") {
            Self::Stopping
        } else if s.eq_ignore_ascii_case("stopped") {
            Self::Stopped
        } else if s.eq_ignore_ascii_case("failed") {
            Self::Failed
        } else {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown state: {display}"
            )));
        };
        Ok(state)
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
    /// One-way voice broadcast from the platform to the device (media sender).
    Broadcast,
}

impl MediaPurpose {
    /// Whether this purpose sends audio from the platform to the device and
    /// therefore requires a media-node sender resource.
    pub const fn requires_media_sender(self) -> bool {
        matches!(self, Self::Talk | Self::Broadcast)
    }
}

impl std::fmt::Display for MediaPurpose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Live => "live",
            Self::Playback => "playback",
            Self::Talk => "talk",
            Self::Broadcast => "broadcast",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for MediaPurpose {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let purpose = if s.eq_ignore_ascii_case("live") {
            Self::Live
        } else if s.eq_ignore_ascii_case("playback") {
            Self::Playback
        } else if s.eq_ignore_ascii_case("talk") {
            Self::Talk
        } else if s.eq_ignore_ascii_case("broadcast") {
            Self::Broadcast
        } else {
            Self::Unknown
        };
        if purpose == Self::Unknown {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown media purpose: {display}"
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

    /// Validates the error code and message length.
    pub fn validate(&self) -> crate::Result<()> {
        if self.code.len() > MAX_MEDIA_SESSION_ERROR_CODE_BYTES {
            return Err(DomainError::invalid_argument(
                "media session error code must not exceed 128 bytes",
            ));
        }
        if self.message.len() > MAX_MEDIA_SESSION_ERROR_MESSAGE_BYTES {
            return Err(DomainError::invalid_argument(
                "media session error message must not exceed 2048 bytes",
            ));
        }
        Ok(())
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
    /// Generation of the session; incremented when a new binding must be created
    /// during migration or retry. 同一 generation 最多一个有效 MediaBinding。
    #[serde(default)]
    generation: u64,
    /// Playback time window persisted so a migration can rebuild the same range.
    /// Only meaningful when `purpose` is `Playback`.
    #[serde(default)]
    playback_start_time: Option<UtcTimestamp>,
    #[serde(default)]
    playback_end_time: Option<UtcTimestamp>,
    #[serde(default)]
    playback_scale: Option<f64>,
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
            generation: 0,
            playback_start_time: None,
            playback_end_time: None,
            playback_scale: None,
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
            generation: session.generation,
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
        if let Some(ref error) = error {
            error.validate()?;
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

    /// Session generation; incremented when the session must establish a new
    /// physical binding after migration or retry.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Bumps the session generation and returns a domain event.
    pub fn bump_generation(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        if self.state.is_terminal() {
            return Err(DomainError::already_terminal(
                "MediaSession",
                format!("{:?}", self.state),
            ));
        }
        self.generation += 1;
        self.bump(clock);
        Ok(DomainEvent::MediaSessionGenerationBumped {
            media_session_id: self.media_session_id,
            tenant_id: self.tenant_id,
            generation: self.generation,
            updated_at: self.updated_at,
        })
    }

    /// Playback start time, meaningful only for playback sessions.
    pub fn playback_start_time(&self) -> Option<UtcTimestamp> {
        self.playback_start_time
    }

    /// Playback end time, meaningful only for playback sessions.
    pub fn playback_end_time(&self) -> Option<UtcTimestamp> {
        self.playback_end_time
    }

    /// Playback scale, meaningful only for playback sessions.
    pub fn playback_scale(&self) -> Option<f64> {
        self.playback_scale
    }

    /// Sets the playback window on a non-terminal session.
    pub fn set_playback_window(
        &mut self,
        start_time: UtcTimestamp,
        end_time: UtcTimestamp,
        scale: f64,
    ) {
        self.playback_start_time = Some(start_time);
        self.playback_end_time = Some(end_time);
        self.playback_scale = Some(scale);
    }

    /// Whether the session is terminal.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::{IdGenerator, ResourceId, ResourceKind};

    #[test]
    fn media_session_error_rejects_oversized_code_and_message() {
        let error = MediaSessionError::new("x".repeat(129), "msg");
        assert!(matches!(
            error.validate(),
            Err(DomainError::InvalidArgument { .. })
        ));

        let error = MediaSessionError::new("code", "x".repeat(2049));
        assert!(matches!(
            error.validate(),
            Err(DomainError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn media_session_failed_rejects_oversized_error() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let tenant_id = ids.generate_tenant_id();
        let device_id = ids.generate_device_id();
        let channel_id = ids.generate_channel_id();
        let operation_id = ids.generate_operation_id();
        let media_session_id = ids.generate_media_session_id();
        let scope = match crate::IdempotencyScope::new(
            tenant_id,
            "u",
            cheetah_signal_types::ResourceRef {
                tenant_id,
                kind: ResourceKind::Device,
                id: ResourceId::Device(device_id),
            },
            "key",
        ) {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        let mut session = match MediaSession::new(
            &clock,
            media_session_id,
            tenant_id,
            device_id,
            channel_id,
            MediaPurpose::Live,
            MediaSessionDesiredState::Active,
            OwnerEpoch::default(),
            operation_id,
            scope,
            None,
        ) {
            Ok((s, _)) => s,
            Err(e) => panic!("{e}"),
        };

        let result = session.failed(MediaSessionError::new("code", "x".repeat(2049)), &clock);
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }
}
