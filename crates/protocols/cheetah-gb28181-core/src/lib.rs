//! Sans-I/O GB28181 SIP/SDP protocol core.
//!
//! This crate provides message types, headers, URI handling, a streaming
//! SIP parser/encoder and a limited SDP parser/encoder. It does not perform
//! any network I/O; transport drivers are implemented in
//! `cheetah-gb28181-driver-tokio`.

#![warn(missing_docs)]

pub mod access;
pub mod sdp;
pub mod sip;

pub use access::{AccessInput, AccessOutput, GbAccessMachine};
pub use sdp::{
    RtpMap, SdpAttribute, SdpConnection, SdpConnectionType, SdpDirection, SdpError, SdpMedia,
    SdpOrigin, SdpParserConfig, SdpSession, SdpSetup, SdpTime, encode_sdp, parse_sdp,
};

pub use sip::dialog::{Dialog, DialogEvent, DialogId, DialogOutput, DialogRole, DialogState};
pub use sip::digest::{
    DigestAlgorithm, DigestChallenge, DigestClient, DigestContext, DigestError, DigestQop,
    DigestReplayCache, DigestResponse,
};
pub use sip::encoder::encode_message;
pub use sip::error::{SipError, SipErrorKind};
pub use sip::headers::{HeaderName, HeaderValue, SipHeaders};
pub use sip::message::{Body, Method, RequestLine, ResponseClass, SipMessage, StatusLine};
pub use sip::parser::{SipParser, SipParserConfig};
pub use sip::transaction::{
    BranchPolicy, TimerKind, TimerSet, Transaction, TransactionConfig, TransactionEvent,
    TransactionHalf, TransactionKey, TransactionKind, TransactionOutput, TransactionRole,
    TransportKind,
};
pub use sip::uri::{Scheme, SipUri};
