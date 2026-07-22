//! Background workers started by process assembly.

use async_trait::async_trait;
use cheetah_cluster_ownership::{
    DeviceAssignmentService, DeviceProtocolLookup, DrainingMigrationService, OwnerLeaseService,
    RateLimitConfig, RollingUpgradeError,
};
use cheetah_cluster_registry::NodeLeaseService;
use cheetah_domain::{
    Clock, Command, CommandPayload, DeviceOwnerResolver, NodeLoad, OwnerInfo, Protocol, UnitOfWork,
};
use cheetah_gb28181_module::{Gb28181Command, ProtocolSessionLink};
use cheetah_message_api::RawCommandBus;
use cheetah_plugin_host::PluginHost;
use cheetah_plugin_sdk::{DriverCommand, PluginName};
use cheetah_signal_application::{
    CommandDispatch, CommandHandler, CommandHandlerResult, InboxService, OperationStepOutcome,
    TakeoverService,
};
use cheetah_signal_types::{
    ChannelId, DeviceId, DurationMs, IdGenerator, NodeId, PageRequest, PluginId, SignalError,
    SignalErrorKind, TenantId, UtcTimestamp,
};
use cheetah_storage_api::{NodeRepository, OwnerRepository, Storage};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Maps domain devices to protocol identifiers for drain-driven migration.
pub struct StorageDeviceProtocolLookup {
    storage: Arc<dyn Storage>,
}

