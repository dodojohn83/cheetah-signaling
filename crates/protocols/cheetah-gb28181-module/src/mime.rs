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
    let lower = without_params.to_ascii_lowercase();
    match lower.as_str() {
        "application/manscdp+xml" | "application/xml" | "text/xml" => Ok(ContentType::Manscdp),
        "application/mansrtsp+xml" => Ok(ContentType::Mansrtsp),
        _ => {
            if profile.has(CompatibilityCapability::MimeAlias) {
                if MANSCDP_ALIASES.contains(&lower.as_str()) {
                    return Ok(ContentType::Manscdp);
                }
                if MANSRTSP_ALIASES.contains(&lower.as_str()) {
                    return Ok(ContentType::Mansrtsp);
                }
            }
            Err(AccessError::UnsupportedContentType(lower))
        }
    }
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
}
