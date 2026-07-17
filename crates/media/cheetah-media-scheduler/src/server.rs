//! gRPC server for the media node registry with optional TLS and mTLS.

use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_server::{
    MediaClusterRegistry, MediaClusterRegistryServer,
};
use cheetah_signal_types::SecretStore;
use cheetah_signal_types::config::GrpcConfig;
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::server::{ServerTlsConfig, TcpConnectInfo, TcpIncoming, TlsConnectInfo};
use tonic::transport::{Certificate, Identity, Server};
use tonic::{Request, Status};
use x509_parser::prelude::GeneralName;

use crate::error::SchedulerError;
use crate::grpc::PeerIdentity;

const DEFAULT_GRPC_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_GRPC_CONCURRENCY_LIMIT: usize = 256;
const DEFAULT_GRPC_MAX_CONCURRENT_STREAMS: u32 = 100;
const DEFAULT_GRPC_KEEPALIVE: Duration = Duration::from_secs(60);

/// Running gRPC server handle.
#[derive(Debug)]
pub struct GrpcServer {
    /// Bound socket address.
    pub local_addr: SocketAddr,
    /// Shutdown signal.
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl GrpcServer {
    /// Starts the media registry gRPC server.
    ///
    /// When `grpc.tls_cert_ref` and `grpc.tls_key_ref` are present the server serves
    /// over TLS. If `grpc.tls_client_ca_ref` is also present, clients must present a
    /// certificate signed by that CA. The DNS SANs and subject common names of the
    /// peer certificate are collected into a [`PeerIdentity`] for mTLS node
    /// verification.
    pub async fn start<R>(
        config: &GrpcConfig,
        registry: R,
        secret_store: Option<Arc<dyn SecretStore>>,
    ) -> Result<Self, SchedulerError>
    where
        R: MediaClusterRegistry + Send + Sync + 'static,
    {
        let addr: SocketAddr = format!("{}:{}", config.listen_addr, config.port)
            .parse()
            .map_err(|e| SchedulerError::Transport(format!("invalid listen address: {e}")))?;

        let service = MediaClusterRegistryServer::with_interceptor(registry, mtls_interceptor);

        let mut server = Server::builder()
            .timeout(DEFAULT_GRPC_TIMEOUT)
            .concurrency_limit_per_connection(DEFAULT_GRPC_CONCURRENCY_LIMIT)
            .max_concurrent_streams(DEFAULT_GRPC_MAX_CONCURRENT_STREAMS);

        server = match (config.tls_cert_ref.as_ref(), config.tls_key_ref.as_ref()) {
            (Some(cert_ref), Some(key_ref)) => {
                let secret_store = secret_store.ok_or_else(|| {
                    SchedulerError::Tls(
                        "TLS certificate references require a secret store".to_string(),
                    )
                })?;

                let cert_pem = secret_store.get(cert_ref).map_err(|e| {
                    SchedulerError::Tls(format!("failed to load TLS certificate: {e}"))
                })?;
                let key_pem = secret_store.get(key_ref).map_err(|e| {
                    SchedulerError::Tls(format!("failed to load TLS private key: {e}"))
                })?;

                if rustls::crypto::CryptoProvider::get_default().is_none() {
                    let _ = rustls::crypto::ring::default_provider().install_default();
                }

                let identity = Identity::from_pem(
                    cert_pem.expose_secret().as_bytes(),
                    key_pem.expose_secret().as_bytes(),
                );
                let tls_config = ServerTlsConfig::new().identity(identity);

                if let Some(ca_ref) = config.tls_client_ca_ref.as_ref() {
                    let ca_pem = secret_store.get(ca_ref).map_err(|e| {
                        SchedulerError::Tls(format!("failed to load TLS client CA: {e}"))
                    })?;
                    let client_ca = Certificate::from_pem(ca_pem.expose_secret().as_bytes());
                    let tls_config = tls_config
                        .client_ca_root(client_ca)
                        .client_auth_optional(false);
                    server.tls_config(tls_config).map_err(|e| {
                        SchedulerError::Tls(format!("failed to configure mTLS: {e}"))
                    })?
                } else {
                    server
                        .tls_config(tls_config)
                        .map_err(|e| SchedulerError::Tls(format!("failed to configure TLS: {e}")))?
                }
            }
            (None, None) => {
                if config.tls_client_ca_ref.is_some() {
                    return Err(SchedulerError::Tls(
                        "grpc.tls_client_ca_ref requires grpc.tls_cert_ref and grpc.tls_key_ref"
                            .to_string(),
                    ));
                }
                server
            }
            _ => {
                return Err(SchedulerError::Tls(
                    "grpc.tls_cert_ref and grpc.tls_key_ref must both be set or unset".to_string(),
                ));
            }
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        let incoming = TcpIncoming::bind(addr)
            .map_err(|e| SchedulerError::Transport(format!("failed to bind gRPC listener: {e}")))?
            .with_nodelay(Some(true))
            .with_keepalive(Some(DEFAULT_GRPC_KEEPALIVE));
        let local_addr = incoming.local_addr().map_err(|e| {
            SchedulerError::Transport(format!("failed to get gRPC local address: {e}"))
        })?;
        let router = server.add_service(service);

        tokio::spawn(async move {
            if let Err(e) = router
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = rx.await;
                })
                .await
            {
                tracing::error!("gRPC server error: {e}");
            }
        });

        Ok(Self {
            local_addr,
            shutdown: tx,
        })
    }

    /// Requests a graceful shutdown.
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}

#[allow(clippy::unnecessary_wraps)]
fn mtls_interceptor(mut request: Request<()>) -> Result<Request<()>, Status> {
    if let Some(identity) = request
        .extensions()
        .get::<TlsConnectInfo<TcpConnectInfo>>()
        .and_then(|tls| tls.peer_certs())
        .and_then(|certs| certs.first().cloned())
        .and_then(|cert| extract_peer_identity(cert.as_ref()))
    {
        request.extensions_mut().insert(identity);
    }
    Ok(request)
}

fn extract_peer_identity(cert_der: &[u8]) -> Option<PeerIdentity> {
    let (_, cert) = x509_parser::parse_x509_certificate(cert_der).ok()?;
    let mut identities: Vec<String> = Vec::new();

    if let Some(san) = cert.subject_alternative_name().ok().flatten() {
        for name in san.value.general_names.iter() {
            if let GeneralName::DNSName(dns) = name {
                identities.push(dns.to_string());
            }
        }
    }

    for cn in cert.subject().iter_common_name() {
        if let Ok(value) = cn.as_str() {
            identities.push(value.to_string());
        }
    }

    if identities.is_empty() {
        None
    } else {
        Some(PeerIdentity(identities))
    }
}
