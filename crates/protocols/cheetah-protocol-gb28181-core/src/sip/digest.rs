//! RFC 2617/7616 Digest authentication for SIP.
//!
//! This is a Sans-I/O server-side implementation. It generates and validates
//! nonces, parses `Authorization` Digest responses, computes H(A1)/H(A2), and
//! performs constant-time response comparison. All time values are supplied by
//! the caller as a monotonically non-decreasing second counter.

pub(crate) mod context;
pub(crate) mod nonce;
pub(crate) mod replay_cache;
pub(crate) mod response;

pub use context::DigestContext;
pub use replay_cache::DigestReplayCache;
pub use response::{DigestAlgorithm, DigestChallenge, DigestError, DigestQop, DigestResponse};