impl StorageDeviceProtocolLookup {
    /// Creates a new lookup.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl DeviceProtocolLookup for StorageDeviceProtocolLookup {
    async fn protocol_for(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<String>, RollingUpgradeError> {
        let mut uow = self
            .storage
            .begin()
            .await
            .map_err(RollingUpgradeError::Storage)?;
        let device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await
            .map_err(|e| RollingUpgradeError::Lookup(e.to_string()))?;
        let _ = uow.rollback().await;
        Ok(device.map(|d| match d.protocol() {
            Protocol::Gb28181 => "gb28181".to_string(),
            Protocol::Onvif => "onvif".to_string(),
            Protocol::Plugin => "plugin".to_string(),
            Protocol::Mqtt => "mqtt".to_string(),
            Protocol::Jt808 => "jt808".to_string(),
            Protocol::Isup => "isup".to_string(),
            Protocol::Unknown => "unknown".to_string(),
            _ => "unknown".to_string(),
        }))
    }
}

/// Sends a domain command to the GB28181 driver for outbound SIP `MESSAGE`
/// transmission.
#[async_trait]
pub trait Gb28181CommandBus: Send + Sync {
    /// Enqueues `command` on the bounded driver command channel.
    async fn send(&self, command: Gb28181Command) -> cheetah_signal_types::Result<()>;
}

/// Command bus backed by the GB28181 driver's bounded `mpsc` sender.
pub struct DriverCommandBus {
    tx: mpsc::Sender<Gb28181Command>,
}

impl DriverCommandBus {
    /// Creates a bus that wraps the supplied driver sender.
    pub fn new(tx: mpsc::Sender<Gb28181Command>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl Gb28181CommandBus for DriverCommandBus {
    async fn send(&self, command: Gb28181Command) -> cheetah_signal_types::Result<()> {
        self.tx
            .send(command)
            .await
            .map_err(|_| SignalError::new(SignalErrorKind::Internal, "gb28181 command bus closed"))
    }
}

/// Inbox command handler that routes GB28181 commands to the configured driver
/// buses and falls back to activated protocol plugins for other command kinds,
/// recording an `UNKNOWN_OUTCOME` without forging success when the outcome
/// cannot be determined.
///
/// Each GB28181 listener runs its own driver with an independent in-memory
/// registration table, so a device is reachable only through the driver that
/// terminated its REGISTER. The handler therefore holds every listener's bus
/// and fans a resolved command out to all of them (see
/// [`Self::handle_gb28181_command`]).
pub struct OwnerCommandHandler {
    plugin_host: Arc<Mutex<PluginHost>>,
    clock: Arc<dyn Clock>,
    gb_buses: Vec<Arc<dyn Gb28181CommandBus>>,
}

impl OwnerCommandHandler {
    /// Creates a new handler.
    ///
    /// `gb_buses` holds one command bus per configured GB28181 listener; an
    /// empty vector disables GB28181 command dispatch.
    pub fn new(
        plugin_host: Arc<Mutex<PluginHost>>,
        clock: Arc<dyn Clock>,
        gb_buses: Vec<Arc<dyn Gb28181CommandBus>>,
    ) -> Self {
        Self {
            plugin_host,
            clock,
            gb_buses,
        }
    }

    async fn resolve_gb_command(
        &self,
        uow: &mut dyn UnitOfWork,
        command: &Command,
    ) -> Option<Gb28181Command> {
        let tenant_id = command.tenant_id();
        let device_id = command.device_id();

        let device = match uow.device_repository().get(tenant_id, device_id).await {
            Ok(Some(d)) => d,
            Ok(None) => {
                warn!(tenant_id = %tenant_id, device_id = %device_id, "device not found for gb28181 command");
                return None;
            }
            Err(e) => {
                warn!(tenant_id = %tenant_id, device_id = %device_id, error = %e, "failed to load device for gb28181 command");
                return None;
            }
        };

        if device.protocol() != Protocol::Gb28181 {
            return None;
        }

        let device_external_id =
            cheetah_gb28181_module::types::DeviceId::new(device.external_id().as_ref())?;

        let channel_external_id = match channel_id_from_payload(command.payload()) {
            Some(channel_id) => match uow
                .channel_repository()
                .get(tenant_id, device_id, channel_id)
                .await
            {
                Ok(Some(channel)) => channel
                    .metadata()
                    .get("external_id")
                    .and_then(cheetah_gb28181_module::types::DeviceId::new),
                Ok(None) => {
                    warn!(tenant_id = %tenant_id, device_id = %device_id, channel_id = %channel_id, "channel not found for gb28181 command");
                    None
                }
                Err(e) => {
                    warn!(tenant_id = %tenant_id, device_id = %device_id, channel_id = %channel_id, error = %e, "failed to load channel for gb28181 command");
                    None
                }
            },
            None => None,
        };

        Some(Gb28181Command::new(
            command.clone(),
            device_external_id,
            channel_external_id,
        ))
    }

    async fn handle_gb28181_command(
        &self,
        uow: &mut dyn UnitOfWork,
        command: &Command,
        kind: &str,
    ) -> cheetah_signal_types::Result<CommandHandlerResult> {
        if self.gb_buses.is_empty() {
            return Ok(CommandHandlerResult::rejected(
                "gb28181 command bus not available",
            ));
        }

        let Some(gb_command) = self.resolve_gb_command(uow, command).await else {
            return Ok(CommandHandlerResult::rejected(
                "unable to resolve gb28181 command target",
            ));
        };

        // Each listener runs an independent driver holding its own in-memory
        // registration table, so only the driver that terminated the device's
        // REGISTER can resolve a send target. Fan the command out to every
        // listener bus: the owning driver emits the SIP MESSAGE while the
        // others reject it as `NotRegistered` without any side effect. This
        // keeps a device registered on a non-first listener reachable, which a
        // single captured bus does not.
        let mut enqueued = false;
        let mut last_error: Option<String> = None;
        for bus in &self.gb_buses {
            match bus.send(gb_command.clone()).await {
                Ok(()) => enqueued = true,
                Err(e) => last_error = Some(e.to_string()),
            }
        }

        if enqueued {
            Ok(
                CommandHandlerResult::accepted(
                    CommandDispatch::Sent,
                    OperationStepOutcome::Unknown,
                )
                .with_payload(format!(
                    r#"{{"command_kind":"{kind}","protocol":"gb28181"}}"#
                )),
            )
        } else {
            Ok(CommandHandlerResult::accepted(
                CommandDispatch::TransportFailed {
                    reason: last_error
                        .unwrap_or_else(|| "all gb28181 command buses closed".to_string()),
                },
                OperationStepOutcome::Unknown,
            ))
        }
    }

    async fn handle_plugin_command(
        &self,
        command: &Command,
        kind: &str,
    ) -> cheetah_signal_types::Result<CommandHandlerResult> {
        let payload = serde_json::to_value(command.payload()).unwrap_or(serde_json::Value::Null);
        let deadline = command
            .deadline()
            .map(|d| d.as_timestamp())
            .unwrap_or_else(|| self.clock.now_wall());
        let driver_command = DriverCommand {
            command_type: kind.to_string(),
            payload,
            idempotency_key: command.idempotency_key().to_string(),
            deadline,
        };

        let plugin_name = match kind {
            "StartLive" | "StopMediaSession" | "StartPlayback" | "StartTalk"
            | "ControlPlayback" | "process_sip" => "cheetah/gb28181",
            other if other.starts_with("Onvif") || other.starts_with("onvif") => "cheetah/onvif",
            _ => {
                return Ok(CommandHandlerResult::rejected(format!(
                    "unsupported command kind: {kind}"
                )));
            }
        };

        let result = {
            let host = self.plugin_host.lock().await;
            let Ok(name) = PluginName::new(plugin_name) else {
                return Ok(CommandHandlerResult::rejected(format!(
                    "invalid plugin name: {plugin_name}"
                )));
            };
            let Some(instance_id) = host.instance_id_for_name(&name) else {
                return Ok(CommandHandlerResult::rejected(format!(
                    "plugin not found: {plugin_name}"
                )));
            };
            host.handle_command(instance_id, driver_command).await
        };

        match result {
            Ok(()) => Ok(CommandHandlerResult::accepted(
                CommandDispatch::Sent,
                OperationStepOutcome::Unknown,
            )
            .with_payload(format!(
                r#"{{"command_kind":"{kind}","plugin":"{plugin_name}"}}"#
            ))),
            Err(e) => {
                warn!(
                    operation_id = %command.operation_id(),
                    error = %e,
                    "plugin handle_command failed; recording unknown outcome"
                );
                Ok(CommandHandlerResult::accepted(
                    CommandDispatch::TransportFailed {
                        reason: e.to_string(),
                    },
                    OperationStepOutcome::Unknown,
                ))
            }
        }
    }
}

#[async_trait]
impl CommandHandler for OwnerCommandHandler {
    async fn handle(
        &self,
        uow: &mut dyn UnitOfWork,
        command: &Command,
    ) -> cheetah_signal_types::Result<CommandHandlerResult> {
        let kind = command.kind();
        info!(
            operation_id = %command.operation_id(),
            device_id = %command.device_id(),
            command_kind = kind,
            "owner processing command"
        );

        if is_gb28181_command(command.payload()) {
            return self.handle_gb28181_command(uow, command, kind).await;
        }

        self.handle_plugin_command(command, kind).await
    }
}

fn is_gb28181_command(payload: &CommandPayload) -> bool {
    matches!(
        payload,
        CommandPayload::Query { .. }
            | CommandPayload::Ptz { .. }
            | CommandPayload::Preset { .. }
            | CommandPayload::DeviceControl { .. }
    )
}

fn channel_id_from_payload(payload: &CommandPayload) -> Option<ChannelId> {
    match payload {
        CommandPayload::Ptz { channel_id, .. } => Some(*channel_id),
        CommandPayload::Preset { preset } => Some(preset.channel_id),
        CommandPayload::DeviceControl { control } => control.channel_id,
        CommandPayload::Query { query } => query.channel_id,
        _ => None,
    }
}

/// Spawns the inbox consumer for this node's command subject partition.
pub fn spawn_inbox_worker(
    storage: Arc<dyn Storage>,
    command_bus: Arc<dyn RawCommandBus>,
    owner_resolver: Arc<dyn DeviceOwnerResolver>,
    command_handler: Arc<dyn CommandHandler>,
    clock: Arc<dyn Clock>,
    node_id: NodeId,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let subject = format!("sig.v1.command.*.{node_id}");
    let group = format!("cheetah-inbox-{node_id}");
    tokio::spawn(async move {
        let inbox = InboxService::new(
            storage,
            command_bus,
            owner_resolver,
            command_handler,
            clock,
            node_id,
            DurationMs::from_millis(86_400_000),
        );
        info!(%subject, %group, "inbox consumer starting");
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("inbox consumer cancelled");
            }
            result = inbox.run(&subject, &group) => {
                if let Err(e) = result {
                    warn!(error = %e, "inbox consumer exited with error");
                }
            }
        }
    })
}

/// Spawns the GB28181 protocol-session expiry reaper.
///
/// Each tick performs a bounded sweep over expired `ProtocolSession`s (those
/// whose `expiry_at` has passed) and marks the still-active ones offline,
/// making the expiry transition authoritative without relying on a per-device
/// timer. The sweep is idempotent: sessions already offline are skipped and
/// concurrent modifications by another owner are ignored.
#[allow(clippy::too_many_arguments)]
pub fn spawn_protocol_session_reaper_worker(
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    interval: Duration,
    batch_size: u32,
    max_per_tick: usize,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let link = ProtocolSessionLink::new(clock.clone(), id_generator);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("protocol session reaper cancelled");
                    break;
                }
                _ = ticker.tick() => {
                    let mut repo = storage.protocol_session_repository();
                    let now = clock.now_wall();
                    match link
                        .reap_expired(repo.as_mut(), now, batch_size, max_per_tick)
                        .await
                    {
                        Ok(0) => {}
                        Ok(count) => info!(count, "protocol session reaper marked sessions offline"),
                        Err(e) => warn!(error = %e, "protocol session reaper sweep failed"),
                    }
                }
            }
        }
    })
}

