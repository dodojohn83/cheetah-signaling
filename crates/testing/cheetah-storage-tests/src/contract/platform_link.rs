//! Cascade platform link repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{DomainError, PlatformDirection};
use cheetah_signal_types::{PageRequest, Revision, TenantId};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    revision_concurrency(storage, fixtures).await?;
    tenant_isolation(storage, fixtures).await?;
    delete_semantics(storage, fixtures).await?;
    listing(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let link = fixtures.platform_link(tenant_id, PlatformDirection::Upstream, 1)?;
    let link_id = link.platform_link_id();
    let remote = link.identity().remote.clone();

    let mut repo = storage.platform_link_repository();
    repo.save(&link).await?;

    let loaded = repo
        .get(tenant_id, link_id)
        .await?
        .ok_or("link not found after save")?;
    assert_eq!(loaded.direction(), PlatformDirection::Upstream);
    assert_eq!(loaded.identity().remote.as_str(), remote.as_str());
    assert!(loaded.credential().credential_ref.starts_with("secret://"));

    let by_remote = repo
        .get_by_remote_identity(tenant_id, PlatformDirection::Upstream, remote.clone())
        .await?
        .ok_or("link not found by remote identity")?;
    assert_eq!(by_remote.platform_link_id(), link_id);

    let wrong_direction = repo
        .get_by_remote_identity(tenant_id, PlatformDirection::Downstream, remote)
        .await?;
    assert!(
        wrong_direction.is_none(),
        "remote identity lookup must be direction scoped"
    );

    let absent = repo.get(TenantId::default(), link_id).await?;
    assert!(absent.is_none(), "wrong tenant must not see the link");

    Ok(())
}

async fn revision_concurrency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let mut link = fixtures.platform_link(tenant_id, PlatformDirection::Upstream, 2)?;
    let link_id = link.platform_link_id();

    let mut repo = storage.platform_link_repository();
    repo.save(&link).await?;

    link.record_keepalive(fixtures.clock());
    assert_eq!(link.revision(), Revision(1));
    repo.save(&link).await?;

    let stale = link.clone();
    let result = repo.save(&stale).await;
    assert!(
        matches!(
            result,
            Err(DomainError::ConcurrentModification {
                expected: 0,
                found: 1,
            })
        ),
        "saving a stale revision must conflict, got {result:?}"
    );

    let loaded = repo
        .get(tenant_id, link_id)
        .await?
        .ok_or("link missing after concurrent save")?;
    assert_eq!(loaded.revision(), Revision(1));

    Ok(())
}

async fn tenant_isolation(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_a = fixtures.tenant_id();
    let tenant_b = fixtures.tenant_id();

    let link_a = fixtures.platform_link(tenant_a, PlatformDirection::Upstream, 3)?;
    let link_b = fixtures.platform_link(tenant_b, PlatformDirection::Upstream, 4)?;

    let mut repo = storage.platform_link_repository();
    repo.save(&link_a).await?;
    repo.save(&link_b).await?;

    let a_cross = repo.get(tenant_a, link_b.platform_link_id()).await?;
    let b_cross = repo.get(tenant_b, link_a.platform_link_id()).await?;
    assert!(a_cross.is_none(), "tenant_a must not see tenant_b's link");
    assert!(b_cross.is_none(), "tenant_b must not see tenant_a's link");

    Ok(())
}

async fn delete_semantics(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let link = fixtures.platform_link(tenant_id, PlatformDirection::Upstream, 5)?;
    let link_id = link.platform_link_id();

    let mut repo = storage.platform_link_repository();
    repo.save(&link).await?;

    let wrong = repo.delete(tenant_id, link_id, Revision(5)).await;
    assert!(
        matches!(
            wrong,
            Err(DomainError::ConcurrentModification {
                expected: 5,
                found: 0,
            })
        ),
        "deleting with a stale revision must conflict, got {wrong:?}"
    );

    repo.delete(tenant_id, link_id, link.revision()).await?;
    assert!(
        repo.get(tenant_id, link_id).await?.is_none(),
        "link must be gone after delete"
    );

    let missing = repo.delete(tenant_id, link_id, Revision(0)).await;
    assert!(
        matches!(missing, Err(DomainError::NotFound { .. })),
        "deleting a missing link must be NotFound, got {missing:?}"
    );

    Ok(())
}

async fn listing(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let up1 = fixtures.platform_link(tenant_id, PlatformDirection::Upstream, 6)?;
    let up2 = fixtures.platform_link(tenant_id, PlatformDirection::Upstream, 7)?;
    let down = fixtures.platform_link(tenant_id, PlatformDirection::Downstream, 8)?;

    let mut repo = storage.platform_link_repository();
    repo.save(&up1).await?;
    repo.save(&up2).await?;
    repo.save(&down).await?;

    // Paginate the upstream links one at a time via a stable cursor.
    let mut upstream_ids = Vec::new();
    let mut cursor = None;
    loop {
        let mut page = PageRequest::new(1)?;
        page.cursor = cursor;
        let result = repo
            .list(tenant_id, Some(PlatformDirection::Upstream), page)
            .await?;
        for link in &result.items {
            assert_eq!(link.direction(), PlatformDirection::Upstream);
            upstream_ids.push(link.platform_link_id());
        }
        match result.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(upstream_ids.contains(&up1.platform_link_id()));
    assert!(upstream_ids.contains(&up2.platform_link_id()));
    assert!(!upstream_ids.contains(&down.platform_link_id()));

    let all = repo.list(tenant_id, None, PageRequest::new(50)?).await?;
    assert_eq!(all.items.len(), 3, "unfiltered list must return every link");

    Ok(())
}
