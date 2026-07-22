//! RFC 2617/7616 Digest authentication for SIP.
//!
//! This is a Sans-I/O implementation. It generates and validates nonces, parses
//! `Authorization` Digest responses, computes H(A1)/H(A2), builds client
//! `Authorization` headers, and performs constant-time response comparison. All
//! time values are supplied by the caller as a monotonically non-decreasing
//! second counter.

pub(crate) mod client;
pub(crate) mod context;
pub(crate) mod nonce;
pub(crate) mod rate_limit;
pub(crate) mod replay_cache;
pub(crate) mod response;

pub use client::DigestClient;
pub use context::DigestContext;
pub use rate_limit::AuthRateLimiter;
pub use replay_cache::DigestReplayCache;
pub use response::{DigestAlgorithm, DigestChallenge, DigestError, DigestQop, DigestResponse};
