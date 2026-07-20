//! Axum extractors for request context and pagination.

use crate::rate_limit::RateKey;
use crate::{ApiState, AuthContext, HttpError};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use cheetah_signal_types::{
    CorrelationId, Deadline, DurationMs, MessageId, PageRequest, RequestContext, SignalError,
    SignalErrorKind, TenantId, validate_traceparent, validate_tracestate,
};
use std::net::SocketAddr;
use std::sync::Arc;

/// Request context resolved from auth and headers.
#[derive(Clone, Debug)]
pub struct ApiRequestContext(pub RequestContext);

impl std::ops::Deref for ApiRequestContext {
    type Target = RequestContext;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ApiRequestContext {
    /// Checks that the principal has the required scope or `system_admin`.
    pub fn require_scope(&self, scope: &str) -> Result<(), HttpError> {
        if self
            .0
            .principal
            .scopes
            .iter()
            .any(|s| s == "system_admin" || s == scope)
        {
            Ok(())
        } else {
            Err(HttpError::PermissionDenied(format!(
                "missing {scope} scope"
            )))
        }
    }
}

impl FromRequestParts<Arc<ApiState>> for ApiRequestContext {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let auth = AuthContext::from_request_parts(parts, state).await?;

        let source_ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip().to_string());

        let tenant_id = resolve_tenant_id(parts, &auth)?;

        let message_id: MessageId = if let Some(header) = parts.headers.get("x-request-id") {
            parse_message_or_correlation_id(header, "x-request-id")?.into()
        } else {
            state.id_generator.generate_message_id()
        };

        let correlation_id: CorrelationId =
            if let Some(header) = parts.headers.get("x-correlation-id") {
                parse_message_or_correlation_id(header, "x-correlation-id")?.into()
            } else {
                state.id_generator.generate_correlation_id()
            };

        let traceparent = parts
            .headers
            .get("traceparent")
            .and_then(|v| v.to_str().ok())
            .and_then(validate_traceparent)
            .map(|s| s.to_string());
        let tracestate = parts
            .headers
            .get("tracestate")
            .and_then(|v| v.to_str().ok())
            .and_then(validate_tracestate)
            .map(|s| s.to_string());

        check_rate_limit(parts, state, &tenant_id)?;

        let now = state.clock.now_wall();
        let timeout_ms = i64::try_from(state.config.read_timeout_ms).unwrap_or(5000);
        let deadline = Deadline::from_timestamp(
            now.checked_add(DurationMs::from_millis(timeout_ms))
                .unwrap_or(now),
        );

        let request_context = RequestContext {
            tenant_id,
            principal: auth.principal,
            message_id,
            correlation_id,
            traceparent,
            tracestate,
            deadline: Some(deadline),
            node_id: Some(state.config.node_id),
            source_ip,
        };

        let span = tracing::Span::current();
        span.record("tenant_id", request_context.tenant_id.to_string());
        span.record("request_id", request_context.message_id.to_string());
        if let Some(node_id) = request_context.node_id {
            span.record("node_id", node_id.to_string());
        }

        Ok(Self(request_context))
    }
}

fn check_rate_limit(
    parts: &Parts,
    state: &ApiState,
    tenant_id: &TenantId,
) -> Result<(), HttpError> {
    if state.rate_limiter.is_disabled() {
        return Ok(());
    }
    let ip = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
        .map(|c| c.0.ip())
        .unwrap_or_else(|| [0, 0, 0, 0].into());
    let protocol = parts.uri.path().split('/').nth(1).unwrap_or("").to_string();
    let node = state.config.node_id.to_string();
    let key = RateKey {
        source: ip,
        tenant: tenant_id.to_string(),
        protocol,
        node,
    };
    if state.rate_limiter.check(&key) {
        Ok(())
    } else {
        Err(HttpError::RateLimited(
            "tenant rate limit exceeded".to_string(),
        ))
    }
}

fn resolve_tenant_id(parts: &Parts, auth: &AuthContext) -> Result<TenantId, HttpError> {
    let Some(value) = parts.headers.get("x-tenant-id") else {
        return auth.tenant_id.ok_or_else(|| {
            HttpError::Unauthenticated(
                "tenant id is required via x-tenant-id header or token claim".to_string(),
            )
        });
    };

    let text = value.to_str().map_err(|_| {
        HttpError::Signal(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "x-tenant-id header is not valid UTF-8",
        ))
    })?;
    let header = text.parse::<TenantId>()?;

    if let Some(auth_tenant) = auth.tenant_id
        && header != auth_tenant
    {
        return Err(HttpError::PermissionDenied(
            "tenant header does not match token tenant".to_string(),
        ));
    }

    Ok(header)
}

fn parse_message_or_correlation_id(
    header: &axum::http::HeaderValue,
    name: &str,
) -> Result<uuid::Uuid, HttpError> {
    let text = header
        .to_str()
        .map_err(|_| HttpError::Unauthenticated(format!("{name} header is not valid UTF-8")))?;
    text.parse::<uuid::Uuid>()
        .map_err(|e| HttpError::Unauthenticated(format!("invalid {name}: {e}")))
}

/// Query parameters for list endpoints.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ListQuery {
    /// Opaque pagination cursor.
    pub cursor: Option<String>,
    /// Maximum number of items to return.
    pub page_size: u32,
    /// Protocol filter.
    pub protocol: Option<String>,
    /// Lifecycle or status filter.
    pub status: Option<String>,
    /// Name prefix filter.
    pub name_prefix: Option<String>,
    /// Updated after timestamp.
    pub updated_after: Option<String>,
    /// Optional parent device id filter.
    pub device_id: Option<String>,
    /// Optional media session purpose filter.
    pub purpose: Option<String>,
    /// Optional enabled flag filter for webhooks.
    pub enabled: Option<bool>,
    /// Optional event type filter for webhooks.
    pub event_type: Option<String>,
}

