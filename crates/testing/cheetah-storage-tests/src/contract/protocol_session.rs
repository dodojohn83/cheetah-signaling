//! Protocol session repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{DomainError, Protocol};
use cheetah_signal_types::{DurationMs, PageRequest, Revision, TenantId};
use cheetah_storage_api::Storage;

const ONE_HOUR: DurationMs = DurationMs::from_millis(3_600_000);

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    revision_concurrency(storage, fixtures).await?;
    tenant_isolation(storage, fixtures).await?;
    delete_semantics(storage, fixtures).await?;
    expiration(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let session = fixtures.protocol_session(tenant_id, device_id, ONE_HOUR)?;
    let session_id = session.protocol_session_id();

    let mut repo = storage.protocol_session_repository();
    repo.save(&session).await?;

    let loaded = repo
        .get(tenant_id, session_id)
        .await?
        .ok_or("session not found after save")?;
    assert_eq!(loaded.device_id(), device_id);
    assert_eq!(
        loaded.protocol_identity().as_str(),
        session.protocol_identity().as_str()
    );
    assert_eq!(loaded.registration().call_id, "call-id-0001");

    let by_device = repo
        .get_by_device(tenant_id, Protocol::Gb28181, device_id)
        .await?
        .ok_or("session not found by device")?;
    assert_eq!(by_device.protocol_session_id(), session_id);

    let by_identity = repo
        .get_by_identity(
            tenant_id,
            Protocol::Gb28181,
            session.protocol_identity().clone(),
        )
        .await?
        .ok_or("session not found by identity")?;
    assert_eq!(by_identity.protocol_session_id(), session_id);

    let absent = repo.get(TenantId::default(), session_id).await?;
    assert!(absent.is_none(), "wrong tenant must not see the session");

    Ok(())
}

async fn revision_concurrency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut session = fixtures.protocol_session(tenant_id, device_id, ONE_HOUR)?;
    let session_id = session.protocol_session_id();

    let mut repo = storage.protocol_session_repository();
    repo.save(&session).await?;

    session.record_keepalive(fixtures.clock());
    assert_eq!(session.revision(), Revision(1));
    repo.save(&session).await?;

    let stale = session.clone();
    let result = repo.save(&stale).await;
    assert!(
        matches!(
            result,
            Err(DomainError::ConcurrentModification {
                expected: 0,
                found: 1,
            })
        ),
        "saving a stale revision must report expected=0 and found=1, got {result:?}"
    );

    let loaded = repo
        .get(tenant_id, session_id)
        .await?
        .ok_or("session missing after concurrent save")?;
    assert_eq!(loaded.revision(), Revision(1));

    Ok(())
}

async fn tenant_isolation(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_a = fixtures.tenant_id();
    let tenant_b = fixtures.tenant_id();
    let device_a = fixtures.device_id();
    let device_b = fixtures.device_id();

    let session_a = fixtures.protocol_session(tenant_a, device_a, ONE_HOUR)?;
    let session_b = fixtures.protocol_session(tenant_b, device_b, ONE_HOUR)?;

    let mut repo = storage.protocol_session_repository();
    repo.save(&session_a).await?;
    repo.save(&session_b).await?;

    let a = repo.get(tenant_a, session_a.protocol_session_id()).await?;
    let b = repo.get(tenant_b, session_b.protocol_session_id()).await?;
    let a_cross = repo.get(tenant_a, session_b.protocol_session_id()).await?;
    let b_cross = repo.get(tenant_b, session_a.protocol_session_id()).await?;

    assert_eq!(a.ok_or("tenant_a session missing")?.tenant_id(), tenant_a);
    assert_eq!(b.ok_or("tenant_b session missing")?.tenant_id(), tenant_b);
    assert!(
        a_cross.is_none(),
        "tenant_a must not see tenant_b's session"
    );
    assert!(
        b_cross.is_none(),
        "tenant_b must not see tenant_a's session"
    );

    let cross_by_device = repo
        .get_by_device(tenant_a, Protocol::Gb28181, device_b)
        .await?;
    assert!(
        cross_by_device.is_none(),
        "get_by_device must be tenant scoped"
    );

    Ok(())
}

async fn delete_semantics(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let session = fixtures.protocol_session(tenant_id, device_id, ONE_HOUR)?;
    let session_id = session.protocol_session_id();

    let mut repo = storage.protocol_session_repository();
    repo.save(&session).await?;

    let wrong = repo.delete(tenant_id, session_id, Revision(5)).await;
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

    repo.delete(tenant_id, session_id, session.revision())
        .await?;
    assert!(
        repo.get(tenant_id, session_id).await?.is_none(),
        "session must be gone after delete"
    );

    let missing = repo.delete(tenant_id, session_id, Revision(0)).await;
    assert!(
        matches!(missing, Err(DomainError::NotFound { .. })),
        "deleting a missing session must be NotFound, got {missing:?}"
    );

    Ok(())
}

async fn expiration(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let expiring = fixtures.protocol_session(
        tenant_id,
        fixtures.device_id(),
        DurationMs::from_millis(1_000),
    )?;
    let fresh = fixtures.protocol_session(tenant_id, fixtures.device_id(), ONE_HOUR)?;
    let expiring_id = expiring.protocol_session_id();
    let fresh_id = fresh.protocol_session_id();

    let mut repo = storage.protocol_session_repository();
    repo.save(&expiring).await?;
    repo.save(&fresh).await?;

    fixtures.advance(DurationMs::from_millis(2_000));
    let now = fixtures.clock().now_wall();

    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let mut page = PageRequest::new(1)?;
        page.cursor = cursor;
        let result = repo.list_expired(now, page).await?;
        for session in &result.items {
            assert!(
                session.is_expired(now),
                "list_expired must only return expired sessions"
            );
            ids.push(session.protocol_session_id());
        }
        match result.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    assert!(
        ids.contains(&expiring_id),
        "expired session must appear in the sweep"
    );
    assert!(
        !ids.contains(&fresh_id),
        "unexpired session must not appear in the sweep"
    );

    Ok(())
}
