//! Standalone migration tool for Cheetah Signaling.
//!
//! Reads tenant, device, channel and secret-reference records from an old
//! system (CSV or JSON) and imports them into the target storage backend.
//! The tool supports dry-run, cutover filtering, idempotent re-runs and
//! per-batch checkpoint commits.

#![warn(missing_docs)]

pub mod clock;
pub mod error;
pub mod importer;
pub mod mappers;
pub mod model;
pub mod source;
