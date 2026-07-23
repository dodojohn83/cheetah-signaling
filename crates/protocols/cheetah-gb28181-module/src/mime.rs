//! Vendor Content-Type alias resolution for GB28181 SIP MESSAGE bodies.

use crate::error::AccessError;
use cheetah_domain::{CompatibilityCapability, CompatibilityProfile};

/// Recognized GB28181 XML payload content families.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContentType {
    /// MANSCDP command/response payload (GB/T 28181).
    Manscdp,
    /// MANSRTSP stream-control payload.
    Mansrtsp,
}

const MANSCDP_CANONICAL: &str = "application/manscdp+xml";

const MANSCDP_ALIASES: &[&str] = &[
    "application/kslp+xml",
    "application/ksdu+xml",
    "application/kssp+xml",
    "application/klcp+xml",
];

const MANSRTSP_ALIASES: &[&str] = &["application/ksptz+xml"];

/// Maximum byte length of a Content-Type media type before its parameters are stripped.
const MAX_CONTENT_TYPE_BYTES: usize = 256;

/// Maximum characters from an unrecognized Content-Type to include in an error message.
const MAX_CONTENT_TYPE_ERROR_CHARS: usize = 64;

fn is_manscdp(without_params: &str) -> bool {
    without_params.eq_ignore_ascii_case(MANSCDP_CANONICAL)
        || without_params.eq_ignore_ascii_case("application/xml")
        || without_params.eq_ignore_ascii_case("text/xml")
}

fn is_mansrtsp(without_params: &str) -> bool {
    without_params.eq_ignore_ascii_case("application/mansrtsp+xml")
}

fn is_manscdp_alias(without_params: &str) -> bool {
    MANSCDP_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(without_params))
}

fn is_mansrtsp_alias(without_params: &str) -> bool {
    MANSRTSP_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(without_params))
}

fn truncate_content_type_error(s: &str) -> String {
    s.chars().take(MAX_CONTENT_TYPE_ERROR_CHARS).collect()
}

/// Resolves a raw Content-Type header value to a canonical content family.
///
/// Missing or empty values default to MANSCDP. Vendor aliases are only accepted
/// when [`CompatibilityCapability::MimeAlias`] is enabled in `profile`; without
/// the capability only the canonical forms are accepted.
pub(crate) fn resolve_vendor_content_type(
    raw: Option<&str>,
    profile: &CompatibilityProfile,
) -> Result<ContentType, AccessError> {
    let raw = raw.unwrap_or(MANSCDP_CANONICAL).trim();
    if raw.is_empty() {
        return Ok(ContentType::Manscdp);
    }
    let without_params = raw.split(';').next().unwrap_or(raw).trim();
    if without_params.len() > MAX_CONTENT_TYPE_BYTES {
        return Err(AccessError::UnsupportedContentType(
            truncate_content_type_error(without_params),
        ));
    }

    if is_manscdp(without_params) {
        return Ok(ContentType::Manscdp);
    }
    if is_mansrtsp(without_params) {
        return Ok(ContentType::Mansrtsp);
    }

    if profile.has(CompatibilityCapability::MimeAlias) {
        if is_manscdp_alias(without_params) {
            return Ok(ContentType::Manscdp);
        }
        if is_mansrtsp_alias(without_params) {
            return Ok(ContentType::Mansrtsp);
        }
    }

    Err(AccessError::UnsupportedContentType(
        truncate_content_type_error(without_params),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn canonical_content_types_resolve_without_profile() {
        let default = CompatibilityProfile::default();
        for ct in [
            "application/manscdp+xml",
            "application/xml",
            "text/xml",
            "application/xml; charset=GBK",
        ] {
            assert_eq!(
                resolve_vendor_content_type(Some(ct), &default).unwrap(),
                ContentType::Manscdp,
                "{ct} should default to MANSCDP"
            );
        }
        assert_eq!(
            resolve_vendor_content_type(Some("application/mansrtsp+xml"), &default).unwrap(),
            ContentType::Mansrtsp
        );
        assert_eq!(
            resolve_vendor_content_type(None, &default).unwrap(),
            ContentType::Manscdp
        );
    }

    #[test]
    fn vendor_aliases_require_capability() {
        let default = CompatibilityProfile::default();
        assert!(resolve_vendor_content_type(Some("application/kslp+xml"), &default).is_err());

        let profile = CompatibilityProfile {
            capabilities: vec![CompatibilityCapability::MimeAlias],
            ..Default::default()
        };
        assert_eq!(
            resolve_vendor_content_type(Some("application/kslp+xml"), &profile).unwrap(),
            ContentType::Manscdp
        );
        assert_eq!(
            resolve_vendor_content_type(Some("application/ksptz+xml"), &profile).unwrap(),
            ContentType::Mansrtsp
        );
    }

    #[test]
    fn content_type_resolution_is_case_insensitive() {
        let default = CompatibilityProfile::default();
        assert_eq!(
            resolve_vendor_content_type(Some("APPLICATION/MANSCDP+XML"), &default).unwrap(),
            ContentType::Manscdp
        );
        assert_eq!(
            resolve_vendor_content_type(Some("Application/Xml; charset=GBK"), &default).unwrap(),
            ContentType::Manscdp
        );
        assert_eq!(
            resolve_vendor_content_type(Some("APPLICATION/MANSRTSP+XML"), &default).unwrap(),
            ContentType::Mansrtsp
        );
    }

    #[test]
    fn oversized_content_type_is_rejected() {
        let default = CompatibilityProfile::default();
        let long = "x".repeat(MAX_CONTENT_TYPE_BYTES + 1);
        let ct = format!("{long}/xml");
        let err = resolve_vendor_content_type(Some(&ct), &default).unwrap_err();
        match err {
            AccessError::UnsupportedContentType(msg) => {
                assert!(
                    msg.len() <= MAX_CONTENT_TYPE_ERROR_CHARS,
                    "error message should be bounded"
                );
            }
            other => panic!("expected UnsupportedContentType, got {other:?}"),
        }
    }
}
