//! Signed-nonce generation and validation for digest authentication.

use super::response::DigestError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// Generates a server-signed, time-stamped nonce.
///
/// The returned nonce is 64 hexadecimal characters: 16 for the timestamp, 16
/// for the per-challenge counter, and 32 for the HMAC-SHA256 tag (first 16
/// bytes). The counter ensures that multiple challenges issued within the same
/// second remain unique.
pub(super) fn generate_nonce(
    secret: &[u8],
    timestamp: u64,
    counter: u64,
) -> Result<String, DigestError> {
    let ts_bytes = timestamp.to_be_bytes();
    let counter_bytes = counter.to_be_bytes();
    let signature = nonce_signature(secret, &ts_bytes, &counter_bytes)?;
    let mut out = String::with_capacity(64);
    out.push_str(&hex::encode(ts_bytes));
    out.push_str(&hex::encode(counter_bytes));
    out.push_str(&hex::encode(&signature[..16]));
    Ok(out)
}

/// Validates a server-signed nonce and returns the embedded timestamp.
pub(super) fn validate_nonce(
    nonce: &str,
    secret: &[u8],
    now: u64,
    ttl: u64,
) -> Result<u64, DigestError> {
    if nonce.len() != 64 || !nonce.is_ascii() {
        return Err(DigestError::InvalidNonce);
    }
    let bytes = hex::decode(nonce).map_err(|_| DigestError::InvalidNonce)?;
    if bytes.len() != 32 {
        return Err(DigestError::InvalidNonce);
    }
    let ts_bytes = &bytes[..8];
    let counter_bytes = &bytes[8..16];
    let sig = &bytes[16..32];
    let ts_array: [u8; 8] = ts_bytes.try_into().map_err(|_| DigestError::InvalidNonce)?;
    let timestamp = u64::from_be_bytes(ts_array);

    let expected = nonce_signature(secret, ts_bytes, counter_bytes)?;
    if sig.ct_eq(&expected[..16]).unwrap_u8() == 0 {
        return Err(DigestError::InvalidNonce);
    }

    if now.saturating_sub(timestamp) > ttl {
        return Err(DigestError::StaleNonce);
    }

    Ok(timestamp)
}

fn nonce_signature(
    secret: &[u8],
    timestamp: &[u8],
    counter: &[u8],
) -> Result<Vec<u8>, DigestError> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| DigestError::InvalidNonce)?;
    mac.update(timestamp);
    mac.update(counter);
    Ok(mac.finalize().into_bytes().to_vec())
}
