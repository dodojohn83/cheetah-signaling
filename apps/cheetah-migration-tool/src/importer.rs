//! Core migration orchestrator.

use crate::error::MigrationError;
use crate::mappers::{
    MappedAggregate, ParentProtocols, event_for, map_channel, map_record, parse_protocol,
    stable_device_id, stable_tenant_id,
};
use crate::model::{EntityType, OldRecord};
use crate::source::RecordSource;
use cheetah_domain::{Device, DeviceRepository, DomainError, DomainEvent, Protocol};
use cheetah_signal_types::{
    Clock, DeviceId, Event, ProtocolIdentity, ResourceId, ResourceKind, TenantId,
};
use cheetah_storage_api::Storage;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// Migration execution options.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Maximum records to process before committing a checkpoint.
    pub checkpoint_every: usize,
    /// Only import records whose external ID appears in this set (empty = all).
    pub cutover_ids: HashSet<String>,
    /// If true, validate and summarize but do not write to the target database.
    pub dry_run: bool,
    /// If true, skip records that already exist in the target database.
    pub skip_existing: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            checkpoint_every: 100,
            cutover_ids: HashSet::new(),
            dry_run: false,
            skip_existing: true,
        }
    }
}

/// Result of an import run.
#[derive(Clone, Debug, Default)]
pub struct ImportResult {
    /// Number of records read from the source.
    pub records_read: usize,
    /// Number of records successfully mapped and imported/validated.
    pub records_imported: usize,
    /// Number of records skipped because they were filtered out by cutover.
    pub records_skipped: usize,
    /// Number of invalid records.
    pub records_invalid: usize,
    /// Number of records that already existed and were skipped with skip_existing=true.
    pub records_skipped_existing: usize,
    /// Number of records that conflicted with an existing aggregate and could not be imported.
    pub records_conflicting: usize,
    /// Number of records carrying plaintext secrets that must be re-entered.
    pub records_with_secrets: usize,
    /// Human-readable action items for the operator.
    pub action_items: Vec<String>,
    /// Per-category counts for the summary.
    pub counts_by_kind: BTreeMap<String, usize>,
}

/// Migrates data from an old system into Cheetah Signaling storage.
pub struct Importer {
    storage: Option<Arc<dyn Storage>>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Importer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Importer").finish_non_exhaustive()
    }
}

impl Importer {
    /// Creates a new importer backed by the given storage and clock.
    /// `storage` may be `None` when `ImportOptions::dry_run` is `true`, in which
    /// case no database access occurs.
    pub fn new(storage: Option<Arc<dyn Storage>>, clock: Arc<dyn Clock>) -> Self {
        Self { storage, clock }
    }

