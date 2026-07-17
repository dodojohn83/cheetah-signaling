//! Test kit for Cheetah plugin contract tests.
//!
//! This crate provides the building blocks used by both the example out-of-process
//! plugin and the host-side integration tests:
//!
//! - [`TestCerts`](certs::TestCerts) generates a CA and server/client key pairs
//!   that satisfy the out-of-process host's mTLS identity verifier.
//! - [`FakePluginRuntime`](server::FakePluginRuntime) is a minimal `PluginRuntime`
//!   gRPC service.
//! - [`MockHost`](host::MockHost) implements the SDK's `DeviceSink` and
//!   `CommandSource` ports and records events/commands for assertions.
//!
//! These utilities are only intended for tests and examples; they are not part of
//! the production code path.

pub mod certs;
pub mod host;
pub mod server;

pub use certs::{CertPaths, TestCerts};
pub use host::MockHost;
pub use server::FakePluginRuntime;
