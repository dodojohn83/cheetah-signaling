//! Shared driver state and access-machine invocation helpers.
//!
//! [`Shared`] holds the state machine, event sink and bounded transport limits
//! that every UDP receive loop and TCP connection task references. The access
//! machine is guarded by a synchronous [`Mutex`] that is only ever held for the
//! duration of a `process`/`tick` call — never across an `.await` — so device
//! state stays owned by a single logical worker.

use crate::error::DriverError;
use crate::sink::EventSink;
use cheetah_gb28181_core::{AccessInput, AccessOutput, GbAccessMachine, SipParserConfig};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// State shared by all transport tasks of a single driver.
pub(crate) struct Shared<M: GbAccessMachine> {
    access: Mutex<M>,
    sink: Arc<dyn EventSink<M::Event>>,
    parser_config: SipParserConfig,
    max_datagram_size: usize,
    tcp_read_chunk_bytes: usize,
    tcp_idle_timeout: Duration,
    max_connections_per_source: usize,
    conn_permits: Arc<Semaphore>,
    per_source: Mutex<HashMap<IpAddr, usize>>,
    started_at: Instant,
}

impl<M: GbAccessMachine> Shared<M> {
    /// Creates shared state from validated limits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        access: M,
        sink: Arc<dyn EventSink<M::Event>>,
        parser_config: SipParserConfig,
        max_datagram_size: usize,
        tcp_read_chunk_bytes: usize,
        tcp_idle_timeout: Duration,
        max_connections: usize,
        max_connections_per_source: usize,
    ) -> Self {
        Self {
            access: Mutex::new(access),
            sink,
            parser_config,
            max_datagram_size,
            tcp_read_chunk_bytes,
            tcp_idle_timeout,
            max_connections_per_source,
            conn_permits: Arc::new(Semaphore::new(max_connections)),
            per_source: Mutex::new(HashMap::new()),
            started_at: Instant::now(),
        }
    }

    /// Parser limits used for both UDP datagrams and TCP streams.
    pub(crate) fn parser_config(&self) -> SipParserConfig {
        self.parser_config
    }

    /// Maximum accepted UDP datagram size in bytes.
    pub(crate) fn max_datagram_size(&self) -> usize {
        self.max_datagram_size
    }

    /// Per-read chunk size for TCP streams in bytes.
    pub(crate) fn tcp_read_chunk_bytes(&self) -> usize {
        self.tcp_read_chunk_bytes
    }

    /// Idle timeout for TCP connections.
    pub(crate) fn tcp_idle_timeout(&self) -> Duration {
        self.tcp_idle_timeout
    }

    /// Shared connection-permit semaphore used for the global TCP limit and for
    /// bounded drain on shutdown.
    pub(crate) fn conn_permits(&self) -> Arc<Semaphore> {
        self.conn_permits.clone()
    }

    /// Monotonic seconds since the driver started, used as the machine clock.
    pub(crate) fn now_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }

    /// Emits an event through the sink.
    pub(crate) fn emit(&self, event: M::Event) {
        self.sink.emit(event);
    }

    /// Feeds a parsed message to the access machine, returning its outputs.
    pub(crate) fn process_message(
        &self,
        source: SocketAddr,
        message: cheetah_gb28181_core::SipMessage,
    ) -> Result<Vec<AccessOutput<M::Event>>, DriverError> {
        let input = AccessInput {
            source,
            now: self.now_seconds(),
            message,
        };
        let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
        access
            .process(input)
            .map_err(|e| DriverError::Access(Box::new(e)))
    }

    /// Advances the access machine's timers.
    pub(crate) fn tick(&self, now: u64) -> Result<Vec<AccessOutput<M::Event>>, DriverError> {
        let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
        access
            .tick(now)
            .map_err(|e| DriverError::Access(Box::new(e)))
    }

    /// Attempts to reserve a per-source connection slot.
    ///
    /// Returns a guard that releases the slot on drop, or `None` when the source
    /// has reached its configured connection limit.
    pub(crate) fn acquire_source(self: &Arc<Self>, ip: IpAddr) -> Option<SourceSlot<M>> {
        let mut map = self.per_source.lock().unwrap_or_else(|e| e.into_inner());
        let count = map.entry(ip).or_insert(0);
        if *count >= self.max_connections_per_source {
            if *count == 0 {
                map.remove(&ip);
            }
            return None;
        }
        *count += 1;
        Some(SourceSlot {
            shared: self.clone(),
            ip,
        })
    }

    fn release_source(&self, ip: IpAddr) {
        let mut map = self.per_source.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = map.get_mut(&ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&ip);
            }
        }
    }
}

/// RAII guard for a per-source TCP connection slot.
pub(crate) struct SourceSlot<M: GbAccessMachine> {
    shared: Arc<Shared<M>>,
    ip: IpAddr,
}

impl<M: GbAccessMachine> Drop for SourceSlot<M> {
    fn drop(&mut self) {
        self.shared.release_source(self.ip);
    }
}
