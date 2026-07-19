//! Deterministic test doubles for ports defined in [`crate::ports`].
//!
//! Enable with the `test-support` feature. The doubles are `Send + Sync` and
//! use interior mutability so they can be shared across async tasks in tests.

use crate::{
    ChannelId, Clock, CorrelationId, DeliveryId, DeviceId, DurationMs, EndpointId, EventId,
    IdGenerator, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId,
    NetworkFaultPolicy, NodeId, NodeInstanceId, OperationId, PluginId, ProtocolSessionId,
    RandomSource, TenantId, UtcTimestamp, WebhookId,
};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use time::OffsetDateTime;

/// A deterministic wall and monotonic clock for tests.
#[derive(Debug)]
pub struct FakeClock {
    state: Mutex<ClockState>,
}

#[derive(Debug, Clone, Copy)]
struct ClockState {
    wall: OffsetDateTime,
    monotonic_ms: i64,
}

impl FakeClock {
    /// Creates a clock starting at the Unix epoch.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ClockState {
                wall: OffsetDateTime::UNIX_EPOCH,
                monotonic_ms: 0,
            }),
        }
    }

    /// Creates a clock starting at the provided wall time.
    pub fn at(wall: UtcTimestamp) -> Self {
        Self {
            state: Mutex::new(ClockState {
                wall: wall.as_offset(),
                monotonic_ms: 0,
            }),
        }
    }

    /// Advances the wall clock by `duration`.
    pub fn advance_wall(&self, duration: DurationMs) {
        let mut state = self.lock();
        let Some(new) = state.wall.checked_add(duration.as_duration()) else {
            panic!("fake wall time overflow");
        };
        state.wall = new;
    }

    /// Sets the wall clock to `wall`.
    pub fn set_wall(&self, wall: UtcTimestamp) {
        self.lock().wall = wall.as_offset();
    }

    /// Advances the monotonic clock by `duration`.
    pub fn advance_monotonic(&self, duration: DurationMs) {
        let mut state = self.lock();
        let delta = duration.as_millis();
        let Some(new) = state.monotonic_ms.checked_add(delta) else {
            panic!("fake monotonic time overflow");
        };
        state.monotonic_ms = new;
    }

    /// Returns the current wall time.
    pub fn wall(&self) -> UtcTimestamp {
        UtcTimestamp::from_offset(self.lock().wall)
    }

    /// Returns the current monotonic time.
    pub fn monotonic(&self) -> DurationMs {
        DurationMs::from_millis(self.lock().monotonic_ms)
    }

    fn lock(&self) -> MutexGuard<'_, ClockState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now_wall(&self) -> UtcTimestamp {
        self.wall()
    }

    fn now_monotonic(&self) -> DurationMs {
        self.monotonic()
    }
}

/// A deterministic ID generator for tests.
///
/// Each identifier is built from an incrementing counter encoded as a UUID.
/// Counter values are unique across all ID kinds produced by the same generator.
#[derive(Debug, Default)]
pub struct FakeIdGenerator {
    counter: AtomicU64,
}

impl FakeIdGenerator {
    /// Creates a new generator starting at counter `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a generator starting at `counter`.
    pub fn starting_at(counter: u64) -> Self {
        Self {
            counter: AtomicU64::new(counter),
        }
    }

    fn next_uuid(&self) -> uuid::Uuid {
        let value = self.counter.fetch_add(1, Ordering::Relaxed);
        uuid::Uuid::from_u64_pair(0, value)
    }
}

macro_rules! generate_id {
    ($method:ident -> $id:ident) => {
        fn $method(&self) -> $id {
            $id::from_uuid(self.next_uuid())
        }
    };
}

impl IdGenerator for FakeIdGenerator {
    generate_id!(generate_tenant_id -> TenantId);
    generate_id!(generate_device_id -> DeviceId);
    generate_id!(generate_endpoint_id -> EndpointId);
    generate_id!(generate_channel_id -> ChannelId);
    generate_id!(generate_protocol_session_id -> ProtocolSessionId);
    generate_id!(generate_media_session_id -> MediaSessionId);
    generate_id!(generate_media_binding_id -> MediaBindingId);

    fn generate_media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch {
        MediaNodeInstanceEpoch(self.counter.fetch_add(1, Ordering::Relaxed))
    }

    generate_id!(generate_operation_id -> OperationId);
    generate_id!(generate_node_id -> NodeId);
    generate_id!(generate_node_instance_id -> NodeInstanceId);
    generate_id!(generate_plugin_id -> PluginId);
    generate_id!(generate_event_id -> EventId);
    generate_id!(generate_message_id -> MessageId);
    generate_id!(generate_correlation_id -> CorrelationId);
    generate_id!(generate_webhook_id -> WebhookId);
    generate_id!(generate_delivery_id -> DeliveryId);
}

/// A deterministic random source backed by `fastrand`.
#[derive(Debug)]
pub struct FakeRandom {
    inner: Mutex<fastrand::Rng>,
}

