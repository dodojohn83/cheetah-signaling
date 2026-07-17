//! Integration tests for the media registry gRPC server with TLS and mTLS.

#![allow(clippy::unwrap_used)]

use cheetah_media_scheduler::config::MediaRegistryConfig;
use cheetah_media_scheduler::grpc::MediaClusterRegistryService;
use cheetah_media_scheduler::registry::InMemoryMediaNodeRegistry;
use cheetah_media_scheduler::server::GrpcServer;
use cheetah_signal_contracts::cheetah::common::v1::RegisterMediaNodeRequest;
use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_client::MediaClusterRegistryClient;
use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCapability, MediaNodeCapacity, MediaNodeRegistration,
};
use cheetah_signal_types::{
    ChannelId, Clock, CorrelationId, DeliveryId, DeviceId, DurationMs, EndpointId, EventId,
    IdGenerator, MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, MessageId, NodeId,
    OperationId, PluginId, ProtocolSessionId, Result as SignalResult, SecretStore, SignalError,
    SignalErrorKind, TenantId, UtcTimestamp, WebhookId,
};
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair, KeyUsagePurpose,
};
use secrecy::SecretString;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

const NODE_ID: &str = "22222222-2222-2222-2222-222222222222";

fn node_id() -> NodeId {
    NodeId::from_str(NODE_ID).unwrap()
}

struct ManualClock {
    wall: Mutex<OffsetDateTime>,
}

impl ManualClock {
    fn new(wall: OffsetDateTime) -> Self {
        Self {
            wall: Mutex::new(wall),
        }
    }
}

impl Clock for ManualClock {
    fn now_wall(&self) -> UtcTimestamp {
        UtcTimestamp::from_offset(*self.wall.lock().unwrap())
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::from_millis(0)
    }
}

struct TestIdGenerator;

impl IdGenerator for TestIdGenerator {
    fn generate_tenant_id(&self) -> TenantId {
        TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn generate_device_id(&self) -> DeviceId {
        DeviceId::from_str("11111111-1111-1111-1111-111111111112").unwrap()
    }

    fn generate_endpoint_id(&self) -> EndpointId {
        EndpointId::from_str("11111111-1111-1111-1111-111111111113").unwrap()
    }

    fn generate_channel_id(&self) -> ChannelId {
        ChannelId::from_str("11111111-1111-1111-1111-111111111114").unwrap()
    }

    fn generate_protocol_session_id(&self) -> ProtocolSessionId {
        ProtocolSessionId::from_str("11111111-1111-1111-1111-111111111115").unwrap()
    }

    fn generate_media_session_id(&self) -> MediaSessionId {
        MediaSessionId::from_str("11111111-1111-1111-1111-111111111116").unwrap()
    }

    fn generate_media_binding_id(&self) -> MediaBindingId {
        MediaBindingId::from_str("11111111-1111-1111-1111-111111111117").unwrap()
    }

    fn generate_media_node_instance_epoch(&self) -> MediaNodeInstanceEpoch {
        MediaNodeInstanceEpoch(1)
    }

    fn generate_operation_id(&self) -> OperationId {
        OperationId::from_str("11111111-1111-1111-1111-111111111118").unwrap()
    }

    fn generate_node_id(&self) -> NodeId {
        node_id()
    }

    fn generate_plugin_id(&self) -> PluginId {
        PluginId::from_str("11111111-1111-1111-1111-111111111119").unwrap()
    }

    fn generate_event_id(&self) -> EventId {
        EventId::from_str("11111111-1111-1111-1111-11111111111a").unwrap()
    }

    fn generate_message_id(&self) -> MessageId {
        MessageId::from_str("11111111-1111-1111-1111-11111111111b").unwrap()
    }

    fn generate_correlation_id(&self) -> CorrelationId {
        CorrelationId::from_str("11111111-1111-1111-1111-11111111111c").unwrap()
    }

    fn generate_webhook_id(&self) -> WebhookId {
        WebhookId::from_str("11111111-1111-1111-1111-11111111111d").unwrap()
    }

    fn generate_delivery_id(&self) -> DeliveryId {
        DeliveryId::from_str("11111111-1111-1111-1111-11111111111e").unwrap()
    }
}

#[derive(Debug)]
struct TestSecretStore {
    secrets: Mutex<HashMap<String, SecretString>>,
}

impl TestSecretStore {
    fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }

