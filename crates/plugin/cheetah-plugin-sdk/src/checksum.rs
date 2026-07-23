//! Manifest integrity helpers.
//!
//! The SDK supports SHA-256 and HMAC-SHA256 digests for validating a
//! manifest payload. Signature verification with asymmetric keys is intentionally
//! out of scope for the SDK core and is the responsibility of the host loader.

use crate::error::PluginError;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Maximum byte length of a checksum algorithm name.
pub(crate) const MAX_ALGORITHM_BYTES: usize = 32;
/// Maximum byte length of a hex digest string.
pub(crate) const MAX_DIGEST_HEX_BYTES: usize = 256;

type HmacSha256 = Hmac<Sha256>;

/// Truncates `s` at a UTF-8 character boundary so it is at most `max` bytes.
fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut idx = max;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

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
    if algorithm.len() > MAX_ALGORITHM_BYTES {
        return Err(PluginError::InvalidManifest(format!(
            "checksum algorithm exceeds {MAX_ALGORITHM_BYTES} bytes"
        )));
    }
    if expected_digest_hex.len() > MAX_DIGEST_HEX_BYTES {
        return Err(PluginError::InvalidChecksum);
    }

    let computed = if algorithm.eq_ignore_ascii_case("sha256") {
        let mut hasher = Sha256::new();
        hasher.update(payload);
        hex::encode(hasher.finalize())
    } else if algorithm.eq_ignore_ascii_case("hmac-sha256") {
        let mut mac = HmacSha256::new_from_slice(secret)
            .map_err(|e| PluginError::InvalidManifest(format!("invalid HMAC key: {e}")))?;
        mac.update(payload);
        hex::encode(mac.finalize().into_bytes())
    } else {
        let display = truncate_at_char_boundary(algorithm, MAX_ALGORITHM_BYTES);
        return Err(PluginError::InvalidManifest(format!(
            "unsupported checksum algorithm: {display}"
        )));
    };

    if constant_time_eq_ignore_case(&computed, expected_digest_hex) {
        Ok(())
    } else {
        Err(PluginError::InvalidChecksum)
    }
}

/// Case-insensitive constant-time equality.
///
/// `expected` may contain uppercase or lowercase hex digits; `computed` is
/// already lowercase. No temporary lowercase copy is allocated.
fn constant_time_eq_ignore_case(computed: &str, expected: &str) -> bool {
    if computed.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in computed.bytes().zip(expected.bytes()) {
        diff |= x ^ y.to_ascii_lowercase();
    }
    diff == 0
}
