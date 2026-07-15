//! SIP URI model and string-safe parsing.

use super::error::{SipError, SipErrorKind};

/// SIP scheme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scheme {
    /// Unencrypted SIP.
    Sip,
    /// TLS-encrypted SIP.
    Sips,
}

impl std::fmt::Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scheme::Sip => write!(f, "sip"),
            Scheme::Sips => write!(f, "sips"),
        }
    }
}

/// A SIP URI per RFC 3261 with parameter storage.
///
/// The `password` component is never exposed by `Debug`; wire encoding is
/// available only through the explicit [`SipUri::encode`] method.
#[derive(Clone, Eq, PartialEq)]
pub struct SipUri {
    scheme: Scheme,
    user: Option<String>,
    password: Option<String>,
    host: String,
    port: Option<u16>,
    parameters: Vec<(String, Option<String>)>,
}

impl SipUri {
    /// Parses a SIP URI from text.
    ///
    /// # Errors
    ///
    /// Returns `SipErrorKind::InvalidUri` when the value is not a valid SIP URI.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, SipError> {
        let raw = raw.as_ref();
        Self::parse_inner(raw)
    }

    fn parse_inner(raw: &str) -> Result<Self, SipError> {
        if raw.len() > 2048 {
            return Err(SipError::new(
                SipErrorKind::InvalidUri,
                None,
                "URI too long",
            ));
        }

        let (scheme, rest) = raw
            .split_once(':')
            .ok_or_else(|| SipError::new(SipErrorKind::InvalidUri, None, "missing scheme"))?;

        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "sip" => Scheme::Sip,
            "sips" => Scheme::Sips,
            _ => {
                return Err(SipError::new(
                    SipErrorKind::InvalidUri,
                    None,
                    "unsupported scheme",
                ));
            }
        };

        let (userinfo, hostport) = rest.rsplit_once('@').map_or((None, rest), |(u, h)| {
            if u.is_empty() {
                (None, h)
            } else {
                (Some(u), h)
            }
        });

        let (user, password) = userinfo.map_or((None, None), |info| match info.split_once(':') {
            Some((u, p)) if !u.is_empty() => (Some(u), Some(p)),
            _ => (Some(info), None),
        });

        let (hostport, param_part) = hostport
            .split_once(';')
            .map_or((hostport, None), |(hp, p)| (hp, Some(p)));

        let (host, port) = if hostport.starts_with('[') {
            // IPv6 reference [host]:port
            let end = hostport.find(']').ok_or_else(|| {
                SipError::new(SipErrorKind::InvalidUri, None, "unterminated IPv6")
            })?;
            let host = &hostport[..=end];
            let port = if hostport.len() > end + 1 && hostport.as_bytes()[end + 1] == b':' {
                Some(&hostport[end + 2..])
            } else {
                None
            };
            (host, port)
        } else {
            hostport
                .rsplit_once(':')
                .map_or((hostport, None), |(h, p)| (h, Some(p)))
        };

        let host = host.trim();
        if host.is_empty() {
            return Err(SipError::new(SipErrorKind::InvalidUri, None, "empty host"));
        }

        let port = port
            .map(|p| {
                p.parse::<u16>()
                    .map_err(|_| SipError::new(SipErrorKind::InvalidUri, None, "invalid port"))
            })
            .transpose()?;

        let mut parameters = Vec::new();
        if let Some(param_part) = param_part {
            for token in param_part.split(';') {
                if token.is_empty() {
                    continue;
                }
                let (k, v) = token.split_once('=').map_or((token, None), |(k, v)| {
                    (k, if v.is_empty() { None } else { Some(v) })
                });
                parameters.push((k.to_string(), v.map(String::from)));
            }
        }

        Ok(Self {
            scheme,
            user: user.map(String::from),
            password: password.map(String::from),
            host: host.to_string(),
            port,
            parameters,
        })
    }

    /// URI scheme.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// Optional user part. Does not include the password.
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Returns true if the URI contained a password.
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Host; may be an IPv4/IPv6 address or domain name.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Optional port.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// URI parameters as key-value pairs. `None` value indicates flag parameter.
    pub fn parameters(&self) -> &[(String, Option<String>)] {
        &self.parameters
    }

    /// Encodes the URI to wire form.
    ///
    /// # Warning
    ///
    /// This includes any password that was present in the original URI.
    pub fn encode(&self) -> String {
        let mut out = format!("{}:", self.scheme);
        if let Some(user) = &self.user {
            out.push_str(user);
            if let Some(password) = &self.password {
                out.push(':');
                out.push_str(password);
            }
            out.push('@');
        }
        out.push_str(&self.host);
        if let Some(port) = self.port {
            out.push(':');
            out.push_str(&port.to_string());
        }
        for (k, v) in &self.parameters {
            out.push(';');
            out.push_str(k);
            if let Some(v) = v {
                out.push('=');
                out.push_str(v);
            }
        }
        out
    }
}

impl std::fmt::Debug for SipUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SipUri")
            .field("scheme", &self.scheme)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("host", &self.host)
            .field("port", &self.port)
            .field("parameters", &self.parameters)
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple_uri() {
        let uri = SipUri::parse("sip:alice@example.com:5060;transport=udp").unwrap();
        assert_eq!(uri.scheme(), Scheme::Sip);
        assert_eq!(uri.user(), Some("alice"));
        assert!(!uri.has_password());
        assert_eq!(uri.host(), "example.com");
        assert_eq!(uri.port(), Some(5060));
        assert_eq!(uri.parameters().len(), 1);
        assert_eq!(uri.encode(), "sip:alice@example.com:5060;transport=udp");
    }

    #[test]
    fn password_is_split_and_round_tripped_but_redacted_in_debug() {
        let uri = SipUri::parse("sip:alice:secret@example.com:5060").unwrap();
        assert_eq!(uri.user(), Some("alice"));
        assert!(uri.has_password());
        assert_eq!(uri.encode(), "sip:alice:secret@example.com:5060");

        let debug = format!("{uri:?}");
        assert!(!debug.contains("secret"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(SipUri::parse("alice@example.com").is_err());
    }
}
