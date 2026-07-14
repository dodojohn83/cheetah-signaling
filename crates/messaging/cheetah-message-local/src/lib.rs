//! In-process [`tokio`] message bus implementation for single-node deployments.
//!
//! The bus preserves the same proto envelope boundary as the NATS
//! implementation: commands and events are encoded to
//! [`CommandEnvelope`]/[`EventEnvelope`] before being handed to the transport
//! and decoded again on the consumer side.
#![doc = include_str!("../README.md")]

pub mod bus;

pub use bus::InProcessMessageBus;