    fn put(&self, key: &str, value: String) {
        self.secrets
            .lock()
            .unwrap()
            .insert(key.to_string(), SecretString::new(value.into_boxed_str()));
    }
}

impl SecretStore for TestSecretStore {
    fn get(&self, key: &str) -> SignalResult<SecretString> {
        self.secrets
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))
    }

    fn put(&self, key: &str, value: SecretString) -> SignalResult<()> {
        self.secrets.lock().unwrap().insert(key.to_string(), value);
        Ok(())
    }

    fn delete(&self, key: &str) -> SignalResult<()> {
        self.secrets.lock().unwrap().remove(key);
        Ok(())
    }

    fn rotate(&self, key: &str) -> SignalResult<SecretString> {
        let mut secrets = self.secrets.lock().unwrap();
        let prev = secrets.remove(key);
        if let Some(prev) = prev {
            secrets.insert(key.to_string(), prev.clone());
            Ok(prev)
        } else {
            Err(SignalError::new(
                SignalErrorKind::NotFound,
                "secret not found",
            ))
        }
    }
}

fn test_config(require_mtls: bool) -> MediaRegistryConfig {
    let mut config = MediaRegistryConfig::test();
    config.require_mtls = require_mtls;
    config
}

fn build_service(config: MediaRegistryConfig) -> MediaClusterRegistryService {
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(config.clone()));
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let id_generator = Arc::new(TestIdGenerator);
    MediaClusterRegistryService::new(registry, clock, id_generator, config)
}

fn registration(node_id: &str) -> RegisterMediaNodeRequest {
    RegisterMediaNodeRequest {
        node: Some(MediaNodeRegistration {
            node_id: node_id.to_string(),
            listen_addr: "http://127.0.0.1:9000".to_string(),
            capability: Some(MediaCapability {
                protocol: "gb28181".to_string(),
                operations: vec!["live".to_string()],
                constraints: Default::default(),
            }),
            region: "region-1".to_string(),
            capacity: Some(MediaNodeCapacity {
                max_sessions: 4,
                max_bandwidth_mbps: 1000,
                max_cpu_percent: 80,
            }),
            instance_id: "instance-1".to_string(),
        }),
    }
}

fn make_ca() -> CertifiedIssuer<'static, KeyPair> {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "test-ca");
    CertifiedIssuer::self_signed(ca_params, ca_key).unwrap()
}

fn make_server_cert(
    ca: &CertifiedIssuer<'static, KeyPair>,
) -> (rcgen::Certificate, rcgen::KeyPair) {
    let server_key = KeyPair::generate().unwrap();
    let mut server_params = CertificateParams::new(vec!["127.0.0.1".to_string()]).unwrap();
    server_params
        .distinguished_name
        .push(DnType::CommonName, "server");
    let server_cert = server_params.signed_by(&server_key, ca).unwrap();
    (server_cert, server_key)
}

fn make_client_cert(
    ca: &CertifiedIssuer<'static, KeyPair>,
    cn: &str,
) -> (rcgen::Certificate, rcgen::KeyPair) {
    let client_key = KeyPair::generate().unwrap();
    let mut client_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    client_params
        .distinguished_name
        .push(DnType::CommonName, cn);
    let client_cert = client_params.signed_by(&client_key, ca).unwrap();
    (client_cert, client_key)
}

async fn plain_client(addr: SocketAddr) -> MediaClusterRegistryClient<tonic::transport::Channel> {
    let channel = Endpoint::new(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    MediaClusterRegistryClient::new(channel)
}

async fn tls_client(
    addr: SocketAddr,
    ca_pem: &str,
) -> MediaClusterRegistryClient<tonic::transport::Channel> {
    let tls = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca_pem));
    let channel = Endpoint::new(format!("https://{addr}"))
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap();
    MediaClusterRegistryClient::new(channel)
}

async fn mtls_client(
    addr: SocketAddr,
    ca_pem: &str,
    cert_pem: &str,
    key_pem: &str,
) -> MediaClusterRegistryClient<tonic::transport::Channel> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca_pem))
        .identity(Identity::from_pem(cert_pem, key_pem));
    let channel = Endpoint::new(format!("https://{addr}"))
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap();
    MediaClusterRegistryClient::new(channel)
}

