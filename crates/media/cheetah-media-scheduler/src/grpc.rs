//! gRPC `MediaClusterRegistry` service implementation.

use crate::config::MediaRegistryConfig;
use crate::error::SchedulerError;
use crate::metrics::MediaMetrics;
use crate::model::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
use crate::registry::MediaNodeRegistry;
use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_server::MediaClusterRegistry;
use cheetah_signal_contracts::cheetah::common::v1::{
    DeregisterMediaNodeRequest, DeregisterMediaNodeResponse, DrainMediaNodeRequest,
    DrainMediaNodeResponse, HeartbeatMediaNodeRequest, HeartbeatMediaNodeResponse,
    RegisterMediaNodeRequest, RegisterMediaNodeResponse,
};
use cheetah_signal_contracts::cheetah::media::v1 as media_proto;
use cheetah_signal_types::{
    AuditEvent, AuditLog, AuditOutcome, Clock, IdGenerator, NodeId, SafeDetails, is_internal_ip,
};
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
    metrics: Arc<MediaMetrics>,
    audit: Arc<dyn AuditLog>,
    node_id: NodeId,
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
        metrics: Arc<MediaMetrics>,
        audit: Arc<dyn AuditLog>,
        node_id: NodeId,
    ) -> Self {
        Self {
            registry,
            clock,
            id_generator,
            config,
            metrics,
            audit,
            node_id,
        }
    }

    fn record_audit(
        &self,
        action: &str,
        actor: &str,
        node_id: NodeId,
        outcome: AuditOutcome,
        details: Option<String>,
    ) {
        self.audit.record(AuditEvent {
            timestamp: self.clock.now_wall(),
            action: action.to_string(),
            actor: actor.to_string(),
            tenant_id: None,
            target_type: "media_node".to_string(),
            target_id: Some(node_id.to_string()),
            outcome,
            request_id: self.id_generator.generate_message_id().to_string(),
            correlation_id: None,
            source_ip: None,
            node_id: self.node_id,
            details: details.map(SafeDetails::new),
        });
    }
}

#[async_trait::async_trait]
impl MediaClusterRegistry for MediaClusterRegistryService {
    #[allow(deprecated)]
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
        validate_control_endpoint(&registration.listen_addr, &self.config).await?;
        validate_registration_fields(&registration, &self.config)?;

        let node_id = parse_node_id(&registration.node_id)?;
        // An empty instance_id means the node did not transmit a stable process
        // identity. To avoid inheriting stale state from a previous registration
        // with the same node_id (e.g. after a restart), always mint a fresh
        // instance identifier for the new process.
        let instance_id = if registration.instance_id.is_empty() {
            self.id_generator.generate_node_id().to_string()
        } else {
            registration.instance_id
        };
        let node = MediaNode {
            node_id,
            instance_id,
            instance_epoch: 0,
            zone: registration.zone,
            region: registration.region,
            network_zones: registration.network_zones,
            labels: std::collections::BTreeMap::new(),
            control_endpoint: registration.listen_addr.clone(),
            media_addresses: Vec::new(),
            capabilities: if registration.capabilities.is_empty() {
                registration
                    .capability
                    .into_iter()
                    .map(from_media_capability)
                    .collect()
            } else {
                registration
                    .capabilities
                    .into_iter()
                    .map(from_media_capability)
                    .collect()
            },
            capacity: registration.capacity.map(from_media_capacity).unwrap_or(
                crate::model::MediaNodeCapacity {
                    max_sessions: 1,
                    max_bandwidth_mbps: 0,
                    max_cpu_percent: 100,
                },
            ),
            load: 0,
            session_count: 0,
            health: MediaNodeHealth::Healthy,
            draining: false,
            status: NodeStatus::Active,
            last_heartbeat_at: None,
            lease_until: None,
            generation: 0,
            contract_version: 1,
            revision: 0,
        };

        let node = self
            .registry
            .register(node, self.config.default_lease_ttl_ms, self.clock.as_ref())
            .await
            .map_err(map_scheduler_error)?;

        self.metrics.record_register();
        self.record_audit(
            "media_node.register",
            &actor_from_identity(&identity),
            node_id,
            AuditOutcome::Success,
            Some(format!(
                "listen_addr={}",
                scrub_endpoint(&registration.listen_addr)
            )),
        );

