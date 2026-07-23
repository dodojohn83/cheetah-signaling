//! Shared driver state and access-machine invocation helpers.
//!
//! [`Shared`] holds the state machine, event sink and bounded transport limits
//! that every UDP receive loop and TCP connection task references. The access
//! machine is guarded by a synchronous [`Mutex`] that is only ever held for the
//! duration of a `process`/`tick` call — never across an `.await` — so device
//! state stays owned by a single logical worker.

use crate::error::DriverError;
use crate::sink::EventSink;
use cheetah_gb28181_core::{
    AccessInput, AccessOutput, CompatibilityProfile, DialogManager, DialogManagerConfig,
    DialogRouting, GbAccessMachine, HeaderName, ManagerConfig, ManagerOutput, RequestOutcome,
    RequestRoute, SipMessage, SipParserConfig, TransactionManager, TransportKind, route_request,
};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::debug;

/// An action the transport layer must perform after processing a message.
// The `Send` variant intentionally carries an owned `SipMessage`; the size
// difference against `Emit` is acceptable for this short-lived internal enum.
#[allow(clippy::large_enum_variant)]
pub(crate) enum DriverAction<E> {
    /// Transmit a SIP message to `target`.
    Send {
        /// Message to transmit.
        message: SipMessage,
        /// Destination endpoint.
        target: SocketAddr,
    },
    /// Emit a domain/protocol event.
    Emit(E),
}

/// State shared by all transport tasks of a single driver.
pub(crate) struct Shared<M: GbAccessMachine> {
    access: Mutex<M>,
    sink: Arc<dyn EventSink<M::Event>>,
    parser_config: SipParserConfig,
    compatibility_profile: Option<CompatibilityProfile>,
    max_datagram_size: usize,
    tcp_read_chunk_bytes: usize,
    tcp_idle_timeout: Duration,
    max_connections_per_source: usize,
    conn_permits: Arc<Semaphore>,
    per_source: Mutex<HashMap<IpAddr, usize>>,
    started_at: Instant,
    /// Transaction table for unreliable (UDP) transports.
    udp_txns: Mutex<TransactionManager<SocketAddr>>,
    /// Transaction table for reliable (TCP) transports.
    tcp_txns: Mutex<TransactionManager<SocketAddr>>,
    /// Dialog table shared across transports (dialogs are keyed by Call-ID and
    /// tags, not by transport).
    dialogs: Mutex<DialogManager>,
}

