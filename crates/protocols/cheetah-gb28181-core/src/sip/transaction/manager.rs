//! Bounded transaction table that drives the client/server FSMs.
//!
//! [`TransactionManager`] owns the RFC 3261 client and server transaction state
//! machines, keyed by [`TransactionKey`]. It provides the glue that the tasks in
//! `03_architecture_transport_and_runtime.md` call for:
//!
//! - **retransmission** — request/response retransmissions are produced by the
//!   underlying FSMs (timers A/E for requests, G/H for INVITE final responses)
//!   and surfaced as [`ManagerOutput::Send`] on [`TransactionManager::tick`];
//! - **duplicate detection** — a retransmitted request that matches an existing
//!   server transaction replays the cached response instead of being delivered
//!   to the transaction user (TU) again;
//! - **late / out-of-order messages** — a response with no matching client
//!   transaction is dropped with a bounded diagnostic rather than delivered;
//! - **deadlines** — timers B and F terminate a transaction that never
//!   completes, surfaced as [`ManagerOutput::Failure`];
//! - **bounded size and TTL** — the per-role maps never exceed a configured
//!   capacity (least-recently-active entries are evicted) and terminated or
//!   idle entries are reclaimed after a configured time-to-live.
//!
//! The manager is generic over an opaque routing target `T` (for example a
//! `SocketAddr` in the Tokio driver) so it stays free of transport concerns.
//! Time is expressed as a monotonic [`Duration`]; callers supply `now` and the
//! manager never reads a clock itself.

use std::collections::HashMap;
use std::time::Duration;

use super::key::{BranchPolicy, TransactionKey, TransactionRole};
use super::state_machine::{Transaction, TransactionConfig, TransactionEvent, TransactionOutput};
use super::timers::TimerKind;
use crate::{Method, SipErrorKind, SipMessage};

/// Default maximum number of live transactions per role (client and server).
pub const DEFAULT_MAX_TRANSACTIONS: usize = 4096;
/// Absolute maximum number of live transactions per role. Larger values would
/// allow an unbounded HashMap to grow and disable eviction.
const MAX_TRANSACTIONS_LIMIT: usize = 1_048_576;
/// Default idle/terminated time-to-live before an entry is reclaimed.
pub const DEFAULT_TRANSACTION_TTL: Duration = Duration::from_secs(64);

/// Configuration for a [`TransactionManager`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagerConfig {
    /// Timer configuration and transport reliability applied to every
    /// transaction created by this manager.
    pub transaction: TransactionConfig,
    /// Branch-matching policy used when deriving transaction keys.
    pub branch_policy: BranchPolicy,
    /// Maximum number of live transactions per role.
    pub max_transactions: usize,
    /// Time-to-live after which idle or terminated entries are reclaimed.
    pub ttl: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            transaction: TransactionConfig::default(),
            branch_policy: BranchPolicy::default(),
            max_transactions: DEFAULT_MAX_TRANSACTIONS,
            ttl: DEFAULT_TRANSACTION_TTL,
        }
    }
}

/// An instruction produced by the transaction manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagerOutput<T> {
    /// Transmit a SIP message to the transaction's remote endpoint `target`.
    Send {
        /// Message to transmit.
        message: SipMessage,
        /// Opaque routing target of the transaction.
        target: T,
    },
    /// Deliver a SIP message to the transaction user for business handling.
    Deliver(SipMessage),
    /// A transaction failed (timeout, cancellation or transport failure).
    Failure(SipErrorKind),
}

/// Outcome of feeding an inbound request to the manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestOutcome<T> {
    /// Server transaction key when the request is tracked by a transaction.
    ///
    /// The caller uses this key with [`TransactionManager::provide_response`]
    /// to hand the TU's response back so it is cached for retransmission.
    pub key: Option<TransactionKey>,
    /// True when the request must be delivered to the transaction user.
    ///
    /// Retransmissions that were absorbed (duplicate detection) set this to
    /// `false`; the accompanying [`Self::outputs`] replay the cached response.
    pub deliver: bool,
    /// Immediate outputs (for example a replayed cached response).
    pub outputs: Vec<ManagerOutput<T>>,
}

/// A single tracked transaction plus its routing target and armed timers.
#[derive(Clone, Debug)]
struct Entry<T> {
    transaction: Transaction,
    target: T,
    timers: HashMap<TimerKind, Duration>,
    last_activity: Duration,
}

impl<T: Clone> Entry<T> {
    fn new(transaction: Transaction, target: T, now: Duration) -> Self {
        Self {
            transaction,
            target,
            timers: HashMap::new(),
            last_activity: now,
        }
    }