#[tokio::test]
async fn plain_grpc_server_responds_to_register() {
    let service = build_service(test_config(false));
    let config = cheetah_signal_types::config::GrpcConfig {
        listen_addr: "127.0.0.1".to_string(),
        port: 0,
        tls_cert_ref: None,
        tls_key_ref: None,
        tls_client_ca_ref: None,
    };

    let server = GrpcServer::start(&config, service, None).await.unwrap();
    let mut client = plain_client(server.local_addr).await;

    let response = client
        .register_media_node(registration(NODE_ID))
        .await
        .unwrap();
    assert_eq!(response.into_inner().node.unwrap().node_id, NODE_ID);

    server.shutdown();
}

#[tokio::test]
async fn tls_grpc_server_responds_to_register_without_mtls() {
    let ca = make_ca();
    let (server_cert, server_key) = make_server_cert(&ca);
    let secret_store = Arc::new(TestSecretStore::new());
    secret_store.put("server-cert", server_cert.pem());
    secret_store.put("server-key", server_key.serialize_pem());

    let service = build_service(test_config(false));
    let config = cheetah_signal_types::config::GrpcConfig {
        listen_addr: "127.0.0.1".to_string(),
        port: 0,
        tls_cert_ref: Some("server-cert".to_string()),
        tls_key_ref: Some("server-key".to_string()),
        tls_client_ca_ref: None,
    };

    let server = GrpcServer::start(&config, service, Some(secret_store))
        .await
        .unwrap();
    let ca_pem = ca.as_ref().pem();
    let mut client = tls_client(server.local_addr, &ca_pem).await;

    let response = client
        .register_media_node(registration(NODE_ID))
        .await
        .unwrap();
    assert_eq!(response.into_inner().node.unwrap().node_id, NODE_ID);

    server.shutdown();
}

#[tokio::test]
async fn mtls_grpc_server_accepts_matching_identity() {
    let ca = make_ca();
    let (server_cert, server_key) = make_server_cert(&ca);
    let (client_cert, client_key) = make_client_cert(&ca, NODE_ID);

    let secret_store = Arc::new(TestSecretStore::new());
    secret_store.put("server-cert", server_cert.pem());
    secret_store.put("server-key", server_key.serialize_pem());
    secret_store.put("client-ca", ca.as_ref().pem());

    let service = build_service(test_config(true));
    let config = cheetah_signal_types::config::GrpcConfig {
        listen_addr: "127.0.0.1".to_string(),
        port: 0,
        tls_cert_ref: Some("server-cert".to_string()),
        tls_key_ref: Some("server-key".to_string()),
        tls_client_ca_ref: Some("client-ca".to_string()),
    };

    let server = GrpcServer::start(&config, service, Some(secret_store))
        .await
        .unwrap();

    let ca_pem = ca.as_ref().pem();
    let client_cert_pem = client_cert.pem();
    let client_key_pem = client_key.serialize_pem();
    let mut client = mtls_client(
        server.local_addr,
        &ca_pem,
        &client_cert_pem,
        &client_key_pem,
    )
    .await;

    let response = client
        .register_media_node(registration(NODE_ID))
        .await
        .unwrap();
    assert_eq!(response.into_inner().node.unwrap().node_id, NODE_ID);

    server.shutdown();
}

#[tokio::test]
async fn mtls_grpc_server_rejects_mismatched_identity() {
    let ca = make_ca();
    let (server_cert, server_key) = make_server_cert(&ca);
    let (client_cert, client_key) = make_client_cert(&ca, "other-node-id");

    let secret_store = Arc::new(TestSecretStore::new());
    secret_store.put("server-cert", server_cert.pem());
    secret_store.put("server-key", server_key.serialize_pem());
    secret_store.put("client-ca", ca.as_ref().pem());

    let service = build_service(test_config(true));
    let config = cheetah_signal_types::config::GrpcConfig {
        listen_addr: "127.0.0.1".to_string(),
        port: 0,
        tls_cert_ref: Some("server-cert".to_string()),
        tls_key_ref: Some("server-key".to_string()),
        tls_client_ca_ref: Some("client-ca".to_string()),
    };

    let server = GrpcServer::start(&config, service, Some(secret_store))
        .await
        .unwrap();

    let ca_pem = ca.as_ref().pem();
    let client_cert_pem = client_cert.pem();
    let client_key_pem = client_key.serialize_pem();
    let mut client = mtls_client(
        server.local_addr,
        &ca_pem,
        &client_cert_pem,
        &client_key_pem,
    )
    .await;

    let status = client
        .register_media_node(registration(NODE_ID))
        .await
        .unwrap_err();
    assert_eq!(status.code(), tonic::Code::PermissionDenied);

    server.shutdown();
}
