//! Tokio-based UDP/TCP driver for the GB28181 access module.
//!
//! The driver binds any number of UDP and TCP addresses (IPv4 and IPv6),
//! parses incoming SIP messages, forwards them to a Sans-I/O
//! [`GbAccessMachine`](cheetah_gb28181_core::GbAccessMachine), and executes the
//! produced outputs (SIP responses, domain events). TCP connections are framed
//! incrementally so half/coalesced messages are handled correctly, and both
//! connection counts and per-connection buffers are bounded. Cancellation and a
//! bounded shutdown drain release sockets, permits and per-source slots.
//!
//! Domain events are emitted through a generic [`EventSink`](sink::EventSink) so
//! the caller can forward them to a message bus or log them.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod sink;

mod shared;
mod tcp;
mod udp;

pub use config::DriverConfig;
pub use error::DriverError;

use cheetah_gb28181_core::GbAccessMachine;
use shared::{DriverAction, Shared};
use sink::EventSink;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// A Tokio transport driver that executes a GB28181 access state machine over
/// UDP and/or TCP.
///
/// The type name is retained for backward compatibility; the driver is no
/// longer UDP-only.
pub struct Gb28181UdpDriver<M: GbAccessMachine> {
    shared: Arc<Shared<M>>,
    udp_sockets: Vec<Arc<UdpSocket>>,
    tcp_listeners: Vec<TcpListener>,
    udp_addrs: Vec<SocketAddr>,
    tcp_addrs: Vec<SocketAddr>,
    tick_interval: std::time::Duration,
    max_connections: usize,
    shutdown_drain: std::time::Duration,
    command_tx: mpsc::Sender<M::CommandInput>,
    command_rx: Option<mpsc::Receiver<M::CommandInput>>,
}

impl<M: GbAccessMachine> std::fmt::Debug for Gb28181UdpDriver<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181UdpDriver")
            .field("udp_addrs", &self.udp_addrs)
            .field("tcp_addrs", &self.tcp_addrs)
            .finish_non_exhaustive()
    }
}