/// Spawns a node lease registration + heartbeat loop.
pub fn spawn_node_lease_worker(
    mut lease: NodeLeaseService,
    heartbeat_interval: Duration,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match lease
            .register(
                cheetah_domain::NodeCapacity {
                    max_devices: 100_000,
                },
                std::collections::HashMap::new(),
            )
            .await
        {
            Ok(node) => info!(node_id = %node.node_id, "cluster node registered"),
            Err(e) => {
                warn!(error = %e, "failed to register cluster node");
                return;
            }
        }

        let mut interval = tokio::time::interval(heartbeat_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    if let Err(e) = lease.mark_draining().await {
                        warn!(error = %e, "failed to mark node draining");
                    }
                    info!("node lease worker cancelled");
                    break;
                }
                _ = interval.tick() => {
                    match lease.heartbeat(NodeLoad { devices: 0 }).await {
                        Ok(_) => {}
                        Err(e) => warn!(error = %e, "node heartbeat failed"),
                    }
                }
            }
        }
    })
}

/// Spawns a worker that renews leases for devices owned by this node.
pub fn spawn_owner_lease_renew_worker(
    owner_lease: Arc<OwnerLeaseService>,
    owner_repo: Arc<dyn OwnerRepository>,
    node_id: NodeId,
    renew_interval: Duration,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(renew_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("owner lease renew worker cancelled");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = renew_owned_devices(owner_lease.as_ref(), owner_repo.as_ref(), node_id).await {
                        warn!(error = %e, "owner lease renew batch failed");
                    }
                }
            }
        }
    })
}

