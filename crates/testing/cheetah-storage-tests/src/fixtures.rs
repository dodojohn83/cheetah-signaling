//! Test fixtures for repository contract tests.

use cheetah_domain::{
    BackoffPolicy, Channel, ChannelKind, ChannelStatus, Clock, ClusterNode, CommandPayload,
    CompatibilityProfile, Device, DeviceKind, DomainError, GbPlatformLink, LocalIdentity,
    MediaBinding, MediaPurpose, MediaSession, MediaSessionDesiredState, NewPlatformLink,
    NewProtocolSession, NodeCapacity, NodeLoad, Operation, PlatformAcl, PlatformCredential,
    PlatformDirection, PlatformEndpoint, PlatformIdentityPair, Protocol, ProtocolSession,
    PtzCapabilities, PtzDirection, RegistrationInfo, SessionEndpoint, SipTransport,
    SubscriptionLimits, WebhookConfig,
    in_memory::{InMemoryClock, InMemoryIdGenerator},
};
use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, IdGenerator, NodeId, NodeInstanceId, OwnerEpoch, Principal,
    PrincipalKind, ProtocolIdentity, RequestContext, ResourceId, ResourceKind, ResourceRef,
    TenantId,
};
use std::collections::BTreeMap;

/// A bundle of deterministic helpers used by contract tests.
#[derive(Debug)]
pub struct Fixtures {
    clock: InMemoryClock,
    id_generator: InMemoryIdGenerator,
}

impl Fixtures {
    /// Creates a new fixture bundle starting at the Unix epoch.
    pub fn new() -> Self {
        Self {
            clock: InMemoryClock::new(),
            id_generator: InMemoryIdGenerator::new(),
        }
    }

    /// Advances the clock by the given duration.
    pub fn advance(&self, duration: DurationMs) {
        self.clock.advance(duration);
    }

    /// Returns the clock as a trait object.
    pub fn clock(&self) -> &dyn cheetah_signal_types::Clock {
        &self.clock
    }

    /// Returns the id generator as a trait object.
    pub fn id_generator(&self) -> &dyn IdGenerator {
        &self.id_generator
    }

    /// Generates a new tenant id.
    pub fn tenant_id(&self) -> TenantId {
        self.id_generator.generate_tenant_id()
    }

    /// Generates a new device id.
    pub fn device_id(&self) -> DeviceId {
        self.id_generator.generate_device_id()
    }

    /// Generates a new node id.
    pub fn node_id(&self) -> NodeId {
        self.id_generator.generate_node_id()
    }

    /// Generates a new node instance id.
    pub fn node_instance_id(&self) -> NodeInstanceId {
        self.id_generator.generate_node_instance_id()
    }

    /// Creates a request context for the given tenant.
    pub fn request_context(&self, tenant_id: TenantId) -> RequestContext {
        RequestContext {
            tenant_id,
            principal: Principal {
                id: "test-user".to_string(),
                kind: PrincipalKind::User,
                scopes: vec!["read".to_string(), "write".to_string()],
            },
            message_id: self.id_generator.generate_message_id(),
            correlation_id: self.id_generator.generate_correlation_id(),
            traceparent: None,
            tracestate: None,
            deadline: None,
            node_id: Some(self.id_generator.generate_node_id()),
            source_ip: None,
        }
    }

