//! Validation, replay, and policy tests for digest authentication.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_gb28181_core::{
    DigestAlgorithm, DigestContext, DigestError, DigestQop, DigestReplayCache, Method,
};
use secrecy::SecretString;

mod digest_common;
use digest_common::*;

#[test]
fn no_qop_nonce_can_be_reused_within_ttl() -> Result<(), DigestError> {
    let ctx = ctx_md5().qop(None)?;
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
    let ctx = DigestContext::new("example.com", SERVER_SECRET)?
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
    let ctx = ctx_md5().qop(None)?;
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
    let ctx = DigestContext::new("example.com", SERVER_SECRET)?
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
fn qop_downgrade_is_rejected() -> Result<(), DigestError> {
    let ctx = ctx_md5();
    let challenge = ctx.generate_challenge(1000)?;
    // Server offered auth qop, but client omits it.
    let resp = make_response(
        "alice",
        "secret",
        "example.com",
        &challenge.nonce,
        "sip:registrar@example.com",
        &Method::Register,
        1,
        "clientnonce",
        None,
        DigestAlgorithm::Md5,
    );
    let mut cache = DigestReplayCache::new(16);
    let Err(err) = ctx.validate(
        &resp,
        &Method::Register,
        "sip:registrar@example.com",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    ) else {
        panic!("expected qop mismatch");
    };
    assert!(matches!(err, DigestError::InvalidQop));
    Ok(())
}
