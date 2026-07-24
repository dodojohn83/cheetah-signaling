//! Authentication and RBAC for the HTTP API.

use crate::router::RequestId;
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

/// Maximum byte length of an `X-Request-Id` value recorded in audit logs.
const MAX_REQUEST_ID_BYTES: usize = 128;
/// Maximum byte length of an `X-Correlation-Id` value recorded in audit logs.
const MAX_CORRELATION_ID_BYTES: usize = 128;

/// Maximum byte length of a JWT `sub` claim stored as the principal id.
const MAX_JWT_SUB_BYTES: usize = 256;
/// Maximum number of scopes/roles extracted from a JWT.
const MAX_JWT_SCOPES: usize = 64;
/// Maximum byte length of an individual JWT scope/role string.
const MAX_JWT_SCOPE_BYTES: usize = 64;
/// Maximum byte length of a bearer token passed to `jsonwebtoken::decode`.
///
/// JWTs with RS256 are normally well under a few kilobytes; extremely long
/// tokens waste memory on base64 decoding and can be used to amplify request
/// processing before signature validation.
const MAX_JWT_TOKEN_BYTES: usize = 8_192;

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
    if token.len() > MAX_JWT_TOKEN_BYTES {
        return Err(HttpError::Unauthenticated(
            "JWT token exceeds maximum length".to_string(),
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
    let tenant_id = if let Some(ref t) = claims.tenant_id {
        Some(
            t.parse::<TenantId>()
                .map_err(|e| HttpError::Unauthenticated(format!("invalid tenant_id claim: {e}")))?,
        )
    } else {
        None
    };

    let principal = principal_from_jwt_claims(claims)?;

    Ok(AuthContext {
        principal,
        tenant_id,
    })
}

fn principal_from_jwt_claims(claims: JwtClaims) -> Result<Principal, HttpError> {
    if claims.sub.is_empty() {
        return Err(HttpError::Unauthenticated(
            "JWT sub claim must not be empty".to_string(),
        ));
    }
    if claims.sub.len() > MAX_JWT_SUB_BYTES {
        return Err(HttpError::Unauthenticated(
            "JWT sub claim exceeds maximum length".to_string(),
        ));
    }

    let mut scopes = Vec::new();
    if !claims.scope.is_empty() {
        let parts = claims.scope.split_whitespace();
        if parts.clone().count() > MAX_JWT_SCOPES {
            return Err(HttpError::Unauthenticated(
                "JWT scope claim contains too many entries".to_string(),
            ));
        }
        for part in parts {
            if part.len() > MAX_JWT_SCOPE_BYTES {
                return Err(HttpError::Unauthenticated(
                    "JWT scope entry exceeds maximum length".to_string(),
                ));
            }
            scopes.push(part.to_string());
        }
    }

    if claims.scopes.len() > MAX_JWT_SCOPES {
        return Err(HttpError::Unauthenticated(
            "JWT scopes claim contains too many entries".to_string(),
        ));
    }
    for scope in &claims.scopes {
        if scope.len() > MAX_JWT_SCOPE_BYTES {
            return Err(HttpError::Unauthenticated(
                "JWT scopes entry exceeds maximum length".to_string(),
            ));
        }
        scopes.push(scope.clone());
    }

    if claims.roles.len() > MAX_JWT_SCOPES {
        return Err(HttpError::Unauthenticated(
            "JWT roles claim contains too many entries".to_string(),
        ));
    }
    for role in &claims.roles {
        if role.len() > MAX_JWT_SCOPE_BYTES {
            return Err(HttpError::Unauthenticated(
                "JWT roles entry exceeds maximum length".to_string(),
            ));
        }
        scopes.push(role.clone());
    }

    Ok(Principal {
        id: claims.sub,
        kind: PrincipalKind::User,
        scopes,
    })
}

/// Truncates `s` at a UTF-8 character boundary so it is at most `max` bytes.
fn clamp_id_string(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

fn record_auth_audit(parts: &Parts, state: &ApiState, result: &Result<AuthContext, HttpError>) {
    let request_id = parts
        .extensions
        .get::<RequestId>()
        .map(|r| clamp_id_string(&r.0, MAX_REQUEST_ID_BYTES))
        .unwrap_or_else(|| state.id_generator.generate_message_id().to_string());
    let correlation_id = parts
        .headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| clamp_id_string(s, MAX_CORRELATION_ID_BYTES));
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

#[cfg(test)]
mod tests {
    use super::*;
    use cheetah_signal_types::config::SecurityConfig;
    use secrecy::SecretString;

    fn valid_claims() -> JwtClaims {
        JwtClaims {
            sub: "user-123".to_string(),
            tenant_id: None,
            scope: "viewer operator".to_string(),
            scopes: vec!["tenant_admin".to_string()],
            roles: vec!["system_admin".to_string()],
        }
    }

    #[test]
    fn principal_from_valid_jwt_claims() {
        let principal = match principal_from_jwt_claims(valid_claims()) {
            Ok(p) => p,
            Err(_) => panic!("expected valid principal"),
        };
        assert_eq!(principal.id, "user-123");
        assert!(principal.scopes.contains(&"viewer".to_string()));
        assert!(principal.scopes.contains(&"operator".to_string()));
        assert!(principal.scopes.contains(&"tenant_admin".to_string()));
        assert!(principal.scopes.contains(&"system_admin".to_string()));
    }

    #[test]
    fn rejects_empty_sub_claim() {
        let mut claims = valid_claims();
        claims.sub = String::new();
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_oversized_sub_claim() {
        let mut claims = valid_claims();
        claims.sub = "x".repeat(MAX_JWT_SUB_BYTES + 1);
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_too_many_scope_entries() {
        let mut claims = valid_claims();
        claims.scope = (0..=MAX_JWT_SCOPES)
            .map(|i| format!("scope-{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_oversized_scope_entry() {
        let mut claims = valid_claims();
        claims.scope = "x".repeat(MAX_JWT_SCOPE_BYTES + 1);
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_too_many_scopes_claim_entries() {
        let mut claims = valid_claims();
        claims.scopes = (0..=MAX_JWT_SCOPES).map(|i| format!("scope-{i}")).collect();
        claims.scope.clear();
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_too_many_roles_claim_entries() {
        let mut claims = valid_claims();
        claims.roles = (0..=MAX_JWT_SCOPES).map(|i| format!("role-{i}")).collect();
        claims.scope.clear();
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[test]
    fn rejects_oversized_role_entry() {
        let mut claims = valid_claims();
        claims.roles = vec!["x".repeat(MAX_JWT_SCOPE_BYTES + 1)];
        claims.scope.clear();
        assert!(principal_from_jwt_claims(claims).is_err());
    }

    #[tokio::test]
    async fn authenticate_bearer_rejects_oversized_token() {
        let config = SecurityConfig {
            jwt_public_key_ref: SecretString::from("not-a-key".to_string()),
            ..Default::default()
        };
        let token = "x".repeat(MAX_JWT_TOKEN_BYTES + 1);
        let result = authenticate_bearer(&config, &token).await;
        assert!(
            matches!(
                result,
                Err(HttpError::Unauthenticated(ref msg))
                    if msg == "JWT token exceeds maximum length"
            ),
            "expected token length rejection, got {result:?}"
        );
    }

    #[tokio::test]
    async fn authenticate_bearer_accepts_token_at_limit() {
        let config = SecurityConfig {
            jwt_public_key_ref: SecretString::from("not-a-key".to_string()),
            ..Default::default()
        };
        let token = "x".repeat(MAX_JWT_TOKEN_BYTES);
        let result = authenticate_bearer(&config, &token).await;
        // Token is at the limit, so it passes the length check and fails on
        // the invalid public key configuration rather than on length.
        assert!(
            matches!(result, Err(HttpError::Internal(_))),
            "expected key parsing failure after length check, got {result:?}"
        );
    }
}