    /// Runs an import from the provided source using the given options.
    pub async fn import(
        &self,
        source: &dyn RecordSource,
        options: &ImportOptions,
    ) -> Result<ImportResult, MigrationError> {
        if options.checkpoint_every == 0 {
            return Err(MigrationError::other("checkpoint_every must be at least 1"));
        }

        let records = source.read_records().await?;
        let mut result = ImportResult {
            records_read: records.len(),
            ..ImportResult::default()
        };

        let mut devices: Vec<Device> = Vec::new();
        let mut device_events: Vec<Event<DomainEvent>> = Vec::new();
        let mut channel_records: Vec<(OldRecord, Protocol)> = Vec::new();

        let parent_protocols = build_parent_protocols(&records);

        for (row, record) in records.iter().enumerate() {
            if !options.cutover_ids.is_empty() && !options.cutover_ids.contains(&record.external_id)
            {
                result.records_skipped += 1;
                continue;
            }

            *result
                .counts_by_kind
                .entry(format!("{:?}", record.entity_type))
                .or_insert(0) += 1;

            match record.entity_type {
                EntityType::Device | EntityType::Gb28181Platform | EntityType::OnvifEndpoint => {
                    let entity = match map_record(self.clock.as_ref(), record, &parent_protocols) {
                        Ok(entity) => entity,
                        Err(e) => {
                            tracing::warn!(row = row + 1, error = %e, "skipping invalid record");
                            result.records_invalid += 1;
                            continue;
                        }
                    };

                    if record.has_secret() {
                        result.records_with_secrets += 1;
                        result.action_items.extend(entity.actions.clone());
                    }

                    match entity.entity {
                        MappedAggregate::Device(d)
                        | MappedAggregate::Gb28181Platform(d)
                        | MappedAggregate::OnvifEndpoint(d) => {
                            devices.push(d);
                            device_events.extend(entity.events);
                        }
                        _ => {}
                    }
                }
                EntityType::Channel => {
                    let parent_protocol = parent_protocols
                        .get(&(record.tenant_id.clone(), record.parent_device_id.clone()))
                        .copied()
                        .unwrap_or_else(|| {
                            parse_protocol(&record.protocol).unwrap_or(Protocol::Gb28181)
                        });
                    channel_records.push((record.clone(), parent_protocol));
                }
                EntityType::SecretReference | EntityType::Tenant | EntityType::Unknown => {}
            }
        }

        if options.dry_run {
            result.records_imported = devices.len();
            for (record, parent_protocol) in &channel_records {
                let parent_device_id = stable_device_id(
                    &record.tenant_id,
                    &record.parent_device_id,
                    *parent_protocol,
                );
                match map_channel(
                    self.clock.as_ref(),
                    record,
                    *parent_protocol,
                    parent_device_id,
                ) {
                    Ok(entity) => {
                        result.records_imported += 1;
                        if record.has_secret() {
                            result.records_with_secrets += 1;
                            result.action_items.extend(entity.actions);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(row = 0, error = %e, "skipping invalid channel");
                        result.records_invalid += 1;
                    }
                }
            }
        } else {
            let storage = self.storage.as_ref().ok_or_else(|| {
                MigrationError::other("storage is required for non-dry-run import")
            })?;
            self.flush_devices(storage, &devices, &device_events, options, &mut result)
                .await?;
            self.flush_channels(storage, &channel_records, options, &mut result)
                .await?;
        }

        Ok(result)
    }

    async fn flush_devices(
        &self,
        storage: &Arc<dyn Storage>,
        devices: &[Device],
        device_events: &[Event<DomainEvent>],
        options: &ImportOptions,
        result: &mut ImportResult,
    ) -> Result<(), MigrationError> {
        let mut event_offset: usize = 0;
        for chunk in devices.chunks(options.checkpoint_every) {
            let events_chunk = &device_events[event_offset..event_offset + chunk.len()];
            event_offset += chunk.len();

            let mut uow = storage.begin().await?;
            let mut written: usize = 0;
            for (device, event) in chunk.iter().zip(events_chunk.iter()) {
                let existing = uow
                    .device_repository()
                    .get_by_external_id(
                        device.tenant_id(),
                        device.protocol(),
                        device.external_id().clone(),
                    )
                    .await?;
                match existing {
                    Some(mut existing) if !options.skip_existing => {
                        let domain_event = match existing.update(
                            self.clock.as_ref(),
                            Some(device.name().to_string()),
                            Some(device.kind()),
                            None,
                            None,
                            Some(device.authority().to_string()),
                            Some(device.capabilities().to_vec()),
                            Some(device.metadata().clone()),
                        ) {
                            Ok(ev) => ev,
                            Err(
                                DomainError::ConcurrentModification { .. }
                                | DomainError::InvalidTransition { .. },
                            ) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        };
                        match uow.device_repository().save(&existing).await {
                            Ok(()) => {}
                            Err(DomainError::ConcurrentModification { .. }) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        }
                        uow.outbox()
                            .append(event_for(
                                self.clock.as_ref(),
                                existing.tenant_id(),
                                ResourceKind::Device,
                                ResourceId::Device(existing.device_id()),
                                domain_event,
                                existing.revision().0,
                            ))
                            .await?;
                        written += 1;
                    }
                    Some(_) => {
                        result.records_skipped_existing += 1;
                    }
                    None => {
                        match uow.device_repository().save(device).await {
                            Ok(()) => {}
                            Err(DomainError::ConcurrentModification { .. }) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        }
                        uow.outbox().append(event.clone()).await?;
                        written += 1;
                    }
                }
            }
            uow.commit().await?;
            result.records_imported += written;
        }
        Ok(())
    }

    async fn flush_channels(
        &self,
        storage: &Arc<dyn Storage>,
        channel_records: &[(OldRecord, Protocol)],
        options: &ImportOptions,
        result: &mut ImportResult,
    ) -> Result<(), MigrationError> {
        let mut resolver = ParentDeviceResolver::new();

        for chunk in channel_records.chunks(options.checkpoint_every) {
            let mut uow = storage.begin().await?;
            let mut written: usize = 0;

            for (record, parent_protocol) in chunk {
                let tenant_id = stable_tenant_id(&record.tenant_id);
                let parent_external_id = match ProtocolIdentity::new(&record.parent_device_id) {
                    Ok(id) => id,
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping channel with invalid parent id");
                        result.records_invalid += 1;
                        continue;
                    }
                };

                let parent_device_id = match resolver
                    .resolve(
                        uow.device_repository(),
                        tenant_id,
                        *parent_protocol,
                        &parent_external_id,
                    )
                    .await?
                {
                    Some(id) => id,
                    None => {
                        tracing::warn!(
                            tenant_id = %tenant_id,
                            parent_external_id = %parent_external_id,
                            "skipping channel with missing parent device",
                        );
                        result.records_invalid += 1;
                        continue;
                    }
                };

                let mapped = match map_channel(
                    self.clock.as_ref(),
                    record,
                    *parent_protocol,
                    parent_device_id,
                ) {
                    Ok(entity) => entity,
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping invalid channel");
                        result.records_invalid += 1;
                        continue;
                    }
                };

                if record.has_secret() {
                    result.records_with_secrets += 1;
                    result.action_items.extend(mapped.actions.clone());
                }

                let channel = match mapped.entity {
                    MappedAggregate::Channel(c) => c,
                    _ => continue,
                };
                let event = mapped
                    .events
                    .into_iter()
                    .next()
                    .ok_or_else(|| MigrationError::other("map_channel produced no events"))?;

                let existing = uow
                    .channel_repository()
                    .get(
                        channel.tenant_id(),
                        channel.device_id(),
                        channel.channel_id(),
                    )
                    .await?;
                match existing {
                    Some(mut existing) if !options.skip_existing => {
                        let domain_event = match existing.update(
                            self.clock.as_ref(),
                            Some(channel.kind()),
                            Some(channel.name().to_string()),
                            Some(channel.enabled()),
                            None,
                            Some(channel.stream_profiles().to_vec()),
                            Some(channel.ptz_capabilities().clone()),
                            Some(channel.metadata().clone()),
                        ) {
                            Ok(ev) => ev,
                            Err(
                                DomainError::ConcurrentModification { .. }
                                | DomainError::InvalidTransition { .. },
                            ) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        };
                        match uow.channel_repository().save(&existing).await {
                            Ok(()) => {}
                            Err(DomainError::ConcurrentModification { .. }) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        }
                        uow.outbox()
                            .append(event_for(
                                self.clock.as_ref(),
                                existing.tenant_id(),
                                ResourceKind::Channel,
                                ResourceId::Channel(existing.channel_id()),
                                domain_event,
                                existing.revision().0,
                            ))
                            .await?;
                        written += 1;
                    }
                    Some(_) => {
                        result.records_skipped_existing += 1;
                    }
                    None => {
                        match uow.channel_repository().save(&channel).await {
                            Ok(()) => {}
                            Err(DomainError::ConcurrentModification { .. }) => {
                                result.records_conflicting += 1;
                                continue;
                            }
                            Err(e) => return Err(e.into()),
                        }
                        uow.outbox().append(event).await?;
                        written += 1;
                    }
                }
            }
            uow.commit().await?;
            result.records_imported += written;
        }
        Ok(())
    }
}

