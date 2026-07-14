//! gRPC `MediaClusterRegistry` service implementation.

use crate::config::MediaRegistryConfig;
use crate::error::SchedulerError;
use crate::model::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
use crate::registry::MediaNodeRegistry;
use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_server::MediaClusterRegistry;
use cheetah_signal_contracts::cheetah::common::v1::{
    DeregisterMediaNodeRequest, DeregisterMediaNodeResponse, DrainMediaNodeRequest,
    DrainMediaNodeResponse, HeartbeatMediaNodeRequest, HeartbeatMediaNodeResponse,
    RegisterMediaNodeRequest, RegisterMediaNodeResponse,
};
use cheetah_signal_contracts::cheetah::media::v1 as media_proto;
use cheetah_signal_types::{Clock, IdGenerator, NodeId};
use std::str::FromStr;
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Peer identity extracted from the TLS layer and inserted into request extensions.
#[derive(Clone, Debug)]
pub struct PeerIdentity(pub String);

/// gRPC service for media node lifecycle.
pub struct MediaClusterRegistryService {
    registry: Arc<dyn MediaNodeRegistry>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    config: MediaRegistryConfig,
}

impl std::fmt::Debug for MediaClusterRegistryService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaClusterRegistryService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl MediaClusterRegistryService {
    /// Creates a new media cluster registry service.
    pub fn new(
        registry: Arc<dyn MediaNodeRegistry>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        config: MediaRegistryConfig,
    ) -> Self {
        Self {
            registry,
            clock,
            id_generator,
            config,
        }
    }
}

#[async_trait::async_trait]
impl MediaClusterRegistry for MediaClusterRegistryService {
    async fn register_media_node(
        &self,
        request: Request<RegisterMediaNodeRequest>,
    ) -> Result<Response<RegisterMediaNodeResponse>, Status> {
        let identity = request.extensions().get::<PeerIdentity>().cloned();
        let registration = request
            .into_inner()
            .node
            .ok_or_else(|| Status::invalid_argument("missing node registration"))?;
        check_identity(&identity, &self.config, &registration.node_id)?;

        let node_id = parse_node_id(&registration.node_id)?;
        let instance_id = self.id_generator.generate_node_id().to_string();
        let node = MediaNode {
            node_id,
            instance_id,
            instance_epoch: 0,
            zone: registration.region.clone(),
            region: registration.region,
            labels: std::collections::BTreeMap::new(),
            control_endpoint: registration.listen_addr,
            media_addresses: Vec::new(),
            capabilities: registration
                .capability
                .map(from_media_capability)
                .into_iter()
                .collect(),
            capacity: registration
                .capacity
                .map(from_media_capacity)
                .unwrap_or_default(),
            load: 0,
            session_count: 0,
            health: MediaNodeHealth::Healthy,
            draining: false,
            status: NodeStatus::Active,
            last_heartbeat_at: None,
            lease_until: None,
            generation: 0,
            contract_version: 1,
        };

        let node = self
            .registry
            .register(node, self.config.default_lease_ttl_ms, self.clock.as_ref())
            .await
            .map_err(map_scheduler_error)?;

        Ok(Response::new(RegisterMediaNodeResponse {
            node: Some(to_media_node_info(node)?),
        }))
    }

    async fn heartbeat_media_node(
        &self,
        request: Request<HeartbeatMediaNodeRequest>,
    ) -> Result<Response<HeartbeatMediaNodeResponse>, Status> {
        let identity = request.extensions().get::<PeerIdentity>().cloned();
        let heartbeat = request
            .into_inner()
            .heartbeat
            .ok_or_else(|| Status::invalid_argument("missing heartbeat"))?;
        check_identity(&identity, &self.config, &heartbeat.node_id)?;

        let node_id = parse_node_id(&heartbeat.node_id)?;
        let node = self
            .registry
            .heartbeat(
                node_id,
                heartbeat.load,
                heartbeat.session_count,
                self.clock.as_ref(),
            )
            .await
            .map_err(map_scheduler_error)?;

        Ok(Response::new(HeartbeatMediaNodeResponse {
            node: Some(to_media_node_info(node)?),
        }))
    }

    async fn drain_media_node(
        &self,
        request: Request<DrainMediaNodeRequest>,
    ) -> Result<Response<DrainMediaNodeResponse>, Status> {
        let identity = request.extensions().get::<PeerIdentity>().cloned();
        let drain = request
            .into_inner()
            .drain
            .ok_or_else(|| Status::invalid_argument("missing drain request"))?;
        check_identity(&identity, &self.config, &drain.node_id)?;

        let node_id = parse_node_id(&drain.node_id)?;
        let node = self
            .registry
            .drain(node_id, drain.drain, self.clock.as_ref())
            .await
            .map_err(map_scheduler_error)?;

        Ok(Response::new(DrainMediaNodeResponse {
            node: Some(to_media_node_info(node)?),
        }))
    }