impl<M: GbAccessMachine + Send + 'static> Gb28181UdpDriver<M> {
    /// Binds all configured UDP and TCP addresses and returns the driver plus a
    /// primary local address.
    ///
    /// The primary address is the first bound UDP address, or the first bound
    /// TCP address when no UDP address is configured. Returns
    /// [`DriverError::NoBindAddress`] when neither transport is configured.
    pub async fn bind(
        config: DriverConfig,
        access: M,
        sink: Arc<dyn EventSink<M::Event>>,
    ) -> Result<(Self, SocketAddr), DriverError> {
        if config.udp_binds.is_empty() && config.tcp_binds.is_empty() {
            return Err(DriverError::NoBindAddress);
        }

        let mut udp_sockets = Vec::with_capacity(config.udp_binds.len());
        let mut udp_addrs = Vec::with_capacity(config.udp_binds.len());
        for addr in &config.udp_binds {
            let socket = UdpSocket::bind(addr).await.map_err(DriverError::Bind)?;
            let local = socket.local_addr().map_err(DriverError::Bind)?;
            udp_sockets.push(Arc::new(socket));
            udp_addrs.push(local);
        }

        let mut tcp_listeners = Vec::with_capacity(config.tcp_binds.len());
        let mut tcp_addrs = Vec::with_capacity(config.tcp_binds.len());
        for addr in &config.tcp_binds {
            let listener = TcpListener::bind(addr).await.map_err(DriverError::Bind)?;
            let local = listener.local_addr().map_err(DriverError::Bind)?;
            tcp_listeners.push(listener);
            tcp_addrs.push(local);
        }

        let primary = udp_addrs
            .first()
            .or_else(|| tcp_addrs.first())
            .copied()
            .ok_or(DriverError::NoBindAddress)?;

        let (command_tx, command_rx) =
            mpsc::channel::<M::CommandInput>(config.command_channel_capacity);

        let shared = Arc::new(Shared::new(
            access,
            sink,
            config.parser_config,
            config.compatibility_profile,
            config.max_datagram_size,
            config.tcp_read_chunk_bytes,
            config.tcp_idle_timeout,
            config.max_tcp_connections,
            config.max_tcp_connections_per_source,
            config.manager_config,
        ));

        Ok((
            Self {
                shared,
                udp_sockets,
                tcp_listeners,
                udp_addrs,
                tcp_addrs,
                tick_interval: config.tick_interval,
                max_connections: config.max_tcp_connections,
                shutdown_drain: config.shutdown_drain,
                command_tx,
                command_rx: Some(command_rx),
            },
            primary,
        ))
    }

    /// Local UDP addresses the driver bound to.
    pub fn udp_addrs(&self) -> &[SocketAddr] {
        &self.udp_addrs
    }

    /// Local TCP addresses the driver bound to.
    pub fn tcp_addrs(&self) -> &[SocketAddr] {
        &self.tcp_addrs
    }

    /// Returns a sender for the bounded command channel.
    ///
    /// Each call clones the sender; the driver will not start the command
    /// consumer until [`run`](Self::run) or [`run_with_cancellation`](Self::run_with_cancellation)
    /// is invoked.
    pub fn command_bus(&self) -> mpsc::Sender<M::CommandInput> {
        self.command_tx.clone()
    }

    /// Runs the driver until the process aborts the task.
    ///
    /// Equivalent to [`run_with_cancellation`](Self::run_with_cancellation) with
    /// a token that is never cancelled. Prefer the cancellable variant for
    /// graceful shutdown.
    pub async fn run(self) -> Result<(), DriverError> {
        self.run_with_cancellation(CancellationToken::new()).await
    }

    /// Runs the driver until `cancel` is triggered, then performs a bounded
    /// drain of in-flight TCP connections.
    ///
    /// On cancellation the UDP receive loops, TCP accept loops and the tick
    /// loop stop immediately. In-flight TCP connections observe the same token
    /// and close; the driver waits up to `shutdown_drain` for their permits to
    /// be released before returning.
    pub async fn run_with_cancellation(
        mut self,
        cancel: CancellationToken,
    ) -> Result<(), DriverError> {
        let command_rx = self.command_rx.take();
        let Self {
            shared,
            udp_sockets,
            tcp_listeners,
            tick_interval,
            max_connections,
            shutdown_drain,
            ..
        } = self;

        let mut set: JoinSet<()> = JoinSet::new();

        // The ticker reuses the bound UDP sockets to emit transaction
        // retransmissions produced on timer expiry.
        let ticker_sockets = udp_sockets.clone();
        let command_socket = udp_sockets.first().cloned();

        for socket in udp_sockets {
            let shared = shared.clone();
            let cancel = cancel.clone();
            set.spawn(async move { udp::run_udp(shared, socket, cancel).await });
        }

        for listener in tcp_listeners {
            let shared = shared.clone();
            let cancel = cancel.clone();
            set.spawn(async move { tcp::run_tcp_listener(shared, listener, cancel).await });
        }

        if let Some(mut command_rx) = command_rx {
            let shared = shared.clone();
            let cancel = cancel.clone();
            set.spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        Some(command) = command_rx.recv() => {
                            match shared.handle_command(command) {
                                Ok(actions) => {
                                    for action in actions {
                                        match action {
                                            DriverAction::Send { message, target } => {
                                                if let Some(socket) = command_socket.as_ref() {
                                                    let bytes = cheetah_gb28181_core::encode_message(&message);
                                                    if let Err(e) = socket.send_to(&bytes, target).await {
                                                        warn!(error = %e, target = %target, "failed to send command request");
                                                    }
                                                } else {
                                                    warn!("no UDP socket available for command transmission");
                                                }
                                            }
                                            DriverAction::Emit(event) => shared.emit(event),
                                        }
                                    }
                                }
                                Err(e) => warn!(error = %e, "handle_command failed"),
                            }
                        }
                        else => break,
                    }
                }
            });
        }

        {
            let shared = shared.clone();
            let cancel = cancel.clone();
            set.spawn(
                async move { run_ticker(shared, ticker_sockets, tick_interval, cancel).await },
            );
        }

        cancel.cancelled().await;
        info!("gb28181 driver cancellation requested; draining");

        let _ = timeout(shutdown_drain, async {
            while set.join_next().await.is_some() {}
        })
        .await;
        set.shutdown().await;

        if max_connections > 0 {
            let permits = shared.conn_permits();
            let _ = timeout(
                shutdown_drain,
                permits.acquire_many_owned(max_connections as u32),
            )
            .await;
        }

        Ok(())
    }
}

/// Periodic tick loop that advances the access machine's timers and the SIP
/// transaction tables, emitting events and any transaction retransmissions.
async fn run_ticker<M>(
    shared: Arc<Shared<M>>,
    udp_sockets: Vec<Arc<UdpSocket>>,
    interval: std::time::Duration,
    cancel: CancellationToken,
) where
    M: GbAccessMachine + Send + 'static,
{
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => {
                let now = shared.now_seconds();
                match shared.tick_access(now) {
                    Ok(actions) => {
                        for action in actions {
                            if let shared::DriverAction::Emit(event) = action {
                                shared.emit(event);
                            }
                        }
                    }
                    Err(e) => warn!(error = %e, "failed to process access tick"),
                }

                for action in shared.tick_transactions(shared.now_monotonic()) {
                    match action {
                        shared::DriverAction::Send { message, target } => {
                            let bytes = cheetah_gb28181_core::encode_message(&message);
                            // Send the retransmission from the first bound UDP
                            // socket; TCP transactions never retransmit.
                            if let Some(socket) = udp_sockets.first()
                                && let Err(e) = socket.send_to(&bytes, target).await
                            {
                                warn!(error = %e, target = %target, "failed to send transaction retransmission");
                            }
                        }
                        shared::DriverAction::Emit(event) => shared.emit(event),
                    }
                }
            }
        }
    }
}
