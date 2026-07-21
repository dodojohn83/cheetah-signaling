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
pub use sip::dialog_manager::{
    DEFAULT_DIALOG_TTL, DEFAULT_MAX_DIALOGS, DialogManager, DialogManagerConfig, DialogRouting,
};
pub use sip::digest::{
    AuthRateLimiter, DigestAlgorithm, DigestChallenge, DigestClient, DigestContext, DigestError,
    DigestQop, DigestReplayCache, DigestResponse,
};
pub use sip::encoder::encode_message;
pub use sip::endpoint::{
    DEFAULT_SIP_PORT, EndpointRoute, RouteUpdateContext, Rport, ViaRouteParams,
    socket_addr_from_uri,
};
pub use sip::error::{SipError, SipErrorKind};
pub use sip::headers::{HeaderName, HeaderValue, SipHeaders};
pub use sip::message::{Body, Method, RequestLine, ResponseClass, SipMessage, StatusLine};
pub use sip::parser::{SipParser, SipParserConfig};
pub use sip::routing::{RequestRoute, route_request};
pub use sip::transaction::{
    BranchPolicy, DEFAULT_MAX_TRANSACTIONS, DEFAULT_TRANSACTION_TTL, ManagerConfig, ManagerOutput,
    RequestOutcome, TimerKind, TimerSet, Transaction, TransactionConfig, TransactionEvent,
    TransactionHalf, TransactionKey, TransactionKind, TransactionManager, TransactionOutput,
    TransactionRole, TransportKind,
};
pub use sip::uri::{Scheme, SipUri};
