//! Northbound HTTP API for Cheetah Signaling.
#![doc = include_str!("../README.md")]

pub mod auth;
pub mod error;
pub mod extract;
pub mod handlers;
pub mod metrics;
pub mod openapi;
pub mod router;
pub mod state;
pub mod webhook;

pub use auth::AuthContext;
pub use error::{FieldViolation, HttpError, ProblemDetails};
pub use extract::{ApiRequestContext, IdempotencyKey, ListQuery};
pub use router::build_router;
pub use state::{ApiConfig, ApiServer, ApiState};
