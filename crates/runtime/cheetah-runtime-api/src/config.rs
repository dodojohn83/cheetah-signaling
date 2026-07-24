//! Runtime configuration and validation.

use crate::RuntimeError;

/// Maximum number of fixed shard workers.
const MAX_SHARD_COUNT: usize = 1024;
/// Maximum bounded capacity of a shard MPSC mailbox.
const MAX_MAILBOX_CAPACITY: usize = 1_000_000;
/// Maximum bounded capacity of the output channel.
const MAX_OUTPUT_CHANNEL_CAPACITY: usize = 1_000_000;
/// Maximum bounded capacity of the timer command MPSC channel.
const MAX_TIMER_COMMAND_CHANNEL_CAPACITY: usize = 1_000_000;
/// Maximum messages processed per shard poll.
const MAX_MESSAGES_PER_POLL: usize = 1_000_000;
/// Maximum consecutive messages for a single device per poll.
const MAX_CONSECUTIVE_PER_DEVICE: usize = 1_000_000;
/// Maximum number of timers waiting to be dispatched to a shard.
const MAX_PENDING_DISPATCH: usize = 1_000_000;
/// Maximum number of sessions held in the session registry.
const MAX_SESSIONS: usize = 10_000_000;
/// Maximum idle timeout for an actor before lazy eviction (24 hours).
const MAX_ACTOR_IDLE_TIMEOUT_MS: u64 = 86_400_000;
/// Maximum number of distinct admission rate buckets.
const MAX_ADMISSION_KEYS: usize = 1_000_000;
/// Maximum number of coalescer tracked keys.
const MAX_COALESCE_TRACKED: usize = 1_000_000;
/// Maximum dead-letter queue capacity.
const MAX_DEAD_LETTER_CAPACITY: usize = 1_000_000;

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
        validate_in_range(
            "admission.rate_max_keys",
            self.rate_max_keys,
            1,
            MAX_ADMISSION_KEYS,
        )?;
        validate_in_range(
            "admission.coalesce_max_tracked",
            self.coalesce_max_tracked,
            1,
            MAX_COALESCE_TRACKED,
        )?;
        validate_in_range(
            "admission.dead_letter_capacity",
            self.dead_letter_capacity,
            1,
            MAX_DEAD_LETTER_CAPACITY,
        )?;
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
    /// Validates that all configured bounds are within allowed ranges.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        validate_in_range("shard_count", self.shard_count, 1, MAX_SHARD_COUNT)?;
        validate_in_range(
            "shard_mailbox_capacity",
            self.shard_mailbox_capacity,
            1,
            MAX_MAILBOX_CAPACITY,
        )?;
        validate_in_range(
            "output_channel_capacity",
            self.output_channel_capacity,
            1,
            MAX_OUTPUT_CHANNEL_CAPACITY,
        )?;
        validate_in_range(
            "timer_command_channel_capacity",
            self.timer_command_channel_capacity,
            1,
            MAX_TIMER_COMMAND_CHANNEL_CAPACITY,
        )?;
        if self.timer_tick_resolution_ms == 0 {
            return Err(RuntimeError::InvalidArgument(
                "timer_tick_resolution_ms must be greater than 0".into(),
            ));
        }
        validate_in_range(
            "max_messages_per_poll",
            self.max_messages_per_poll,
            1,
            MAX_MESSAGES_PER_POLL,
        )?;
        validate_in_range(
            "max_consecutive_per_device",
            self.max_consecutive_per_device,
            1,
            MAX_CONSECUTIVE_PER_DEVICE,
        )?;
        validate_in_range(
            "max_pending_dispatch",
            self.max_pending_dispatch,
            1,
            MAX_PENDING_DISPATCH,
        )?;
        validate_in_range("max_sessions", self.max_sessions, 1, MAX_SESSIONS)?;
        if self.actor_idle_timeout_ms > MAX_ACTOR_IDLE_TIMEOUT_MS {
            return Err(RuntimeError::InvalidArgument(format!(
                "actor_idle_timeout_ms must not exceed {MAX_ACTOR_IDLE_TIMEOUT_MS}"
            )));
        }
        self.admission.validate()?;
        Ok(())
    }
}

fn validate_in_range(name: &str, value: usize, min: usize, max: usize) -> Result<(), RuntimeError> {
    if value < min || value > max {
        return Err(RuntimeError::InvalidArgument(format!(
            "{name} must be between {min} and {max}"
        )));
    }
    Ok(())
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

    #[test]
    fn shard_count_above_max_is_invalid() {
        let config = RuntimeConfig {
            shard_count: MAX_SHARD_COUNT + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn shard_count_at_max_is_valid() {
        let config = RuntimeConfig {
            shard_count: MAX_SHARD_COUNT,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn mailbox_capacity_above_max_is_invalid() {
        let config = RuntimeConfig {
            shard_mailbox_capacity: MAX_MAILBOX_CAPACITY + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_messages_per_poll_above_max_is_invalid() {
        let config = RuntimeConfig {
            max_messages_per_poll: MAX_MESSAGES_PER_POLL + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_pending_dispatch_above_max_is_invalid() {
        let config = RuntimeConfig {
            max_pending_dispatch: MAX_PENDING_DISPATCH + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn max_sessions_above_max_is_invalid() {
        let config = RuntimeConfig {
            max_sessions: MAX_SESSIONS + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn actor_idle_timeout_above_max_is_invalid() {
        let config = RuntimeConfig {
            actor_idle_timeout_ms: MAX_ACTOR_IDLE_TIMEOUT_MS + 1,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn admission_rate_max_keys_above_max_is_invalid() {
        let config = RuntimeConfig {
            admission: AdmissionPolicyConfig {
                rate_max_keys: MAX_ADMISSION_KEYS + 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn admission_coalesce_max_tracked_above_max_is_invalid() {
        let config = RuntimeConfig {
            admission: AdmissionPolicyConfig {
                coalesce_max_tracked: MAX_COALESCE_TRACKED + 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn admission_dead_letter_capacity_above_max_is_invalid() {
        let config = RuntimeConfig {
            admission: AdmissionPolicyConfig {
                dead_letter_capacity: MAX_DEAD_LETTER_CAPACITY + 1,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
