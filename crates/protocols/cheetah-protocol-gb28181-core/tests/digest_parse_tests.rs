//! Parsing, challenge encoding, and configuration tests for digest authentication.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_protocol_gb28181_core::{
    DigestAlgorithm, DigestContext, DigestError, DigestQop, DigestReplayCache, DigestResponse,
    Method,
};
use secrecy::SecretString;

mod digest_common;
use digest_common::*;

#[test]
fn challenge_defaults_to_sha256() -> Result<(), DigestError> {
    let ctx = DigestContext::new("example.com", SERVER_SECRET)?;
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
    let ctx = DigestContext::new("example.com", SERVER_SECRET)?;
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
fn parse_does_not_panic_on_non_ascii_boundary() {
    // "中" is 3 bytes, "ä" is 2 bytes. Byte index 7 falls inside the
    // two-byte character, which used to trigger a panic when the parser
    // sliced `value[..7]` without a char-boundary check.
    let value = "中中ä";
    assert!(DigestResponse::parse(value).is_err());
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
fn short_server_secret_is_rejected() {
    let Err(err) = DigestContext::new("example.com", b"short") else {
        panic!("expected short server secret to be rejected");
    };
    assert!(matches!(err, DigestError::WeakSecret));
}

#[test]
fn auth_int_qop_cannot_be_configured() {
    let ctx = DigestContext::new("example.com", SERVER_SECRET).unwrap();
    let Err(err) = ctx.qop(Some(DigestQop::AuthInt)) else {
        panic!("expected AuthInt qop to be rejected at configuration time");
    };
    assert!(matches!(err, DigestError::InvalidQop));
}

#[test]
fn challenge_header_round_trips_quoted_realm_and_strips_crlf() -> Result<(), DigestError> {
    let ctx = DigestContext::new("foo\r\n\"bar", SERVER_SECRET)?;
    let challenge = ctx.generate_challenge(1000)?;
    let header = challenge.to_header_value();

    // CRLF is stripped from quoted values to prevent header injection.
    assert!(!header.contains('\r'));
    assert!(!header.contains('\n'));
    // The embedded quote is escaped in the wire form and must round-trip.
    assert!(header.contains(r##"realm="foo\"bar""##));

    // The sanitized stored realm must match the parsed realm so validation
    // succeeds for clients that echo the challenge realm.
    let resp = make_response(
        "alice",
        "secret",
        r##"foo"bar"##,
        &challenge.nonce,
        "sip:b@e",
        &Method::Register,
        1,
        "cn",
        Some(DigestQop::Auth),
        DigestAlgorithm::Sha256,
    );

    let response_value = format!(
        r##"Digest username="alice", realm="foo\"bar", nonce="{}", uri="sip:b@e", response="{}", cnonce="cn", nc="00000001", qop="auth", algorithm="SHA-256""##,
        challenge.nonce, resp.response
    );
    let parsed = DigestResponse::parse(&response_value)?;
    assert_eq!(parsed.realm, r##"foo"bar"##);

    let mut cache = DigestReplayCache::new(64);
    ctx.validate(
        &parsed,
        &Method::Register,
        "sip:b@e",
        &SecretString::from("secret"),
        &mut cache,
        1000,
    )
}
