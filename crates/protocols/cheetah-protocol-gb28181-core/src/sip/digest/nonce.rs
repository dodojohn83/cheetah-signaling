//! Signed-nonce generation and validation for digest authentication.

use super::response::DigestError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub(super) fn generate_nonce(secret: &[u8], timestamp: u64) -> Result<String, DigestError> {
    let ts_bytes = timestamp.to_be_bytes();
    let signature = nonce_signature(secret, &ts_bytes)?;
    let mut out = String::with_capacity(48);
    out.push_str(&hex::encode(ts_bytes));
    out.push_str(&hex::encode(&signature[..16]));
    Ok(out)
}

pub(super) fn validate_nonce(
    nonce: &str,
    secret: &[u8],
    now: u64,
    ttl: u64,
) -> Result<u64, DigestError> {
    if nonce.len() != 48 || !nonce.is_ascii() {
        return Err(DigestError::InvalidNonce);
    }
    let bytes = hex::decode(nonce).map_err(|_| DigestError::InvalidNonce)?;
    if bytes.len() != 24 {
        return Err(DigestError::InvalidNonce);
    }
    let (ts_bytes, sig) = bytes.split_at(8);
    let ts_array: [u8; 8] = ts_bytes.try_into().map_err(|_| DigestError::InvalidNonce)?;
    let timestamp = u64::from_be_bytes(ts_array);

    let expected = nonce_signature(secret, ts_bytes)?;
    if sig.ct_eq(&expected[..16]).unwrap_u8() == 0 {
        return Err(DigestError::InvalidNonce);
    }

    if now.saturating_sub(timestamp) > ttl {
        return Err(DigestError::StaleNonce);
    }

    Ok(timestamp)
}

fn nonce_signature(secret: &[u8], timestamp: &[u8]) -> Result<Vec<u8>, DigestError> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| DigestError::InvalidNonce)?;
    mac.update(timestamp);
    Ok(mac.finalize().into_bytes().to_vec())
}