impl Default for ListQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            page_size: cheetah_signal_types::DEFAULT_PAGE_SIZE,
            protocol: None,
            status: None,
            name_prefix: None,
            updated_after: None,
            device_id: None,
            purpose: None,
            enabled: None,
            event_type: None,
        }
    }
}

impl ListQuery {
    /// Converts to a validated `PageRequest`.
    pub fn page_request(&self) -> Result<PageRequest, HttpError> {
        let mut req = PageRequest::new(self.page_size).map_err(HttpError::Signal)?;
        if let Some(cursor) = &self.cursor {
            req = req.with_cursor(cursor);
        }
        Ok(req)
    }
}

/// Extractor for the `Idempotency-Key` header.
#[derive(Clone, Debug)]
pub struct IdempotencyKey(pub String);

impl FromRequestParts<Arc<ApiState>> for IdempotencyKey {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let key = idempotency_key_from_header(parts.headers.get("idempotency-key"))?;
        Ok(Self(key))
    }
}

fn idempotency_key_from_header(
    value: Option<&axum::http::HeaderValue>,
) -> Result<String, HttpError> {
    match value {
        Some(v) => {
            let key = v.to_str().map_err(|_| {
                HttpError::Signal(SignalError::new(
                    SignalErrorKind::InvalidArgument,
                    "Idempotency-Key header is not valid UTF-8",
                ))
            })?;
            if key.is_empty() {
                Ok(uuid::Uuid::now_v7().to_string())
            } else {
                Ok(key.to_string())
            }
        }
        None => Ok(uuid::Uuid::now_v7().to_string()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn idempotency_key_from_header_rejects_non_utf8() {
        let value = axum::http::HeaderValue::from_bytes(&[0xff]).unwrap();
        assert!(idempotency_key_from_header(Some(&value)).is_err());
    }

    #[test]
    fn idempotency_key_from_header_uses_valid_value() {
        let value = axum::http::HeaderValue::from_static("my-key");
        assert_eq!(idempotency_key_from_header(Some(&value)).unwrap(), "my-key");
    }

    #[test]
    fn idempotency_key_from_header_generates_uuid_when_missing() {
        let key = idempotency_key_from_header(None).unwrap();
        assert!(!key.is_empty());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::AuthContext;
    use axum::http::Request;
    use cheetah_signal_types::{Principal, PrincipalKind};
    use std::str::FromStr;

    fn auth_with_tenant(tenant_id: Option<TenantId>) -> AuthContext {
        AuthContext {
            principal: Principal {
                id: "u".to_string(),
                kind: PrincipalKind::User,
                scopes: vec![],
            },
            tenant_id,
        }
    }

    fn parts_with_header(name: &str, value: &[u8]) -> Parts {
        let req = Request::builder()
            .header(name, value)
            .body(())
            .expect("request build");
        let (parts, _) = req.into_parts();
        parts
    }

    #[test]
    fn resolve_tenant_id_uses_valid_header() {
        let tenant = TenantId::from_str("018f3e7a-6a7d-7c9e-8b1a-2b3c4d5e6f7a").unwrap();
        let parts = parts_with_header("x-tenant-id", tenant.to_string().as_bytes());
        let auth = auth_with_tenant(None);
        assert_eq!(resolve_tenant_id(&parts, &auth).unwrap(), tenant);
    }

    #[test]
    fn resolve_tenant_id_falls_back_to_auth_claim() {
        let tenant = TenantId::from_str("018f3e7a-6a7d-7c9e-8b1a-2b3c4d5e6f7a").unwrap();
        let parts = Request::builder()
            .body(())
            .expect("request build")
            .into_parts()
            .0;
        let auth = auth_with_tenant(Some(tenant));
        assert_eq!(resolve_tenant_id(&parts, &auth).unwrap(), tenant);
    }

    #[test]
    fn resolve_tenant_id_rejects_non_utf8_header() {
        let parts = parts_with_header("x-tenant-id", &[0xff, 0xfe]);
        let auth = auth_with_tenant(None);
        let err = resolve_tenant_id(&parts, &auth).expect_err("non-utf8 header");
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn resolve_tenant_id_rejects_mismatch_with_auth_claim() {
        let header_tenant = TenantId::from_str("018f3e7a-6a7d-7c9e-8b1a-2b3c4d5e6f7a").unwrap();
        let auth_tenant = TenantId::from_str("028f3e7a-6a7d-7c9e-8b1a-2b3c4d5e6f7a").unwrap();
        let parts = parts_with_header("x-tenant-id", header_tenant.to_string().as_bytes());
        let auth = auth_with_tenant(Some(auth_tenant));
        let err = resolve_tenant_id(&parts, &auth).expect_err("mismatched tenant");
        assert_eq!(err.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn resolve_tenant_id_rejects_missing_when_auth_has_no_tenant() {
        let parts = Request::builder()
            .body(())
            .expect("request build")
            .into_parts()
            .0;
        let auth = auth_with_tenant(None);
        let err = resolve_tenant_id(&parts, &auth).expect_err("missing tenant");
        assert_eq!(err.status(), axum::http::StatusCode::UNAUTHORIZED);
    }
}
