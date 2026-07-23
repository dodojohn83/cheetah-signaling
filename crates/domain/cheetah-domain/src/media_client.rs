//! Port for media control clients used by the scheduler and event consumer.
//!
//! This trait is implemented by transport adapters such as the gRPC media
//! control client. It keeps the scheduler crate from depending on a concrete
//! transport implementation, while still using typed domain commands and
//! events.

use crate::ports::{MediaNodeCommand, MediaNodeCommandResult};
use crate::{DomainError, MediaNodeEvent, MediaNodeSessionRef};
use cheetah_signal_types::{MediaNodeInstanceEpoch, NodeId, Page, PageRequest, TenantId};
use futures::Stream;
use std::pin::Pin;

/// Subscription request for a media node event stream.
#[derive(Clone, Debug)]
pub struct MediaSubscriptionRequest {
    /// Media node being subscribed to.
    pub media_node_id: NodeId,
    /// Instance epoch of the media node.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Signaling node that is consuming the stream.
    pub source_node_id: NodeId,
    /// Cursor to resume from; empty for a new stream.
    pub resume_cursor: String,
    /// Maximum number of events per batch.
    pub max_batch_size: u64,
    /// Contract version to advertise to the media node.
    pub contract_version: u32,
    /// Optional tenant filter; `None` listens to all tenants on the node.
    pub tenant_id: Option<TenantId>,
}

/// Port implemented by media control clients.
#[async_trait::async_trait]
pub trait MediaClient: Send + Sync + std::fmt::Debug {
    /// Executes a command against the media node at `endpoint`.
    async fn execute(
        &self,
        endpoint: &str,
        command: &MediaNodeCommand,
    ) -> Result<MediaNodeCommandResult, DomainError>;

    /// Lists active sessions on the media node at `endpoint`.
    async fn list_sessions(
        &self,
        endpoint: &str,
        tenant_id: TenantId,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
        page: PageRequest,
    ) -> Result<Page<MediaNodeSessionRef>, DomainError>;

    /// Subscribes to the media node event stream at `endpoint`.
    async fn subscribe(
        &self,
        endpoint: &str,
        request: MediaSubscriptionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<MediaNodeEvent, DomainError>> + Send + 'static>>,
        DomainError,
    >;
}
