//! Server-generated `WWW-Authenticate` digest challenge.

use std::fmt;

use super::types::{
    DigestAlgorithm, DigestError, DigestQop, split_commas, unquote, write_quoted_param,
    write_raw_param,
};

/// A server-generated `WWW-Authenticate` challenge.
#[derive(Clone, Eq, PartialEq)]
pub struct DigestChallenge {
    /// Authentication realm.
    pub realm: String,
    /// Signed nonce.
    pub nonce: String,
    /// Opaque value echoed by the client.
    pub opaque: Option<String>,
    /// Whether the previously supplied nonce was stale.
    pub stale: bool,
    /// Algorithm to use.
    pub algorithm: DigestAlgorithm,
    /// QoP offered.
    pub qop: Option<DigestQop>,
}

impl fmt::Debug for DigestChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestChallenge")
            .field("realm", &self.realm)
            .field("nonce", &"[REDACTED]")
            .field("opaque", &self.opaque.as_ref().map(|_| "[REDACTED]"))
            .field("stale", &self.stale)
            .field("algorithm", &self.algorithm)
            .field("qop", &self.qop)
            .finish()
    }
}

impl DigestChallenge {
    /// Parses a `WWW-Authenticate` challenge value.
    ///
    /// The leading `Digest ` token is optional.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        let value = value.trim();
        let value = if value.len() > 7
            && value.is_char_boundary(7)
            && value[..7].eq_ignore_ascii_case("digest ")
        {
            &value[7..]
        } else {
            value
        };

        let mut realm = None;
        let mut nonce = None;
        let mut algorithm = None;
        let mut qop = None;
        let mut stale = false;
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
                "realm" => realm = Some(value.into_owned()),
                "nonce" => nonce = Some(value.into_owned()),
                "algorithm" => {
                    algorithm = DigestAlgorithm::parse(value.as_ref());
                }
                "qop" => {
                    qop = DigestQop::parse(value.as_ref());
                }
                "stale" => stale = value.eq_ignore_ascii_case("true"),
                "opaque" => opaque = Some(value.into_owned()),
                _ => {}
            }
        }

        Ok(Self {
            realm: realm.ok_or_else(|| DigestError::Malformed("missing realm".to_string()))?,
            nonce: nonce.ok_or_else(|| DigestError::Malformed("missing nonce".to_string()))?,
            opaque,
            stale,
            algorithm: algorithm.ok_or(DigestError::UnknownAlgorithm)?,
            qop,
        })
    }

    /// Encodes the challenge as the value of a `WWW-Authenticate` header.
    pub fn to_header_value(&self) -> String {
        let mut out = String::from("Digest");
        let mut first = true;
        write_quoted_param(&mut out, &mut first, "realm", &self.realm);
        write_quoted_param(&mut out, &mut first, "nonce", &self.nonce);
        write_raw_param(&mut out, &mut first, "algorithm", self.algorithm.as_wire());
        if let Some(opaque) = &self.opaque {
            write_quoted_param(&mut out, &mut first, "opaque", opaque);
        }
        if self.stale {
            write_raw_param(&mut out, &mut first, "stale", "true");
        }
        if let Some(qop) = self.qop {
            write_quoted_param(&mut out, &mut first, "qop", qop.as_wire());
        }
        out
    }
}
