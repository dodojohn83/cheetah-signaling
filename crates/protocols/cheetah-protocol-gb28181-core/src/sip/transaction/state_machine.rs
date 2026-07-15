//! Sans-I/O SIP transaction state machine.
//!
//! Implements RFC 3261 client and server transaction FSMs. The machine accepts
//! `TransactionEvent`s carrying a monotonic `Duration` and emits ordered
//! `TransactionOutput`s (send to transport, deliver to TU, set/cancel timers,
//! complete, or failure). Transport drivers are responsible for firing timers
//! and mapping transport failures to `TransactionEvent::TransportError`.

pub use super::timers::{TimerKind, TransactionConfig, TransportKind};
use super::{client::ClientTransaction, server::ServerTransaction};
use crate::{Method, SipErrorKind, SipMessage};
use std::time::Duration;

/// An input to a transaction state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionEvent {
    /// A SIP request received from the network (server transaction), or a
    /// request retransmission / ACK / CANCEL that matches an existing server
    /// transaction.
    Request(SipMessage),
    /// A SIP response. For client transactions this is from the network;
    /// for server transactions it is a response supplied by the TU.
    Response(SipMessage),
    /// A transaction timer fired.
    Timer(TimerKind),
    /// The transport layer reported an unrecoverable failure.
    TransportError,
}

/// An output from a transaction state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionOutput {
    /// Send the message to the transport layer.
    SendMessage(SipMessage),
    /// Deliver the message to the transaction user (TU).
    Deliver(SipMessage),
    /// Arm a timer that should fire at the given absolute monotonic deadline.
    SetTimer {
        /// Timer to arm.
        kind: TimerKind,
        /// Absolute monotonic deadline.
        deadline: Duration,
    },
    /// Disarm a timer.
    CancelTimer(TimerKind),
    /// The transaction has reached the Terminated state and may be destroyed.
    Complete,
    /// The transaction has failed; the TU should be informed.
    Failure(SipErrorKind),
}

/// A client or server SIP transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Transaction {
    /// Client transaction (UAC side).
    Client(Box<ClientTransaction>),
    /// Server transaction (UAS side).
    Server(Box<ServerTransaction>),
}

impl Transaction {
    /// Creates a client transaction for the given request.
    ///
    /// Returns `None` if the request is not a request message.
    pub fn new_client(request: SipMessage, config: TransactionConfig) -> Option<Self> {
        if !matches!(request, SipMessage::Request { .. }) {
            return None;
        }
        Some(Self::Client(Box::new(ClientTransaction::new(
            request, config,
        ))))
    }

    /// Creates a server transaction for the given request.
    ///
    /// Returns `None` if the request is not a request message.
    pub fn new_server(request: SipMessage, config: TransactionConfig) -> Option<Self> {
        if !matches!(request, SipMessage::Request { .. }) {
            return None;
        }
        Some(Self::Server(Box::new(ServerTransaction::new(
            request, config,
        ))))
    }

    /// Processes an event at the given monotonic time.
    #[must_use]
    pub fn process(&mut self, event: TransactionEvent, now: Duration) -> Vec<TransactionOutput> {
        match self {
            Self::Client(t) => t.process(event, now),
            Self::Server(t) => t.process(event, now),
        }
    }

    /// Returns true if the transaction has terminated.
    pub fn is_terminated(&self) -> bool {
        match self {
            Self::Client(t) => t.is_terminated(),
            Self::Server(t) => t.is_terminated(),
        }
    }

    /// Convenience constructor for a client INVITE transaction.
    pub fn client_invite(request: SipMessage, config: TransactionConfig) -> Option<Self> {
        let method = match &request {
            SipMessage::Request { line, .. } => line.method.clone(),
            _ => return None,
        };
        if method != Method::Invite {
            return None;
        }
        Self::new_client(request, config)
    }

    /// Convenience constructor for a server INVITE transaction.
    pub fn server_invite(request: SipMessage, config: TransactionConfig) -> Option<Self> {
        let method = match &request {
            SipMessage::Request { line, .. } => line.method.clone(),
            _ => return None,
        };
        if method != Method::Invite {
            return None;
        }
        Self::new_server(request, config)
    }
}

/// True if the status code is provisional (1xx).
pub(super) fn is_provisional(code: u16) -> bool {
    (100..200).contains(&code)
}

/// True if the status code is successful (2xx).
pub(super) fn is_success(code: u16) -> bool {
    (200..300).contains(&code)
}

/// True if the status code is a final failure (3xx-6xx).
pub(super) fn is_failure(code: u16) -> bool {
    (300..700).contains(&code)
}

/// Extracts the request method from a `SipMessage::Request`.
pub(super) fn request_method(msg: &SipMessage) -> Option<Method> {
    match msg {
        SipMessage::Request { line, .. } => Some(line.method.clone()),
        _ => None,
    }
}

/// Extracts the status code from a `SipMessage::Response`.
pub(super) fn response_code(msg: &SipMessage) -> Option<u16> {
    match msg {
        SipMessage::Response { line, .. } => Some(line.code),
        _ => None,
    }
}