        Ok(Response::new(RegisterMediaNodeResponse {
            node: Some(to_media_node_info(node, &self.config, self.clock.as_ref())),
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
                &heartbeat.lease_id,
                heartbeat.instance_epoch,
                heartbeat.load,
                heartbeat.session_count,
                self.clock.as_ref(),
            )
            .await
            .map_err(map_scheduler_error)?;

        Ok(Response::new(HeartbeatMediaNodeResponse {
            node: Some(to_media_node_info(node, &self.config, self.clock.as_ref())),
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

        if drain.drain {
            self.metrics.record_drain();
        }
        self.record_audit(
            "media_node.drain",
            &actor_from_identity(&identity),
            node_id,
            AuditOutcome::Success,
            Some(format!("drain={}", drain.drain)),
        );

        Ok(Response::new(DrainMediaNodeResponse {
            node: Some(to_media_node_info(node, &self.config, self.clock.as_ref())),
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

        self.metrics.record_deregister();
        self.record_audit(
            "media_node.deregister",
            &actor_from_identity(&identity),
            node_id,
            AuditOutcome::Success,
            if deregister.reason.is_empty() {
                None
            } else {
                Some(format!("reason={}", deregister.reason))
            },
        );

        Ok(Response::new(DeregisterMediaNodeResponse {
            node: Some(to_media_node_info(node, &self.config, self.clock.as_ref())),
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
        version: cap.version,
        runtime_state: cap.runtime_state,
    }
}

fn from_media_capacity(cap: media_proto::MediaNodeCapacity) -> MediaNodeCapacity {
    MediaNodeCapacity {
        max_sessions: cap.max_sessions,
        max_bandwidth_mbps: cap.max_bandwidth_mbps,
        max_cpu_percent: cap.max_cpu_percent,
    }
}

#[allow(deprecated)]
fn to_media_node_info(
    node: MediaNode,
    config: &MediaRegistryConfig,
    clock: &dyn Clock,
) -> media_proto::MediaNodeInfo {
    let capabilities: Vec<_> = node
        .capabilities
        .into_iter()
        .map(to_media_capability)
        .collect();
    media_proto::MediaNodeInfo {
        node_id: node.node_id.to_string(),
        listen_addr: node.control_endpoint,
        capability: capabilities.first().cloned(),
        capabilities,
        region: node.region,
        owner_epoch: node.instance_epoch,
        last_heartbeat_at: node.last_heartbeat_at.map(|ts| ts.to_prost_timestamp()),
        status: to_proto_status(node.status) as i32,
        capacity: Some(media_proto::MediaNodeCapacity {
            max_sessions: node.capacity.max_sessions,
            max_bandwidth_mbps: node.capacity.max_bandwidth_mbps,
            max_cpu_percent: node.capacity.max_cpu_percent,
            available_sessions: node
                .capacity
                .max_sessions
                .saturating_sub(node.session_count),
            available_bandwidth_mbps: node.capacity.max_bandwidth_mbps,
            available_cpu_percent: node.capacity.max_cpu_percent.saturating_sub(node.load),
        }),
        instance_id: node.instance_id.clone(),
        zone: node.zone,
        network_zones: node.network_zones,
        load: node.load,
        session_count: node.session_count,
        lease_id: node.instance_id,
        lease_ttl_ms: config.default_lease_ttl_ms,
        heartbeat_interval_ms: config.heartbeat_interval_ms,
        cluster_time: Some(clock.now_wall().to_prost_timestamp()),
        accepted_contract_version: config.accepted_contract_version,
    }
}

fn to_media_capability(cap: MediaCapability) -> media_proto::MediaCapability {
    media_proto::MediaCapability {
        protocol: cap.protocol,
        operations: cap.operations,
        constraints: cap.constraints.into_iter().collect(),
        version: cap.version,
        runtime_state: cap.runtime_state,
    }
}

fn to_proto_status(status: NodeStatus) -> media_proto::MediaNodeStatus {
    match status {
        NodeStatus::Active => media_proto::MediaNodeStatus::Active,
        NodeStatus::Draining => media_proto::MediaNodeStatus::Draining,
        NodeStatus::Left => media_proto::MediaNodeStatus::Left,
    }
}

fn validate_registration_fields(
    registration: &media_proto::MediaNodeRegistration,
    config: &MediaRegistryConfig,
) -> Result<(), Status> {
    let max = config.max_string_field_length;
    for (name, value) in [
        ("node_id", registration.node_id.as_str()),
        ("instance_id", registration.instance_id.as_str()),
        ("region", registration.region.as_str()),
    ] {
        if value.len() > max {
            return Err(Status::invalid_argument(format!(
                "field '{name}' exceeds {max} bytes"
            )));
        }
    }

    #[allow(deprecated)]
    for cap in registration
        .capabilities
        .iter()
        .chain(registration.capability.iter())
    {
        if cap.protocol.len() > max {
            return Err(Status::invalid_argument(format!(
                "field 'capabilities[].protocol' exceeds {max} bytes"
            )));
        }
        if cap.operations.len() > config.max_capability_operations {
            return Err(Status::invalid_argument(format!(
                "capabilities[].operations exceeds {} entries",
                config.max_capability_operations
            )));
        }
        for op in &cap.operations {
            if op.len() > max {
                return Err(Status::invalid_argument(format!(
                    "field 'capabilities[].operations' exceeds {max} bytes"
                )));
            }
        }
        if cap.constraints.len() > config.max_capability_constraints {
            return Err(Status::invalid_argument(format!(
                "capabilities[].constraints exceeds {} entries",
                config.max_capability_constraints
            )));
        }
        for (k, v) in &cap.constraints {
            if k.len() > max || v.len() > max {
                return Err(Status::invalid_argument(format!(
                    "field 'capabilities[].constraints' exceeds {max} bytes"
                )));
            }
        }
    }

    if let Some(capacity) = &registration.capacity {
        if capacity.max_sessions == 0 {
            return Err(Status::invalid_argument(
                "capacity.max_sessions must be greater than 0".to_string(),
            ));
        }
        if capacity.max_cpu_percent > config.max_reported_load_percent {
            return Err(Status::invalid_argument(format!(
                "capacity.max_cpu_percent exceeds {}%",
                config.max_reported_load_percent
            )));
        }
    }

    Ok(())
}

async fn validate_control_endpoint(
    endpoint: &str,
    config: &MediaRegistryConfig,
) -> Result<(), Status> {
    if endpoint.len() > config.max_endpoint_uri_length {
        return Err(Status::invalid_argument(format!(
            "control endpoint exceeds {} bytes",
            config.max_endpoint_uri_length
        )));
    }
    let uri = endpoint
        .parse::<tonic::transport::Uri>()
        .map_err(|_| Status::invalid_argument("invalid control endpoint URI"))?;
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| Status::invalid_argument("missing control endpoint scheme"))?;
    if !config.allowed_endpoint_schemes.iter().any(|s| s == scheme) {
        return Err(Status::invalid_argument(format!(
            "control endpoint scheme '{scheme}' is not allowed"
        )));
    }
    let host = uri
        .host()
        .ok_or_else(|| Status::invalid_argument("missing control endpoint host"))?;
    if host.is_empty() {
        return Err(Status::invalid_argument("empty control endpoint host"));
    }

    if !config.allow_internal_endpoints {
        let port = uri
            .port_u16()
            .unwrap_or_else(|| if scheme == "https" { 443 } else { 80 });
        let host_has_internal = host_is_internal(host, port, config.endpoint_dns_lookup_timeout_ms)
            .await
            .map_err(|e| {
                Status::invalid_argument(format!("control endpoint validation failed: {e}"))
            })?;
        if host_has_internal {
            return Err(Status::invalid_argument(
                "internal control endpoint is not allowed",
            ));
        }
    }
    Ok(())
}

async fn host_is_internal(host: &str, port: u16, timeout_ms: u64) -> Result<bool, Status> {
    if let Ok(ip) = std::net::IpAddr::from_str(host) {
        return Ok(is_internal_ip(ip));
    }

    let lookup = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| Status::invalid_argument("control endpoint DNS lookup timed out"))?
    .map_err(|e| Status::invalid_argument(format!("control endpoint DNS lookup failed: {e}")))?;

    for addr in lookup {
        if is_internal_ip(addr.ip()) {
            return Ok(true);
        }
    }
    Ok(false)
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

fn actor_from_identity(identity: &Option<PeerIdentity>) -> String {
    identity
        .as_ref()
        .map(|i| i.0.clone())
        .unwrap_or_else(|| "media-node".to_string())
}

fn scrub_endpoint(endpoint: &str) -> String {
    endpoint
        .rsplit_once('@')
        .map(|(_, host)| host.to_string())
        .unwrap_or_else(|| endpoint.to_string())
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::model::{MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
    use crate::registry::MediaNodeRegistry;
    use cheetah_signal_types::test_support::{FakeClock, FakeIdGenerator};
    use cheetah_signal_types::{MediaBindingId, NoOpAuditLog, TenantId};
    use std::str::FromStr;
    use std::sync::Mutex;

    struct FakeRegistry {
        node: Mutex<Option<MediaNode>>,
    }

    impl FakeRegistry {
        fn lock_node(&self) -> std::sync::MutexGuard<'_, Option<MediaNode>> {
            match self.node.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            }
        }
    }

    #[async_trait::async_trait]
    impl MediaNodeRegistry for FakeRegistry {
        async fn register(
            &self,
            node: MediaNode,
            _lease_ttl_ms: u64,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            *self.lock_node() = Some(node.clone());
            Ok(node)
        }

        async fn heartbeat(
            &self,
            _node_id: NodeId,
            _lease_id: &str,
            _instance_epoch: u64,
            load: u64,
            session_count: u64,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            let mut guard = self.lock_node();
            let node = guard.as_mut().ok_or_else(|| {
                SchedulerError::NodeNotFound("test fake: node not registered".into())
            })?;
            node.load = load;
            node.session_count = session_count;
            Ok(node.clone())
        }

        async fn drain(
            &self,
            _node_id: NodeId,
            drain: bool,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            let mut guard = self.lock_node();
            let node = guard.as_mut().ok_or_else(|| {
                SchedulerError::NodeNotFound("test fake: node not registered".into())
            })?;
            node.draining = drain;
            Ok(node.clone())
        }

        async fn deregister(
            &self,
            _node_id: NodeId,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            let mut guard = self.lock_node();
            let node = guard.take().ok_or_else(|| {
                SchedulerError::NodeNotFound("test fake: node not registered".into())
            })?;
            Ok(node)
        }

        async fn get(&self, _node_id: NodeId, _clock: &dyn Clock) -> Option<MediaNode> {
            self.lock_node().clone()
        }

        async fn list_active(&self, _clock: &dyn Clock) -> Vec<MediaNode> {
            self.lock_node()
                .as_ref()
                .filter(|n| !n.draining)
                .cloned()
                .into_iter()
                .collect()
        }

        async fn reserve(
            &self,
            _node_id: NodeId,
            _tenant_id: TenantId,
            _binding_id: MediaBindingId,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            let guard = self.lock_node();
            let node = guard.as_ref().ok_or_else(|| {
                SchedulerError::NodeNotFound("test fake: node not registered".into())
            })?;
            Ok(node.clone())
        }

        async fn release(
            &self,
            _node_id: NodeId,
            _tenant_id: TenantId,
            _binding_id: MediaBindingId,
            _clock: &dyn Clock,
        ) -> Result<MediaNode, SchedulerError> {
            let guard = self.lock_node();
            let node = guard.as_ref().ok_or_else(|| {
                SchedulerError::NodeNotFound("test fake: node not registered".into())
            })?;
            Ok(node.clone())
        }
    }

    fn fake_existing_node(node_id: NodeId) -> MediaNode {
        MediaNode {
            node_id,
            instance_id: "existing-instance".to_string(),
            instance_epoch: 1,
            zone: "us-east".to_string(),
            region: "us-east".to_string(),
            network_zones: vec!["us-east".to_string()],
            labels: std::collections::BTreeMap::new(),
            control_endpoint: "https://1.1.1.1:443".to_string(),
            media_addresses: Vec::new(),
            capabilities: Vec::new(),
            capacity: MediaNodeCapacity {
                max_sessions: 10,
                max_bandwidth_mbps: 1000,
                max_cpu_percent: 100,
            },
            load: 0,
            session_count: 0,
            health: MediaNodeHealth::Healthy,
            draining: false,
            status: NodeStatus::Active,
            last_heartbeat_at: None,
            lease_until: None,
            generation: 0,
            contract_version: 1,
            revision: 0,
        }
    }

    fn test_config() -> MediaRegistryConfig {
        let mut config = MediaRegistryConfig::test();
        config.require_mtls = false;
        config
    }

    #[tokio::test]
    async fn register_generates_new_instance_id_when_empty()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let node_id = NodeId::from_str("11111111-1111-1111-1111-111111111111")?;
        let registry = Arc::new(FakeRegistry {
            node: Mutex::new(Some(fake_existing_node(node_id))),
        });
        let service = MediaClusterRegistryService::new(
            registry.clone(),
            Arc::new(FakeClock::new()),
            Arc::new(FakeIdGenerator::new()),
            test_config(),
            crate::MediaMetrics::arc(),
            Arc::new(NoOpAuditLog),
            node_id,
        );

        let request = Request::new(RegisterMediaNodeRequest {
            node: Some(media_proto::MediaNodeRegistration {
                node_id: node_id.to_string(),
                listen_addr: "https://1.1.1.1:443".to_string(),
                capability: None,
                capabilities: vec![media_proto::MediaCapability {
                    protocol: "gb28181".to_string(),
                    operations: vec!["live".to_string()],
                    constraints: std::collections::BTreeMap::new(),
                    version: 1,
                    runtime_state: "active".to_string(),
                }],
                region: "us-east".to_string(),
                zone: "us-east".to_string(),
                network_zones: vec!["us-east".to_string()],
                capacity: Some(media_proto::MediaNodeCapacity {
                    max_sessions: 10,
                    max_bandwidth_mbps: 1000,
                    max_cpu_percent: 100,
                    available_sessions: 10,
                    available_bandwidth_mbps: 1000,
                    available_cpu_percent: 100,
                }),
                instance_id: String::new(),
            }),
        });

        let response = service.register_media_node(request).await?;
        let info = response
            .into_inner()
            .node
            .ok_or_else(|| tonic::Status::internal("missing node"))?;

        assert_eq!(info.node_id, node_id.to_string());
        assert_ne!(
            info.instance_id, "existing-instance",
            "empty instance_id must not inherit a stale instance_id from a previous registration"
        );

        let registered = registry
            .lock_node()
            .clone()
            .ok_or_else(|| tonic::Status::internal("missing registered node"))?;
        assert_ne!(registered.instance_id, "existing-instance");
        Ok(())
    }

    #[tokio::test]
    async fn register_preserves_supplied_instance_id()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let node_id = NodeId::from_str("22222222-2222-2222-2222-222222222222")?;
        let registry = Arc::new(FakeRegistry {
            node: Mutex::new(Some(fake_existing_node(node_id))),
        });
        let service = MediaClusterRegistryService::new(
            registry.clone(),
            Arc::new(FakeClock::new()),
            Arc::new(FakeIdGenerator::new()),
            test_config(),
            crate::MediaMetrics::arc(),
            Arc::new(NoOpAuditLog),
            node_id,
        );

        let request = Request::new(RegisterMediaNodeRequest {
            node: Some(media_proto::MediaNodeRegistration {
                node_id: node_id.to_string(),
                listen_addr: "https://1.1.1.1:443".to_string(),
                capability: None,
                capabilities: vec![media_proto::MediaCapability {
                    protocol: "gb28181".to_string(),
                    operations: vec!["live".to_string()],
                    constraints: std::collections::BTreeMap::new(),
                    version: 1,
                    runtime_state: "active".to_string(),
                }],
                region: "us-east".to_string(),
                zone: "us-east".to_string(),
                network_zones: vec!["us-east".to_string()],
                capacity: Some(media_proto::MediaNodeCapacity {
                    max_sessions: 10,
                    max_bandwidth_mbps: 1000,
                    max_cpu_percent: 100,
                    available_sessions: 10,
                    available_bandwidth_mbps: 1000,
                    available_cpu_percent: 100,
                }),
                instance_id: "supplied-id".to_string(),
            }),
        });

        let response = service.register_media_node(request).await?;
        let info = response
            .into_inner()
            .node
            .ok_or_else(|| tonic::Status::internal("missing node"))?;

        assert_eq!(info.instance_id, "supplied-id");
        Ok(())
    }
}
