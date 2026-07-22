//! Periodic media reconciliation worker.
//!
//! Walks all non-deleted tenants at a configured interval and invokes
//! [`MediaService::reconcile`] for each. This provides a recovery path for media
//! state changes that may have been missed by event-driven gap reconciliation,
//! and bounds the maximum time an inconsistent binding/session can persist.

use cheetah_signal_application::MediaService;
use cheetah_signal_types::{
    CorrelationId, MessageId, NodeId, PageRequest, Principal, PrincipalKind, RequestContext,
};
use cheetah_storage_api::Storage;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Spawns a background task that periodically reconciles media sessions for all
/// tenants until the cancellation token fires.
pub fn spawn(
    media_service: MediaService,
    storage: Arc<dyn Storage>,
    node_id: NodeId,
    cancel: CancellationToken,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    if let Err(e) = reconcile_all_tenants(&media_service, storage.as_ref(), node_id).await {
                        warn!(error = %e, "periodic media reconciliation pass failed");
                    }
                }
            }
        }
        info!("periodic media reconciliation worker stopped");
    })
}

async fn reconcile_all_tenants(
    media_service: &MediaService,
    storage: &dyn Storage,
    node_id: NodeId,
) -> Result<(), String> {
    let mut cursor: Option<String> = None;
    loop {
        let page_request = match cursor {
            None => PageRequest::new(1000).map_err(|e| e.to_string())?,
            Some(c) => PageRequest::new(1000)
                .map_err(|e| e.to_string())?
                .with_cursor(c),
        };

        let repo = storage.tenant_repository();
        let page = repo
            .list(None, page_request)
            .await
            .map_err(|e| e.to_string())?;

        for tenant in page.items {
            let tenant_id = tenant.tenant_id;
            let context = RequestContext {
                tenant_id,
                principal: Principal {
                    id: "periodic-media-reconciler".to_string(),
                    kind: PrincipalKind::Service,
                    scopes: vec!["media:reconcile".to_string()],
                },
                message_id: MessageId::generate(),
                correlation_id: CorrelationId::generate(),
                traceparent: None,
                tracestate: None,
                deadline: None,
                node_id: Some(node_id),
                source_ip: None,
            };

            let mut uow = storage
                .begin()
                .await
                .map_err(|e| format!("failed to begin unit of work for tenant {tenant_id}: {e}"))?;

            if let Err(e) = media_service.reconcile(&context, uow.as_mut()).await {
                warn!(tenant_id = %tenant_id, error = %e, "periodic reconcile failed for tenant");
            }
            // `MediaService::reconcile` commits internally; the unit of work is
            // dropped on the next iteration.
        }

        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    Ok(())
}
