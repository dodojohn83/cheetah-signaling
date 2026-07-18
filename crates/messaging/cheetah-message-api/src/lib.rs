//! Sans-I/O messaging ports and proto envelope mapping for Cheetah Signaling.
//!
//! The crate defines low-level bus traits ([`RawCommandBus`], [`RawEventBus`]),
//! [`Subscription`] and [`AckHandle`], plus helpers for mapping domain
//! [`cheetah_domain::Command`]s and [`cheetah_domain::DomainEvent`]s to the
//! proto [`CommandEnvelope`] and [`EventEnvelope`] defined in
//! [`cheetah_signal_contracts`].
#![doc = include_str!("../README.md")]

pub mod bus;
pub mod mapper;
pub mod publisher;
pub mod subject;

pub use bus::{AckHandle, BusError, Delivery, RawCommandBus, RawEventBus, Subscription};
pub use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, EnvelopeMeta, EventEnvelope, GenericEvent, ResourceRef, Uuid,
};
pub use mapper::{decode_command, decode_event, encode_command, encode_event};
pub use publisher::{RawEventBusPublisher, publish_domain_event};
pub use subject::{command_subject, event_subject, tenant_bucket};
