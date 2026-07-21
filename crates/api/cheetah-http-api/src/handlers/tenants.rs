//! Tenant HTTP handlers.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, JsonBody, ListQuery};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use cheetah_domain::{DomainError, Tenant};
use cheetah_signal_types::{AuditOutcome, Page, TenantId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Tenant response DTO.
#[derive(Clone, Debug, Serialize)]
pub struct TenantDto {
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: cheetah_signal_types::UtcTimestamp,
    pub updated_at: cheetah_signal_types::UtcTimestamp,
}

impl From<&Tenant> for TenantDto {
    fn from(tenant: &Tenant) -> Self {
        Self {
            tenant_id: tenant.tenant_id,
            name: tenant.name.clone(),
            created_at: tenant.created_at,
            updated_at: tenant.updated_at,
        }
    }
}

/// Create-tenant request body.
#[derive(Clone, Debug, Deserialize)]
pub struct CreateTenantRequest {
    /// Optional explicit tenant id (UUIDv7). Generated when omitted.
    pub tenant_id: Option<TenantId>,
    /// Display name.
    pub name: String,
}

pub async fn list_tenants(
    Query(query): Query<ListQuery>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
) -> Result<Json<Page<TenantDto>>, HttpError> {
    ctx.require_scope("system_admin")?;
    let page = query.page_request()?;
    let result = state
        .storage
        .tenant_repository()
        .list(query.name_prefix.as_deref(), page)
        .await
        .map_err(HttpError::from)?;
    Ok(Json(result.map(|t| TenantDto::from(&t))))
}

pub async fn create_tenant(
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    JsonBody(body): JsonBody<CreateTenantRequest>,
) -> Result<(StatusCode, Json<TenantDto>), HttpError> {
    ctx.require_scope("system_admin")?;
    let tenant_id = body
        .tenant_id
        .unwrap_or_else(|| state.id_generator.generate_tenant_id());
    let now = state.clock.now_wall();
    let tenant = Tenant::new(tenant_id, body.name, now).map_err(HttpError::from)?;

    // Idempotent create: if an active tenant with the same id already exists and
    // the name matches, return it; name mismatch is a conflict.
    if let Some(existing) = state
        .storage
        .tenant_repository()
        .get(tenant_id)
        .await
        .map_err(HttpError::from)?
    {
        if existing.name == tenant.name {
            return Ok((StatusCode::OK, Json(TenantDto::from(&existing))));
        }
        return Err(HttpError::from(DomainError::invalid_argument(
            "tenant already exists with a different name",
        )));
    }

    let mut repo = state.storage.tenant_repository();
    repo.save(&tenant).await.map_err(HttpError::from)?;

    crate::audit::record(
        &state,
        &ctx,
        "tenant.create",
        "tenant",
        Some(tenant.tenant_id.to_string()),
        None,
        AuditOutcome::Success,
    );
    Ok((StatusCode::CREATED, Json(TenantDto::from(&tenant))))
}
