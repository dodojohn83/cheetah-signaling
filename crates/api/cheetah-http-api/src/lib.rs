//! Northbound HTTP API for Cheetah Signaling.
#![doc = include_str!("../README.md")]

pub mod audit;
pub mod auth;
pub mod error;
pub mod event_cache;
pub mod extract;
pub mod handlers;
pub mod json_body;
pub mod logging;
pub mod metrics;
pub mod openapi;
pub mod rate_limit;
pub mod router;
pub mod state;
pub mod webhook;

pub use auth::AuthContext;
pub use error::{FieldViolation, HttpError, ProblemDetails};
pub use extract::{ApiRequestContext, IdempotencyKey, IfMatchRevision, ListQuery};
pub use json_body::JsonBody;
pub use router::build_router;
pub use state::{ApiConfig, ApiServer, ApiState};
