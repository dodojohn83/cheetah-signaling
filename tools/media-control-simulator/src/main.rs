//! A fake media control node for testing Cheetah Signaling media commands.
//!
//! Implements the `MediaControl`, `MediaQuery`, `MediaEventStream` and
//! `MediaClusterRegistry` gRPC services from `cheetah.common.v1`.  Commands can
//! be delayed by a configurable latency and a subset can be failed according to
//! a configurable rate.  Events are emitted back through `MediaEventStream`.

use cheetah_signal_contracts::cheetah::common::v1::command_envelope::Command as EnvelopeCommand;
use cheetah_signal_contracts::cheetah::common::v1::event_envelope::Event as EnvelopeEvent;
use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_server::{
    MediaClusterRegistry, MediaClusterRegistryServer,
};
use cheetah_signal_contracts::cheetah::common::v1::media_control_server::{
    MediaControl, MediaControlServer,
};
use cheetah_signal_contracts::cheetah::common::v1::media_event_stream_server::{
    MediaEventStream, MediaEventStreamServer,
};
use cheetah_signal_contracts::cheetah::common::v1::media_query_server::{
    MediaQuery, MediaQueryServer,
};
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandResult, CommandStatus, DeregisterMediaNodeRequest, DeregisterMediaNodeResponse,
    DrainMediaNodeRequest, DrainMediaNodeResponse, ErrorStatus, EventEnvelope,
    HeartbeatMediaNodeRequest, HeartbeatMediaNodeResponse, ListSessionsRequest,
    ListSessionsResponse, MediaControlExecuteRequest, MediaControlExecuteResponse, QueryRequest,
    QueryResponse, RegisterMediaNodeRequest, RegisterMediaNodeResponse, StreamEventsRequest,
    StreamEventsResponse,
};
use cheetah_signal_contracts::cheetah::media::v1 as media;
use clap::Parser;
use futures::StreamExt;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, broadcast};
use tokio::time;
use tokio_stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Request, Response, Status};
use tracing::{info, warn};

/// Runtime configuration.
#[derive(Clone, Debug, Parser)]
#[command(name = "media-control-simulator")]
struct Config {
    /// Address to listen on for gRPC connections.
    #[arg(long, default_value = "127.0.0.1:50051")]
    bind: SocketAddr,

    /// Stable node identifier.
    #[arg(long, default_value = "media-sim-0")]
    node_id: String,

    /// Simulated command latency in milliseconds.
    #[arg(long, default_value = "0")]
    latency_ms: u64,

    /// Fraction of commands that should fail (0.0..1.0).
    #[arg(long, default_value = "0.0")]
    failure_rate: f64,

    /// Maximum sessions advertised as capacity.
    #[arg(long, default_value = "100")]
    max_sessions: u64,

    /// Maximum bandwidth advertised as capacity (Mbps).
    #[arg(long, default_value = "1000")]
    max_bandwidth_mbps: u64,

    /// Maximum CPU percent advertised as capacity.
    #[arg(long, default_value = "80")]
    max_cpu_percent: u64,

    /// Region label.
    #[arg(long, default_value = "default")]
    region: String,

    /// Comma-separated list of supported operations.
    #[arg(long, default_value = "rtp,proxy,record,snapshot")]
    operations: String,

    /// Stable seed for deterministic command failure injection.
    #[arg(long, default_value = "0")]
    seed: u64,
}

