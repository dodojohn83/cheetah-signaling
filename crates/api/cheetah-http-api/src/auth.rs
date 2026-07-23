//! Authentication and RBAC for the HTTP API.

use crate::{ApiState, HttpError};
use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use cheetah_signal_types::{
    AuditEvent, AuditOutcome, Principal, PrincipalKind, SafeDetails, TenantId,
};
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode};
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Authenticated actor and optional tenant claim.
#[derive(Clone, Debug)]
pub struct AuthContext {
    /// Authenticated principal.
    pub principal: Principal,
    /// Tenant identifier if asserted by the token.
    pub tenant_id: Option<TenantId>,
}

impl AuthContext {
    /// Checks that the principal has at least one of the required scopes.
    pub fn require_scope(&self, scope: &str) -> Result<(), HttpError> {
        if self
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

/// Maximum byte length of an `Authorization` header value.
const MAX_AUTHORIZATION_BYTES: usize = 16_384;
/// Maximum byte length of an `X-Api-Key` header value.
const MAX_API_KEY_BYTES: usize = 4_096;

impl FromRequestParts<Arc<ApiState>> for AuthContext {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = match parts.headers.get("authorization") {
            Some(value) => match value.to_str() {
                Ok(text) if text.len() <= MAX_AUTHORIZATION_BYTES => Some(text),
                Ok(_) => {
                    return Err(HttpError::Unauthenticated(
                        "authorization header exceeds maximum length".to_string(),
                    ));
                }
                Err(_) => {
                    return Err(HttpError::Unauthenticated(
                        "authorization header is not valid UTF-8".to_string(),
                    ));
                }
            },
            None => None,
        };

        let bearer_result = if let Some(header) = auth_header {
            let (scheme, token) = split_auth_header(header);
            if scheme.eq_ignore_ascii_case("bearer") {
                Some(authenticate_bearer(&state.config.security, token).await)
            } else {
                // Non-bearer Authorization headers are ignored so that an
                // unrelated proxy-injected Authorization header does not prevent
                // X-Api-Key authentication.
                None
            }
        } else {
            None
        };

        let result = match bearer_result {
            Some(res) => res,
            None => match try_api_key_auth(parts, &state.config.security) {
                Some(res) => res,
                None => Err(HttpError::Unauthenticated(
                    "missing Authorization or X-Api-Key header".to_string(),
                )),
            },
        };

        record_auth_audit(parts, state, &result);
        result
    }
}

fn split_auth_header(header: &str) -> (&str, &str) {
    let mut parts = header.splitn(2, ' ');
    let scheme = parts.next().unwrap_or("");
    let token = parts.next().unwrap_or("").trim();
    (scheme, token)
}

fn try_api_key_auth(
    parts: &Parts,
    security: &cheetah_signal_types::config::SecurityConfig,
) -> Option<Result<AuthContext, HttpError>> {
    match parts.headers.get("x-api-key") {
        Some(value) => match value.to_str() {
            Ok(api_key) if api_key.len() <= MAX_API_KEY_BYTES => {
                Some(authenticate_static_key(security, api_key))
            }
            Ok(_) => Some(Err(HttpError::Unauthenticated(
                "x-api-key header exceeds maximum length".to_string(),
            ))),
            Err(_) => Some(Err(HttpError::Unauthenticated(
                "x-api-key header is not valid UTF-8".to_string(),
            ))),
        },
        None => None,
    }
}

fn authenticate_static_key(
    security: &cheetah_signal_types::config::SecurityConfig,
    provided: &str,
) -> Result<AuthContext, HttpError> {
    let expected = security.static_api_key.expose_secret();
    if expected.is_empty() {
        return Err(HttpError::Unauthenticated(
            "static API key not configured".to_string(),
        ));
    }
    if provided.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 0 {
        return Err(HttpError::Unauthenticated("invalid API key".to_string()));
    }
    Ok(AuthContext {
        principal: Principal {
            id: "static-api-key".to_string(),
            kind: PrincipalKind::Service,
            scopes: vec![
                "viewer".to_string(),
                "operator".to_string(),
                "tenant_admin".to_string(),
                "system_admin".to_string(),
            ],
        },
        tenant_id: None,
    })
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: String,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    roles: Vec<String>,
}

async fn authenticate_bearer(
    security: &cheetah_signal_types::config::SecurityConfig,
    token: &str,
) -> Result<AuthContext, HttpError> {
    let pem = security.jwt_public_key_ref.expose_secret();
    if pem.is_empty() {
        return Err(HttpError::Unauthenticated(
            "JWT authentication not configured".to_string(),
        ));
    }

    // The validation algorithm is fixed to RS256 and not read from the untrusted
    // token header to prevent algorithm confusion attacks.
    let key = DecodingKey::from_rsa_pem(pem.as_bytes())
        .map_err(|e| HttpError::Internal(format!("invalid JWT public key configuration: {e}")))?;
    let mut validation = Validation::new(Algorithm::RS256);
    let mut required_claims = vec!["exp"];
    if !security.jwt_audience.is_empty() {
        validation.set_audience(&security.jwt_audience);
        required_claims.push("aud");
    }
    if !security.jwt_issuer.is_empty() {
        validation.set_issuer(&security.jwt_issuer);
        required_claims.push("iss");
    }
    validation.set_required_spec_claims(&required_claims);
    let token_data: TokenData<JwtClaims> = decode(token, &key, &validation)
        .map_err(|e| HttpError::Unauthenticated(format!("JWT validation failed: {e}")))?;

    let claims = token_data.claims;
    let tenant_id = if let Some(t) = claims.tenant_id {
        Some(
            t.parse::<TenantId>()
                .map_err(|e| HttpError::Unauthenticated(format!("invalid tenant_id claim: {e}")))?,
        )
    } else {
        None
    };

    let mut scopes: Vec<String> = if !claims.scope.is_empty() {
        claims
            .scope
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()
    } else {
        claims.scopes
    };
    scopes.extend(claims.roles);

    Ok(AuthContext {
        principal: Principal {
            id: claims.sub,
            kind: PrincipalKind::User,
            scopes,
        },
        tenant_id,
    })
}

fn record_auth_audit(parts: &Parts, state: &ApiState, result: &Result<AuthContext, HttpError>) {
    let request_id = parts
        .headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.id_generator.generate_message_id().to_string());
    let correlation_id = parts
        .headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let source_ip = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip().to_string());

    let (actor, tenant_id, outcome, details) = match result {
        Ok(ctx) => (
            ctx.principal.id.clone(),
            ctx.tenant_id,
            AuditOutcome::Success,
            None,
        ),
        Err(err) => (
            "anonymous".to_string(),
            None,
            AuditOutcome::Failure {
                reason: err.code().to_string(),
            },
            Some(err.to_string()),
        ),
    };

    let event = AuditEvent {
        timestamp: state.clock.now_wall(),
        action: "auth.authenticate".to_string(),
        actor,
        tenant_id,
        target_type: "session".to_string(),
        target_id: None,
        outcome,
        request_id,
        correlation_id,
        source_ip,
        node_id: state.config.node_id,
        details: details.map(SafeDetails::new),
    };
    state.audit.record(event);
}
