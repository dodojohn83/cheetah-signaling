//! Tenant repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::Tenant;
use cheetah_signal_types::PageRequest;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let now = fixtures.clock().now_wall();
    let mut repo = storage.tenant_repository();

    let a = Tenant::new(fixtures.tenant_id(), "alice", now)?;
    let pct = Tenant::new(fixtures.tenant_id(), "a%bob", now)?;
    let under = Tenant::new(fixtures.tenant_id(), "a_bob", now)?;

    repo.save(&a).await?;
    repo.save(&pct).await?;
    repo.save(&under).await?;

    let pct_page = repo.list(Some("a%"), PageRequest::new(10)?).await?;
    assert_eq!(
        pct_page.items.len(),
        1,
        "% in prefix must be escaped literally"
    );
    assert_eq!(pct_page.items[0].name, "a%bob");

    let under_page = repo.list(Some("a_"), PageRequest::new(10)?).await?;
    assert_eq!(
        under_page.items.len(),
        1,
        "_ in prefix must be escaped literally"
    );
    assert_eq!(under_page.items[0].name, "a_bob");

    let all = repo.list(Some("a"), PageRequest::new(10)?).await?;
    assert_eq!(
        all.items.len(),
        3,
        "plain prefix must match all three tenants"
    );

    Ok(())
}
