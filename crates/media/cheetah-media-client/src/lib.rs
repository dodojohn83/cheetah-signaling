//! Media control gRPC client with connection pooling, retries and circuit breaker.

#![warn(missing_docs)]

pub mod client;
pub mod config;
pub mod error;

pub use client::{MediaControlClient, MediaControlRequest, MediaListSessionsRequest};
pub use config::MediaClientConfig;
pub use error::MediaClientError;
