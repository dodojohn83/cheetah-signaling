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
        let node = guard
            .as_mut()
            .ok_or_else(|| SchedulerError::NodeNotFound("test fake: node not registered".into()))?;
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
        let node = guard
            .as_mut()
            .ok_or_else(|| SchedulerError::NodeNotFound("test fake: node not registered".into()))?;
        node.draining = drain;
        Ok(node.clone())
    }

    async fn deregister(
        &self,
        _node_id: NodeId,
        _clock: &dyn Clock,
    ) -> Result<MediaNode, SchedulerError> {
        let mut guard = self.lock_node();
        let node = guard
            .take()
            .ok_or_else(|| SchedulerError::NodeNotFound("test fake: node not registered".into()))?;
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
        let node = guard
            .as_ref()
            .ok_or_else(|| SchedulerError::NodeNotFound("test fake: node not registered".into()))?;
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
        let node = guard
            .as_ref()
            .ok_or_else(|| SchedulerError::NodeNotFound("test fake: node not registered".into()))?;
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

#[tokio::test]
async fn register_rejects_oversized_zone_and_network_zones()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("33333333-3333-3333-3333-333333333333")?;
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
            zone: "x".repeat(257),
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

    match service.register_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("zone"));
        }
        Ok(_) => panic!("expected registration to be rejected"),
    }
    Ok(())
}

#[tokio::test]
async fn deregister_rejects_oversized_reason()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("44444444-4444-4444-4444-444444444444")?;
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

    let request = Request::new(DeregisterMediaNodeRequest {
        deregister: Some(media_proto::MediaNodeDeregister {
            node_id: node_id.to_string(),
            reason: "x".repeat(257),
        }),
    });

    match service.deregister_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("reason"));
        }
        Ok(_) => panic!("expected deregistration to be rejected"),
    }
    Ok(())
}

#[tokio::test]
async fn register_rejects_oversized_capability_runtime_state()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("55555555-5555-5555-5555-555555555555")?;
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
                runtime_state: "x".repeat(257),
            }],
            region: "us-east".to_string(),
            zone: "zone-a".to_string(),
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

    match service.register_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("runtime_state"));
        }
        Ok(_) => panic!("expected registration to be rejected"),
    }
    Ok(())
}

#[tokio::test]
async fn register_rejects_too_many_network_zones()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("66666666-6666-6666-6666-666666666666")?;
    let mut config = test_config();
    config.max_network_zones = 2;
    let registry = Arc::new(FakeRegistry {
        node: Mutex::new(Some(fake_existing_node(node_id))),
    });
    let service = MediaClusterRegistryService::new(
        registry.clone(),
        Arc::new(FakeClock::new()),
        Arc::new(FakeIdGenerator::new()),
        config,
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
            zone: "zone-a".to_string(),
            network_zones: vec!["a".to_string(), "b".to_string(), "c".to_string()],
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

    match service.register_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("network_zones"));
        }
        Ok(_) => panic!("expected registration to be rejected"),
    }
    Ok(())
}

#[tokio::test]
async fn register_rejects_too_many_capabilities()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("77777777-7777-7777-7777-777777777777")?;
    let mut config = test_config();
    config.max_capabilities = 1;
    let registry = Arc::new(FakeRegistry {
        node: Mutex::new(Some(fake_existing_node(node_id))),
    });
    let service = MediaClusterRegistryService::new(
        registry.clone(),
        Arc::new(FakeClock::new()),
        Arc::new(FakeIdGenerator::new()),
        config,
        crate::MediaMetrics::arc(),
        Arc::new(NoOpAuditLog),
        node_id,
    );

    let capability = media_proto::MediaCapability {
        protocol: "gb28181".to_string(),
        operations: vec!["live".to_string()],
        constraints: std::collections::BTreeMap::new(),
        version: 1,
        runtime_state: "active".to_string(),
    };
    let request = Request::new(RegisterMediaNodeRequest {
        node: Some(media_proto::MediaNodeRegistration {
            node_id: node_id.to_string(),
            listen_addr: "https://1.1.1.1:443".to_string(),
            capability: None,
            capabilities: vec![capability.clone(), capability],
            region: "us-east".to_string(),
            zone: "zone-a".to_string(),
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

    match service.register_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("capabilities"));
        }
        Ok(_) => panic!("expected registration to be rejected"),
    }
    Ok(())
}

#[tokio::test]
async fn heartbeat_rejects_oversized_lease_id()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let node_id = NodeId::from_str("88888888-8888-8888-8888-888888888888")?;
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

    let request = Request::new(HeartbeatMediaNodeRequest {
        heartbeat: Some(media_proto::MediaNodeHeartbeat {
            node_id: node_id.to_string(),
            load: 10,
            session_count: 0,
            lease_id: "x".repeat(257),
            instance_epoch: 1,
        }),
    });

    match service.heartbeat_media_node(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
            assert!(status.message().contains("lease_id"));
        }
        Ok(_) => panic!("expected heartbeat to be rejected"),
    }
    Ok(())
}
