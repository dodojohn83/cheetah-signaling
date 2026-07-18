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

fn channel_record(external_id: &str, parent_device_id: &str, tenant_id: &str) -> OldRecord {
    OldRecord {
        entity_type: EntityType::Channel,
        tenant_id: tenant_id.to_string(),
        external_id: external_id.to_string(),
        name: "Channel".to_string(),
        protocol: "gb28181".to_string(),
        kind: String::new(),
        authority: String::new(),
        parent_device_id: parent_device_id.to_string(),
        channel_kind: "video".to_string(),
        enabled: true,
        metadata: BTreeMap::new(),
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

async fn sqlite_storage() -> Result<(SqliteStorage, tempfile::TempDir), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("test.db");
    Ok((SqliteStorage::new(&path).await?, dir))
}

#[tokio::test]
async fn dry_run_reports_imported_count() -> Result<(), Box<dyn std::error::Error>> {
    let (storage, _dir) = sqlite_storage().await?;
    storage.migration().run().await?;
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
async fn import_skip_existing_counts_skipped_existing() -> Result<(), Box<dyn std::error::Error>> {
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

#[tokio::test]
async fn import_channel_uses_existing_parent_device_id() -> Result<(), Box<dyn std::error::Error>> {
    let (storage, _dir) = sqlite_storage().await?;
    storage.migration().run().await?;
    let storage: Arc<dyn Storage> = Arc::new(storage);
    let clock = Arc::new(SystemClock::new());

    // Seed a runtime-created parent device with a different internal id.
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

    // Import a channel whose parent external id matches the runtime device.
    let channel = channel_record("ch-01", "runtime-cam", "tenant-a");
    let importer = Importer::new(Some(storage.clone()), clock.clone());
    let result = importer
        .import(
            &VecSource(vec![channel]),
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
    let channels = uow
        .channel_repository()
        .list_by_device(tenant_id, runtime_device_id)
        .await?;
    uow.commit().await?;
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].device_id(), runtime_device_id);
    Ok(())
}

#[tokio::test]
async fn dry_run_counts_missing_channel_parent_as_invalid() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(SystemClock::new());
    let importer = Importer::new(None, clock);
    let source = VecSource(vec![channel_record("ch-01", "missing-cam", "tenant-a")]);
    let result = importer
        .import(
            &source,
            &ImportOptions {
                dry_run: true,
                checkpoint_every: 10,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(result.records_read, 1);
    assert_eq!(result.records_imported, 0);
    assert_eq!(result.records_invalid, 1);
    Ok(())
}

#[tokio::test]
async fn dry_run_uses_local_parent_device_in_same_batch() -> Result<(), Box<dyn std::error::Error>>
{
    let clock = Arc::new(SystemClock::new());
    let importer = Importer::new(None, clock);
    let source = VecSource(vec![
        device_record("cam-01", "tenant-a"),
        channel_record("ch-01", "cam-01", "tenant-a"),
    ]);
    let result = importer
        .import(
            &source,
            &ImportOptions {
                dry_run: true,
                checkpoint_every: 10,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(result.records_read, 2);
    assert_eq!(result.records_imported, 2);
    assert_eq!(result.records_invalid, 0);
    Ok(())
}

#[tokio::test]
async fn dry_run_queries_database_when_storage_available() -> Result<(), Box<dyn std::error::Error>>
{
    let (storage, _dir) = sqlite_storage().await?;
    storage.migration().run().await?;
    let storage: Arc<dyn Storage> = Arc::new(storage);
    let clock = Arc::new(SystemClock::new());

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

    let importer = Importer::new(Some(storage), clock.clone());
    let source = VecSource(vec![channel_record("ch-01", "runtime-cam", "tenant-a")]);
    let result = importer
        .import(
            &source,
            &ImportOptions {
                dry_run: true,
                checkpoint_every: 10,
                ..Default::default()
            },
        )
        .await?;
    assert_eq!(result.records_read, 1);
    assert_eq!(result.records_imported, 1);
    assert_eq!(result.records_invalid, 0);
    Ok(())
}
