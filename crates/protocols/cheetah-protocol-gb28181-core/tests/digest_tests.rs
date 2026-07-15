//! Integration tests for the Sans-I/O SIP Digest authentication module.

#![allow(clippy::too_many_arguments, clippy::unwrap_used, clippy::expect_used)]

use cheetah_protocol_gb28181_core::{
    DigestAlgorithm, DigestContext, DigestError, DigestQop, DigestReplayCache, DigestResponse,
    Method,
};
use secrecy::SecretString;
use sha2::{Digest, Sha256, Sha512};

fn make_response(
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    method: &Method,
    nc: u64,
    cnonce: &str,
    qop: Option<DigestQop>,
    algorithm: DigestAlgorithm,
) -> DigestResponse {
    let response = compute_client_response(
        algorithm, username, password, realm, nonce, uri, method, nc, cnonce, qop,
    );
    DigestResponse {
        username: username.to_string(),
        realm: realm.to_string(),
        nonce: nonce.to_string(),
        uri: uri.to_string(),
        response,
        cnonce: if qop.is_some() {
            Some(cnonce.to_string())
        } else {
            None
        },
        nc: if qop.is_some() { Some(nc) } else { None },
        qop,
        algorithm: Some(algorithm),
        opaque: None,
    }
}

fn compute_client_response(
    algorithm: DigestAlgorithm,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    method: &Method,
    nc: u64,
    cnonce: &str,
    qop: Option<DigestQop>,
) -> String {
    let a1 = format!("{username}:{realm}:{password}");
    let ha1 = hash_hex(algorithm, a1.as_bytes());

    let a2 = format!("{method}:{uri}");
    let ha2 = hash_hex(algorithm, a2.as_bytes());

    let a3 = match qop {
        Some(DigestQop::Auth) => format!("{ha1}:{nonce}:{nc:08x}:{cnonce}:auth:{ha2}"),
        _ => format!("{ha1}:{nonce}:{ha2}"),
    };
    hash_hex(algorithm, a3.as_bytes())
}

fn hash_hex(algorithm: DigestAlgorithm, data: &[u8]) -> String {
    match algorithm {
        DigestAlgorithm::Md5 => format!("{:x}", md5::compute(data)),
        DigestAlgorithm::Sha256 => hex::encode(Sha256::digest(data)),
        DigestAlgorithm::Sha512 => hex::encode(Sha512::digest(data)),
    }
}

fn ctx_md5() -> DigestContext {
    DigestContext::new("example.com", b"server-secret").preferred_algorithm(DigestAlgorithm::Md5)
}

#[test]
fn challenge_defaults_to_sha256() -> Result<(), DigestError> {
    let ctx = DigestContext::new("example.com", b"server-secret");
    let challenge = ctx.generate_challenge(1000)?;
    let header = challenge.to_header_value();
    assert!(header.starts_with("Digest "));
    assert!(header.contains("realm=\"example.com\""));
    assert!(header.contains("nonce=\""));
    assert!(header.contains("algorithm=SHA-256"));
    assert!(header.contains("qop=\"auth\""));
    assert!(!header.contains("stale"));
    Ok(())
}

#[test]
fn md5_challenge_when_configured() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;
    let header = challenge.to_header_value();
    assert!(header.contains("algorithm=MD5"));
    Ok(())
}

#[test]
fn stale_challenge_includes_stale_true() -> Result<(), DigestError> {
    let ctx = DigestContext::new("example.com", b"server-secret");
    let challenge = ctx.generate_stale_challenge(1000)?;
    let header = challenge.to_header_value();
    assert!(header.contains("stale=true"));
    Ok(())
}

#[test]
fn parse_digest_response_with_qop() -> Result<(), DigestError> {
    let value = r#"Digest username="alice", realm="example.com", nonce="abc", uri="sip:bob@example.com", response="resp", cnonce="cn", nc="00000001", qop="auth", algorithm="MD5""#;
    let parsed = DigestResponse::parse(value)?;
    assert_eq!(parsed.username, "alice");
    assert_eq!(parsed.realm, "example.com");
    assert_eq!(parsed.nonce, "abc");
    assert_eq!(parsed.uri, "sip:bob@example.com");
    assert_eq!(parsed.response, "resp");
    assert_eq!(parsed.cnonce, Some("cn".to_string()));
    assert_eq!(parsed.nc, Some(1));
    assert_eq!(parsed.qop, Some(DigestQop::Auth));
    assert_eq!(parsed.algorithm, Some(DigestAlgorithm::Md5));
    Ok(())
}

#[test]
fn parse_digest_response_without_qop() -> Result<(), DigestError> {
    let value = r#"username="alice", realm="example.com", nonce="abc", uri="sip:bob@example.com", response="resp", algorithm="MD5""#;
    let parsed = DigestResponse::parse(value)?;
    assert_eq!(parsed.cnonce, None);
    assert_eq!(parsed.nc, None);
    assert_eq!(parsed.qop, None);
    Ok(())
}

#[test]
fn parse_rejects_overly_long_header() {
    let prefix = r#"Digest username="alice","#;
    let padding = "x".repeat(4096);
    let value = format!(
        r#"{prefix} realm="example.com", nonce="{padding}", uri="sip:b@e", response="resp""#
    );
    let result = DigestResponse::parse(&value);
    assert!(matches!(result, Err(DigestError::Malformed(_))));
}

#[test]
fn parse_with_limit_allows_shorter_header() -> Result<(), DigestError> {
    let value = r#"username="alice", realm="example.com", nonce="abc", uri="sip:bob@example.com", response="resp""#;
    DigestResponse::parse_with_limit(value, 256)?;
    Ok(())
}

