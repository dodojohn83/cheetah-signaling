//! Shared repository contract tests.
//!
//! Each submodule is run against both the SQLite and PostgreSQL storage adapters
//! so the two backends share the same behavior contract.

use crate::fixtures::Fixtures;
use cheetah_storage_api::Storage;

pub(crate) mod channel;
pub(crate) mod device;
pub(crate) mod list;
pub(crate) mod media;
pub(crate) mod media_node;
pub(crate) mod node;
pub(crate) mod operation;
pub(crate) mod outbox;
pub(crate) mod outbox_retry;
pub(crate) mod owner;
pub(crate) mod ownership;
pub(crate) mod platform_link;
pub(crate) mod processed_message;
pub(crate) mod protocol_session;
pub(crate) mod step;
pub(crate) mod tenant;
pub(crate) mod transaction;
pub(crate) mod unicode;
pub(crate) mod webhook;

/// Result alias used by contract tests.
pub(crate) type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Runs the full contract suite against `storage`.
pub async fn run_all(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    storage.migration().run().await?;

    device::run(storage, fixtures).await?;
    channel::run(storage, fixtures).await?;
    operation::run(storage, fixtures).await?;
    media::run(storage, fixtures).await?;
    media_node::run(storage, fixtures).await?;
    protocol_session::run(storage, fixtures).await?;
    platform_link::run(storage, fixtures).await?;
    list::run(storage, fixtures).await?;
    outbox::run(storage, fixtures).await?;
    outbox_retry::run(storage, fixtures).await?;
    transaction::run(storage, fixtures).await?;
    processed_message::run(storage, fixtures).await?;
    owner::run(storage, fixtures).await?;
    ownership::run(storage, fixtures).await?;
    node::run(storage, fixtures).await?;
    tenant::run(storage, fixtures).await?;
    webhook::run(storage, fixtures).await?;
    step::run(storage, fixtures).await?;
    unicode::run(storage, fixtures).await?;

    Ok(())
}
