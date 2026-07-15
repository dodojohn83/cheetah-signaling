//! Shared helpers for digest authentication integration tests.

#![allow(clippy::too_many_arguments, dead_code, unused_imports)]

use cheetah_protocol_gb28181_core::{
    DigestAlgorithm, DigestContext, DigestError, DigestQop, DigestResponse, Method,
};
use sha2::{Digest, Sha256, Sha512};

pub const SERVER_SECRET: &[u8] = b"server-secret-must-be-32-bytes-long";

pub fn make_response(
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

pub fn compute_client_response(
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

pub fn hash_hex(algorithm: DigestAlgorithm, data: &[u8]) -> String {
    match algorithm {
        DigestAlgorithm::Md5 => format!("{:x}", md5::compute(data)),
        DigestAlgorithm::Sha256 => hex::encode(Sha256::digest(data)),
        DigestAlgorithm::Sha512 => hex::encode(Sha512::digest(data)),
    }
}

pub fn ctx_md5() -> DigestContext {
    DigestContext::new("example.com", SERVER_SECRET)
        .unwrap()
        .allow_md5(true)
        .preferred_algorithm(DigestAlgorithm::Md5)
}
