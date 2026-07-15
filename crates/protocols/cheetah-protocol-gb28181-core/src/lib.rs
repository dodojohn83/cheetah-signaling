//! Sans-I/O GB28181 SIP protocol core.
//!
//! This crate provides message types, headers, URI handling and a streaming
//! parser/encoder that does not perform any network I/O. Transport drivers
//! are implemented in `cheetah-protocol-gb28181-driver-tokio`.

#![warn(missing_docs)]

pub mod sip;

pub use sip::encoder::encode_message;
pub use sip::error::{SipError, SipErrorKind};
pub use sip::headers::{HeaderName, HeaderValue, SipHeaders};
pub use sip::message::{Body, Method, RequestLine, ResponseClass, SipMessage, StatusLine};
pub use sip::parser::{SipParser, SipParserConfig};
pub use sip::transaction::{
    BranchPolicy, TransactionHalf, TransactionKey, TransactionKind, TransactionRole,
};
pub use sip::uri::{Scheme, SipUri};
