//! Device owner repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::OwnerInfo;
use cheetah_domain::in_memory::InMemoryClock;
use cheetah_signal_types::{Clock, OwnerEpoch, TenantId};
use cheetah_storage_api::{Storage, StorageError};
use std::sync::Arc;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let other_tenant_id = TenantId::generate();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let other_device_id = fixtures.device_id();
    let other_device = fixtures.device(other_tenant_id, other_device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.device_repository().save(&other_device).await?;
    uow.commit().await?;

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let owner_node_id = fixtures.id_generator().generate_node_id();
    let owner = OwnerInfo {
        owner_node_id,
        owner_epoch: OwnerEpoch(1),
        lease_until: None,
    };

    let mut owner_repo = storage.owner_repository(Arc::clone(&clock));
    owner_repo.set(tenant_id, device_id, owner.clone()).await?;

    let loaded = owner_repo.get(tenant_id, device_id).await?;
    assert_eq!(loaded, Some(owner.clone()));

    let other = owner_repo.get(TenantId::default(), device_id).await?;
    assert!(other.is_none(), "owner lease must be tenant-isolated");

    // Cross-tenant writes must be rejected and must not affect the real owner.
    let wrong_set = owner_repo
        .set(other_tenant_id, device_id, owner.clone())
        .await;
    assert!(
        matches!(wrong_set, Err(StorageError::InvalidArgument { .. })),
        "set from a different tenant must fail, got {:?}",
        wrong_set
    );
    let still_owned = owner_repo.get(tenant_id, device_id).await?;
    assert_eq!(
        still_owned,
        Some(owner.clone()),
        "owner must not change after cross-tenant set"
    );

    let other_node_id = fixtures.id_generator().generate_node_id();
    let lease_until = clock
        .now_wall()
        .checked_add(cheetah_signal_types::DurationMs::from_minutes(1))
        .ok_or("lease timestamp overflow")?;
    let wrong_acquire = owner_repo
        .acquire(other_tenant_id, device_id, other_node_id, lease_until)
        .await;
    assert!(
        matches!(wrong_acquire, Err(StorageError::Unavailable { .. })),
        "acquire from a different tenant must fail, got {:?}",
        wrong_acquire
    );
    let still_owned_after_acquire = owner_repo.get(tenant_id, device_id).await?;
    assert_eq!(
        still_owned_after_acquire,
        Some(owner.clone()),
        "owner must not change after cross-tenant acquire"
    );

    // Acquiring the correct tenant succeeds.
    let acquired = owner_repo
        .acquire(tenant_id, device_id, owner_node_id, lease_until)
        .await?;
    assert_eq!(acquired.owner_node_id, owner_node_id);

    owner_repo.clear(tenant_id, device_id).await?;
    let cleared = owner_repo.get(tenant_id, device_id).await?;
    assert!(cleared.is_none(), "cleared owner must not be found");

    Ok(())
}