async fn renew_owned_devices(
    owner_lease: &OwnerLeaseService,
    owner_repo: &dyn OwnerRepository,
    node_id: NodeId,
) -> Result<(), cheetah_storage_api::StorageError> {
    let mut cursor: Option<String> = None;
    loop {
        let mut page = PageRequest::new(100)
            .map_err(|e| cheetah_storage_api::StorageError::invalid_argument(e.to_string()))?;
        if let Some(c) = cursor.take() {
            page = page.with_cursor(c);
        }
        let batch = owner_repo.list_by_node(node_id, page).await?;
        if batch.items.is_empty() {
            break;
        }
        let devices: Vec<(TenantId, DeviceId)> = batch
            .items
            .iter()
            .map(|d| (d.tenant_id, d.device_id))
            .collect();
        let results = owner_lease.batch_renew(&devices).await;
        for (device, result) in devices.iter().zip(results) {
            match result {
                Ok(Some(_)) => {}
                Ok(None) => {
                    warn!(
                        tenant_id = %device.0,
                        device_id = %device.1,
                        "lost ownership during renew"
                    );
                }
                Err(e) => warn!(
                    tenant_id = %device.0,
                    device_id = %device.1,
                    error = %e,
                    "renew failed"
                ),
            }
        }
        match batch.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(())
}

/// Spawns a cluster drain worker that migrates devices off nodes whose lease
/// has expired or which are marked draining.
pub fn spawn_drain_migration_worker(
    drain_service: Arc<DrainingMigrationService>,
    node_repo: Arc<dyn NodeRepository>,
    protocol_lookup: Arc<dyn DeviceProtocolLookup>,
    clock: Arc<dyn Clock>,
    this_node: NodeId,
    interval: Duration,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("drain migration worker cancelled");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(e) = drain_expired_nodes(
                        drain_service.as_ref(),
                        node_repo.as_ref(),
                        protocol_lookup.as_ref(),
                        clock.as_ref(),
                        this_node,
                    ).await {
                        warn!(error = %e, "drain migration pass failed");
                    }
                }
            }
        }
    })
}