impl FakeRandom {
    /// Creates a random source seeded with `seed`.
    pub fn seeded(seed: impl Into<TestSeed>) -> Self {
        Self {
            inner: Mutex::new(fastrand::Rng::with_seed(seed.into().0)),
        }
    }

    fn with_rng<R>(&self, f: impl FnOnce(&mut fastrand::Rng) -> R) -> R {
        let mut rng = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut rng)
    }
}

impl RandomSource for FakeRandom {
    fn fill_bytes(&self, dest: &mut [u8]) {
        self.with_rng(|rng| {
            for byte in dest.iter_mut() {
                *byte = rng.u8(..);
            }
        });
    }

    fn u64(&self) -> u64 {
        self.with_rng(|rng| rng.u64(..))
    }

    fn usize(&self, bound: usize) -> usize {
        self.with_rng(|rng| rng.usize(0..bound))
    }
}

/// A seed value used to make random-dependent tests reproducible.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TestSeed(pub u64);

impl TestSeed {
    /// Reads `CHEETAH_TEST_SEED` from the environment, or generates a random seed.
    ///
    /// The chosen seed is printed to stderr so failing tests can be replayed.
    #[allow(clippy::print_stderr)]
    pub fn from_env_or_random() -> Self {
        if let Ok(value) = std::env::var("CHEETAH_TEST_SEED")
            && let Ok(seed) = value.parse()
        {
            return Self(seed);
        }
        let seed = fastrand::Rng::new().u64(..);
        eprintln!("CHEETAH_TEST_SEED={seed}");
        Self(seed)
    }

    /// Constructs a seed from a raw `u64`.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for TestSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TestSeed {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

impl From<u64> for TestSeed {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A network fault policy that never injects faults.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpNetworkFault;

impl NetworkFaultPolicy for NoOpNetworkFault {
    fn should_drop(&self) -> bool {
        false
    }

    fn latency_ms(&self) -> u64 {
        0
    }
}

/// A configurable deterministic network fault policy for tests.
#[derive(Debug)]
pub struct FakeNetworkFault {
    drop_one_in_n: Mutex<u64>,
    latency_ms: Mutex<u64>,
    rng: Mutex<fastrand::Rng>,
}

impl FakeNetworkFault {
    /// Creates a policy that drops packets with probability `1 / drop_one_in_n`
    /// and adds `latency_ms` to every packet.
    pub fn new(seed: impl Into<TestSeed>, drop_one_in_n: u64, latency_ms: u64) -> Self {
        Self {
            drop_one_in_n: Mutex::new(drop_one_in_n),
            latency_ms: Mutex::new(latency_ms),
            rng: Mutex::new(fastrand::Rng::with_seed(seed.into().0)),
        }
    }

    /// Changes the drop probability to `1 / n`.
    pub fn set_drop_one_in_n(&self, n: u64) {
        *self.drop_one_in_n.lock().unwrap_or_else(|e| e.into_inner()) = n;
    }

    /// Changes the added latency.
    pub fn set_latency_ms(&self, ms: u64) {
        *self.latency_ms.lock().unwrap_or_else(|e| e.into_inner()) = ms;
    }
}

impl NetworkFaultPolicy for FakeNetworkFault {
    fn should_drop(&self) -> bool {
        let n = *self.drop_one_in_n.lock().unwrap_or_else(|e| e.into_inner());
        if n == 0 {
            return false;
        }
        let mut rng = self.rng.lock().unwrap_or_else(|e| e.into_inner());
        rng.u64(1..=n) == n
    }

    fn latency_ms(&self) -> u64 {
        *self.latency_ms.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances_wall_and_monotonic() {
        let clock = FakeClock::new();
        assert_eq!(clock.now_wall().as_unix_seconds(), 0);
        clock.advance_wall(DurationMs::from_millis(1_500));
        assert_eq!(clock.now_wall().as_unix_seconds(), 1);
        clock.advance_monotonic(DurationMs::from_millis(42));
        assert_eq!(clock.now_monotonic().as_millis(), 42);
    }

    #[test]
    fn fake_id_generator_is_deterministic() {
        let generator = FakeIdGenerator::starting_at(7);
        let a = generator.generate_device_id();
        let b = generator.generate_device_id();
        assert_ne!(a, b);
        assert_eq!(a.to_string(), "00000000-0000-0000-0000-000000000007");
        assert_eq!(b.to_string(), "00000000-0000-0000-0000-000000000008");
    }

    #[test]
    fn fake_random_is_reproducible_with_same_seed() {
        let a = FakeRandom::seeded(12345_u64);
        let b = FakeRandom::seeded(12345_u64);
        assert_eq!(a.u64(), b.u64());
        assert_eq!(a.u64(), b.u64());
    }

    #[test]
    fn no_op_network_fault_never_faults() {
        let policy = NoOpNetworkFault;
        assert!(!policy.should_drop());
        assert_eq!(policy.latency_ms(), 0);
    }
}
