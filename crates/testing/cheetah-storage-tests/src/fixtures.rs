//! Test fixtures for repository contract tests.

use cheetah_domain::{
    Channel, ChannelKind, ChannelStatus, Clock, CommandPayload, Device, DeviceKind, DomainError,
    MediaBinding, MediaPurpose, MediaSession, MediaSessionDesiredState, Operation, Protocol,
    PtzCapabilities, PtzDirection,
    in_memory::{InMemoryClock, InMemoryIdGenerator},
};
use cheetah_signal_types::{
    Deadline, DeviceId, DurationMs, IdGenerator, OwnerEpoch, Principal, PrincipalKind,
    ProtocolIdentity, RequestContext, ResourceId, ResourceKind, ResourceRef, TenantId,
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
}

impl Default for Fixtures {
    fn default() -> Self {
        Self::new()
    }
}