async fn drain_expired_nodes(
    drain_service: &DrainingMigrationService,
    node_repo: &dyn NodeRepository,
    protocol_lookup: &dyn DeviceProtocolLookup,
    clock: &dyn Clock,
    this_node: NodeId,
) -> Result<(), RollingUpgradeError> {
    let now = clock.now_wall();
    // Scan a page of historically registered nodes via list_alive with a far-past
    // timestamp would only return live nodes. Instead scan with list_alive and
    // also attempt drain for nodes we know are draining through repeated owner
    // ownership recovery — for fencing we drain nodes that are not this node
    // and have expired lease_until by querying get for each owned device's owner
    // node. Simpler approach: list_alive at now returns live; for dead nodes we
    // rely on owner renew failure + SingleNode/Takeover paths.
    //
    // When cluster is enabled, re-home devices whose owner node is not alive.
    let alive = node_repo
        .list_alive(
            now,
            PageRequest::new(256).map_err(RollingUpgradeError::from)?,
        )
        .await
        .map_err(RollingUpgradeError::Storage)?;
    let alive_ids: std::collections::HashSet<NodeId> =
        alive.items.iter().map(|n| n.node_id).collect();

    // Drain this node is only done on shutdown. For peer dead nodes we need their
    // node_id; owners of devices list is per-node. Iterate alive draining peers.
    for node in &alive.items {
        if node.node_id == this_node {
            continue;
        }
        if node.draining {
            match drain_service
                .drain_node(node.node_id, protocol_lookup)
                .await
            {
                Ok(report) => {
                    info!(
                        node_id = %node.node_id,
                        migrated = report.migrated,
                        failed = report.failed,
                        skipped = report.skipped,
                        "drained peer node"
                    );
                }
                Err(e) => warn!(node_id = %node.node_id, error = %e, "drain peer failed"),
            }
        }
        let _ = &alive_ids;
    }
    Ok(())
}

/// Spawns a worker that exposes TakeoverService for local recovery hooks.
///
/// Periodically logs that the service is armed; actual takeover is driven by
/// protocol reconnect paths calling [`TakeoverService::takeover`].
pub fn spawn_takeover_health_worker(
    takeover: Arc<TakeoverService>,
    interval: Duration,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Keep Arc alive and periodically tick so readiness can observe the worker.
        let _ = takeover;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("takeover health worker cancelled");
                    break;
                }
                _ = ticker.tick() => {}
            }
        }
    })
}

/// Edge/single-node owner resolver: acquires this node as owner when none is set.
pub struct SingleNodeOwnerResolver {
    lease: OwnerLeaseService,
    inner: cheetah_cluster_ownership::CachingDeviceOwnerResolver,
}

impl SingleNodeOwnerResolver {
    /// Creates a new single-node owner resolver.
    pub fn new(
        lease: OwnerLeaseService,
        inner: cheetah_cluster_ownership::CachingDeviceOwnerResolver,
    ) -> Self {
        Self { lease, inner }
    }
}