    async fn deregister_media_node(
        &self,
        request: Request<DeregisterMediaNodeRequest>,
    ) -> Result<Response<DeregisterMediaNodeResponse>, Status> {
        let identity = request.extensions().get::<PeerIdentity>().cloned();
        let deregister = request
            .into_inner()
            .deregister
            .ok_or_else(|| Status::invalid_argument("missing deregister request"))?;
        check_identity(&identity, &self.config, &deregister.node_id)?;

        let node_id = parse_node_id(&deregister.node_id)?;
        let node = self
            .registry
            .deregister(node_id, self.clock.as_ref())
            .await
            .map_err(map_scheduler_error)?;

        Ok(Response::new(DeregisterMediaNodeResponse {
            node: Some(to_media_node_info(node)?),
        }))
    }
}

fn parse_node_id(s: &str) -> Result<NodeId, Status> {
    NodeId::from_str(s).map_err(|e| Status::invalid_argument(format!("invalid node_id: {e}")))
}

fn check_identity(
    identity: &Option<PeerIdentity>,
    config: &MediaRegistryConfig,
    expected: &str,
) -> Result<(), Status> {
    if !config.require_mtls {
        return Ok(());
    }
    match identity {
        Some(PeerIdentity(found)) if found == expected => Ok(()),
        found => Err(Status::permission_denied(format!(
            "mTLS identity mismatch: expected {expected}, found {}",
            found.as_ref().map(|p| p.0.as_str()).unwrap_or("none")
        ))),
    }
}

fn from_media_capability(cap: media_proto::MediaCapability) -> MediaCapability {
    MediaCapability {
        protocol: cap.protocol,
        operations: cap.operations,
        constraints: cap.constraints.into_iter().collect(),
    }
}

fn from_media_capacity(cap: media_proto::MediaNodeCapacity) -> MediaNodeCapacity {
    MediaNodeCapacity {
        max_sessions: cap.max_sessions,
        max_bandwidth_mbps: cap.max_bandwidth_mbps,
        max_cpu_percent: cap.max_cpu_percent,
    }
}

fn to_media_node_info(node: MediaNode) -> Result<media_proto::MediaNodeInfo, Status> {
    Ok(media_proto::MediaNodeInfo {
        node_id: node.node_id.to_string(),
        listen_addr: node.control_endpoint,
        capability: node
            .capabilities
            .into_iter()
            .next()
            .map(to_media_capability),
        region: node.region,
        owner_epoch: node.instance_epoch,
        last_heartbeat_at: node.last_heartbeat_at.map(to_timestamp).transpose()?,
        status: to_proto_status(node.status) as i32,
        capacity: Some(to_media_capacity(node.capacity)),
    })
}

fn to_media_capability(cap: MediaCapability) -> media_proto::MediaCapability {
    media_proto::MediaCapability {
        protocol: cap.protocol,
        operations: cap.operations,
        constraints: cap.constraints.into_iter().collect(),
    }
}

fn to_media_capacity(cap: MediaNodeCapacity) -> media_proto::MediaNodeCapacity {
    media_proto::MediaNodeCapacity {
        max_sessions: cap.max_sessions,
        max_bandwidth_mbps: cap.max_bandwidth_mbps,
        max_cpu_percent: cap.max_cpu_percent,
    }
}

fn to_timestamp(ts: cheetah_signal_types::UtcTimestamp) -> Result<prost_types::Timestamp, Status> {
    let offset = ts.as_offset();
    let seconds = offset.unix_timestamp();
    let nanos = i32::try_from(offset.nanosecond())
        .map_err(|_| Status::internal("timestamp nanos out of range"))?;
    Ok(prost_types::Timestamp { seconds, nanos })
}

fn to_proto_status(status: NodeStatus) -> media_proto::MediaNodeStatus {
    match status {
        NodeStatus::Active => media_proto::MediaNodeStatus::Active,
        NodeStatus::Draining => media_proto::MediaNodeStatus::Draining,
        NodeStatus::Left => media_proto::MediaNodeStatus::Left,
    }
}

fn map_scheduler_error(e: SchedulerError) -> Status {
    match e {
        SchedulerError::NodeNotFound(_) => Status::not_found(e.to_string()),
        SchedulerError::CapacityExhausted(_) => Status::resource_exhausted(e.to_string()),
        SchedulerError::IdentityMismatch { .. } => Status::permission_denied(e.to_string()),
        SchedulerError::InvalidArgument(_) => Status::invalid_argument(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}
