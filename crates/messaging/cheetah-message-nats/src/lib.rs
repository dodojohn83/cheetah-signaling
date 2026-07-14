//! NATS JetStream message bus implementation for Cheetah Signaling.
//!
//! The bus maps domain [`Command`]s and [`Event`]s to proto envelopes and
//! routes them through JetStream streams and consumers.
#![doc = include_str!("../README.md")]

pub mod bus;

pub use bus::NatsBus;