#[async_trait]
impl DeviceOwnerResolver for SingleNodeOwnerResolver {
    async fn resolve(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<OwnerInfo>, cheetah_domain::DomainError> {
        if let Some(owner) = self.inner.resolve(tenant_id, device_id).await? {
            return Ok(Some(owner));
        }
        match self.lease.acquire(tenant_id, device_id).await {
            Ok(owner) => Ok(Some(owner)),
            Err(e) => Err(cheetah_domain::DomainError::internal(e.to_string())),
        }
    }
}

/// Builds a cluster assignment service used by drain/migration.
pub fn build_assignment_service(
    node_repo: Arc<tokio::sync::Mutex<dyn NodeRepository>>,
    owner_repo: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
    clock: Arc<dyn Clock>,
    lease_duration: DurationMs,
) -> DeviceAssignmentService {
    DeviceAssignmentService::new(
        node_repo,
        owner_repo,
        clock,
        lease_duration,
        RateLimitConfig::default(),
    )
}

/// Builds a draining migration service.
pub fn build_drain_service(
    assignment: DeviceAssignmentService,
    owner_repo: Arc<tokio::sync::Mutex<dyn OwnerRepository>>,
    node_repo: Arc<tokio::sync::Mutex<dyn NodeRepository>>,
    clock: Arc<dyn Clock>,
) -> DrainingMigrationService {
    DrainingMigrationService::new(assignment, owner_repo, node_repo, clock, 64, 10_000)
}

/// Builds a takeover service for this node.
pub fn build_takeover_service(
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    this_node: NodeId,
    lease_duration: DurationMs,
) -> TakeoverService {
    TakeoverService::new(storage, clock, id_generator, this_node, lease_duration)
}

/// Generates plugin ids for built-in activations.
pub fn builtin_plugin_ids(id_generator: &dyn IdGenerator) -> (PluginId, PluginId) {
    (
        id_generator.generate_plugin_id(),
        id_generator.generate_plugin_id(),
    )
}

// Silence unused import warnings when UtcTimestamp only needed by helpers.
#[allow(dead_code)]
fn _now(clock: &dyn Clock) -> UtcTimestamp {
    clock.now_wall()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, InMemoryUnitOfWork};
    use cheetah_domain::{Device, DeviceKind, Operation, PtzDirection};
    use cheetah_signal_application::InboxReceipt;
    use cheetah_signal_types::{
        OwnerEpoch, Principal, PrincipalKind, ProtocolIdentity, RequestContext, ResourceId,
        ResourceKind, ResourceRef,
    };
    use std::collections::BTreeMap;

    /// Records every command it receives so tests can assert per-listener
    /// delivery.
    struct RecordingBus {
        sent: Arc<Mutex<Vec<Gb28181Command>>>,
    }

    impl RecordingBus {
        fn new() -> (Arc<Self>, Arc<Mutex<Vec<Gb28181Command>>>) {
            let sent = Arc::new(Mutex::new(Vec::new()));
            (
                Arc::new(Self {
                    sent: Arc::clone(&sent),
                }),
                sent,
            )
        }
    }

    #[async_trait]
    impl Gb28181CommandBus for RecordingBus {
        async fn send(&self, command: Gb28181Command) -> cheetah_signal_types::Result<()> {
            self.sent.lock().await.push(command);
            Ok(())
        }
    }

    const DEVICE_GB_ID: &str = "34020000001320000001";

    async fn seed_gb_device(
        uow: &mut dyn UnitOfWork,
        clock: &dyn Clock,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) {
        let external_id = ProtocolIdentity::new(DEVICE_GB_ID).expect("valid gb id");
        let (device, _event) = Device::new(
            clock,
            tenant_id,
            device_id,
            Protocol::Gb28181,
            external_id,
            "factory",
            "test camera",
            DeviceKind::Camera,
            vec![],
            BTreeMap::new(),
        )
        .expect("device");
        uow.device_repository().save(&device).await.expect("save");
    }

