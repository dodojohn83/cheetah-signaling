//! Domain errors and `Result` alias.

use cheetah_signal_types::{SignalError, SignalErrorKind, clamp_str};

/// Maximum byte length of a human-readable `DomainError` message.
const MAX_DOMAIN_ERROR_MESSAGE_BYTES: usize = 1024;
/// Maximum byte length of a `DomainError` entity or state name.
const MAX_DOMAIN_ERROR_FIELD_BYTES: usize = 128;
/// Maximum byte length of a `DomainError` identifier.
const MAX_DOMAIN_ERROR_ID_BYTES: usize = 256;

fn clamp_msg(message: impl std::fmt::Display) -> String {
    clamp_str(&message.to_string(), MAX_DOMAIN_ERROR_MESSAGE_BYTES)
}

fn clamp_field(value: impl std::fmt::Display) -> String {
    clamp_str(&value.to_string(), MAX_DOMAIN_ERROR_FIELD_BYTES)
}

fn clamp_id(value: impl std::fmt::Display) -> String {
    clamp_str(&value.to_string(), MAX_DOMAIN_ERROR_ID_BYTES)
}

/// Errors returned by domain aggregates and application ports.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum DomainError {
    /// An illegal state transition was requested.
    #[error("invalid transition for {entity}: {from} -> {to}")]
    InvalidTransition {
        /// Entity kind, e.g. "Operation".
        entity: String,
        /// Source state name.
        from: String,
        /// Target state name.
        to: String,
    },
    /// Optimistic concurrency conflict.
    #[error("concurrent modification: expected revision {expected}, found {found}")]
    ConcurrentModification {
        /// Expected revision.
        expected: u64,
        /// Revision found in storage.
        found: u64,
    },
    /// A requested resource does not exist.
    #[error("{entity} not found: {id}")]
    NotFound {
        /// Entity kind.
        entity: String,
        /// Entity identifier.
        id: String,
    },
    /// No owner could be resolved for a device.
    #[error("no owner for device {device_id}")]
    NoOwner {
        /// Device identifier.
        device_id: String,
    },
    /// The owner epoch in the command does not match the current owner.
    #[error("stale owner: expected epoch {expected}, got {found}")]
    StaleOwner {
        /// Expected owner epoch.
        expected: u64,
        /// Owner epoch found.
        found: u64,
    },
    /// Input validation failed.
    #[error("invalid argument: {message}")]
    InvalidArgument {
        /// Human readable message.
        message: String,
    },
    /// An external dependency is unavailable.
    #[error("unavailable: {message}")]
    Unavailable {
        /// Human readable message.
        message: String,
    },
    /// The aggregate is already in a terminal state.
    #[error("{entity} already terminal: {status}")]
    AlreadyTerminal {
        /// Entity kind.
        entity: String,
        /// Terminal state name.
        status: String,
    },
    /// Internal error that should not be exposed.
    #[error("internal: {message}")]
    Internal {
        /// Human readable message.
        message: String,
    },
    /// The requested capability is not supported.
    #[error("not supported: {message}")]
    Unsupported {
        /// Human readable message.
        message: String,
    },
}

impl DomainError {
    /// Creates a `NotFound` error for the given entity and id.
    pub fn not_found(entity: impl std::fmt::Display, id: impl std::fmt::Display) -> Self {
        Self::NotFound {
            entity: clamp_field(entity),
            id: clamp_id(id),
        }
    }

    /// Creates an `InvalidArgument` error with the given message.
    pub fn invalid_argument(message: impl std::fmt::Display) -> Self {
        Self::InvalidArgument {
            message: clamp_msg(message),
        }
    }

    /// Creates an `Unavailable` error with the given message.
    pub fn unavailable(message: impl std::fmt::Display) -> Self {
        Self::Unavailable {
            message: clamp_msg(message),
        }
    }

    /// Creates an `Internal` error with the given message.
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal {
            message: clamp_msg(message),
        }
    }

    /// Creates an `InvalidTransition` error.
    pub fn invalid_transition(
        entity: impl std::fmt::Display,
        from: impl std::fmt::Display,
        to: impl std::fmt::Display,
    ) -> Self {
        Self::InvalidTransition {
            entity: clamp_field(entity),
            from: clamp_field(from),
            to: clamp_field(to),
        }
    }

    /// Creates an `AlreadyTerminal` error.
    pub fn already_terminal(
        entity: impl std::fmt::Display,
        status: impl std::fmt::Display,
    ) -> Self {
        Self::AlreadyTerminal {
            entity: clamp_field(entity),
            status: clamp_field(status),
        }
    }

    /// Creates a `NoOwner` error.
    pub fn no_owner(device_id: impl std::fmt::Display) -> Self {
        Self::NoOwner {
            device_id: clamp_id(device_id),
        }
    }

    /// Creates a `StaleOwner` error.
    pub fn stale_owner(expected: u64, found: u64) -> Self {
        Self::StaleOwner { expected, found }
    }

    /// Creates an `Unsupported` error.
    pub fn not_supported(message: impl std::fmt::Display) -> Self {
        Self::Unsupported {
            message: clamp_msg(message),
        }
    }
}

impl From<DomainError> for SignalError {
    fn from(err: DomainError) -> Self {
        let (kind, message) = match err {
            DomainError::InvalidTransition { entity, from, to } => (
                SignalErrorKind::Conflict,
                format!("invalid transition for {entity}: {from} -> {to}"),
            ),
            DomainError::ConcurrentModification { expected, found } => (
                SignalErrorKind::Conflict,
                format!("concurrent modification: expected {expected}, found {found}"),
            ),
            DomainError::NotFound { entity, id } => (
                SignalErrorKind::NotFound,
                format!("{entity} not found: {id}"),
            ),
            DomainError::NoOwner { device_id } => (
                SignalErrorKind::Unavailable,
                format!("no owner for device {device_id}"),
            ),
            DomainError::StaleOwner { expected, found } => (
                SignalErrorKind::StaleOwner,
                format!("stale owner: expected {expected}, got {found}"),
            ),
            DomainError::InvalidArgument { message } => (SignalErrorKind::InvalidArgument, message),
            DomainError::Unavailable { message } => (SignalErrorKind::Unavailable, message),
            DomainError::AlreadyTerminal { entity, status } => (
                SignalErrorKind::Conflict,
                format!("{entity} already terminal: {status}"),
            ),
            DomainError::Internal { message } => (SignalErrorKind::Internal, message),
            DomainError::Unsupported { message } => (
                SignalErrorKind::Unsupported,
                format!("not supported: {message}"),
            ),
        };
        Self::new(kind, message)
    }
}

/// Short alias for fallible domain operations.
pub type Result<T> = std::result::Result<T, DomainError>;
