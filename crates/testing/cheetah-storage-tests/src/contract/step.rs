//! Operation step repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_signal_types::OwnerEpoch;
use cheetah_storage_api::{OperationStep, Storage};

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let operation_id = fixtures.id_generator().generate_operation_id();

    let step_one = OperationStep::new(
        tenant_id,
        operation_id,
        1,
        OwnerEpoch(1).0,
        "dispatched",
        None,
    );
    let step_two = OperationStep::new(
        tenant_id,
        operation_id,
        2,
        OwnerEpoch(1).0,
        "acknowledged",
        None,
    );

    let mut step_repo = storage.operation_step_repository();
    step_repo.record(step_one.clone()).await?;
    step_repo.record(step_two.clone()).await?;

    let steps = step_repo.list(tenant_id, operation_id).await?;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].attempt, 1);
    assert_eq!(steps[1].attempt, 2);

    Ok(())
}
