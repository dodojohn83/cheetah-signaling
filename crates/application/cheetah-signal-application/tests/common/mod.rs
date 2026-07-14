//! Shared test helpers for application service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryCommandBus, InMemoryDeviceOwnerResolver, InMemoryEventPublisher,
    InMemoryIdGenerator, InMemoryMediaPort, InMemoryUnitOfWork,
    request_context as in_memory_request_context,
};
use cheetah_signal_application::{
    ChannelDescriptor, CommandDispatcher, DeviceService, EventService, MarkDeviceOnlineRequest,
    MediaService, OperationService, RegisterDeviceRequest, ReplaceChannelCatalogRequest,
};
use cheetah_signal_types::{ChannelId, DeviceId, IdGenerator, TenantId};

/// Shared test context for application service integration tests.
#[allow(missing_debug_implementations)]
pub struct TestContext {
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Pre-generated device identifier.
    pub device_id: DeviceId,
    /// Pre-generated channel identifier.
    pub channel_id: ChannelId,
    /// In-memory wall and monotonic clock.
    pub clock: Arc<InMemoryClock>,
    /// In-memory deterministic id generator.
    pub id_generator: Arc<InMemoryIdGenerator>,
    /// In-memory unit of work.
    pub uow: InMemoryUnitOfWork,
    /// Device application service.
    pub device_service: DeviceService,
    /// Operation application service.
    pub operation_service: OperationService,
    /// Media application service.
    pub media_service: MediaService,
    /// Command dispatcher.
    pub command_dispatcher: CommandDispatcher,
    /// Event service.
    pub event_service: EventService,
    /// In-memory device owner resolver.
    pub owner_resolver: Arc<InMemoryDeviceOwnerResolver>,
    /// In-memory command bus.
    pub command_bus: Arc<InMemoryCommandBus>,
    /// In-memory media port.
    pub media_port: Arc<InMemoryMediaPort>,
    /// In-memory event publisher.
    pub event_publisher: Arc<InMemoryEventPublisher>,
}

/// Creates a fully wired test context using in-memory ports.
pub fn setup() -> TestContext {
    let clock = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let clock_dyn: Arc<dyn cheetah_signal_types::Clock> = clock.clone();
    let id_gen_dyn: Arc<dyn cheetah_signal_types::IdGenerator> = id_generator.clone();

    let owner_resolver = Arc::new(InMemoryDeviceOwnerResolver::new());
    let command_bus = Arc::new(InMemoryCommandBus::new());
    let media_port = Arc::new(InMemoryMediaPort::new(id_gen_dyn.clone()));
    let event_publisher = Arc::new(InMemoryEventPublisher::new());

    let device_service = DeviceService::new(clock_dyn.clone(), id_gen_dyn.clone());
    let operation_service = OperationService::new(clock_dyn.clone(), id_gen_dyn.clone());
    let media_service = MediaService::new(
        clock_dyn.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        media_port.clone(),
    );
    let command_dispatcher = CommandDispatcher::new(
        clock_dyn.clone(),
        id_gen_dyn.clone(),
        owner_resolver.clone(),
        command_bus.clone(),
    );
    let event_service = EventService::new();

    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let channel_id = id_generator.generate_channel_id();

    TestContext {
        tenant_id,
        device_id,
        channel_id,
        clock,
        id_generator,
        uow: InMemoryUnitOfWork::new(),
        device_service,
        operation_service,
        media_service,
        command_dispatcher,
        event_service,
        owner_resolver,
        command_bus,
        media_port,
        event_publisher,
    }
}

/// Builds a request context for the given test context.
pub fn request_context(ctx: &TestContext) -> cheetah_signal_types::RequestContext {
    in_memory_request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock)
}

/// Registers a device, marks it online, and replaces the channel catalog.
pub async fn register_device_and_channel(
    ctx: &mut TestContext,
) -> cheetah_signal_application::DeviceDto {
    let context = request_context(ctx);
    let device = ctx
        .device_service
        .register_or_update_device(
            &context,
            &mut ctx.uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-1".to_string(),
                authority: Some("authority".to_string()),
                name: "camera-01".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .expect("register device should succeed")
        .device;

    ctx.device_service
        .mark_device_online(
            &context,
            &mut ctx.uow,
            device.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();

    ctx.device_service
        .replace_channel_catalog(
            &context,
            &mut ctx.uow,
            device.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: None,
                    name: "ch1".to_string(),
                    kind: "video".to_string(),
                    enabled: true,
                    status: Some("online".to_string()),
                    stream_profiles: Vec::new(),
                    ptz_capabilities: None,
                    metadata: None,
                }],
            },
        )
        .await
        .unwrap();

    device
}
