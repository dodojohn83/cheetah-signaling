//! Core migration orchestrator.

use crate::error::MigrationError;
use crate::mappers::{
    MappedAggregate, MappedEntity, ParentProtocols, event_for, map_record, parse_protocol,
};
use crate::model::{EntityType, OldRecord};
use crate::source::RecordSource;
use cheetah_domain::{Channel, Device, DomainEvent};
use cheetah_signal_types::{Clock, Event, ResourceId, ResourceKind};
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
        let mut channels: Vec<Channel> = Vec::new();
        let mut channel_events: Vec<Event<DomainEvent>> = Vec::new();
        let mut pending: Vec<MappedEntity> = Vec::new();

        let parent_protocols = build_parent_protocols(&records);

        for (row, record) in records.iter().enumerate() {
            if !options.cutover_ids.is_empty() && !options.cutover_ids.contains(&record.external_id)
            {
                result.records_skipped += 1;
                continue;
            }

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

            *result
                .counts_by_kind
                .entry(format!("{:?}", record.entity_type))
                .or_insert(0) += 1;

            pending.push(entity);

            if pending.len() >= options.checkpoint_every {
                self.drain(
                    pending.drain(..),
                    &mut devices,
                    &mut device_events,
                    &mut channels,
                    &mut channel_events,
                );
            }
        }

        if !pending.is_empty() {
            self.drain(
                pending.drain(..),
                &mut devices,
                &mut device_events,
                &mut channels,
                &mut channel_events,
            );
        }

        if options.dry_run {
            result.records_imported = devices.len() + channels.len();
        } else {
            let storage = self.storage.as_ref().ok_or_else(|| {
                MigrationError::other("storage is required for non-dry-run import")
            })?;
            self.flush_devices(storage, &devices, &device_events, options, &mut result)
                .await?;
            self.flush_channels(storage, &channels, &channel_events, options, &mut result)
                .await?;
        }

        Ok(result)
    }

    fn drain(
        &self,
        entities: impl Iterator<Item = MappedEntity>,
        devices: &mut Vec<Device>,
        device_events: &mut Vec<Event<DomainEvent>>,
        channels: &mut Vec<Channel>,
        channel_events: &mut Vec<Event<DomainEvent>>,
    ) {
        for entity in entities {
            let events = entity.events;
            match entity.entity {
                MappedAggregate::Device(d)
                | MappedAggregate::Gb28181Platform(d)
                | MappedAggregate::OnvifEndpoint(d) => {
                    devices.push(d);
                    device_events.extend(events);
                }
                MappedAggregate::Channel(c) => {
                    channels.push(c);
                    channel_events.extend(events);
                }
                _ => {}
            }
        }
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
                        let domain_event = existing.update(
                            self.clock.as_ref(),
                            Some(device.name().to_string()),
                            Some(device.kind()),
                            None,
                            None,
                            Some(device.authority().to_string()),
                            Some(device.capabilities().to_vec()),
                            Some(device.metadata().clone()),
                        )?;
                        uow.device_repository().save(&existing).await?;
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
                        uow.device_repository().save(device).await?;
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
        channels: &[Channel],
        channel_events: &[Event<DomainEvent>],
        options: &ImportOptions,
        result: &mut ImportResult,
    ) -> Result<(), MigrationError> {
        let mut event_offset: usize = 0;
        for chunk in channels.chunks(options.checkpoint_every) {
            let events_chunk = &channel_events[event_offset..event_offset + chunk.len()];
            event_offset += chunk.len();

            let mut uow = storage.begin().await?;
            let mut written: usize = 0;
            for (channel, event) in chunk.iter().zip(events_chunk.iter()) {
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
                        let domain_event = existing.update(
                            self.clock.as_ref(),
                            Some(channel.kind()),
                            Some(channel.name().to_string()),
                            Some(channel.enabled()),
                            None,
                            Some(channel.stream_profiles().to_vec()),
                            Some(channel.ptz_capabilities().clone()),
                            Some(channel.metadata().clone()),
                        )?;
                        uow.channel_repository().save(&existing).await?;
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
                        uow.channel_repository().save(channel).await?;
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
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::mappers::{event_for, stable_tenant_id};
    use crate::model::{EntityType, OldRecord};
    use cheetah_domain::{DeviceKind, Protocol};
    use cheetah_signal_types::{DeviceId, ProtocolIdentity, ResourceId, ResourceKind};
    use cheetah_storage_sqlite::SqliteStorage;
    use std::collections::BTreeMap;

    fn device_record(external_id: &str, tenant_id: &str) -> OldRecord {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "location".to_string(),
            serde_json::Value::String("hallway".to_string()),
        );
        OldRecord {
            entity_type: EntityType::Device,
            tenant_id: tenant_id.to_string(),
            external_id: external_id.to_string(),
            name: "Cam".to_string(),
            protocol: "gb28181".to_string(),
            kind: "camera".to_string(),
            authority: "192.0.2.1:5060".to_string(),
            parent_device_id: external_id.to_string(),
            channel_kind: String::new(),
            enabled: true,
            metadata,
            secret_fields: String::new(),
        }
    }

    struct VecSource(Vec<OldRecord>);

    #[async_trait::async_trait]
    impl RecordSource for VecSource {
        async fn read_records(&self) -> Result<Vec<OldRecord>, MigrationError> {
            Ok(self.0.clone())
        }
    }

    async fn sqlite_storage()
    -> Result<(SqliteStorage, tempfile::TempDir), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.db");
        Ok((SqliteStorage::new(&path).await?, dir))
    }

    #[tokio::test]
    async fn dry_run_reports_imported_count() -> Result<(), Box<dyn std::error::Error>> {
        let (storage, _dir) = sqlite_storage().await?;
        let storage: Option<Arc<dyn Storage>> = Some(Arc::new(storage));
        let clock = Arc::new(SystemClock::new());
        let importer = Importer::new(storage, clock);
        let source = VecSource(vec![device_record("cam-01", "tenant-a")]);
        let options = ImportOptions {
            dry_run: true,
            checkpoint_every: 10,
            ..Default::default()
        };
        let result = importer.import(&source, &options).await?;
        assert_eq!(result.records_read, 1);
        assert_eq!(result.records_imported, 1);
        Ok(())
    }

    #[tokio::test]
    async fn checkpoint_zero_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (storage, _dir) = sqlite_storage().await?;
        let storage: Option<Arc<dyn Storage>> = Some(Arc::new(storage));
        let clock = Arc::new(SystemClock::new());
        let importer = Importer::new(storage, clock);
        let source = VecSource(Vec::new());
        let options = ImportOptions {
            checkpoint_every: 0,
            ..Default::default()
        };
        let outcome = importer.import(&source, &options).await;
        assert!(outcome.is_err());
        match outcome {
            Err(MigrationError::Other(_)) => {}
            _ => panic!("expected MigrationError::Other"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn import_appends_domain_events_to_outbox() -> Result<(), Box<dyn std::error::Error>> {
        let (storage, _dir) = sqlite_storage().await?;
        storage.migration().run().await?;
        let storage: Arc<dyn Storage> = Arc::new(storage);
        let clock = Arc::new(SystemClock::new());
        let importer = Importer::new(Some(storage.clone()), clock.clone());
        let source = VecSource(vec![device_record("cam-01", "tenant-a")]);
        let options = ImportOptions {
            dry_run: false,
            checkpoint_every: 10,
            ..Default::default()
        };
        let result = importer.import(&source, &options).await?;
        assert_eq!(result.records_read, 1);
        assert_eq!(result.records_imported, 1);

        let mut uow = storage.begin().await?;
        let events = uow.outbox().pending(clock.now_wall(), 100).await?;
        uow.commit().await?;
        assert_eq!(events.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn import_overwrites_existing_when_skip_existing_false()
    -> Result<(), Box<dyn std::error::Error>> {
        let (storage, _dir) = sqlite_storage().await?;
        storage.migration().run().await?;
        let storage: Arc<dyn Storage> = Arc::new(storage);
        let clock = Arc::new(SystemClock::new());
        let importer = Importer::new(Some(storage.clone()), clock.clone());
        let mut record = device_record("cam-01", "tenant-a");
        record.name = "first".to_string();
        let source = VecSource(vec![record.clone()]);
        let first = importer
            .import(
                &source,
                &ImportOptions {
                    dry_run: false,
                    checkpoint_every: 10,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(first.records_imported, 1);

        record.name = "second".to_string();
        let source = VecSource(vec![record]);
        let second = importer
            .import(
                &source,
                &ImportOptions {
                    dry_run: false,
                    skip_existing: false,
                    checkpoint_every: 10,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(second.records_imported, 1);
        assert_eq!(second.records_conflicting, 0);

        let mut uow = storage.begin().await?;
        let events = uow.outbox().pending(clock.now_wall(), 100).await?;
        uow.commit().await?;
        assert_eq!(events.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn import_skip_existing_counts_skipped_existing() -> Result<(), Box<dyn std::error::Error>>
    {
        let (storage, _dir) = sqlite_storage().await?;
        storage.migration().run().await?;
        let storage: Arc<dyn Storage> = Arc::new(storage);
        let clock = Arc::new(SystemClock::new());
        let importer = Importer::new(Some(storage.clone()), clock.clone());
        let record = device_record("cam-01", "tenant-a");
        let source = VecSource(vec![record.clone()]);
        let first = importer
            .import(
                &source,
                &ImportOptions {
                    dry_run: false,
                    checkpoint_every: 10,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(first.records_imported, 1);

        let source = VecSource(vec![record]);
        let second = importer
            .import(
                &source,
                &ImportOptions {
                    dry_run: false,
                    checkpoint_every: 10,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(second.records_imported, 0);
        assert_eq!(second.records_skipped_existing, 1);
        assert_eq!(second.records_conflicting, 0);
        Ok(())
    }

    #[tokio::test]
    async fn import_overwrite_uses_existing_device_id() -> Result<(), Box<dyn std::error::Error>> {
        let (storage, _dir) = sqlite_storage().await?;
        storage.migration().run().await?;
        let storage: Arc<dyn Storage> = Arc::new(storage);
        let clock = Arc::new(SystemClock::new());

        // Seed a runtime-created device with the same external identity but a
        // different internal device_id (UUIDv7). Use the stable tenant id the
        // migration tool will derive from the string "tenant-a".
        let tenant_id = stable_tenant_id("tenant-a");
        let external_id = ProtocolIdentity::new("runtime-cam")?;
        let runtime_device_id = DeviceId::generate();
        let (runtime_device, registered) = cheetah_domain::Device::new(
            clock.as_ref(),
            tenant_id,
            runtime_device_id,
            Protocol::Gb28181,
            external_id,
            "192.0.2.1:5060",
            "Runtime",
            DeviceKind::Camera,
            Vec::new(),
            BTreeMap::new(),
        )?;

        let mut uow = storage.begin().await?;
        uow.device_repository().save(&runtime_device).await?;
        uow.outbox()
            .append(event_for(
                clock.as_ref(),
                tenant_id,
                ResourceKind::Device,
                ResourceId::Device(runtime_device_id),
                registered,
                0,
            ))
            .await?;
        uow.commit().await?;

        // Import a record with the same external identity.
        let mut record = device_record("runtime-cam", "tenant-a");
        record.name = "Imported".to_string();
        let importer = Importer::new(Some(storage.clone()), clock.clone());
        let result = importer
            .import(
                &VecSource(vec![record]),
                &ImportOptions {
                    dry_run: false,
                    skip_existing: false,
                    checkpoint_every: 10,
                    ..Default::default()
                },
            )
            .await?;
        assert_eq!(result.records_imported, 1);

        let mut uow = storage.begin().await?;
        let events = uow.outbox().pending(clock.now_wall(), 100).await?;
        uow.commit().await?;

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1].event.aggregate_ref.id,
            ResourceId::Device(runtime_device_id),
            "DeviceUpdated must reference the existing runtime device id"
        );
        Ok(())
    }
}