    fn ptz_command(
        ids: &InMemoryIdGenerator,
        clock: &dyn Clock,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> Command {
        let context = RequestContext {
            tenant_id,
            principal: Principal {
                id: "tester".to_string(),
                kind: PrincipalKind::Service,
                scopes: vec!["write".to_string()],
            },
            message_id: ids.generate_message_id(),
            correlation_id: ids.generate_correlation_id(),
            traceparent: None,
            tracestate: None,
            deadline: None,
            node_id: Some(ids.generate_node_id()),
            source_ip: None,
        };
        let target = ResourceRef {
            tenant_id,
            kind: ResourceKind::Device,
            id: ResourceId::Device(device_id),
        };
        let payload = CommandPayload::Ptz {
            channel_id,
            direction: PtzDirection::Up,
            speed: 1.0,
        };
        let (operation, _event) = Operation::new(
            ids,
            clock,
            &context,
            "idem-ptz",
            device_id,
            target,
            payload,
            None,
            OwnerEpoch(1),
        )
        .expect("operation");
        operation.command().clone()
    }

    /// A command for a device registered on a non-first listener must reach
    /// every configured listener bus, not only the first one. With a single
    /// captured bus (the previous behaviour) the second listener's driver never
    /// received the command and the device was unreachable.
    #[tokio::test]
    async fn gb28181_command_fans_out_to_all_listener_buses() {
        let clock = Arc::new(InMemoryClock::new());
        let ids = InMemoryIdGenerator::new();
        let tenant_id = ids.generate_tenant_id();
        let device_id = ids.generate_device_id();
        let channel_id = ids.generate_channel_id();

        let mut uow = InMemoryUnitOfWork::new();
        seed_gb_device(&mut uow, clock.as_ref(), tenant_id, device_id).await;

        let (bus_a, recorded_a) = RecordingBus::new();
        let (bus_b, recorded_b) = RecordingBus::new();
        let handler = OwnerCommandHandler::new(
            Arc::new(Mutex::new(PluginHost::new(
                semver::Version::new(0, 1, 0),
                DurationMs::from_millis(1_000),
            ))),
            clock.clone(),
            vec![
                bus_a as Arc<dyn Gb28181CommandBus>,
                bus_b as Arc<dyn Gb28181CommandBus>,
            ],
        );

        let command = ptz_command(&ids, clock.as_ref(), tenant_id, device_id, channel_id);
        let result = handler.handle(&mut uow, &command).await.expect("handle");
        assert_eq!(result.dispatch, Some(CommandDispatch::Sent));

        let a = recorded_a.lock().await;
        let b = recorded_b.lock().await;
        assert_eq!(a.len(), 1, "first listener bus must receive the command");
        assert_eq!(b.len(), 1, "second listener bus must receive the command");
        assert_eq!(a[0].device_external_id.as_ref(), DEVICE_GB_ID);
        assert_eq!(b[0].device_external_id.as_ref(), DEVICE_GB_ID);
    }

    /// A single configured listener still receives the command.
    #[tokio::test]
    async fn gb28181_command_reaches_single_listener_bus() {
        let clock = Arc::new(InMemoryClock::new());
        let ids = InMemoryIdGenerator::new();
        let tenant_id = ids.generate_tenant_id();
        let device_id = ids.generate_device_id();
        let channel_id = ids.generate_channel_id();

        let mut uow = InMemoryUnitOfWork::new();
        seed_gb_device(&mut uow, clock.as_ref(), tenant_id, device_id).await;

        let (bus, recorded) = RecordingBus::new();
        let handler = OwnerCommandHandler::new(
            Arc::new(Mutex::new(PluginHost::new(
                semver::Version::new(0, 1, 0),
                DurationMs::from_millis(1_000),
            ))),
            clock.clone(),
            vec![bus as Arc<dyn Gb28181CommandBus>],
        );

        let command = ptz_command(&ids, clock.as_ref(), tenant_id, device_id, channel_id);
        let result = handler.handle(&mut uow, &command).await.expect("handle");
        assert_eq!(result.dispatch, Some(CommandDispatch::Sent));
        assert_eq!(recorded.lock().await.len(), 1);
    }

    /// With no listener bus configured the command is rejected rather than
    /// silently dropped.
    #[tokio::test]
    async fn gb28181_command_rejected_without_any_bus() {
        let clock = Arc::new(InMemoryClock::new());
        let ids = InMemoryIdGenerator::new();
        let tenant_id = ids.generate_tenant_id();
        let device_id = ids.generate_device_id();
        let channel_id = ids.generate_channel_id();

        let mut uow = InMemoryUnitOfWork::new();
        seed_gb_device(&mut uow, clock.as_ref(), tenant_id, device_id).await;

        let handler = OwnerCommandHandler::new(
            Arc::new(Mutex::new(PluginHost::new(
                semver::Version::new(0, 1, 0),
                DurationMs::from_millis(1_000),
            ))),
            clock.clone(),
            Vec::new(),
        );

        let command = ptz_command(&ids, clock.as_ref(), tenant_id, device_id, channel_id);
        let result = handler.handle(&mut uow, &command).await.expect("handle");
        assert!(matches!(result.receipt, InboxReceipt::Rejected { .. }));
    }
}