    /// Applies raw transaction outputs, updating the armed-timer table and
    /// translating them into manager outputs. Returns `true` when the
    /// transaction reached its terminal state and must be removed.
    fn absorb(
        &mut self,
        outputs: Vec<TransactionOutput>,
        now: Duration,
        sink: &mut Vec<ManagerOutput<T>>,
    ) -> bool {
        self.last_activity = now;
        let mut terminated = false;
        for output in outputs {
            match output {
                TransactionOutput::SendMessage(message) => sink.push(ManagerOutput::Send {
                    message,
                    target: self.target.clone(),
                }),
                TransactionOutput::Deliver(message) => sink.push(ManagerOutput::Deliver(message)),
                TransactionOutput::SetTimer { kind, deadline } => {
                    self.timers.insert(kind, deadline);
                }
                TransactionOutput::CancelTimer(kind) => {
                    self.timers.remove(&kind);
                }
                TransactionOutput::Complete => terminated = true,
                TransactionOutput::Failure(kind) => sink.push(ManagerOutput::Failure(kind)),
            }
        }
        terminated || self.transaction.is_terminated()
    }

    /// Returns the timer kinds whose deadline is at or before `now`.
    fn due_timers(&self, now: Duration) -> Vec<TimerKind> {
        let mut due: Vec<TimerKind> = self
            .timers
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(kind, _)| *kind)
            .collect();
        // Deterministic ordering keeps outputs stable for golden tests.
        due.sort_by_key(|kind| *kind as u8);
        due
    }
}

/// Owns the SIP client and server transactions for one shard/worker.
#[derive(Clone, Debug)]
pub struct TransactionManager<T> {
    config: ManagerConfig,
    client: HashMap<TransactionKey, Entry<T>>,
    server: HashMap<TransactionKey, Entry<T>>,
}

impl<T: Clone> TransactionManager<T> {
    /// Creates a manager with the given configuration.
    pub fn new(mut config: ManagerConfig) -> Self {
        config.max_transactions = config.max_transactions.clamp(1, MAX_TRANSACTIONS_LIMIT);
        Self {
            config,
            client: HashMap::new(),
            server: HashMap::new(),
        }
    }

    /// Number of live server transactions.
    pub fn server_len(&self) -> usize {
        self.server.len()
    }

    /// Number of live client transactions.
    pub fn client_len(&self) -> usize {
        self.client.len()
    }

    /// Derives the server-side transaction key for a message under the
    /// manager's branch policy.
    pub fn server_key(&self, message: &SipMessage) -> Option<TransactionKey> {
        TransactionKey::for_message(message, TransactionRole::Uas, self.config.branch_policy)
    }

    /// Derives the client-side transaction key for a message under the
    /// manager's branch policy.
    pub fn client_key(&self, message: &SipMessage) -> Option<TransactionKey> {
        TransactionKey::for_message(message, TransactionRole::Uac, self.config.branch_policy)
    }

    /// Feeds an inbound request (received from the network) to the server-side
    /// transaction layer.
    ///
    /// A new request creates a server transaction and is delivered to the TU. A
    /// retransmission that matches an existing transaction replays the cached
    /// response and is not redelivered. An `ACK` or `CANCEL` with no matching
    /// transaction is delivered to the TU (for dialog / stateless handling)
    /// without creating a transaction.
    pub fn handle_request(
        &mut self,
        message: SipMessage,
        target: T,
        now: Duration,
    ) -> RequestOutcome<T> {
        let method = match &message {
            SipMessage::Request { line, .. } => line.method.clone(),
            SipMessage::Response { .. } => {
                return RequestOutcome {
                    key: None,
                    deliver: false,
                    outputs: Vec::new(),
                };
            }
        };

        let Some(key) = self.server_key(&message) else {
            // No usable transaction key: deliver statelessly so the TU can still
            // handle the request, but do not track a transaction.
            return RequestOutcome {
                key: None,
                deliver: true,
                outputs: Vec::new(),
            };
        };

        if let Some(entry) = self.server.get_mut(&key) {
            // Existing server transaction: retransmission, ACK or CANCEL.
            let mut outputs = Vec::new();
            let raw = entry
                .transaction
                .process(TransactionEvent::Request(message), now);
            let deliver = raw
                .iter()
                .any(|o| matches!(o, TransactionOutput::Deliver(_)));
            let terminated = entry.absorb(raw, now, &mut outputs);
            if terminated {
                self.server.remove(&key);
            }
            // The delivered request is conveyed via `deliver` + `key`; the
            // caller already holds the original message, so drop the redundant
            // `Deliver` output and surface only sends/failures.
            outputs.retain(|o| !matches!(o, ManagerOutput::Deliver(_)));
            return RequestOutcome {
                key: Some(key),
                deliver,
                outputs,
            };
        }

        // No existing transaction. An orphaned ACK (e.g. ACK to a 2xx whose
        // INVITE server transaction already terminated) must not create a new
        // server transaction; it is delivered to the dialog layer instead.
        if matches!(method, Method::Ack) {
            return RequestOutcome {
                key: None,
                deliver: true,
                outputs: Vec::new(),
            };
        }

        let Some(mut transaction) = Transaction::new_server(message, self.config.transaction)
        else {
            return RequestOutcome {
                key: None,
                deliver: true,
                outputs: Vec::new(),
            };
        };

        self.ensure_server_capacity(now);
        let bootstrap = match &mut transaction {
            Transaction::Server(t) => t.bootstrap(),
            Transaction::Client(_) => Vec::new(),
        };
        let mut entry = Entry::new(transaction, target, now);
        let mut outputs = Vec::new();
        let deliver = bootstrap
            .iter()
            .any(|o| matches!(o, TransactionOutput::Deliver(_)));
        let terminated = entry.absorb(bootstrap, now, &mut outputs);
        if !terminated {
            self.server.insert(key.clone(), entry);
        }
        // The delivered request is conveyed via `deliver` + `key`.
        outputs.retain(|o| !matches!(o, ManagerOutput::Deliver(_)));
        RequestOutcome {
            key: Some(key),
            deliver,
            outputs,
        }
    }

