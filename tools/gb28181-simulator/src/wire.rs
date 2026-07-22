//! Shared wire-level types for the deterministic harness.
//!
//! Frames carry already-encoded SIP bytes between endpoints together with the
//! semantic metadata the fault engine and transcript need.  Raw payload bytes
//! are never recorded in the transcript; only semantic summaries are.

use crate::scenario::MessageClass;
use cheetah_gb28181_core::{Method, SipMessage};

/// A simulated network endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// The signalling platform / registrar peer.
    Platform,
    /// A simulated device by index.
    Device(u32),
}

impl Endpoint {
    /// Whether this endpoint is a device.
    pub fn is_device(self) -> bool {
        matches!(self, Endpoint::Device(_))
    }

    /// Stable label for RNG stream and transcript use.
    pub fn label(self) -> String {
        match self {
            Endpoint::Platform => "platform".to_string(),
            Endpoint::Device(i) => format!("device-{i}"),
        }
    }
}

/// An encoded frame in flight between two endpoints.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Source endpoint.
    pub from: Endpoint,
    /// Destination endpoint.
    pub to: Endpoint,
    /// Encoded SIP bytes.
    pub bytes: Vec<u8>,
    /// Coarse semantic class used for fault targeting.
    pub class: MessageClass,
    /// Short semantic descriptor recorded in the transcript (never raw bytes).
    pub summary: String,
}

impl Frame {
    /// Whether the frame originates from a device.
    pub fn from_device(&self) -> bool {
        self.from.is_device()
    }
}

/// Classifies a message into a coarse [`MessageClass`] for fault targeting.
pub fn classify(msg: &SipMessage) -> MessageClass {
    match msg {
        SipMessage::Request { line, body, .. } => match line.method {
            Method::Register => MessageClass::Register,
            Method::Invite | Method::Ack | Method::Bye | Method::Cancel | Method::Info => {
                MessageClass::Media
            }
            Method::Message => classify_message_body(body),
            _ => MessageClass::Message,
        },
        SipMessage::Response { .. } => MessageClass::Any,
    }
}

fn classify_message_body(body: &[u8]) -> MessageClass {
    let text = String::from_utf8_lossy(body);
    if text.contains("Keepalive") {
        MessageClass::Keepalive
    } else if text.contains("Catalog") {
        MessageClass::Catalog
    } else {
        MessageClass::Message
    }
}

/// Produces a semantic, payload-free transcript summary for a message.
pub fn summarize(msg: &SipMessage) -> String {
    match msg {
        SipMessage::Request { line, .. } => {
            format!("{:?} {}", line.method, class_tag(classify(msg)))
        }
        SipMessage::Response { line, .. } => format!("{}", line.code),
    }
}

fn class_tag(class: MessageClass) -> &'static str {
    match class {
        MessageClass::Any => "",
        MessageClass::Register => "register",
        MessageClass::Keepalive => "keepalive",
        MessageClass::Catalog => "catalog",
        MessageClass::Media => "media",
        MessageClass::Message => "message",
    }
}
