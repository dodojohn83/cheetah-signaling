//! Core migration orchestrator.

use crate::error::MigrationError;
use crate::mappers::{MappedAggregate, MappedEntity, map_record};
use crate::source::RecordSource;
use cheetah_domain::{Channel, Device};
use cheetah_signal_types::Clock;
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
    /// Number of records that conflicted with an existing aggregate.
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
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Importer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Importer").finish_non_exhaustive()
    }
}

impl Importer {
    /// Creates a new importer backed by the given storage and clock.
    pub fn new(storage: Arc<dyn Storage>, clock: Arc<dyn Clock>) -> Self {
        Self { storage, clock }
    }

    /// Runs an import from the provided source using the given options.
    pub async fn import(
        &self,
        source: &dyn RecordSource,
        options: &ImportOptions,
    ) -> Result<ImportResult, MigrationError> {
        let records = source.read_records().await?;
        let mut result = ImportResult {
            records_read: records.len(),
            ..ImportResult::default()
        };

        let mut devices: Vec<Device> = Vec::new();
        let mut channels: Vec<Channel> = Vec::new();
        let mut pending: Vec<MappedEntity> = Vec::new();

        for (row, record) in records.iter().enumerate() {
            if !options.cutover_ids.is_empty() && !options.cutover_ids.contains(&record.external_id)
            {
                result.records_skipped += 1;
                continue;
            }

            let entity = match map_record(self.clock.as_ref(), record) {
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
                    options,
                    &mut devices,
                    &mut channels,
                    &mut result,
                );
            }
        }

        if !pending.is_empty() {
            self.drain(
                pending.drain(..),
                options,
                &mut devices,
                &mut channels,
                &mut result,
            );
        }

        if !options.dry_run {
            self.flush_devices(&devices, options, &mut result).await?;
            self.flush_channels(&channels, options, &mut result).await?;
        } else {
            result.records_imported = devices.len() + channels.len();
        }

        Ok(result)
    }

    fn drain(
        &self,
        entities: impl Iterator<Item = MappedEntity>,
        options: &ImportOptions,
        devices: &mut Vec<Device>,
        channels: &mut Vec<Channel>,
        result: &mut ImportResult,
    ) {
        for entity in entities {
            if options.dry_run {
                result.records_imported += 1;
                continue;
            }
            match entity.entity {
                MappedAggregate::Device(d)
                | MappedAggregate::Gb28181Platform(d)
                | MappedAggregate::OnvifEndpoint(d) => devices.push(d),
                MappedAggregate::Channel(c) => channels.push(c),
                _ => {}
            }
        }
    }

    async fn flush_devices(
        &self,
        devices: &[Device],
        options: &ImportOptions,
        result: &mut ImportResult,
    ) -> Result<(), MigrationError> {
        for chunk in devices.chunks(options.checkpoint_every) {
            let mut uow = self.storage.begin().await?;
            let mut written: usize = 0;
            {
                let repo = uow.device_repository();
                for device in chunk {
                    let should_write = if options.skip_existing {
                        repo.get_by_external_id(
                            device.tenant_id(),
                            device.protocol(),
                            device.external_id().clone(),
                        )
                        .await?
                        .is_none()
                    } else {
                        true
                    };
                    if should_write {
                        repo.save(device).await?;
                        written += 1;
                    } else {
                        result.records_conflicting += 1;
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
        channels: &[Channel],
        options: &ImportOptions,
        result: &mut ImportResult,
    ) -> Result<(), MigrationError> {
        for chunk in channels.chunks(options.checkpoint_every) {
            let mut uow = self.storage.begin().await?;
            let mut written: usize = 0;
            {
                let repo = uow.channel_repository();
                for channel in chunk {
                    let should_write = if options.skip_existing {
                        repo.get(
                            channel.tenant_id(),
                            channel.device_id(),
                            channel.channel_id(),
                        )
                        .await?
                        .is_none()
                    } else {
                        true
                    };
                    if should_write {
                        repo.save(channel).await?;
                        written += 1;
                    } else {
                        result.records_conflicting += 1;
                    }
                }
            }
            uow.commit().await?;
            result.records_imported += written;
        }
        Ok(())
    }
}