impl Config {
    fn operations(&self) -> Vec<String> {
        self.operations
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn capability(&self) -> media::MediaCapability {
        media::MediaCapability {
            protocol: "gb28181".to_string(),
            operations: self.operations(),
            constraints: Default::default(),
            version: 1,
            runtime_state: "active".to_string(),
        }
    }

    fn capacity(&self) -> media::MediaNodeCapacity {
        media::MediaNodeCapacity {
            max_sessions: self.max_sessions,
            max_bandwidth_mbps: self.max_bandwidth_mbps,
            max_cpu_percent: self.max_cpu_percent,
            available_sessions: self.max_sessions,
            available_bandwidth_mbps: self.max_bandwidth_mbps,
            available_cpu_percent: self.max_cpu_percent,
        }
    }

    fn effective_failure_rate(&self) -> f64 {
        if self.failure_rate.is_finite() {
            self.failure_rate.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// A synthetic session stored by the simulator.
#[derive(Clone, Debug)]
struct SimSession {
    status: String,
    media_session_id: String,
    device_id: String,
    channel_id: String,
    media_node_instance_epoch: u64,
}

/// Shared simulator state.
struct State {
    config: Config,
    rng: Mutex<StdRng>,
    sessions: Mutex<HashMap<String, SimSession>>,
    events: broadcast::Sender<StreamEventsResponse>,
    node_info: Mutex<media::MediaNodeInfo>,
}

impl State {
    #[allow(deprecated)]
    fn new(config: Config) -> Self {
        let events = broadcast::channel(1024).0;
        let capabilities = vec![config.capability()];
        let capacity = Some(config.capacity());
        let rng = StdRng::seed_from_u64(config.seed);
        let node_info = media::MediaNodeInfo {
            node_id: config.node_id.clone(),
            listen_addr: config.bind.to_string(),
            capability: Some(config.capability()),
            capabilities,
            region: config.region.clone(),
            owner_epoch: 0,
            last_heartbeat_at: now_timestamp(),
            status: media::MediaNodeStatus::Active as i32,
            capacity,
            instance_id: uuid::Uuid::now_v7().to_string(),
            zone: config.region.clone(),
            network_zones: vec![],
            load: 0,
            session_count: 0,
        };
        Self {
            config,
            rng: Mutex::new(rng),
            sessions: Mutex::new(HashMap::new()),
            events,
            node_info: Mutex::new(node_info),
        }
    }

    async fn should_fail(&self) -> bool {
        let mut rng = self.rng.lock().await;
        rng.r#gen::<f64>() < self.config.effective_failure_rate()
    }

    async fn maybe_sleep(&self) {
        if self.config.latency_ms > 0 {
            time::sleep(Duration::from_millis(self.config.latency_ms)).await;
        }
    }

    fn emit_event(&self, envelope: EventEnvelope) {
        let _ = self.events.send(StreamEventsResponse {
            event: Some(envelope),
        });
    }

    fn make_result(
        status: CommandStatus,
        operation_id: String,
        error: Option<ErrorStatus>,
    ) -> CommandResult {
        CommandResult {
            status: status as i32,
            operation_id,
            error,
        }
    }
}

fn now_timestamp() -> Option<prost_types::Timestamp> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    Some(prost_types::Timestamp {
        seconds: duration.as_secs() as i64,
        nanos: duration.subsec_nanos() as i32,
    })
}

fn invalid_argument(message: impl Into<String>) -> Status {
    Status::invalid_argument(message.into())
}

fn injected_error() -> ErrorStatus {
    ErrorStatus {
        code: "INJECTED_FAILURE".to_string(),
        message: "failure injected by simulator".to_string(),
        retryable: false,
        violations: vec![],
    }
}

fn make_event(event: media::media_event::Event) -> EventEnvelope {
    EventEnvelope {
        meta: None,
        aggregate: None,
        aggregate_sequence: 0,
        event: Some(EnvelopeEvent::MediaEvent(media::MediaEvent {
            event: Some(event),
            ..Default::default()
        })),
    }
}

#[tonic::async_trait]
impl MediaControl for State {
    async fn execute(
        &self,
        request: Request<MediaControlExecuteRequest>,
    ) -> Result<Response<MediaControlExecuteResponse>, Status> {
        let request = request.into_inner();
        let envelope = request
            .command
            .ok_or_else(|| invalid_argument("missing command envelope"))?;
        let operation_id = envelope.operation_id.clone();

        self.maybe_sleep().await;

        if self.should_fail().await {
            let result =
                State::make_result(CommandStatus::Failed, operation_id, Some(injected_error()));
            return Ok(Response::new(MediaControlExecuteResponse {
                result: Some(result),
            }));
        }

        let command = match envelope.command {
            Some(EnvelopeCommand::MediaCommand(m)) => m,
            _ => return Err(invalid_argument("expected media command")),
        };

        let sub = command
            .command
            .ok_or_else(|| invalid_argument("empty media command"))?;
        let result = handle_media_command(self, &operation_id, sub).await;
        Ok(Response::new(MediaControlExecuteResponse {
            result: Some(result),
        }))
    }
}

async fn handle_media_command(
    state: &State,
    operation_id: &str,
    command: media::media_command::Command,
) -> CommandResult {
    use media::media_command::Command;

    match command {
        Command::NegotiateRtp(req) => {
            let local_sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Simulated\r\nt=0 0\r\nm=video 10000 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n".to_string();
            let session = media::RtpSession {
                media_session_id: req.media_session_id.clone(),
                device_id: req.device_id.clone(),
                channel_id: req.channel_id.clone(),
                remote_sdp: req.remote_sdp.clone(),
                local_sdp: local_sdp.clone(),
                status: "negotiated".to_string(),
                ..Default::default()
            };
            let sim = SimSession {
                status: "negotiated".to_string(),
                media_session_id: req.media_session_id.clone(),
                device_id: req.device_id.clone(),
                channel_id: req.channel_id.clone(),
                media_node_instance_epoch: 1,
            };
            state
                .sessions
                .lock()
                .await
                .insert(req.media_session_id.clone(), sim);
            state.emit_event(make_event(media::media_event::Event::RtpNegotiated(
                session.clone(),
            )));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StartRtp(req) => {
            let mut sessions = state.sessions.lock().await;
            let mut session = sessions
                .get(&req.media_session_id)
                .cloned()
                .unwrap_or_else(|| SimSession {
                    status: "starting".to_string(),
                    media_session_id: req.media_session_id.clone(),
                    device_id: req.device_id.clone(),
                    channel_id: req.channel_id.clone(),
                    media_node_instance_epoch: 1,
                });
            session.status = "active".to_string();
            sessions.insert(req.media_session_id.clone(), session);
            let rtp = media::RtpSession {
                media_session_id: req.media_session_id.clone(),
                device_id: req.device_id.clone(),
                channel_id: req.channel_id.clone(),
                remote_sdp: req.remote_sdp.clone(),
                local_sdp: req.local_sdp.clone(),
                status: "active".to_string(),
                ..Default::default()
            };
            state.emit_event(make_event(media::media_event::Event::StreamStarted(rtp)));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StopRtp(req) => {
            let _ = state.sessions.lock().await.remove(&req.media_session_id);
            let stopped = media::MediaSessionRefStopped {
                session: Some(req.clone()),
                reason: "injected stop".to_string(),
            };
            state.emit_event(make_event(media::media_event::Event::StreamStopped(
                stopped,
            )));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StartProxy(req) => {
            let sim = SimSession {
                status: "active".to_string(),
                media_session_id: req.media_session_id.clone(),
                device_id: req.device_id.clone(),
                channel_id: req.channel_id.clone(),
                media_node_instance_epoch: 1,
            };
            state
                .sessions
                .lock()
                .await
                .insert(req.media_session_id.clone(), sim);
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StopProxy(req) => {
            let _ = state.sessions.lock().await.remove(&req.media_session_id);
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StartRecord(req) => {
            let session = media::RecordSession {
                media_session_id: req.media_session_id.clone(),
                record_id: req.record_id.clone(),
                storage_path: req.storage_path.clone(),
                duration_ms: 0,
                status: "recording".to_string(),
                ..Default::default()
            };
            state.emit_event(make_event(media::media_event::Event::RecordStarted(
                session,
            )));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::StopRecord(req) => {
            let session = media::RecordSession {
                media_session_id: req.media_session_id.clone(),
                record_id: req.media_session_id.clone(),
                storage_path: "/tmp".to_string(),
                duration_ms: 1000,
                status: "stopped".to_string(),
                ..Default::default()
            };
            state.emit_event(make_event(media::media_event::Event::RecordStopped(
                session,
            )));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::TakeSnapshot(req) => {
            let snapshot = media::Snapshot {
                snapshot_id: req.snapshot_id.clone(),
                media_session_id: req.media_session_id.clone(),
                image_url: format!("http://127.0.0.1:8080/snapshots/{}", req.snapshot_id),
                created_at: now_timestamp(),
                ..Default::default()
            };
            state.emit_event(make_event(media::media_event::Event::SnapshotTaken(
                snapshot,
            )));
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::Query(_) => {
            // Queries are handled by the dedicated MediaQuery service.
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
        Command::Control(req) => {
            warn!(
                media_session_id = %req.media_session_id,
                command_type = %req.command_type,
                "ignored generic media control payload"
            );
            State::make_result(CommandStatus::Completed, operation_id.to_string(), None)
        }
    }
}

#[tonic::async_trait]
impl MediaQuery for State {
    async fn query(
        &self,
        request: Request<QueryRequest>,
    ) -> Result<Response<QueryResponse>, Status> {
        let inner = request.into_inner();
        let query = inner
            .query
            .as_ref()
            .ok_or_else(|| invalid_argument("missing query"))?;
        let mut result = media::QueryResult {
            media_session_id: query
                .parameters
                .get("media_session_id")
                .cloned()
                .unwrap_or_default(),
            snapshots: vec![],
            has_more: false,
        };
        if query.query_type == "snapshots" {
            result.snapshots.push(media::Snapshot {
                snapshot_id: "sim-snapshot-0".to_string(),
                media_session_id: result.media_session_id.clone(),
                image_url: "http://127.0.0.1:8080/snapshots/sim-snapshot-0".to_string(),
                created_at: now_timestamp(),
                ..Default::default()
            });
        }
        Ok(Response::new(QueryResponse {
            result: Some(result),
        }))
    }

    async fn list_sessions(
        &self,
        request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let _ = request.into_inner();
        let sessions = self.sessions.lock().await;
        let refs: Vec<media::MediaSessionRef> = sessions
            .values()
            .map(|s| media::MediaSessionRef {
                media_session_id: s.media_session_id.clone(),
                device_id: s.device_id.clone(),
                channel_id: s.channel_id.clone(),
                media_node_instance_epoch: s.media_node_instance_epoch,
            })
            .collect();
        Ok(Response::new(ListSessionsResponse {
            sessions: refs,
            next_page_token: String::new(),
        }))
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEventsResponse, Status>> + Send>>;

#[tonic::async_trait]
impl MediaEventStream for State {
    type StreamEventsStream = EventStream;

    async fn stream_events(
        &self,
        _request: Request<StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

        let rx = self.events.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(|r| {
            std::future::ready(match r {
                Ok(event) => Some(Ok(event)),
                Err(BroadcastStreamRecvError::Lagged(_)) => {
                    warn!("media event stream lagged; dropping events");
                    None
                }
            })
        });
        Ok(Response::new(Box::pin(stream)))
    }
}

#[tonic::async_trait]
impl MediaClusterRegistry for State {
    #[allow(deprecated)]
    async fn register_media_node(
        &self,
        request: Request<RegisterMediaNodeRequest>,
    ) -> Result<Response<RegisterMediaNodeResponse>, Status> {
        let inner = request.into_inner();
        let registration = inner
            .node
            .ok_or_else(|| invalid_argument("missing node registration"))?;
        let mut node_info = self.node_info.lock().await;
        node_info.node_id = registration.node_id.clone();
        node_info.listen_addr = registration.listen_addr.clone();
        node_info.capabilities = if registration.capabilities.is_empty() {
            registration.capability.into_iter().collect()
        } else {
            registration.capabilities
        };
        node_info.capability = node_info.capabilities.first().cloned();
        node_info.region = registration.region.clone();
        node_info.zone = registration.zone;
        node_info.network_zones = registration.network_zones;
        node_info.capacity = registration.capacity;
        node_info.last_heartbeat_at = now_timestamp();
        if !registration.instance_id.is_empty() {
            node_info.instance_id = registration.instance_id;
        }
        node_info.status = media::MediaNodeStatus::Active as i32;
        Ok(Response::new(RegisterMediaNodeResponse {
            node: Some(node_info.clone()),
        }))
    }

    async fn heartbeat_media_node(
        &self,
        request: Request<HeartbeatMediaNodeRequest>,
    ) -> Result<Response<HeartbeatMediaNodeResponse>, Status> {
        let inner = request.into_inner();
        let mut node_info = self.node_info.lock().await;
        if let Some(hb) = inner.heartbeat
            && !hb.node_id.is_empty()
        {
            node_info.node_id = hb.node_id;
        }
        node_info.last_heartbeat_at = now_timestamp();
        if node_info.status != media::MediaNodeStatus::Draining as i32 {
            node_info.status = media::MediaNodeStatus::Active as i32;
        }
        Ok(Response::new(HeartbeatMediaNodeResponse {
            node: Some(node_info.clone()),
        }))
    }

    async fn drain_media_node(
        &self,
        request: Request<DrainMediaNodeRequest>,
    ) -> Result<Response<DrainMediaNodeResponse>, Status> {
        let _ = request.into_inner();
        let mut node_info = self.node_info.lock().await;
        node_info.status = media::MediaNodeStatus::Draining as i32;
        Ok(Response::new(DrainMediaNodeResponse {
            node: Some(node_info.clone()),
        }))
    }

    async fn deregister_media_node(
        &self,
        request: Request<DeregisterMediaNodeRequest>,
    ) -> Result<Response<DeregisterMediaNodeResponse>, Status> {
        let _ = request.into_inner();
        let mut node_info = self.node_info.lock().await;
        node_info.status = media::MediaNodeStatus::Left as i32;
        Ok(Response::new(DeregisterMediaNodeResponse {
            node: Some(node_info.clone()),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let state = Arc::new(State::new(config.clone()));
    let addr = config.bind;

    info!(%addr, node_id = %config.node_id, "starting media control simulator");

    tonic::transport::Server::builder()
        .add_service(MediaControlServer::from_arc(state.clone()))
        .add_service(MediaQueryServer::from_arc(state.clone()))
        .add_service(MediaEventStreamServer::from_arc(state.clone()))
        .add_service(MediaClusterRegistryServer::from_arc(state.clone()))
        .serve(addr)
        .await?;

    Ok(())
}
