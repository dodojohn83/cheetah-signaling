//! Media control client integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_media_client::{MediaClientConfig, MediaControlClient, MediaControlRequest};
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandResult, CommandStatus, MediaControlExecuteRequest, MediaControlExecuteResponse,
    media_control_server::{MediaControl, MediaControlServer},
};
use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCommand, MediaControlPayload, media_command,
};
use cheetah_signal_types::{
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OperationId, OwnerEpoch,
    TenantId,
};
use std::str::FromStr;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};

#[derive(Default)]
struct MockMediaControl;

#[async_trait::async_trait]
impl MediaControl for MockMediaControl {
    async fn execute(
        &self,
        _request: Request<MediaControlExecuteRequest>,
    ) -> Result<Response<MediaControlExecuteResponse>, Status> {
        Ok(Response::new(MediaControlExecuteResponse {
            result: Some(CommandResult {
                status: CommandStatus::Completed as i32,
                operation_id: "op-1".to_string(),
                error: None,
            }),
        }))
    }
}

fn request() -> MediaControlRequest {
    let session_id = MediaSessionId::from_str("44444444-4444-4444-4444-444444444444").unwrap();
    MediaControlRequest {
        request_id: "req-1".to_string(),
        tenant_id: TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap(),
        media_session_id: session_id,
        media_binding_id: MediaBindingId::from_str("33333333-3333-3333-3333-333333333333").unwrap(),
        operation_id: OperationId::from_str("55555555-5555-5555-5555-555555555555").unwrap(),
        owner_epoch: OwnerEpoch(1),
        source_node_id: NodeId::from_str("66666666-6666-6666-6666-666666666666").unwrap(),
        media_node_id: NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap(),
        target_media_node_instance_epoch: MediaNodeInstanceEpoch(7),
        deadline: None,
        idempotency_key: "idem-1".to_string(),
        contract_version: 1,
        command: MediaCommand {
            command: Some(media_command::Command::Control(MediaControlPayload {
                media_session_id: session_id.to_string(),
                command_type: "noop".to_string(),
                payload: vec![],
            })),
            target_media_node_instance_epoch: 0,
            context: None,
        },
    }
}

#[tokio::test]
async fn execute_reaches_server_and_returns_result() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let service = MediaControlServer::new(MockMediaControl);
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .unwrap();
    });

    let client = MediaControlClient::new(MediaClientConfig::test());
    let response = client
        .execute(&format!("http://{addr}"), request())
        .await
        .unwrap();

    let result = response.result.expect("missing command result");
    assert_eq!(result.status, CommandStatus::Completed as i32);
    assert_eq!(result.operation_id, "op-1");
}
