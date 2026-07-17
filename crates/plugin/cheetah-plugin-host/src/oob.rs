//! Out-of-process plugin driver bridge.
//!
//! `OutOfProcessFactory` spawns a plugin binary that exposes the
//! `cheetah.plugin.v1.PluginRuntime` gRPC service and returns an
//! `OutOfProcessDriver` that forwards [`ProtocolDriver`] calls over the wire.
//!
//! The wire protocol is JSON-over-gRPC: each `CallDriver` RPC carries a
//! `method` name and a JSON payload. The plugin is responsible for decoding the
//! payload and encoding the response. `handle_command` responses may include a
//! list of `ProtocolEvent`s that the host emits on the plugin's behalf.

use async_trait::async_trait;
use cheetah_plugin_sdk::{
    CapabilityDescriptor, DriverCommand, DriverContext, HealthReport, PluginError, PluginName,
    ProtocolCapability, ProtocolDriver, ProtocolDriverFactory, ProtocolEvent,
};
use cheetah_signal_contracts::cheetah::plugin::v1::{
    PluginRuntimeCallDriverRequest, PluginRuntimeCallDriverResponse,
    plugin_runtime_client::PluginRuntimeClient,
};
use cheetah_signal_types::DurationMs;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tokio::time::sleep;
use tonic::transport::Channel;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Configuration for spawning and connecting to an out-of-process plugin.
#[derive(Clone, Debug)]
pub struct OutOfProcessConfig {
    /// Path to the plugin executable.
    pub command: PathBuf,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
    /// Extra environment variables passed to the plugin.
    pub env: HashMap<String, String>,
    /// Address the plugin is expected to listen on, e.g. `127.0.0.1:0` for an
    /// ephemeral port. The host waits for a TCP connection here before issuing
    /// any RPC.
    pub listen_address: String,
    /// Maximum time to wait for the plugin to become reachable.
    pub startup_timeout: DurationMs,
    /// Maximum time between TCP readiness probes.
    pub startup_poll_interval: DurationMs,
    /// Maximum gRPC request/response payload size in bytes.
    pub max_message_size: usize,
}

impl Default for OutOfProcessConfig {
    fn default() -> Self {
        Self {
            command: PathBuf::new(),
            args: Vec::new(),
            env: HashMap::new(),
            listen_address: "127.0.0.1:0".to_string(),
            startup_timeout: DurationMs::from_seconds(30),
            startup_poll_interval: DurationMs::from_millis(250),
            max_message_size: 4 * 1024 * 1024,
        }
    }
}

impl OutOfProcessConfig {
    /// Creates a config from its required fields.
    pub fn new(command: impl AsRef<Path>, listen_address: impl Into<String>) -> Self {
        Self {
            command: command.as_ref().to_path_buf(),
            listen_address: listen_address.into(),
            ..Self::default()
        }
    }
}

/// Factory that creates [`OutOfProcessDriver`] instances by spawning a plugin
/// process and connecting to its gRPC endpoint.
#[derive(Debug)]
pub struct OutOfProcessFactory {
    name: PluginName,
    capabilities: Vec<ProtocolCapability>,
    config: OutOfProcessConfig,
}

impl OutOfProcessFactory {
    /// Creates a factory for the given plugin name, capabilities and spawn config.
    pub fn new(
        name: PluginName,
        capabilities: Vec<ProtocolCapability>,
        config: OutOfProcessConfig,
    ) -> Self {
        Self {
            name,
            capabilities,
            config,
        }
    }
}

#[async_trait]
impl ProtocolDriverFactory for OutOfProcessFactory {
    fn name(&self) -> PluginName {
        self.name.clone()
    }

    fn capabilities(&self) -> Vec<ProtocolCapability> {
        self.capabilities.clone()
    }

    async fn create(
        &self,
        config: serde_json::Value,
    ) -> Result<Box<dyn ProtocolDriver>, PluginError> {
        let driver = OutOfProcessDriver::spawn(self.config.clone(), config).await?;
        Ok(Box::new(driver))
    }
}

struct ProcessState {
    child: Child,
    _shutdown_tx: oneshot::Sender<()>,
    listen_address: String,
}

impl fmt::Debug for ProcessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessState")
            .field("child_id", &self.child.id())
            .field("listen_address", &self.listen_address)
            .finish_non_exhaustive()
    }
}

/// A [`ProtocolDriver`] that delegates all calls to an out-of-process plugin
/// over gRPC.
pub struct OutOfProcessDriver {
    client: Mutex<PluginRuntimeClient<Channel>>,
    process: Mutex<ProcessState>,
}