#[test]
fn no_qop_nonce_can_be_reused_within_ttl() -> Result<(), DigestError> {
    let ctx = ctx_md5().qop(None);
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        0,
        "",
        None,
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )?;

    // A second request with the same nonce and no qop should succeed because
    // replay tracking is only meaningful when nonce-count/cnonce are present.
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1001,
    )
}

#[test]
fn md5_auth_with_qop_succeeds() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )
}

#[test]
fn sha256_auth_with_qop_succeeds() -> Result<(), DigestError> {
    let ctx = DigestContext::new("example.com", b"server-secret")
        .preferred_algorithm(DigestAlgorithm::Sha256)
        .allow_md5(false);
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Sha256,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )
}

#[test]
fn md5_without_qop_succeeds() -> Result<(), DigestError> {
    let ctx = ctx_md5().qop(None);
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        0,
        "",
        None,
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )
}

#[test]
fn wrong_password_fails() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "wrong",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected validation to fail");
    };
    assert!(matches!(err, DigestError::InvalidResponse));
    Ok(())
}

#[test]
fn expired_nonce_is_stale() -> Result<(), DigestError> {
    let ctx = ctx_md5().nonce_ttl_seconds(60);
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        2000,
    ) else {
        panic!("expected validation to fail");
    };
    assert!(matches!(err, DigestError::StaleNonce));
    Ok(())
}

#[test]
fn tampered_nonce_fails_signature() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let mut nonce = challenge.nonce.clone();
    let Some(last) = nonce.pop() else {
        panic!("nonce was empty");
    };
    nonce.push(if last == 'f' { '0' } else { 'f' });

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected validation to fail");
    };
    assert!(matches!(err, DigestError::InvalidNonce));
    Ok(())
}

#[test]
fn replay_is_detected() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )?;

    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected replay detection");
    };
    assert!(matches!(err, DigestError::ReplayDetected));
    Ok(())
}

#[test]
fn md5_disallowed_by_policy() -> Result<(), DigestError> {
    let ctx = DigestContext::new("example.com", b"server-secret")
        .allow_md5(false)
        .preferred_algorithm(DigestAlgorithm::Sha256);
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected algorithm policy failure");
    };
    assert!(matches!(err, DigestError::AlgorithmNotAllowed));
    Ok(())
}

#[test]
fn auth_int_qop_is_rejected() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::AuthInt),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected qop rejection");
    };
    assert!(matches!(err, DigestError::InvalidQop));
    Ok(())
}

#[test]
fn missing_qop_fields_is_invalid() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let mut resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );
    resp.cnonce = None;

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected qop validation failure");
    };
    assert!(matches!(err, DigestError::InvalidQop));
    Ok(())
}

#[test]
fn realm_mismatch_fails() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let mut resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );
    resp.realm = "other.com".to_string();

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected realm mismatch");
    };
    assert!(matches!(err, DigestError::RealmMismatch));
    Ok(())
}

#[test]
fn uri_mismatch_fails() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let mut resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );
    resp.uri = "sip:other@example.com".to_string();

    let mut cache = DigestReplayCache::new(64);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected uri mismatch");
    };
    assert!(matches!(err, DigestError::UriMismatch));
    Ok(())
}

#[test]
fn same_nc_with_different_cnonce_is_replay() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp1 = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp1,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )?;

    let mut resp2 = resp1.clone();
    resp2.cnonce = Some("othernonce".to_string());
    resp2.response = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "othernonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    )
    .response;

    let Err(err) = ctx.validate(
        &resp2,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected replay detection for same nc with different cnonce");
    };
    assert!(matches!(err, DigestError::ReplayDetected));
    Ok(())
}

#[test]
fn out_of_order_nc_is_replay() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;

    let resp_high = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        2,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &resp_high,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )?;

    let resp_low = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        Some(DigestQop::Auth),
        DigestAlgorithm::Md5,
    );

    let Err(err) = ctx.validate(
        &resp_low,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected replay detection for out-of-order nc");
    };
    assert!(matches!(err, DigestError::ReplayDetected));
    Ok(())
}

#[test]
fn parse_unescapes_quotes_and_backslashes() -> Result<(), DigestError> {
    let value = r##"username="alice\\smith", realm="foo\"bar", nonce="abc", uri="sip:b@e", response="resp""##;
    let parsed = DigestResponse::parse(value)?;
    assert_eq!(parsed.username, r"alice\smith");
    assert_eq!(parsed.realm, r##"foo"bar"##);
    Ok(())
}

#[test]
fn challenge_header_round_trips_quoted_realm_and_strips_crlf() -> Result<(), DigestError> {
    let ctx = DigestContext::new("foo\r\n\"bar", b"server-secret");
    let challenge = ctx.generate_challenge(1000)?;
    let header = challenge.to_header_value();

    // CRLF is stripped from quoted values to prevent header injection.
    assert!(!header.contains('\r'));
    assert!(!header.contains('\n'));
    // The embedded quote is escaped in the wire form and must round-trip.
    assert!(header.contains(r##"realm="foo\"bar""##));

    let response_value = format!(
        r##"Digest username="alice", realm="foo\"bar", nonce="{}", uri="sip:b@e", response="resp", algorithm="SHA-256""##,
        challenge.nonce
    );
    let parsed = DigestResponse::parse(&response_value)?;
    assert_eq!(parsed.realm, r##"foo"bar"##);
    Ok(())
}
