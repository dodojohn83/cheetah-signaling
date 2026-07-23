//! `MediaClient` port implementation for the tonic gRPC media control client.

use crate::client::MediaControlClient;
use crate::error::MediaClientError;
use crate::mapper::{
    build_list_sessions_request, build_media_control_request, build_subscribe_request,
    map_command_result, map_media_event, map_proto_session_ref,
};
use cheetah_domain::{
    DomainError, MediaClient, MediaNodeCommand, MediaNodeCommandResult, MediaNodeEvent,
    MediaNodeSessionRef, MediaSubscriptionRequest,
};
use cheetah_signal_types::{MediaNodeInstanceEpoch, NodeId, Page, TenantId};
use futures::{Stream, StreamExt};
use std::pin::Pin;

#[async_trait::async_trait]
impl MediaClient for MediaControlClient {
    async fn execute(
        &self,
        endpoint: &str,
        command: &MediaNodeCommand,
    ) -> Result<MediaNodeCommandResult, DomainError> {
        let request = build_media_control_request(command)?;
        let response = MediaControlClient::execute(self, endpoint, request)
            .await
            .map_err(DomainError::from)?;
        map_command_result(response)
    }

    async fn list_sessions(
        &self,
        endpoint: &str,
        tenant_id: TenantId,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
        page: cheetah_signal_types::PageRequest,
    ) -> Result<Page<MediaNodeSessionRef>, DomainError> {
        let request =
            build_list_sessions_request(tenant_id, media_node_id, media_node_instance_epoch, &page);
        let response = MediaControlClient::list_sessions(self, endpoint, request)
            .await
            .map_err(DomainError::from)?;

        let mut items = Vec::with_capacity(response.sessions.len());
        for proto in &response.sessions {
            items.push(map_proto_session_ref(proto)?);
        }

        let next_cursor = if response.next_page_token.is_empty() {
            None
        } else {
            Some(response.next_page_token.clone())
        };

        Ok(Page {
            items,
            next_cursor,
            total: None,
        })
    }

    async fn subscribe(
        &self,
        endpoint: &str,
        request: MediaSubscriptionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<MediaNodeEvent, DomainError>> + Send + 'static>>,
        DomainError,
    > {
        crate::client::validate_media_target(
            endpoint,
            request.media_node_id,
            request.media_node_instance_epoch,
        )
        .map_err(DomainError::from)?;

        let proto_request = build_subscribe_request(&request);
        let stream = MediaControlClient::subscribe(self, endpoint, proto_request)
            .await
            .map_err(DomainError::from)?;

        let mapped = stream.map(|result| {
            result
                .map(|event| map_media_event(&event))
                .map_err(|status| DomainError::from(MediaClientError::Grpc(status)))
        });

        Ok(Box::pin(mapped))
    }
}
