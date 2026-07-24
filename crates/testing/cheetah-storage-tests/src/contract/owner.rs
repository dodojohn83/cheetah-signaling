//! Device owner repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::OwnerInfo;
use cheetah_signal_types::{OwnerEpoch, PageRequest, TenantId};
use cheetah_storage_api::{OwnedDevice, Storage};

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let owner_node_id = fixtures.id_generator().generate_node_id();
    let owner = OwnerInfo {
        owner_node_id,
        owner_epoch: OwnerEpoch(1),
        lease_until: None,
    };

    let owner_repo = storage.owner_repository();
    owner_repo.set(tenant_id, device_id, owner.clone()).await?;

    let loaded = owner_repo.get(tenant_id, device_id).await?;
    assert_eq!(loaded, Some(owner.clone()));

    let other = owner_repo.get(TenantId::default(), device_id).await?;
    assert!(other.is_none(), "owner lease must be tenant-isolated");

    // Pagination: insert additional devices owned by the same node plus one
    // owned by a different node. The list_by_node scan should return exactly
    // the three devices owned by owner_node_id and produce a terminating cursor.
    let device_b = fixtures.device_id();
    let device_b_aggregate = fixtures.device(tenant_id, device_b)?;
    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_b_aggregate).await?;
    uow.commit().await?;
    owner_repo
        .set(
            tenant_id,
            device_b,
            OwnerInfo {
                owner_node_id,
                owner_epoch: OwnerEpoch(2),
                lease_until: None,
            },
        )
        .await?;

    let device_c = fixtures.device_id();
    let device_c_aggregate = fixtures.device(tenant_id, device_c)?;
    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_c_aggregate).await?;
    uow.commit().await?;
    owner_repo
        .set(
            tenant_id,
            device_c,
            OwnerInfo {
                owner_node_id,
                owner_epoch: OwnerEpoch(3),
                lease_until: None,
            },
        )
        .await?;

    let other_node = fixtures.id_generator().generate_node_id();
    let device_d = fixtures.device_id();
    let device_d_aggregate = fixtures.device(tenant_id, device_d)?;
    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_d_aggregate).await?;
    uow.commit().await?;
    owner_repo
        .set(
            tenant_id,
            device_d,
            OwnerInfo {
                owner_node_id: other_node,
                owner_epoch: OwnerEpoch(4),
                lease_until: None,
            },
        )
        .await?;

    let mut seen: Vec<OwnedDevice> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let page_request = match &cursor {
            None => PageRequest::new(2)?,
            Some(c) => {
                let mut req = PageRequest::new(2)?;
                req.cursor = Some(c.clone());
                req
            }
        };
        let page = owner_repo.list_by_node(owner_node_id, page_request).await?;
        seen.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    assert_eq!(
        seen.len(),
        3,
        "list_by_node should return exactly 3 owned devices"
    );
    assert!(
        seen.iter().all(|o| o.owner.owner_node_id == owner_node_id),
        "list_by_node must only return devices owned by the requested node"
    );
    assert!(
        !seen.iter().any(|o| o.device_id == device_d),
        "list_by_node must not return devices owned by another node"
    );

    owner_repo.clear(tenant_id, device_id).await?;
    let cleared = owner_repo.get(tenant_id, device_id).await?;
    assert!(cleared.is_none(), "cleared owner must not be found");

    Ok(())
}
