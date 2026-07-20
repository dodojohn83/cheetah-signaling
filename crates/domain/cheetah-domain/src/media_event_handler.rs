//! Port for applying media-node events to the control plane.

use crate::{DomainError, MediaNodeCallback, UnitOfWork};
use cheetah_signal_types::RequestContext;

/// Handles a media-node event callback without committing the unit of work.
///
/// The caller is responsible for inbox de-duplication, cursor persistence and
/// `UnitOfWork::commit` so that side effects and cursor advance remain atomic.
#[async_trait::async_trait]
pub trait MediaEventHandler: Send + Sync {
    /// Applies the callback to the domain aggregates in `uow`.
    async fn handle_media_event(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        callback: MediaNodeCallback,
    ) -> Result<(), DomainError>;
}