impl fmt::Debug for OutOfProcessDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutOfProcessDriver").finish_non_exhaustive()
    }
}

impl OutOfProcessDriver {
    /// Spawns the plugin process, waits for the gRPC endpoint to become ready,
    /// and returns a connected driver.
    async fn spawn(
        runtime: OutOfProcessConfig,
        _config: serde_json::Value,
    ) -> Result<Self, PluginError> {
        let mut cmd = Command::new(&runtime.command);
        cmd.args(&runtime.args)
            .envs(&runtime.env)
            .env("CHEETAH_PLUGIN_LISTEN_ADDRESS", &runtime.listen_address)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| PluginError::Driver(format!("failed to spawn plugin: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginError::Driver("plugin stdout was not captured".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PluginError::Driver("plugin stderr was not captured".to_string()))?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let plugin_name = runtime.command.display().to_string();
        tokio::spawn(async move {
            let _ = forward_logs(plugin_name, stdout, stderr, shutdown_rx).await;
        });

        wait_for_ready(
            &runtime.listen_address,
            runtime.startup_timeout,
            runtime.startup_poll_interval,
        )
        .await?;

        let endpoint = format!("http://{}", runtime.listen_address);
        let channel = Channel::from_shared(endpoint.clone())
            .map_err(|e| PluginError::Driver(format!("invalid plugin endpoint {endpoint}: {e}")))?
            .connect()
            .await
            .map_err(|e| {
                PluginError::Driver(format!("failed to connect to plugin at {endpoint}: {e}"))
            })?;

        let client = PluginRuntimeClient::new(channel)
            .max_decoding_message_size(runtime.max_message_size)
            .max_encoding_message_size(runtime.max_message_size);

        Ok(Self {
            client: Mutex::new(client),
            process: Mutex::new(ProcessState {
                child,
                _shutdown_tx: shutdown_tx,
                listen_address: runtime.listen_address,
            }),
        })
    }

    async fn call_method(
        &self,
        method: &str,
        payload: serde_json::Value,
        timeout: DurationMs,
    ) -> Result<serde_json::Value, PluginError> {
        let correlation_id = Uuid::now_v7().to_string();
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| PluginError::Driver(format!("failed to encode {method} payload: {e}")))?;

        let request = PluginRuntimeCallDriverRequest {
            correlation_id,
            method: method.to_string(),
            payload: payload_bytes,
            timeout_ms: timeout.as_millis().max(0) as u64,
        };

        let rpc_timeout = Duration::from_millis(timeout.as_millis().max(0) as u64);
        let mut client = self.client.lock().await;
        let response = tokio::time::timeout(rpc_timeout, client.call_driver(request))
            .await
            .map_err(|_| PluginError::Cancelled)?
            .map_err(|e| PluginError::Driver(format!("{method} RPC failed: {e}")))?;

        decode_response(&response.into_inner(), method)
    }
}

#[async_trait]
impl ProtocolDriver for OutOfProcessDriver {
    async fn start(&self, ctx: &dyn DriverContext) -> Result<(), PluginError> {
        let payload = serde_json::json!({
            "plugin_name": ctx.plugin_name().to_string(),
            "config": ctx.config(),
            "budget": ctx.budget(),
        });
        let _ = self
            .call_method("start", payload, DurationMs::from_seconds(30))
            .await?;
        Ok(())
    }

    async fn drain(
        &self,
        _ctx: &dyn DriverContext,
        timeout: DurationMs,
    ) -> Result<(), PluginError> {
        let payload = serde_json::json!({
            "timeout_ms": timeout.as_millis(),
        });
        let _ = self.call_method("drain", payload, timeout).await?;
        Ok(())
    }

    async fn shutdown(&self, _ctx: &dyn DriverContext) -> Result<(), PluginError> {
        let payload = serde_json::json!({});
        let _ = self
            .call_method("shutdown", payload, DurationMs::from_seconds(30))
            .await;

        let mut process = self.process.lock().await;
        if let Err(e) = process.child.start_kill() {
            warn!(error = %e, "failed to send kill signal to plugin process");
        }
        let _ = process.child.wait().await;
        Ok(())
    }

    async fn handle_command(
        &self,
        ctx: &dyn DriverContext,
        command: DriverCommand,
    ) -> Result<(), PluginError> {
        let payload = serde_json::json!({
            "plugin_name": ctx.plugin_name().to_string(),
            "command": command,
        });
        let response = self
            .call_method("handle_command", payload, DurationMs::from_seconds(30))
            .await?;

        let events: Vec<ProtocolEvent> = response
            .get("events")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        for event in events {
            ctx.device_sink()
                .emit_event(event)
                .await
                .map_err(|e| PluginError::Driver(format!("event sink error: {e}")))?;
        }

        Ok(())
    }