/// Caches lookups from `(tenant, protocol, external_id)` to the actual
/// persisted device id so a parent device referenced by many channels is
/// only queried once per import run.
struct ParentDeviceResolver {
    cache: std::collections::HashMap<(TenantId, Protocol, String), Option<DeviceId>>,
}

impl ParentDeviceResolver {
    fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    async fn resolve(
        &mut self,
        repo: &mut dyn DeviceRepository,
        tenant_id: TenantId,
        protocol: Protocol,
        external_id: &ProtocolIdentity,
    ) -> Result<Option<DeviceId>, MigrationError> {
        let key = (tenant_id, protocol, external_id.as_str().to_string());
        if let Some(cached) = self.cache.get(&key) {
            return Ok(*cached);
        }

        let device = repo
            .get_by_external_id(tenant_id, protocol, external_id.clone())
            .await?;
        let id = device.map(|d| d.device_id());
        self.cache.insert(key, id);
        Ok(id)
    }
}

fn build_parent_protocols(records: &[OldRecord]) -> ParentProtocols {
    let mut map = ParentProtocols::new();
    for record in records {
        if matches!(
            record.entity_type,
            EntityType::Device | EntityType::Gb28181Platform | EntityType::OnvifEndpoint
        ) && let Ok(protocol) = parse_protocol(&record.protocol)
        {
            map.insert(
                (record.tenant_id.clone(), record.external_id.clone()),
                protocol,
            );
        }
    }
    map
}

#[cfg(test)]
mod tests;
