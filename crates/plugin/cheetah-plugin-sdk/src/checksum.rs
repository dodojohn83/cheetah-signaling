//! Manifest integrity helpers.
//!
//! The SDK supports SHA-256 and HMAC-SHA256 digests for validating a
//! manifest payload. Signature verification with asymmetric keys is intentionally
//! out of scope for the SDK core and is the responsibility of the host loader.

use crate::error::PluginError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Verifies a manifest payload against a declared checksum.
///
/// For `hmac-sha256` the `secret` must be provided; for `sha256` it is ignored
/// and may be empty.
pub fn verify_manifest_checksum(
    payload: &[u8],
    algorithm: &str,
    expected_digest_hex: &str,
    secret: &[u8],
) -> Result<(), PluginError> {
    let computed = match algorithm {
        "sha256" => {
            let mut hasher = Sha256::new();
            hasher.update(payload);
            hex::encode(hasher.finalize())
        }
        "hmac-sha256" => {
            let mut mac = HmacSha256::new_from_slice(secret)
                .map_err(|e| PluginError::InvalidManifest(format!("invalid HMAC key: {e}")))?;
            mac.update(payload);
            hex::encode(mac.finalize().into_bytes())
        }
        other => {
            return Err(PluginError::InvalidManifest(format!(
                "unsupported checksum algorithm: {other}"
            )));
        }
    };

    if constant_time_eq(&computed, &expected_digest_hex.to_ascii_lowercase()) {
        Ok(())
    } else {
        Err(PluginError::InvalidChecksum)
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}
