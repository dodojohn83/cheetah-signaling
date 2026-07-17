//! Fake [`PluginRuntime`] gRPC server for contract testing.
//!
//! The server binds to an address chosen by the caller and uses the caller's
//! TLS configuration. It implements the full `PluginRuntime` service with a
//! small set of built-in methods useful for host-side integration tests.

use cheetah_plugin_sdk::ProtocolEvent;
use cheetah_signal_contracts::cheetah::plugin::v1::{
    PluginRuntimeCallDriverRequest, PluginRuntimeCallDriverResponse, PluginRuntimeStreamRequest,
    PluginRuntimeStreamResponse,
    plugin_runtime_server::{PluginRuntime, PluginRuntimeServer},
};
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming, transport::Server};

/// Default out-of-process plugin gRPC server used by the example plugin and
/// by host integration tests that cannot depend on a real plugin binary.
#[derive(Default, Debug, Clone, Copy)]
pub struct FakePluginRuntime;

impl FakePluginRuntime {
    /// Start a gRPC server on `addr` using `tls_config` and return a future
    /// that resolves when the server shuts down.
    ///
    /// The returned future should be spawned with `tokio::spawn`.
    pub async fn serve(
        addr: std::net::SocketAddr,
        tls_config: tonic::transport::ServerTlsConfig,
    ) -> Result<(), tonic::transport::Error> {
        let mut server = Server::builder().tls_config(tls_config)?;
        server
            .add_service(PluginRuntimeServer::new(FakePluginRuntime))
            .serve(addr)
            .await
    }
}

#[tonic::async_trait]
impl PluginRuntime for FakePluginRuntime {
    async fn call_driver(
        &self,
        request: Request<PluginRuntimeCallDriverRequest>,
    ) -> Result<Response<PluginRuntimeCallDriverResponse>, Status> {
        let req = request.into_inner();
        let payload: serde_json::Value =
            serde_json::from_slice(&req.payload).unwrap_or(serde_json::Value::Null);

        let (ok, output, error_code, error_message) = match req.method.as_str() {
            "health" => (
                true,
                json!({
                    "status": "healthy",
                    "message": "fake plugin is healthy",
                    "metrics": {},
                }),
                String::new(),
                String::new(),
            ),
            "probe" => {
                let target = payload["target"].as_str().unwrap_or("");
                if target.is_empty() {
                    (
                        false,
                        serde_json::Value::Null,
                        "driver".to_string(),
                        "empty target".to_string(),
                    )
                } else {
                    (
                        true,
                        json!({
                            "protocol": "fake",
                            "direction": "outbound",
                            "metadata": {},
                        }),
                        String::new(),
                        String::new(),
                    )
                }
            }
            "start" | "drain" | "shutdown" => {
                (true, serde_json::Value::Null, String::new(), String::new())
            }
            "handle_command" => (
                true,
                handle_command_response(&payload),
                String::new(),
                String::new(),
            ),
            _ => (
                false,
                serde_json::Value::Null,
                "unsupported".to_string(),
                format!("method {} not supported", req.method),
            ),
        };

        let payload = serde_json::to_vec(&output).map_err(|e| {
            Status::internal(format!("failed to encode fake response payload: {e}"))
        })?;

        Ok(Response::new(PluginRuntimeCallDriverResponse {
            correlation_id: req.correlation_id,
            ok,
            error_code,
            error_message,
            payload,
        }))
    }

    type StreamStream = ReceiverStream<Result<PluginRuntimeStreamResponse, Status>>;

    async fn stream(
        &self,
        _request: Request<Streaming<PluginRuntimeStreamRequest>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Build the `handle_command` response payload.
///
/// If the incoming command has `command_type == "register"`, the fake plugin
/// simulates a successful registration by returning a `device_registered` event
/// in the response. Otherwise it returns an empty event list.
fn handle_command_response(payload: &serde_json::Value) -> serde_json::Value {
    let command = &payload["command"];
    let event_type = command["command_type"].as_str().unwrap_or("");
    if event_type == "register" {
        let device_id = command["payload"]["device_id"]
            .as_str()
            .unwrap_or("unknown");
        let event = ProtocolEvent {
            event_type: "device_registered".to_string(),
            payload: json!({"device_id": device_id}),
            tenant_id: None,
        };
        json!({"events": vec![event]})
    } else {
        json!({"events": Vec::<ProtocolEvent>::new()})
    }
}
