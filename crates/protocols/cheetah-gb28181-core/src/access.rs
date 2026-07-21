//! Sans-I/O GB28181 access state machine contract.
//!
//! This module defines the inputs, outputs and trait boundary for the GB28181
//! access state machine. Concrete implementations live in
//! `cheetah-gb28181-module` and are executed by `cheetah-gb28181-driver-tokio`.
//! Keeping the contract in core lets the driver depend only on core while the
//! module supplies the GB28181-specific business mapping.

use crate::SipMessage;
use std::fmt;
use std::net::SocketAddr;

/// An input to a GB28181 access state machine.
#[derive(Clone)]
pub struct AccessInput {
    /// Source address of the message.
    pub source: SocketAddr,
    /// Monotonic second counter used for nonce TTL and replay windows.
    pub now: u64,
    /// Parsed SIP message.
    pub message: SipMessage,
}

impl fmt::Debug for AccessInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessInput")
            .field("source", &self.source)
            .field("now", &self.now)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

/// An output from a GB28181 access state machine.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AccessOutput<E> {
    /// Send a SIP response to the transport.
    SendResponse(SipMessage),
    /// Emit a domain/protocol event for downstream consumers.
    EmitEvent(E),
}

impl<E: fmt::Debug> fmt::Debug for AccessOutput<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessOutput::SendResponse(_) => {
                f.debug_tuple("SendResponse").field(&"[REDACTED]").finish()
            }
            AccessOutput::EmitEvent(event) => f.debug_tuple("EmitEvent").field(event).finish(),
        }
    }
}

/// Sans-I/O wire state machine for GB28181 device access.
///
/// Implementations receive parsed SIP messages and periodic ticks, then
/// produce SIP responses and typed events. They must not perform I/O.
pub trait GbAccessMachine {
    /// Event type emitted by this machine.
    type Event: Send + Clone + fmt::Debug + 'static;
    /// Error type returned on internal failures.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Processes a single SIP message and returns ordered outputs.
    fn process(
        &mut self,
        input: AccessInput,
    ) -> Result<Vec<AccessOutput<Self::Event>>, Self::Error>;

    /// Processes a periodic tick (typically once per second) for expiry and
    /// heartbeat timeouts.
    fn tick(&mut self, now: u64) -> Result<Vec<AccessOutput<Self::Event>>, Self::Error>;
}