    /// Hands a TU-generated response back to its server transaction so it is
    /// sent, cached for retransmission and (for final responses) armed with the
    /// completion timers.
    pub fn provide_response(
        &mut self,
        key: &TransactionKey,
        response: SipMessage,
        now: Duration,
    ) -> Vec<ManagerOutput<T>> {
        let mut outputs = Vec::new();
        if let Some(entry) = self.server.get_mut(key) {
            let raw = entry
                .transaction
                .process(TransactionEvent::Response(response), now);
            let terminated = entry.absorb(raw, now, &mut outputs);
            if terminated {
                self.server.remove(key);
            }
        }
        outputs
    }

    /// Feeds an inbound response (received from the network) to the client-side
    /// transaction layer.
    ///
    /// A response that matches a live client transaction is delivered to the TU
    /// (and, for a non-2xx INVITE final response, triggers an `ACK`). A response
    /// with no matching transaction is a late, retransmitted or forked response
    /// and is dropped.
    pub fn handle_response(&mut self, message: SipMessage, now: Duration) -> Vec<ManagerOutput<T>> {
        if !matches!(message, SipMessage::Response { .. }) {
            return Vec::new();
        }
        let Some(key) = self.client_key(&message) else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        if let Some(entry) = self.client.get_mut(&key) {
            let raw = entry
                .transaction
                .process(TransactionEvent::Response(message), now);
            let terminated = entry.absorb(raw, now, &mut outputs);
            if terminated {
                self.client.remove(&key);
            }
        }
        outputs
    }

    /// Starts a client transaction for an outbound request, returning the
    /// initial send plus the armed retransmission and timeout timers.
    ///
    /// Returns an empty vector when the message is not a request or lacks a
    /// usable transaction key.
    pub fn start_client_transaction(
        &mut self,
        request: SipMessage,
        target: T,
        now: Duration,
    ) -> Vec<ManagerOutput<T>> {
        let Some(key) = self.client_key(&request) else {
            return Vec::new();
        };
        let Some(mut transaction) = Transaction::new_client(request, self.config.transaction)
        else {
            return Vec::new();
        };
        self.ensure_client_capacity(now);
        let bootstrap = match &mut transaction {
            Transaction::Client(t) => t.bootstrap(now),
            Transaction::Server(_) => Vec::new(),
        };
        let mut entry = Entry::new(transaction, target, now);
        let mut outputs = Vec::new();
        let terminated = entry.absorb(bootstrap, now, &mut outputs);
        if !terminated {
            self.client.insert(key, entry);
        }
        outputs
    }

    /// Advances every transaction's timers, reclaims terminated and idle
    /// entries, and returns the resulting sends and failures.
    pub fn tick(&mut self, now: Duration) -> Vec<ManagerOutput<T>> {
        let mut outputs = Vec::new();
        Self::tick_map(&mut self.server, self.config.ttl, now, &mut outputs);
        Self::tick_map(&mut self.client, self.config.ttl, now, &mut outputs);
        outputs
    }

