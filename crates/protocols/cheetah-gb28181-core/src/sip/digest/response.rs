//! Parsed `Authorization` digest response.

use secrecy::zeroize::Zeroizing;
use std::fmt;

use super::types::{
    DigestAlgorithm, DigestError, DigestQop, split_commas, unquote, write_quoted_param,
    write_raw_param,
};

/// Parsed `Authorization` Digest response.
#[derive(Clone, Eq, PartialEq)]
pub struct DigestResponse {
    /// username quoted in the response.
    pub username: String,
    /// realm quoted in the response.
    pub realm: String,
    /// nonce from the challenge.
    pub nonce: String,
    /// URI used in the A2 computation.
    pub uri: String,
    /// Client-computed response digest.
    pub response: String,
    /// Client nonce, required when qop is present.
    pub cnonce: Option<String>,
    /// Nonce count, required when qop is present.
    pub nc: Option<u64>,
    /// QoP value from the response.
    pub qop: Option<DigestQop>,
    /// Algorithm used by the client.
    pub algorithm: Option<DigestAlgorithm>,
    /// Opaque value echoed from the challenge.
    pub opaque: Option<String>,
}

impl fmt::Debug for DigestResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestResponse")
            .field("username", &"[REDACTED]")
            .field("realm", &self.realm)
            .field("nonce", &"[REDACTED]")
            .field("uri", &"[REDACTED]")
            .field("response", &"[REDACTED]")
            .field("cnonce", &self.cnonce.as_ref().map(|_| "[REDACTED]"))
            .field("nc", &self.nc)
            .field("qop", &self.qop)
            .field("algorithm", &self.algorithm)
            .field("opaque", &"[REDACTED]")
            .finish()
    }
}

impl DigestResponse {
    /// Default maximum length for a `Digest` `Authorization` header value.
    pub const DEFAULT_MAX_HEADER_LEN: usize = 2048;

    /// Parses a `Digest ...` `Authorization` header value.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        Self::parse_with_limit(value, Self::DEFAULT_MAX_HEADER_LEN)
    }

    /// Parses a `Digest ...` `Authorization` header value, rejecting inputs
    /// longer than `max_len` bytes.
    pub fn parse_with_limit(value: &str, max_len: usize) -> Result<Self, DigestError> {
        if value.len() > max_len {
            return Err(DigestError::Malformed(format!(
                "digest header exceeds maximum length of {max_len}"
            )));
        }
        let value = value.trim();
        let value = if value.len() > 7
            && value.is_char_boundary(7)
            && value[..7].eq_ignore_ascii_case("digest ")
        {
            &value[7..]
        } else {
            value
        };

        let mut username = None;
        let mut realm = None;
        let mut nonce = None;
        let mut uri = None;
        let mut response = None;
        let mut cnonce = None;
        let mut nc = None;
        let mut qop = None;
        let mut algorithm = None;
        let mut opaque = None;

        for part in split_commas(value) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some(eq) = part.find('=') else {
                return Err(DigestError::Malformed(
                    "missing '=' in digest parameter".to_string(),
                ));
            };
            let key = part[..eq].trim().to_ascii_lowercase();
            let raw = part[eq + 1..].trim();
            let value = unquote(raw);

            match key.as_str() {
                "username" => username = Some(value.into_owned()),
                "realm" => realm = Some(value.into_owned()),
                "nonce" => nonce = Some(value.into_owned()),
                "uri" => uri = Some(value.into_owned()),
                "response" => response = Some(value.into_owned()),
                "cnonce" => cnonce = Some(value.into_owned()),
                "nc" => {
                    nc = Some(
                        u64::from_str_radix(value.as_ref(), 16)
                            .map_err(|_| DigestError::Malformed("bad nc value".to_string()))?,
                    );
                }
                "qop" => {
                    qop = DigestQop::parse(value.as_ref())
                        .map(Some)
                        .ok_or(DigestError::InvalidQop)?;
                }
                "algorithm" => {
                    algorithm = Some(
                        DigestAlgorithm::parse(value.as_ref())
                            .ok_or(DigestError::UnknownAlgorithm)?,
                    );
                }
                "opaque" => opaque = Some(value.into_owned()),
                _ => {}
            }
        }

        Ok(Self {
            username: username
                .ok_or_else(|| DigestError::Malformed("missing username".to_string()))?,
            realm: realm.ok_or_else(|| DigestError::Malformed("missing realm".to_string()))?,
            nonce: nonce.ok_or_else(|| DigestError::Malformed("missing nonce".to_string()))?,
            uri: uri.ok_or_else(|| DigestError::Malformed("missing uri".to_string()))?,
            response: response
                .ok_or_else(|| DigestError::Malformed("missing response".to_string()))?,
            cnonce,
            nc,
            qop,
            algorithm,
            opaque,
        })
    }

    /// Computes the expected `response` digest value for the supplied
    /// parameters. Useful for tests and clients that need to build an
    /// `Authorization` header.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_response(
        algorithm: DigestAlgorithm,
        username: &str,
        realm: &str,
        password: &str,
        nonce: &str,
        nc: u64,
        cnonce: Option<&str>,
        qop: Option<DigestQop>,
        method: &str,
        uri: &str,
    ) -> String {
        let a1 = Zeroizing::new(format!("{username}:{realm}:{password}"));
        let ha1 = Zeroizing::new(algorithm.hash_hex(a1.as_bytes()));

        let a2 = format!("{method}:{uri}");
        let ha2 = algorithm.hash_hex(a2.as_bytes());

        let a3: Zeroizing<String> = match qop {
            Some(DigestQop::Auth) => {
                let cnonce = cnonce.unwrap_or("");
                let nc = format!("{nc:08x}");
                Zeroizing::new(format!("{}:{nonce}:{nc}:{cnonce}:auth:{ha2}", ha1.as_str()))
            }
            _ => Zeroizing::new(format!("{}:{nonce}:{ha2}", ha1.as_str())),
        };

        algorithm.hash_hex(a3.as_bytes())
    }

    /// Encodes this response as the value of an `Authorization` header.
    pub fn to_header_value(&self) -> String {
        let mut out = String::from("Digest");
        let mut first = true;
        write_quoted_param(&mut out, &mut first, "username", &self.username);
        write_quoted_param(&mut out, &mut first, "realm", &self.realm);
        write_quoted_param(&mut out, &mut first, "nonce", &self.nonce);
        write_quoted_param(&mut out, &mut first, "uri", &self.uri);
        if let Some(algorithm) = self.algorithm {
            write_raw_param(&mut out, &mut first, "algorithm", algorithm.as_wire());
        }
        write_quoted_param(&mut out, &mut first, "response", &self.response);
        if let Some(cnonce) = &self.cnonce {
            write_quoted_param(&mut out, &mut first, "cnonce", cnonce);
        }
        if let Some(nc) = self.nc {
            write_raw_param(&mut out, &mut first, "nc", &format!("{nc:08x}"));
        }
        if let Some(qop) = self.qop {
            write_raw_param(&mut out, &mut first, "qop", qop.as_wire());
        }
        if let Some(opaque) = &self.opaque {
            write_quoted_param(&mut out, &mut first, "opaque", opaque);
        }
        out
    }
}
