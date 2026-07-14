//! Authentication and RBAC for the HTTP API.

use crate::{ApiState, HttpError};
use axum::{extract::FromRequestParts, http::request::Parts};
use cheetah_signal_types::{Principal, PrincipalKind, TenantId};
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode};
use secrecy::ExposeSecret;
use serde::Deserialize;
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

impl FromRequestParts<Arc<ApiState>> for AuthContext {
    type Rejection = HttpError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        if let Some(header) = auth_header {
            let (scheme, token) = split_auth_header(header);
            let scheme = scheme.to_lowercase();
            if scheme == "bearer" {
                return authenticate_bearer(&state.config.security, token).await;
            }
        }

        if let Some(api_key) = parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
            return authenticate_static_key(&state.config.security, api_key);
        }

        Err(HttpError::Unauthenticated(
            "missing Authorization or X-Api-Key header".to_string(),
        ))
    }
}

fn split_auth_header(header: &str) -> (&str, &str) {
    let mut parts = header.splitn(2, ' ');
    let scheme = parts.next().unwrap_or("");
    let token = parts.next().unwrap_or("").trim();
    (scheme, token)
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
    validation.set_required_spec_claims(&["exp"]);
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