    fn tick_map(
        map: &mut HashMap<TransactionKey, Entry<T>>,
        ttl: Duration,
        now: Duration,
        outputs: &mut Vec<ManagerOutput<T>>,
    ) {
        let mut remove = Vec::new();
        for (key, entry) in map.iter_mut() {
            let mut terminated = false;
            for kind in entry.due_timers(now) {
                // A previous timer in this batch may have cancelled this one.
                if !entry.timers.contains_key(&kind) {
                    continue;
                }
                entry.timers.remove(&kind);
                let raw = entry
                    .transaction
                    .process(TransactionEvent::Timer(kind), now);
                if entry.absorb(raw, now, outputs) {
                    terminated = true;
                    break;
                }
            }
            if terminated
                || entry.transaction.is_terminated()
                || now.saturating_sub(entry.last_activity) >= ttl
            {
                remove.push(key.clone());
            }
        }
        for key in remove {
            map.remove(&key);
        }
    }

    fn ensure_server_capacity(&mut self, now: Duration) {
        Self::ensure_capacity(&mut self.server, self.config.max_transactions, now);
    }

    fn ensure_client_capacity(&mut self, now: Duration) {
        Self::ensure_capacity(&mut self.client, self.config.max_transactions, now);
    }

    /// Ensures room for one more entry, evicting terminated entries first and
    /// then the least-recently-active entry when still at capacity.
    fn ensure_capacity(map: &mut HashMap<TransactionKey, Entry<T>>, max: usize, _now: Duration) {
        if max == 0 || map.len() < max {
            return;
        }
        if let Some(key) = map
            .iter()
            .find(|(_, e)| e.transaction.is_terminated())
            .map(|(k, _)| k.clone())
        {
            map.remove(&key);
            return;
        }
        if let Some(key) = map
            .iter()
            .min_by_key(|(_, e)| e.last_activity)
            .map(|(k, _)| k.clone())
        {
            map.remove(&key);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{SipParser, SipParserConfig, StatusLine};

    const T: () = ();

    fn parse(data: &str) -> SipMessage {
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    fn register(branch: &str, cseq: u32) -> SipMessage {
        parse(&format!(
            "REGISTER sip:registrar.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.168.1.1:5060;branch={branch}\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:alice@example.com>\r\n\
             Call-ID: call-reg@example.com\r\n\
             CSeq: {cseq} REGISTER\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    fn invite(branch: &str) -> SipMessage {
        parse(&format!(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.168.1.1:5060;branch={branch}\r\n\
             From: <sip:alice@example.com>;tag=abc\r\n\
             To: <sip:bob@example.com>\r\n\
             Call-ID: call-inv@example.com\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    fn response_for(
        request: &SipMessage,
        code: u16,
        reason: &str,
        to_tag: Option<&str>,
    ) -> SipMessage {
        let SipMessage::Request { headers, .. } = request else {
            unreachable!()
        };
        let mut h = crate::SipHeaders::new();
        for name in [
            crate::HeaderName::Via,
            crate::HeaderName::From,
            crate::HeaderName::CallId,
            crate::HeaderName::CSeq,
        ] {
            if let Some(v) = headers.get(&name) {
                h.append(name, v.clone());
            }
        }
        let to = headers.get(&crate::HeaderName::To).unwrap().as_str();
        let to = match to_tag {
            Some(tag) => format!("{to};tag={tag}"),
            None => to.to_string(),
        };
        h.append(crate::HeaderName::To, crate::HeaderValue::new(to));
        SipMessage::Response {
            line: StatusLine::new(code, reason),
            headers: h,
            body: Vec::new(),
        }
    }

    fn manager() -> TransactionManager<()> {
        TransactionManager::new(ManagerConfig::default())
    }

    #[test]
    fn max_transactions_is_clamped() {
        let mut huge = TransactionManager::new(ManagerConfig {
            max_transactions: usize::MAX,
            ..ManagerConfig::default()
        });
        let req1 = register("z9hG4bKhuge1", 1);
        let req2 = register("z9hG4bKhuge2", 2);
        huge.handle_request(req1, T, Duration::ZERO);
        huge.handle_request(req2, T, Duration::ZERO);
        assert_eq!(
            huge.server_len(),
            2,
            "huge max_transactions must be clamped, not zero"
        );

        let mut zero = TransactionManager::new(ManagerConfig {
            max_transactions: 0,
            ..ManagerConfig::default()
        });
        let req1 = register("z9hG4bKzero1", 1);
        let req2 = register("z9hG4bKzero2", 2);
        zero.handle_request(req1, T, Duration::ZERO);
        zero.handle_request(req2, T, Duration::ZERO);
        assert_eq!(
            zero.server_len(),
            1,
            "zero max_transactions must be clamped to 1"
        );
    }

    #[test]
    fn new_request_creates_server_transaction_and_delivers() {
        let mut mgr = manager();
        let outcome = mgr.handle_request(register("z9hG4bK1", 1), T, Duration::ZERO);
        assert!(outcome.deliver);
        assert!(outcome.key.is_some());
        assert_eq!(mgr.server_len(), 1);
        assert!(outcome.outputs.is_empty());
    }

    #[test]
    fn duplicate_request_replays_cached_response_without_redelivery() {
        let mut mgr = manager();
        let req = register("z9hG4bK1", 1);
        let outcome = mgr.handle_request(req.clone(), T, Duration::ZERO);
        let key = outcome.key.unwrap();

        // TU answers 200; final response is cached.
        let resp = response_for(&req, 200, "OK", Some("srv"));
        let sent = mgr.provide_response(&key, resp, Duration::ZERO);
        assert!(matches!(sent.as_slice(), [ManagerOutput::Send { .. }]));

        // Non-INVITE server transaction stays in Completed (timer J) for UDP.
        let dup = mgr.handle_request(req, T, Duration::from_millis(100));
        assert!(!dup.deliver, "retransmission must not be redelivered");
        assert!(
            matches!(dup.outputs.as_slice(), [ManagerOutput::Send { .. }]),
            "retransmission must replay the cached final response"
        );
    }

    #[test]
    fn client_transaction_retransmits_then_times_out() {
        let mut mgr = manager();
        let out = mgr.start_client_transaction(register("z9hG4bKc", 1), T, Duration::ZERO);
        // Initial send only (timers are internal to the manager).
        assert!(matches!(out.as_slice(), [ManagerOutput::Send { .. }]));
        assert_eq!(mgr.client_len(), 1);

        // Timer E fires at 500ms -> retransmission.
        let out = mgr.tick(Duration::from_millis(500));
        assert!(
            out.iter().any(|o| matches!(o, ManagerOutput::Send { .. })),
            "timer E must retransmit the request"
        );

        // Timer F (64*T1 = 32s) fires -> timeout failure and eviction.
        let out = mgr.tick(Duration::from_secs(32));
        assert!(
            out.iter()
                .any(|o| matches!(o, ManagerOutput::Failure(SipErrorKind::TransactionTimeout))),
            "timer F must fail the transaction"
        );
        assert_eq!(mgr.client_len(), 0, "timed-out transaction is evicted");
    }

    #[test]
    fn unmatched_response_is_dropped() {
        let mut mgr = manager();
        let resp = response_for(&register("z9hG4bKx", 1), 200, "OK", Some("srv"));
        let out = mgr.handle_response(resp, Duration::ZERO);
        assert!(out.is_empty(), "late/stray response is dropped");
    }

    #[test]
    fn invite_client_acks_non_2xx_final() {
        let mut mgr = manager();
        let inv = invite("z9hG4bKi");
        mgr.start_client_transaction(inv.clone(), T, Duration::ZERO);
        let resp = response_for(&inv, 404, "Not Found", Some("srv"));
        let out = mgr.handle_response(resp, Duration::ZERO);
        // Deliver the response and send the ACK.
        assert!(out.iter().any(|o| matches!(o, ManagerOutput::Deliver(_))));
        assert!(out.iter().any(|o| matches!(o, ManagerOutput::Send { .. })));
    }

    #[test]
    fn server_capacity_is_bounded() {
        let mut mgr = TransactionManager::new(ManagerConfig {
            max_transactions: 2,
            ..ManagerConfig::default()
        });
        mgr.handle_request(register("z9hG4bKa", 1), T, Duration::from_secs(1));
        mgr.handle_request(register("z9hG4bKb", 2), T, Duration::from_secs(2));
        mgr.handle_request(register("z9hG4bKc", 3), T, Duration::from_secs(3));
        assert!(
            mgr.server_len() <= 2,
            "server map must never exceed capacity"
        );
    }

    #[test]
    fn idle_entries_are_reclaimed_by_ttl() {
        let mut mgr = TransactionManager::new(ManagerConfig {
            ttl: Duration::from_secs(10),
            ..ManagerConfig::default()
        });
        mgr.handle_request(register("z9hG4bKa", 1), T, Duration::ZERO);
        assert_eq!(mgr.server_len(), 1);
        let _ = mgr.tick(Duration::from_secs(11));
        assert_eq!(mgr.server_len(), 0, "idle entry reclaimed after TTL");
    }
}
