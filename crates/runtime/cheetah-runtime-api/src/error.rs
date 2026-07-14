//! Runtime error type shared between the API and Tokio implementation.

/// Errors produced by the runtime, admission, timer scheduler, and device actors.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The runtime is shut down.
    #[error("runtime is shut down")]
    Shutdown,

    /// The runtime is overloaded and cannot accept more work.
    #[error("runtime is overloaded")]
    Overloaded,

    /// A configuration or argument value is invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// A device actor produced an error.
    #[error("actor error: {0}")]
    Actor(String),

    /// An internal runtime error occurred.
    #[error("internal error: {0}")]
    Internal(String),
}
