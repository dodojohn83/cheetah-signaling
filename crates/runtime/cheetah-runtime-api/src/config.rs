//! Runtime configuration and validation.

use crate::RuntimeError;

/// Configuration for the runtime, sharding, timer wheel, and admission.
#[derive(Clone, Debug)]
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

    /// Idle timeout after which a loaded actor's in-memory state is lazily
    /// unloaded. A value of `0` disables idle eviction. Authoritative business
    /// state is held by repositories/Operations, so an unloaded actor is
    /// transparently recreated on the next message.
    pub actor_idle_timeout_ms: u64,

    /// Admission-control policy (rate limiting, coalescing, dead-letter and
    /// backlog recovery) applied by the admission controller.
    pub admission: AdmissionPolicyConfig,
}

/// Configuration for the runtime admission policy.
///
/// All limits are bounded so admission state (rate-limit buckets, coalescer
/// keys and the dead-letter queue) can never grow without limit.
#[derive(Clone, Copy, Debug)]
pub struct AdmissionPolicyConfig {
    /// Burst capacity, in messages, of each per-(source, class) rate bucket.
    pub rate_capacity_tokens: u64,
    /// Sustained per-(source, class) rate in messages per second.
    pub rate_refill_tokens_per_sec: u64,
    /// Maximum number of distinct (source, class) rate buckets retained.
    pub rate_max_keys: usize,
    /// Maximum number of (device, class) keys tracked for coalescing.
    pub coalesce_max_tracked: usize,
    /// Maximum number of dead-lettered messages retained for redrive.
    pub dead_letter_capacity: usize,
    /// Aggregate shard-mailbox depth at or above which the runtime enters the
    /// overload state and sheds low-priority work.
    pub backlog_high_watermark: u64,
    /// Aggregate shard-mailbox depth at or below which the runtime recovers
    /// from overload.
    pub backlog_low_watermark: u64,
}

impl Default for AdmissionPolicyConfig {
    fn default() -> Self {
        Self {
            rate_capacity_tokens: 256,
            rate_refill_tokens_per_sec: 128,
            rate_max_keys: 65_536,
            coalesce_max_tracked: 65_536,
            dead_letter_capacity: 8_192,
            backlog_high_watermark: 4_096,
            backlog_low_watermark: 1_024,
        }
    }
}

impl AdmissionPolicyConfig {
    /// Validates that all configured bounds are greater than zero and coherent.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.rate_capacity_tokens == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.rate_capacity_tokens must be greater than 0".into(),
            ));
        }
        if self.rate_refill_tokens_per_sec == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.rate_refill_tokens_per_sec must be greater than 0".into(),
            ));
        }
        if self.rate_max_keys == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.rate_max_keys must be greater than 0".into(),
            ));
        }
        if self.coalesce_max_tracked == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.coalesce_max_tracked must be greater than 0".into(),
            ));
        }
        if self.dead_letter_capacity == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.dead_letter_capacity must be greater than 0".into(),
            ));
        }
        if self.backlog_high_watermark == 0 {
            return Err(RuntimeError::InvalidArgument(
                "admission.backlog_high_watermark must be greater than 0".into(),
            ));
        }
        if self.backlog_low_watermark > self.backlog_high_watermark {
            return Err(RuntimeError::InvalidArgument(
                "admission.backlog_low_watermark must not exceed backlog_high_watermark".into(),
            ));
        }
        Ok(())
    }
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
            actor_idle_timeout_ms: 300_000,
            admission: AdmissionPolicyConfig::default(),
        }
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
        self.admission.validate()?;
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
