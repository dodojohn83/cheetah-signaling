//! Shared contract tests for the SQLite and PostgreSQL storage adapters.
//!
//! The `fixtures` module provides deterministic builders and test inputs.
//! The `contract` module defines the suite of repository assertions that run
//! against any [`cheetah_storage_api::Storage`] implementation.

pub mod contract;
pub mod fixtures;
