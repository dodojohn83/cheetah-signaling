//! Runtime error type shared between the API and Tokio implementation.

/// Maximum byte length of a `RuntimeError` diagnostic message.
const MAX_RUNTIME_ERROR_BYTES: usize = 1024;

fn clamp_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Errors produced by the runtime, admission, timer scheduler, and device actors.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The runtime is shut down.
    #[error("runtime is shut down")]
    Shutdown,

    /// The runtime is overloaded and cannot accept more work.
    #[error("runtime is overloaded")]
    Overloaded,

    /// The runtime is draining and no longer accepts new work.
    #[error("runtime is draining")]
    Draining,

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

impl RuntimeError {
    /// Creates an `InvalidArgument` error with a bounded message.
    pub fn invalid_argument(message: impl std::fmt::Display) -> Self {
        Self::InvalidArgument(clamp_str(&message.to_string(), MAX_RUNTIME_ERROR_BYTES))
    }

    /// Creates an `Actor` error with a bounded message.
    pub fn actor(message: impl std::fmt::Display) -> Self {
        Self::Actor(clamp_str(&message.to_string(), MAX_RUNTIME_ERROR_BYTES))
    }

    /// Creates an `Internal` error with a bounded message.
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal(clamp_str(&message.to_string(), MAX_RUNTIME_ERROR_BYTES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_message_is_clamped() {
        let long = "x".repeat(MAX_RUNTIME_ERROR_BYTES + 10);
        let err = RuntimeError::internal(long);
        if let RuntimeError::Internal(msg) = err {
            assert!(msg.len() <= MAX_RUNTIME_ERROR_BYTES);
            assert!(msg.starts_with("x"));
        } else {
            panic!("expected Internal variant");
        }
    }

    #[test]
    fn actor_message_is_clamped() {
        let long = "x".repeat(MAX_RUNTIME_ERROR_BYTES + 10);
        let err = RuntimeError::actor(long);
        if let RuntimeError::Actor(msg) = err {
            assert!(msg.len() <= MAX_RUNTIME_ERROR_BYTES);
        } else {
            panic!("expected Actor variant");
        }
    }

    #[test]
    fn clamp_respects_utf8_char_boundaries() {
        let text = "x".repeat(MAX_RUNTIME_ERROR_BYTES - 1) + "é";
        let err = RuntimeError::invalid_argument(text);
        if let RuntimeError::InvalidArgument(msg) = err {
            assert!(msg.len() <= MAX_RUNTIME_ERROR_BYTES);
            assert!(msg.ends_with('x'));
        } else {
            panic!("expected InvalidArgument variant");
        }
    }
}