    async fn probe(
        &self,
        _ctx: &dyn DriverContext,
        target: &str,
        timeout: DurationMs,
    ) -> Result<CapabilityDescriptor, PluginError> {
        let payload = serde_json::json!({
            "target": target,
            "timeout_ms": timeout.as_millis(),
        });
        let response = self.call_method("probe", payload, timeout).await?;
        serde_json::from_value(response)
            .map_err(|e| PluginError::Driver(format!("probe response malformed: {e}")))
    }

    async fn health(
        &self,
        _ctx: &dyn DriverContext,
        timeout: DurationMs,
    ) -> Result<HealthReport, PluginError> {
        let payload = serde_json::json!({});
        let response = self.call_method("health", payload, timeout).await?;
        serde_json::from_value(response)
            .map_err(|e| PluginError::Driver(format!("health response malformed: {e}")))
    }
}

async fn forward_logs(
    plugin_name: String,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            line = stdout_reader.next_line() => match line {
                Ok(Some(line)) => info!(plugin = %plugin_name, stream = "stdout", "{line}"),
                Ok(None) => break,
                Err(e) => {
                    warn!(plugin = %plugin_name, stream = "stdout", error = %e, "log read failed");
                    break;
                }
            },
            line = stderr_reader.next_line() => match line {
                Ok(Some(line)) => warn!(plugin = %plugin_name, stream = "stderr", "{line}"),
                Ok(None) => break,
                Err(e) => {
                    warn!(plugin = %plugin_name, stream = "stderr", error = %e, "log read failed");
                    break;
                }
            },
        }
    }
}

async fn wait_for_ready(
    address: &str,
    startup_timeout: DurationMs,
    poll_interval: DurationMs,
) -> Result<(), PluginError> {
    let deadline = std::time::Instant::now()
        + Duration::from_millis(startup_timeout.as_millis().max(0) as u64);
    let poll = Duration::from_millis(poll_interval.as_millis().max(0) as u64);

    while std::time::Instant::now() < deadline {
        match TcpStream::connect(address).await {
            Ok(_stream) => return Ok(()),
            Err(e) => {
                debug!(address = %address, error = %e, "plugin not ready yet");
                sleep(poll).await;
            }
        }
    }

    Err(PluginError::Driver(format!(
        "plugin did not become reachable at {address} within {} ms",
        startup_timeout.as_millis()
    )))
}

fn decode_response(
    response: &PluginRuntimeCallDriverResponse,
    method: &str,
) -> Result<serde_json::Value, PluginError> {
    if response.ok {
        serde_json::from_slice(&response.payload)
            .map_err(|e| PluginError::Driver(format!("{method} response is not valid JSON: {e}")))
    } else {
        Err(map_error_code(
            &response.error_code,
            &response.error_message,
        ))
    }
}

