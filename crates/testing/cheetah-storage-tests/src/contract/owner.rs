//! Device owner repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::OwnerInfo;
use cheetah_domain::in_memory::InMemoryClock;
use cheetah_signal_types::{Clock, OwnerEpoch, TenantId};
use cheetah_storage_api::Storage;
use std::sync::Arc;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let owner_node_id = fixtures.id_generator().generate_node_id();
    let owner = OwnerInfo {
        owner_node_id,
        owner_epoch: OwnerEpoch(1),
        lease_until: None,
    };

    let mut owner_repo = storage.owner_repository(clock);
    owner_repo.set(tenant_id, device_id, owner.clone()).await?;

    let loaded = owner_repo.get(tenant_id, device_id).await?;
    assert_eq!(loaded, Some(owner.clone()));

    let other = owner_repo.get(TenantId::default(), device_id).await?;
    assert!(other.is_none(), "owner lease must be tenant-isolated");

    owner_repo.clear(tenant_id, device_id).await?;
    let cleared = owner_repo.get(tenant_id, device_id).await?;
    assert!(cleared.is_none(), "cleared owner must not be found");

    Ok(())
}