    /// Creates a new device aggregate.
    pub fn device(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> cheetah_domain::Result<Device> {
        let external_id = ProtocolIdentity::new(format!("onvif-{}-cam", device_id.as_uuid()))
            .map_err(|e| DomainError::invalid_argument(e.to_string()))?;
        let (device, _event) = Device::new(
            self.clock(),
            tenant_id,
            device_id,
            Protocol::Onvif,
            external_id,
            "factory",
            "test camera",
            DeviceKind::Camera,
            vec![],
            BTreeMap::new(),
        )?;
        Ok(device)
    }

    /// Creates a new protocol session aggregate that expires after `expires_in`.
    pub fn protocol_session(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
        expires_in: DurationMs,
    ) -> cheetah_domain::Result<ProtocolSession> {
        let protocol_identity = ProtocolIdentity::new(format!(
            "3402000000132000{:012}",
            device_id.as_uuid().as_u128()
        ))
        .map_err(|e| DomainError::invalid_argument(e.to_string()))?;
        let expiry_at = self
            .clock
            .now_wall()
            .checked_add(expires_in)
            .ok_or_else(|| DomainError::internal("session expiry overflow"))?;
        ProtocolSession::new(
            self.clock(),
            NewProtocolSession {
                protocol_session_id: self.id_generator.generate_protocol_session_id(),
                tenant_id,
                device_id,
                protocol: Protocol::Gb28181,
                protocol_identity,
                local_identity: LocalIdentity {
                    listener_id: "listener-a".to_string(),
                    local_device_id: "34020000002000000001".to_string(),
                    domain: "3402000000".to_string(),
                    realm: "3402000000".to_string(),
                },
                transport: SipTransport::Udp,
                endpoint: SessionEndpoint {
                    observed_source: "203.0.113.10:5060".to_string(),
                    contact_uri: "sip:34020000001320000001@203.0.113.10:5060".to_string(),
                    advertised_endpoint: "192.0.2.1:5060".to_string(),
                },
                registration: RegistrationInfo {
                    call_id: "call-id-0001".to_string(),
                    cseq: 1,
                    expires_secs: 3600,
                },
                expiry_at,
                owner_node_id: None,
                owner_epoch: OwnerEpoch::default(),
                compatibility: CompatibilityProfile::default(),
            },
        )
    }

    /// Creates a new cascade platform link aggregate.
    ///
    /// `remote_suffix` differentiates the remote platform identity so multiple
    /// links can coexist in one tenant.
    pub fn platform_link(
        &self,
        tenant_id: TenantId,
        direction: PlatformDirection,
        remote_suffix: u16,
    ) -> cheetah_domain::Result<GbPlatformLink> {
        let local = ProtocolIdentity::new("34020000002000000001")
            .map_err(|e| DomainError::invalid_argument(e.to_string()))?;
        let remote = ProtocolIdentity::new(format!("110000000020000{remote_suffix:05}"))
            .map_err(|e| DomainError::invalid_argument(e.to_string()))?;
        GbPlatformLink::new(
            self.clock(),
            NewPlatformLink {
                platform_link_id: self.id_generator.generate_platform_link_id(),
                tenant_id,
                direction,
                identity: PlatformIdentityPair { local, remote },
                endpoint: PlatformEndpoint {
                    host: "203.0.113.9".to_string(),
                    port: 5060,
                    transport: SipTransport::Udp,
                    realm: "1100000000".to_string(),
                    domain: "1100000000".to_string(),
                },
                credential: PlatformCredential {
                    credential_ref: "secret://upstream".to_string(),
                    allow_md5: false,
                },
                acl: PlatformAcl {
                    allowed_catalog_prefixes: vec!["3402000000".to_string()],
                    allow_control: true,
                    allow_media: true,
                    denied_platform_ids: vec![],
                },
                backoff: BackoffPolicy::default(),
                subscription_limits: SubscriptionLimits::default(),
                register_interval_secs: 3600,
                compatibility_profile_id: None,
                compatibility_profile_revision: 0,
            },
        )
    }

    /// Creates a new channel aggregate.
    pub fn channel(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> cheetah_domain::Result<Channel> {
        let channel_id = self.id_generator.generate_channel_id();
        let (channel, _event) = Channel::new(
            self.clock(),
            tenant_id,
            device_id,
            channel_id,
            ChannelKind::Video,
            "main stream",
            true,
            Some(ChannelStatus::Online),
            vec![],
            PtzCapabilities::new(false, false, false, false, false, false),
            BTreeMap::new(),
        )?;
        Ok(channel)
    }

    /// Creates a new operation aggregate with a PTZ payload.
    pub fn operation(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> cheetah_domain::Result<Operation> {
        let context = self.request_context(tenant_id);
        let channel_id = self.id_generator.generate_channel_id();
        let target = ResourceRef {
            tenant_id,
            kind: ResourceKind::Channel,
            id: ResourceId::Channel(channel_id),
        };
        let payload = CommandPayload::Ptz {
            channel_id,
            direction: PtzDirection::Stop,
            speed: 0.0,
        };
        let deadline = Deadline::from_now(self.clock.now_wall(), DurationMs::from_millis(60_000));
        let (operation, _event) = Operation::new(
            self.id_generator(),
            self.clock(),
            &context,
            self.id_generator.generate_message_id().to_string(),
            device_id,
            target,
            payload,
            deadline,
            OwnerEpoch::default(),
        )?;
        Ok(operation)
    }

    /// Creates a new media session aggregate.
    pub fn media_session(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> cheetah_domain::Result<MediaSession> {
        let channel_id = self.id_generator.generate_channel_id();
        let operation_id = self.id_generator.generate_operation_id();
        let media_session_id = self.id_generator.generate_media_session_id();
        let scope = cheetah_domain::IdempotencyScope::new(
            tenant_id,
            "test-user",
            ResourceRef {
                tenant_id,
                kind: ResourceKind::MediaSession,
                id: ResourceId::MediaSession(media_session_id),
            },
            self.id_generator.generate_message_id().to_string(),
        )?;
        let (session, _event) = MediaSession::new(
            self.clock(),
            media_session_id,
            tenant_id,
            device_id,
            channel_id,
            MediaPurpose::Live,
            MediaSessionDesiredState::Active,
            OwnerEpoch::default(),
            operation_id,
            scope,
            Deadline::from_now(self.clock.now_wall(), DurationMs::from_millis(60_000)),
        )?;
        Ok(session)
    }

    /// Creates a new media binding aggregate.
    pub fn media_binding(
        &self,
        tenant_id: TenantId,
        media_session_id: cheetah_signal_types::MediaSessionId,
        channel_id: cheetah_signal_types::ChannelId,
    ) -> cheetah_domain::Result<MediaBinding> {
        let media_binding_id = self.id_generator.generate_media_binding_id();
        let media_node_id = self.id_generator.generate_node_id();
        let (binding, _event) = MediaBinding::new(
            self.clock(),
            media_binding_id,
            media_session_id,
            tenant_id,
            channel_id,
            media_node_id,
            OwnerEpoch::default(),
            self.id_generator.generate_media_node_instance_epoch(),
        )?;
        Ok(binding)
    }

    /// Creates a new cluster node aggregate.
    pub fn node(
        &self,
        node_id: NodeId,
        instance_id: NodeInstanceId,
    ) -> cheetah_domain::Result<ClusterNode> {
        let now = self.clock.now_wall();
        let mut node = ClusterNode::new(node_id, instance_id, "zone-a", "0.1.0", now);
        node.lease_until = now
            .checked_add(DurationMs::from_millis(60_000))
            .ok_or_else(|| DomainError::internal("node lease overflow"))?;
        node.updated_at = now;
        node.capacity = NodeCapacity { max_devices: 100 };
        node.load = NodeLoad { devices: 0 };
        Ok(node)
    }

    /// Creates a domain event suitable for the outbox.
    pub fn outbox_event(
        &self,
        tenant_id: TenantId,
    ) -> cheetah_signal_types::Event<cheetah_domain::DomainEvent> {
        let device_id = self.device_id();
        cheetah_signal_types::Event::new(
            self.id_generator(),
            self.clock(),
            &self.request_context(tenant_id),
            tenant_id,
            ResourceRef {
                tenant_id,
                kind: ResourceKind::Device,
                id: ResourceId::Device(device_id),
            },
            1,
            cheetah_domain::DomainEvent::DeviceOnlineChanged {
                tenant_id,
                device_id,
                connectivity: cheetah_domain::Connectivity::Online,
                lifecycle: cheetah_domain::DeviceLifecycle::Active,
                reason: None,
            },
        )
    }

    /// Creates a new webhook configuration aggregate.
    pub fn webhook_config(&self, tenant_id: TenantId) -> cheetah_domain::Result<WebhookConfig> {
        WebhookConfig::new(
            self.clock(),
            self.id_generator(),
            tenant_id,
            "https://example.com/webhook".to_string(),
            "secret://test".to_string(),
            vec!["device.online".to_string()],
        )
    }
}

impl Default for Fixtures {
    fn default() -> Self {
        Self::new()
    }
}
