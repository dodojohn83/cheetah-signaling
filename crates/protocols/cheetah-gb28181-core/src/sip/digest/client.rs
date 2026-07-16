//! RFC 2617/7616 Digest authentication client helper.

use super::context::compute_response;
use super::response::{DigestAlgorithm, DigestChallenge, DigestError, DigestQop, DigestResponse};
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

/// Client-side helper for computing SIP `Authorization` Digest responses.
#[derive(Clone, Debug)]
pub struct DigestClient {
    allow_md5: bool,
    preferred_algorithm: DigestAlgorithm,
    qop: Option<DigestQop>,
    nonce_count: u64,
}

impl DigestClient {
    /// Creates a client that prefers SHA-256 and disables MD5 by default.
    pub fn new() -> Self {
        Self {
            allow_md5: false,
            preferred_algorithm: DigestAlgorithm::Sha256,
            qop: None,
            nonce_count: 0,
        }
    }

    /// Derives an unpredictable client nonce (cnonce) from a secret password
    /// and a public context string.
    ///
    /// The result is keyed by the password so that it cannot be predicted by an
    /// attacker who does not know the password, satisfying RFC 2617's
    /// unpredictability requirement for cnonce without requiring the caller to
    /// supply an external random source.
    pub fn derive_cnonce(password: &SecretString, context: &str) -> Result<String, DigestError> {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(password.expose_secret().as_bytes())
            .map_err(|e| DigestError::Malformed(format!("invalid cnonce key: {e}")))?;
        mac.update(context.as_bytes());
        // 16 bytes (32 hex chars) is enough entropy and is a valid SIP token.
        Ok(hex::encode(&mac.finalize().into_bytes()[..16]))
    }

    /// Allows the broken MD5 algorithm for legacy GB28181 interop.
    pub fn allow_md5(mut self, allow: bool) -> Self {
        self.allow_md5 = allow;
        self
    }

    /// Sets the algorithm the client would prefer to use when the server
    /// challenge does not mandate one.
    pub fn preferred_algorithm(mut self, algorithm: DigestAlgorithm) -> Self {
        self.preferred_algorithm = algorithm;
        self
    }

    /// Sets the QoP to request. `auth-int` is not supported.
    pub fn qop(mut self, qop: Option<DigestQop>) -> Result<Self, DigestError> {
        if qop == Some(DigestQop::AuthInt) {
            return Err(DigestError::InvalidQop);
        }
        self.qop = qop;
        Ok(self)
    }

    /// Builds a [`DigestResponse`] for a `WWW-Authenticate` challenge.
    ///
    /// `cnonce` must be supplied by the caller when the challenge offers
    /// `qop=auth`. It should be a fresh, unpredictable value per authorization
    /// context. `method` is the SIP method (e.g. `REGISTER`) and `uri` is the
    /// digest URI used in the A2 computation.
    pub fn authorize(
        &mut self,
        username: &str,
        password: &SecretString,
        method: &str,
        uri: &str,
        challenge: &DigestChallenge,
        cnonce: &str,
    ) -> Result<DigestResponse, DigestError> {
        validate_token(username)?;
        validate_token(uri)?;
        validate_token(cnonce)?;
        validate_token(&challenge.realm)?;
        validate_token(&challenge.nonce)?;
        if let Some(opaque) = &challenge.opaque {
            validate_token(opaque)?;
        }

        if challenge.algorithm == DigestAlgorithm::Md5 && !self.allow_md5 {
            return Err(DigestError::AlgorithmNotAllowed);
        }

        if challenge.qop == Some(DigestQop::AuthInt) {
            return Err(DigestError::InvalidQop);
        }

        // Use the challenge's qop if offered; otherwise fall back to the
        // client's configured qop.
        let qop = challenge.qop.or(self.qop);
        if qop == Some(DigestQop::AuthInt) {
            return Err(DigestError::InvalidQop);
        }

        let (cnonce, nc) = if qop == Some(DigestQop::Auth) {
            validate_token(cnonce)?;
            self.nonce_count = self
                .nonce_count
                .checked_add(1)
                .ok_or_else(|| DigestError::Malformed("nonce count overflow".to_string()))?;
            (Some(cnonce.to_string()), Some(self.nonce_count))
        } else {
            (None, None)
        };

        let algorithm = challenge.algorithm;
        let response = compute_response(
            algorithm,
            username,
            &challenge.realm,
            password.expose_secret(),
            &challenge.nonce,
            nc.unwrap_or(0),
            cnonce.as_deref(),
            qop,
            method,
            uri,
        );

        Ok(DigestResponse {
            username: username.to_string(),
            realm: challenge.realm.clone(),
            nonce: challenge.nonce.clone(),
            uri: uri.to_string(),
            response,
            cnonce,
            nc,
            qop,
            algorithm: Some(algorithm),
            opaque: challenge.opaque.clone(),
        })
    }
}

