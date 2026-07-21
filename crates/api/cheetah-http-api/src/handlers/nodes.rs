//! Node and media node HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, ListQuery};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use cheetah_domain::ClusterNode;
use cheetah_signal_types::{NodeId, Page};
use std::sync::Arc;

pub async fn list_nodes(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<ClusterNode>>, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let now = state.clock.now_wall();
    let result = state
        .storage
        .node_repository()
        .list_alive(now, page)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result))
}

pub async fn list_media_nodes(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<serde_json::Value>>, HttpError> {
    ctx.require_scope("viewer")?;
    let page = query.page_request()?;
    let now = state.clock.now_wall();

    // Prefer the persistent registry (cursor-stable) when available; fall back
    // to the media-port scheduler view for environments without a registry table.
    let registry_page = state
        .storage
        .media_node_repository()
        .list_alive(now, page)
        .await
        .map_err(HttpError::from)?;

    if !registry_page.items.is_empty() || registry_page.next_cursor.is_none() {
        let items: Result<Vec<_>, _> = registry_page
            .items
            .into_iter()
            .map(serde_json::to_value)
            .collect();
        let items = items.map_err(HttpError::from)?;
        return Ok(Json(Page {
            items,
            next_cursor: registry_page.next_cursor,
            total: registry_page.total,
        }));
    }

    // Empty registry with no next_cursor: also consult the live media port.
    let nodes = state
        .media_service
        .list_media_nodes(&ctx.0)
        .await
        .map_err(HttpError::from)?;
    let total = nodes.len() as u64;
    let items: Result<Vec<_>, _> = nodes.into_iter().map(serde_json::to_value).collect();
    let items = items.map_err(HttpError::from)?;
    Ok(Json(Page {
        items,
        next_cursor: None,
        total: Some(total),
    }))
}

/// Drains a media node so its active sessions are migrated away.
pub async fn drain_media_node(
    Path(node_id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<StatusCode, HttpError> {
    ctx.require_scope("system_admin")?;
    let node_id = node_id.parse::<NodeId>().map_err(|e| {
        HttpError::Signal(cheetah_signal_types::SignalError::new(
            cheetah_signal_types::SignalErrorKind::InvalidArgument,
            format!("invalid node_id: {e}"),
        ))
    })?;

    state
        .media_service
        .drain_media_node(&ctx.0, node_id)
        .await
        .map_err(HttpError::from)?;

    state.audit.record(cheetah_signal_types::AuditEvent {
        timestamp: state.clock.now_wall(),
        action: "media_node.drain".to_string(),
        actor: ctx.0.principal.id.clone(),
        tenant_id: Some(ctx.0.tenant_id),
        target_type: "media_node".to_string(),
        target_id: Some(node_id.to_string()),
        outcome: cheetah_signal_types::AuditOutcome::Success,
        request_id: ctx.0.message_id.to_string(),
        correlation_id: Some(ctx.0.correlation_id.to_string()),
        source_ip: None,
        node_id: state.config.node_id,
        details: None,
    });

    Ok(StatusCode::OK)
}

/// Forces cleanup of all active sessions on a media node.
pub async fn force_cleanup_media_node(
    Path(node_id): Path<String>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<serde_json::Value>, HttpError> {
    ctx.require_scope("system_admin")?;
    let node_id = node_id.parse::<NodeId>().map_err(|e| {
        HttpError::Signal(cheetah_signal_types::SignalError::new(
            cheetah_signal_types::SignalErrorKind::InvalidArgument,
            format!("invalid node_id: {e}"),
        ))
    })?;

    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let cleaned = state
        .media_service
        .force_cleanup_node(&ctx.0, &mut *uow, node_id)
        .await
        .map_err(HttpError::from)?;

    state.audit.record(cheetah_signal_types::AuditEvent {
        timestamp: state.clock.now_wall(),
        action: "media_node.force_cleanup".to_string(),
        actor: ctx.0.principal.id.clone(),
        tenant_id: Some(ctx.0.tenant_id),
        target_type: "media_node".to_string(),
        target_id: Some(node_id.to_string()),
        outcome: cheetah_signal_types::AuditOutcome::Success,
        request_id: ctx.0.message_id.to_string(),
        correlation_id: Some(ctx.0.correlation_id.to_string()),
        source_ip: None,
        node_id: state.config.node_id,
        details: Some(format!("cleaned={cleaned}")),
    });

    Ok(Json(serde_json::json!({ "cleaned": cleaned })))
}