fn map_error_code(code: &str, message: &str) -> PluginError {
    match code {
        "invalid_manifest" => PluginError::InvalidManifest(message.to_string()),
        "incompatible_sdk" => PluginError::IncompatibleSdk {
            plugin: message.to_string(),
            host: String::new(),
        },
        "invalid_checksum" => PluginError::InvalidChecksum,
        "unsupported_protocol" => PluginError::UnsupportedProtocol(message.to_string()),
        "resource_budget_exceeded" => PluginError::ResourceBudgetExceeded(message.to_string()),
        "unsupported" => PluginError::Unsupported(message.to_string()),
        "cancelled" => PluginError::Cancelled,
        "transient" => PluginError::Transient(message.to_string()),
        _ => PluginError::Driver(format!("{code}: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostDriverContext;
    use cheetah_plugin_sdk::{
        CommandSource, DeviceSink, HealthStatus, ProtocolDirection, ResourceBudget,
    };
    use cheetah_signal_contracts::cheetah::plugin::v1::{
        PluginRuntimeCallDriverRequest, PluginRuntimeCallDriverResponse,
        PluginRuntimeStreamRequest, PluginRuntimeStreamResponse,
        plugin_runtime_server::{PluginRuntime, PluginRuntimeServer},
    };
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tonic::{Request, Response, Status, Streaming};

    struct FakePlugin;

    #[tonic::async_trait]
    impl PluginRuntime for FakePlugin {
        async fn call_driver(
            &self,
            request: Request<PluginRuntimeCallDriverRequest>,
        ) -> Result<Response<PluginRuntimeCallDriverResponse>, Status> {
            let req = request.into_inner();
            let payload: serde_json::Value =
                serde_json::from_slice(&req.payload).unwrap_or_default();

            let (ok, payload_out, error_code, error_message) = match req.method.as_str() {
                "health" => (
                    true,
                    serde_json::json!({
                        "status": "healthy",
                        "message": "out-of-process plugin is healthy",
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
                            serde_json::json!({
                                "protocol": "fake",
                                "direction": "outbound",
                                "metadata": {},
                            }),
                            String::new(),
                            String::new(),
                        )
                    }
                }
                "start" | "drain" | "shutdown" | "handle_command" => {
                    (true, serde_json::Value::Null, String::new(), String::new())
                }
                _ => (
                    false,
                    serde_json::Value::Null,
                    "unsupported".to_string(),
                    format!("method {} not supported", req.method),
                ),
            };

            Ok(Response::new(PluginRuntimeCallDriverResponse {
                correlation_id: req.correlation_id,
                ok,
                error_code,
                error_message,
                payload: serde_json::to_vec(&payload_out).unwrap_or_default(),
            }))
        }

        type StreamStream =
            tokio_stream::wrappers::ReceiverStream<Result<PluginRuntimeStreamResponse, Status>>;

        async fn stream(
            &self,
            _request: Request<Streaming<PluginRuntimeStreamRequest>>,
        ) -> Result<Response<Self::StreamStream>, Status> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        }
    }

    struct NoopSink;

    #[async_trait]
    impl DeviceSink for NoopSink {
        async fn emit_event(&self, _event: ProtocolEvent) -> Result<(), PluginError> {
            Ok(())
        }
    }

    struct NoopSource;

    #[async_trait]
    impl CommandSource for NoopSource {
        async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError> {
            Ok(None)
        }
    }

    fn plugin_name() -> Result<PluginName, PluginError> {
        PluginName::new("cheetah/fake")
    }

    async fn fake_context() -> Result<HostDriverContext, PluginError> {
        Ok(HostDriverContext::new(
            plugin_name()?,
            serde_json::json!({}),
            ResourceBudget::default(),
            Arc::new(NoopSink),
            Arc::new(NoopSource),
        ))
    }

    async fn connect_to_fake() -> Result<OutOfProcessDriver, PluginError> {
        let addr: std::net::SocketAddr = "127.0.0.1:0"
            .parse()
            .map_err(|e| PluginError::Driver(format!("invalid socket address: {e}")))?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| PluginError::Driver(format!("failed to bind listener: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| PluginError::Driver(format!("failed to read local address: {e}")))?
            .port();

        tokio::spawn(async move {
            let svc = PluginRuntimeServer::new(FakePlugin);
            let _ = tonic::transport::Server::builder()
                .add_service(svc)
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await;
        });

        sleep(Duration::from_millis(50)).await;

        let endpoint = format!("http://127.0.0.1:{port}");
        let channel = Channel::from_shared(endpoint.clone())
            .map_err(|e| PluginError::Driver(format!("invalid endpoint {endpoint}: {e}")))?
            .connect()
            .await
            .map_err(|e| PluginError::Driver(format!("failed to connect to {endpoint}: {e}")))?;
        let client = PluginRuntimeClient::new(channel)
            .max_decoding_message_size(4 * 1024 * 1024)
            .max_encoding_message_size(4 * 1024 * 1024);

        let child = Command::new("cat")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PluginError::Driver(format!("failed to spawn placeholder child: {e}")))?;

        let (shutdown_tx, _shutdown_rx) = oneshot::channel();

        Ok(OutOfProcessDriver {
            client: Mutex::new(client),
            process: Mutex::new(ProcessState {
                child,
                _shutdown_tx: shutdown_tx,
                listen_address: format!("127.0.0.1:{port}"),
            }),
        })
    }

    #[tokio::test]
    async fn health_and_probe_round_trip() -> Result<(), PluginError> {
        let driver = connect_to_fake().await?;
        let ctx = fake_context().await?;

        let report = driver.health(&ctx, DurationMs::from_seconds(5)).await?;
        assert_eq!(report.status, HealthStatus::Healthy);

        let cap = driver
            .probe(&ctx, "127.0.0.1:9999", DurationMs::from_seconds(5))
            .await?;
        assert_eq!(cap.protocol, "fake");
        assert_eq!(cap.direction, ProtocolDirection::Outbound);

        driver.shutdown(&ctx).await
    }
}