impl Default for DigestClient {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_token(value: &str) -> Result<(), DigestError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(DigestError::Malformed(
            "digest value contains line break".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::sip::digest::{DigestContext, DigestReplayCache};
    use secrecy::SecretString;

    #[test]
    fn client_response_validates_against_server_context() {
        let secret = b"this-is-a-very-long-secret-used-for-unit-tests-only";
        let ctx = DigestContext::new("example.com", secret.as_slice()).unwrap();
        let mut client = DigestClient::new();
        let mut replay_cache = DigestReplayCache::new(1024);
        let now = 1000;

        let challenge = ctx.generate_challenge(now).unwrap();
        let password = SecretString::new("secret-password".into());
        let response = client
            .authorize(
                "alice",
                &password,
                "REGISTER",
                "sip:example.com",
                &challenge,
                "client-nonce-1",
            )
            .unwrap();

        assert!(
            ctx.validate(
                &response,
                &crate::Method::Register,
                "sip:example.com",
                &password,
                &mut replay_cache,
                now,
            )
            .is_ok()
        );
    }

    #[test]
    fn response_header_value_round_trips() {
        let response = DigestResponse {
            username: "alice".to_string(),
            realm: "example.com".to_string(),
            nonce: "server-nonce".to_string(),
            uri: "sip:example.com".to_string(),
            response: "0123456789abcdef".to_string(),
            cnonce: Some("client-nonce".to_string()),
            nc: Some(1),
            qop: Some(DigestQop::Auth),
            algorithm: Some(DigestAlgorithm::Sha256),
            opaque: Some("opaque-value".to_string()),
        };

        let header = response.to_header_value();
        assert!(header.contains("qop=auth"));
        assert!(!header.contains("qop=\"auth\""));
        let parsed = DigestResponse::parse(&header).unwrap();
        assert_eq!(parsed.username, "alice");
        assert_eq!(parsed.realm, "example.com");
        assert_eq!(parsed.nonce, "server-nonce");
        assert_eq!(parsed.uri, "sip:example.com");
        assert_eq!(parsed.response, "0123456789abcdef");
        assert_eq!(parsed.cnonce, Some("client-nonce".to_string()));
        assert_eq!(parsed.nc, Some(1));
        assert_eq!(parsed.qop, Some(DigestQop::Auth));
        assert_eq!(parsed.algorithm, Some(DigestAlgorithm::Sha256));
        assert_eq!(parsed.opaque, Some("opaque-value".to_string()));
    }

    #[test]
    fn client_rejects_md5_when_not_allowed() {
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "nonce".to_string(),
            opaque: None,
            stale: false,
            algorithm: DigestAlgorithm::Md5,
            qop: None,
        };
        let mut client = DigestClient::new();
        let password = SecretString::new("secret".into());
        let result = client.authorize(
            "alice",
            &password,
            "REGISTER",
            "sip:example.com",
            &challenge,
            "",
        );
        assert!(matches!(result, Err(DigestError::AlgorithmNotAllowed)));
    }

    #[test]
    fn client_rejects_line_break_injection() {
        let challenge = DigestChallenge {
            realm: "example.com".to_string(),
            nonce: "nonce".to_string(),
            opaque: None,
            stale: false,
            algorithm: DigestAlgorithm::Sha256,
            qop: None,
        };
        let mut client = DigestClient::new();
        let password = SecretString::new("secret".into());
        let result = client.authorize(
            "alice\r\nInjected: 1",
            &password,
            "REGISTER",
            "sip:example.com",
            &challenge,
            "",
        );
        assert!(matches!(result, Err(DigestError::Malformed(_))));
    }

    #[test]
    fn derive_cnonce_is_token_safe_and_context_sensitive() {
        let password = SecretString::new("secret".into());
        let a = DigestClient::derive_cnonce(&password, "ctx-1").unwrap();
        let b = DigestClient::derive_cnonce(&password, "ctx-2").unwrap();
        assert_ne!(a, b);
        assert!(!a.contains('"'));
        assert!(!a.contains('\r'));
        assert!(!a.contains('\n'));
        assert!(!a.contains(' '));
    }
}