impl<M: GbAccessMachine> Shared<M> {
    /// Creates shared state from validated limits.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        access: M,
        sink: Arc<dyn EventSink<M::Event>>,
        parser_config: SipParserConfig,
        compatibility_profile: Option<CompatibilityProfile>,
        max_datagram_size: usize,
        tcp_read_chunk_bytes: usize,
        tcp_idle_timeout: Duration,
        max_connections: usize,
        max_connections_per_source: usize,
        manager_config: ManagerConfig,
    ) -> Self {
        let udp_config = ManagerConfig {
            transaction: cheetah_gb28181_core::TransactionConfig {
                transport: TransportKind::Unreliable,
                ..manager_config.transaction
            },
            ..manager_config
        };
        let tcp_config = ManagerConfig {
            transaction: cheetah_gb28181_core::TransactionConfig {
                transport: TransportKind::Reliable,
                ..manager_config.transaction
            },
            ..manager_config
        };
        Self {
            access: Mutex::new(access),
            sink,
            parser_config,
            compatibility_profile,
            max_datagram_size,
            tcp_read_chunk_bytes,
            tcp_idle_timeout,
            max_connections_per_source,
            conn_permits: Arc::new(Semaphore::new(max_connections)),
            per_source: Mutex::new(HashMap::new()),
            started_at: Instant::now(),
            udp_txns: Mutex::new(TransactionManager::new(udp_config)),
            tcp_txns: Mutex::new(TransactionManager::new(tcp_config)),
            dialogs: Mutex::new(DialogManager::new(DialogManagerConfig {
                max_dialogs: manager_config.max_transactions,
                ..DialogManagerConfig::default()
            })),
        }
    }

    /// Parser limits used for both UDP datagrams and TCP streams.
    pub(crate) fn parser_config(&self) -> SipParserConfig {
        self.parser_config
    }

    /// Compatibility profile applied to SIP parsing/encoding for this listener.
    pub(crate) fn compatibility_profile(&self) -> Option<&CompatibilityProfile> {
        self.compatibility_profile.as_ref()
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

    /// Monotonic elapsed time since the driver started, used as the transaction
    /// clock (sub-second resolution for retransmission timers).
    pub(crate) fn now_monotonic(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Emits an event through the sink.
    pub(crate) fn emit(&self, event: M::Event) {
        self.sink.emit(event);
    }

    fn txns(&self, reliable: bool) -> &Mutex<TransactionManager<SocketAddr>> {
        if reliable {
            &self.tcp_txns
        } else {
            &self.udp_txns
        }
    }

    /// Routes a parsed message through the transaction layer and access
    /// machine, returning the transport actions (sends and events) to perform.
    ///
    /// Inbound responses are matched to client transactions; requests are
    /// tracked by a server transaction (giving retransmission absorption and
    /// duplicate suppression) before being dispatched to the access machine by
    /// method. TU-generated responses are handed back to the server
    /// transaction so they are cached and, on unreliable transports, armed for
    /// retransmission.
    pub(crate) fn handle_incoming(
        &self,
        source: SocketAddr,
        message: SipMessage,
        reliable: bool,
    ) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        match message {
            SipMessage::Response { .. } => self.handle_incoming_response(source, message, reliable),
            SipMessage::Request { .. } => self.handle_incoming_request(source, message, reliable),
        }
    }

    fn handle_incoming_response(
        &self,
        source: SocketAddr,
        message: SipMessage,
        reliable: bool,
    ) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        let now = self.now_monotonic();
        let manager_outputs = {
            let mut txns = self
                .txns(reliable)
                .lock()
                .map_err(|_| DriverError::AccessLock)?;
            txns.handle_response(message, now)
        };
        let mut actions = Vec::new();
        for output in manager_outputs {
            match output {
                ManagerOutput::Send { message, target } => {
                    actions.push(DriverAction::Send { message, target });
                }
                ManagerOutput::Deliver(delivered) => {
                    // A response matched a client transaction; hand it to the TU
                    // for correlation. The access machine ignores responses today
                    // but the path keeps the contract single-entry.
                    let outs = self.process_access(source, delivered)?;
                    actions.extend(outs);
                }
                ManagerOutput::Failure(kind) => {
                    debug!(?kind, "client transaction failed");
                }
            }
        }
        Ok(actions)
    }

    fn handle_incoming_request(
        &self,
        source: SocketAddr,
        message: SipMessage,
        reliable: bool,
    ) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        let now = self.now_monotonic();
        let route = route_request(&message);
        let request = message.clone();

        let RequestOutcome {
            key,
            deliver,
            outputs,
        } = {
            let mut txns = self
                .txns(reliable)
                .lock()
                .map_err(|_| DriverError::AccessLock)?;
            txns.handle_request(message, source, now)
        };

        let mut actions = Vec::new();
        for output in outputs {
            if let ManagerOutput::Send { message, target } = output {
                actions.push(DriverAction::Send { message, target });
            }
        }

        // Only deliver to the transaction user when the transaction layer says
        // this is a new (non-duplicate) request and the method routes to a
        // handler (an orphaned ACK is absorbed by the dialog, not answered).
        let mut deliver_request =
            (deliver && route.is_none_or(|r| r.delivers_to_tu())).then_some(request);

        // In-dialog requests (re-INVITE/BYE/INFO/SUBSCRIBE/NOTIFY/ACK/CANCEL
        // that carry both dialog tags) are routed through the dialog FSM, which
        // uses the dialog route state and absorbs CSeq reorder/duplicates. New
        // out-of-dialog requests fall through to the access machine unchanged.
        if let Some(req) = deliver_request.take() {
            if route.is_some_and(RequestRoute::is_dialog) {
                let mut dialogs = self.dialogs.lock().map_err(|_| DriverError::AccessLock)?;
                match dialogs.handle_request(req, now) {
                    DialogRouting::NoDialog(msg)
                    | DialogRouting::Deliver(msg)
                    | DialogRouting::Terminated(msg) => deliver_request = Some(msg),
                    DialogRouting::Absorbed => {}
                    DialogRouting::Failure(kind) => {
                        debug!(?kind, "in-dialog request rejected by dialog layer");
                    }
                }
            } else {
                deliver_request = Some(req);
            }
        }

        // Preserve the INVITE so a UAS dialog can be established once the TU
        // produces a 2xx final response.
        let establish_invite = match (&deliver_request, route) {
            (Some(req), Some(RequestRoute::Invite)) => Some(req.clone()),
            _ => None,
        };

        if let Some(req) = deliver_request {
            let access_outputs = self.process_access(source, req)?;
            for output in access_outputs {
                match output {
                    DriverAction::Send { message, target } => {
                        // A 2xx final response to an INVITE confirms a UAS
                        // dialog; register it so later in-dialog requests use
                        // dialog routing.
                        if target != source || !matches!(message, SipMessage::Response { .. }) {
                            actions.push(DriverAction::Send { message, target });
                            continue;
                        }
                        if let Some(invite) = establish_invite.as_ref()
                            && is_success_response(&message)
                            && let Some(local_tag) = response_to_tag(&message)
                        {
                            let mut dialogs =
                                self.dialogs.lock().map_err(|_| DriverError::AccessLock)?;
                            if let Err(e) = dialogs.establish_uas(invite, local_tag, now) {
                                debug!(error = %e, "failed to establish UAS dialog from 2xx");
                            }
                        }

                        // Route TU responses through the server transaction so
                        // they are cached and (on UDP) retransmitted.
                        if let Some(key) = &key {
                            let sends = {
                                let mut txns = self
                                    .txns(reliable)
                                    .lock()
                                    .map_err(|_| DriverError::AccessLock)?;
                                txns.provide_response(key, message, now)
                            };
                            for send in sends {
                                if let ManagerOutput::Send { message, target } = send {
                                    actions.push(DriverAction::Send { message, target });
                                }
                            }
                            continue;
                        }
                        actions.push(DriverAction::Send { message, target });
                    }
                    emit @ DriverAction::Emit(_) => actions.push(emit),
                }
            }
        }
        Ok(actions)
    }

    /// Feeds a message to the access machine and maps its outputs to actions.
    fn process_access(
        &self,
        source: SocketAddr,
        message: SipMessage,
    ) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        let input = AccessInput {
            source,
            now: self.now_seconds(),
            message,
        };
        let outputs = {
            let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
            access
                .process(input)
                .map_err(|e| DriverError::Access(Box::new(e)))?
        };
        Ok(outputs
            .into_iter()
            .map(|output| match output {
                AccessOutput::SendResponse(message) => DriverAction::Send {
                    message,
                    target: source,
                },
                AccessOutput::SendMessage { target, message } => {
                    DriverAction::Send { message, target }
                }
                AccessOutput::EmitEvent(event) => DriverAction::Emit(event),
            })
            .collect())
    }

    /// Dispatches a domain command through the access machine and starts a
    /// client transaction for each produced outbound SIP request.
    pub(crate) fn handle_command(
        &self,
        input: M::CommandInput,
    ) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        let outputs = {
            let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
            access
                .process_command(input)
                .map_err(|e| DriverError::Access(Box::new(e)))?
        };

        let mut actions = Vec::new();
        for output in outputs {
            match output {
                AccessOutput::SendMessage { target, message } => {
                    let now = self.now_monotonic();
                    let sends = {
                        let mut txns = self.udp_txns.lock().map_err(|_| DriverError::AccessLock)?;
                        txns.start_client_transaction(message, target, now)
                    };
                    for send in sends {
                        if let ManagerOutput::Send { message, target } = send {
                            actions.push(DriverAction::Send { message, target });
                        }
                    }
                }
                AccessOutput::SendResponse(_) => {
                    // Commands cannot produce responses without a source
                    // transaction; drop stray outputs.
                }
                AccessOutput::EmitEvent(event) => actions.push(DriverAction::Emit(event)),
            }
        }
        Ok(actions)
    }

    /// Advances the access machine's timers, mapping outputs to actions.
    pub(crate) fn tick_access(&self, now: u64) -> Result<Vec<DriverAction<M::Event>>, DriverError> {
        let outputs = {
            let mut access = self.access.lock().map_err(|_| DriverError::AccessLock)?;
            access
                .tick(now)
                .map_err(|e| DriverError::Access(Box::new(e)))?
        };
        // Tick-produced SIP responses/messages have no transaction route here;
        // only events are actionable.
        Ok(outputs
            .into_iter()
            .filter_map(|output| match output {
                AccessOutput::EmitEvent(event) => Some(DriverAction::Emit(event)),
                AccessOutput::SendResponse(_) | AccessOutput::SendMessage { .. } => None,
            })
            .collect())
    }

    /// Advances both transaction tables, returning retransmissions to send.
    ///
    /// Reliable (TCP) transactions never produce retransmissions; their timer
    /// expiries surface as failures which are logged and reclaimed.
    pub(crate) fn tick_transactions(&self, now: Duration) -> Vec<DriverAction<M::Event>> {
        if let Ok(mut dialogs) = self.dialogs.lock() {
            dialogs.tick(now);
        }
        let mut actions = Vec::new();
        for reliable in [false, true] {
            let outputs = match self.txns(reliable).lock() {
                Ok(mut txns) => txns.tick(now),
                Err(_) => continue,
            };
            for output in outputs {
                match output {
                    ManagerOutput::Send { message, target } => {
                        actions.push(DriverAction::Send { message, target });
                    }
                    ManagerOutput::Failure(kind) => debug!(?kind, "transaction timer expired"),
                    ManagerOutput::Deliver(_) => {}
                }
            }
        }
        actions
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

/// Returns true when `message` is a 2xx SIP response.
fn is_success_response(message: &SipMessage) -> bool {
    matches!(message, SipMessage::Response { line, .. } if (200..300).contains(&line.code))
}

/// Extracts the `tag` parameter from a response's `To` header, if present.
fn response_to_tag(message: &SipMessage) -> Option<String> {
    let value = message.headers().get(&HeaderName::To)?.as_str();
    cheetah_gb28181_core::sip::dialog::extract_tag(value).map(str::to_string)
}
