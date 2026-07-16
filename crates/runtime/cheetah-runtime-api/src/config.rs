//! Runtime configuration and validation.

use std::any::Any;
use std::sync::Arc;

use crate::RuntimeError;

/// Configuration for the runtime, sharding, timer wheel, and admission.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// Number of fixed shard workers.
    pub shard_count: usize,

    /// Bounded capacity of each shard MPSC mailbox.
    pub shard_mailbox_capacity: usize,

    /// Bounded capacity of the output channel.
    pub output_channel_capacity: usize,

    /// Bounded capacity of the timer command MPSC channel.
    pub timer_command_channel_capacity: usize,

    /// Timer wheel tick resolution in milliseconds.
    pub timer_tick_resolution_ms: u64,

    /// Maximum messages processed per shard poll.
    pub max_messages_per_poll: usize,

    /// Maximum consecutive messages for a single device per poll.
    pub max_consecutive_per_device: usize,

    /// Maximum number of timers waiting to be dispatched to a shard.
    pub max_pending_dispatch: usize,

    /// Maximum number of sessions held in the session registry.
    pub max_sessions: usize,

    /// Opaque actor-specific configuration passed to `DeviceActor::create`.
    pub actor_config: Option<Arc<dyn Any + Send + Sync>>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            shard_count: 64,
            shard_mailbox_capacity: 8192,
            output_channel_capacity: 8192,
            timer_command_channel_capacity: 8192,
            timer_tick_resolution_ms: 100,
            max_messages_per_poll: 1024,
            max_consecutive_per_device: 16,
            max_pending_dispatch: 65536,
            max_sessions: 1_000_000,
            actor_config: None,
        }
    }
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("shard_count", &self.shard_count)
            .field("shard_mailbox_capacity", &self.shard_mailbox_capacity)
            .field("output_channel_capacity", &self.output_channel_capacity)
            .field(
                "timer_command_channel_capacity",
                &self.timer_command_channel_capacity,
            )
            .field("timer_tick_resolution_ms", &self.timer_tick_resolution_ms)
            .field("max_messages_per_poll", &self.max_messages_per_poll)
            .field(
                "max_consecutive_per_device",
                &self.max_consecutive_per_device,
            )
            .field("max_pending_dispatch", &self.max_pending_dispatch)
            .field("max_sessions", &self.max_sessions)
            .field("actor_config", &self.actor_config.is_some())
            .finish()
    }
}

impl RuntimeConfig {
    /// Validates that all configured bounds are greater than zero.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.shard_count == 0 {
            return Err(RuntimeError::InvalidArgument(
                "shard_count must be greater than 0".into(),
            ));
        }
        if self.shard_mailbox_capacity == 0 {
            return Err(RuntimeError::InvalidArgument(
                "shard_mailbox_capacity must be greater than 0".into(),
            ));
        }
        if self.output_channel_capacity == 0 {
            return Err(RuntimeError::InvalidArgument(
                "output_channel_capacity must be greater than 0".into(),
            ));
        }
        if self.timer_command_channel_capacity == 0 {
            return Err(RuntimeError::InvalidArgument(
                "timer_command_channel_capacity must be greater than 0".into(),
            ));
        }
        if self.timer_tick_resolution_ms == 0 {
            return Err(RuntimeError::InvalidArgument(
                "timer_tick_resolution_ms must be greater than 0".into(),
            ));
        }
        if self.max_messages_per_poll == 0 {
            return Err(RuntimeError::InvalidArgument(
                "max_messages_per_poll must be greater than 0".into(),
            ));
        }
        if self.max_consecutive_per_device == 0 {
            return Err(RuntimeError::InvalidArgument(
                "max_consecutive_per_device must be greater than 0".into(),
            ));
        }
        if self.max_pending_dispatch == 0 {
            return Err(RuntimeError::InvalidArgument(
                "max_pending_dispatch must be greater than 0".into(),
            ));
        }
        if self.max_sessions == 0 {
            return Err(RuntimeError::InvalidArgument(
                "max_sessions must be greater than 0".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let config = RuntimeConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn zero_shard_count_is_invalid() {
        let config = RuntimeConfig {
            shard_count: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn zero_capacity_is_invalid() {
        let config = RuntimeConfig {
            max_sessions: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
