//! WS-Discovery XML parser with configurable limits.

use super::types::{
    Bye, Hello, MetadataVersion, ProbeMatch, ProbeMatches, ResolveMatch, ResolveMatches, Scopes,
    XAddrs, validate_epr,
};
use super::{DiscoveryLimits, LimitTracker, check_datagram_size};
use crate::error::{OnvifError, OnvifResult};
use quick_xml::Reader;
use quick_xml::events::Event;

const WSD_PROBE_MATCH: &str = "ProbeMatch";
const WSD_RESOLVE_MATCH: &str = "ResolveMatch";
const WSA_ENDPOINT_REFERENCE: &str = "EndpointReference";
const WSA_ADDRESS: &str = "Address";
const WSD_TYPES: &str = "Types";
const WSD_SCOPES: &str = "Scopes";
const WSD_X_ADDRS: &str = "XAddrs";
const WSD_METADATA_VERSION: &str = "MetadataVersion";
const WSA_RELATES_TO: &str = "RelatesTo";
const WSD_HELLO: &str = "Hello";
const WSD_BYE: &str = "Bye";

pub fn parse_probe_matches(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<ProbeMatches> {
    check_datagram_size(xml, limits)?;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut tracker = LimitTracker::new(*limits);
    let mut relates_to: Option<String> = None;
    let mut matches: Vec<ProbeMatch> = Vec::new();

    let mut current: Option<ProbeMatchBuilder> = None;
    let mut in_epr = false;
    let mut epr_text: Option<String> = None;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                tracker.start()?;
                let name = local_name(&e.name());
                if name == WSD_PROBE_MATCH {
                    current = Some(ProbeMatchBuilder::default());
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_ENDPOINT_REFERENCE && current.is_some() {
                    in_epr = true;
                    epr_text = None;
                }
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                tracker.empty()?;
                let name = local_name(&e.name());
                if name == WSD_PROBE_MATCH {
                    current = Some(ProbeMatchBuilder::default());
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_ENDPOINT_REFERENCE && current.is_some() {
                    in_epr = true;
                    epr_text = None;
                } else if name == WSA_ADDRESS && in_epr {
                    // Empty <Address/> inside EndpointReference: explicit empty EPR.
                    epr_text = Some(String::new());
                }
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                tracker.end();
                let name = local_name(&e.name());

                if name == WSA_ADDRESS && in_epr {
                    epr_text = Some(text.trim().to_string());
                } else if name == WSA_ENDPOINT_REFERENCE {
                    if let Some(ref mut b) = current {
                        let epr = epr_text.take().unwrap_or_else(|| text.trim().to_string());
                        b.set_endpoint_reference(&epr);
                    }
                    in_epr = false;
                } else if name == WSD_PROBE_MATCH {
                    if let Some(builder) = current.take() {
                        match builder.build(discovered_at) {
                            Ok(m) => {
                                if matches.len() >= limits.max_matches {
                                    return Err(OnvifError::limit_exceeded(
                                        "max probe matches".to_string(),
                                    ));
                                }
                                matches.push(m);
                            }
                            Err(e) => {
                                tracing::warn!("skipping malformed ProbeMatch: {e}");
                            }
                        }
                    }
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_RELATES_TO {
                    relates_to = Some(text.trim().to_string());
                } else if let Some(ref mut b) = current {
                    b.apply(&name, &text);
                }

                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(OnvifError::xml(e)),
            _ => {}
        }
    }

    Ok(ProbeMatches {
        relates_to: relates_to.unwrap_or_default(),
        matches,
    })
}

pub fn parse_hello_bye(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<super::EitherHelloBye> {
    check_datagram_size(xml, limits)?;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut tracker = LimitTracker::new(*limits);
    let mut kind: Option<&'static str> = None;
    let mut builder = HelloByeBuilder::default();
    let mut in_epr = false;
    let mut epr_text: Option<String> = None;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                tracker.start()?;
                let name = local_name(&e.name());
                if kind.is_none() && (name == WSD_HELLO || name == WSD_BYE) {
                    kind = Some(if name == WSD_HELLO { "hello" } else { "bye" });
                } else if name == WSA_ENDPOINT_REFERENCE {
                    in_epr = true;
                    epr_text = None;
                }
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                tracker.empty()?;
                let name = local_name(&e.name());
                if kind.is_none() && (name == WSD_HELLO || name == WSD_BYE) {
                    kind = Some(if name == WSD_HELLO { "hello" } else { "bye" });
                } else if name == WSA_ENDPOINT_REFERENCE {
                    in_epr = true;
                    epr_text = None;
                } else if name == WSA_ADDRESS && in_epr {
                    epr_text = Some(String::new());
                }
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                tracker.end();
                let name = local_name(&e.name());

                if name == WSD_HELLO || name == WSD_BYE {
                    break;
                }

                if name == WSA_ADDRESS && in_epr {
                    epr_text = Some(text.trim().to_string());
                } else if name == WSA_ENDPOINT_REFERENCE {
                    let epr = epr_text.take().unwrap_or_else(|| text.trim().to_string());
                    builder.set_endpoint_reference(&epr);
                    in_epr = false;
                } else {
                    builder.apply(&name, &text);
                }

                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(OnvifError::xml(e)),
            _ => {}
        }
    }

    match kind {
        Some("hello") => Ok(super::EitherHelloBye::Hello(
            builder.build_hello(discovered_at)?,
        )),
        Some("bye") => Ok(super::EitherHelloBye::Bye(
            builder.build_bye(discovered_at)?,
        )),
        _ => Err(OnvifError::missing_field("Hello or Bye".to_string())),
    }
}

pub fn parse_resolve_matches(
    xml: &str,
    discovered_at: u64,
    limits: &DiscoveryLimits,
) -> OnvifResult<ResolveMatches> {
    check_datagram_size(xml, limits)?;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut tracker = LimitTracker::new(*limits);
    let mut relates_to: Option<String> = None;
    let mut matches: Vec<ResolveMatch> = Vec::new();

    let mut current: Option<ResolveMatchBuilder> = None;
    let mut in_epr = false;
    let mut epr_text: Option<String> = None;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                tracker.start()?;
                let name = local_name(&e.name());
                if name == WSD_RESOLVE_MATCH {
                    current = Some(ResolveMatchBuilder::default());
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_ENDPOINT_REFERENCE && current.is_some() {
                    in_epr = true;
                    epr_text = None;
                }
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                tracker.empty()?;
                let name = local_name(&e.name());
                if name == WSD_RESOLVE_MATCH {
                    current = Some(ResolveMatchBuilder::default());
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_ENDPOINT_REFERENCE && current.is_some() {
                    in_epr = true;
                    epr_text = None;
                } else if name == WSA_ADDRESS && in_epr {
                    epr_text = Some(String::new());
                }
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                tracker.end();
                let name = local_name(&e.name());

                if name == WSA_ADDRESS && in_epr {
                    epr_text = Some(text.trim().to_string());
                } else if name == WSA_ENDPOINT_REFERENCE {
                    if let Some(ref mut b) = current {
                        let epr = epr_text.take().unwrap_or_else(|| text.trim().to_string());
                        b.set_endpoint_reference(&epr);
                    }
                    in_epr = false;
                } else if name == WSD_RESOLVE_MATCH {
                    if let Some(builder) = current.take() {
                        match builder.build(discovered_at) {
                            Ok(m) => {
                                if matches.len() >= limits.max_matches {
                                    return Err(OnvifError::limit_exceeded(
                                        "max resolve matches".to_string(),
                                    ));
                                }
                                matches.push(m);
                            }
                            Err(e) => {
                                tracing::warn!("skipping malformed ResolveMatch: {e}");
                            }
                        }
                    }
                    in_epr = false;
                    epr_text = None;
                } else if name == WSA_RELATES_TO {
                    relates_to = Some(text.trim().to_string());
                } else if let Some(ref mut b) = current {
                    b.apply(&name, &text);
                }

                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(OnvifError::xml(e)),
            _ => {}
        }
    }

    Ok(ResolveMatches {
        relates_to: relates_to.unwrap_or_default(),
        matches,
    })
}

fn local_name(name: &quick_xml::name::QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

#[derive(Default)]
struct ProbeMatchBuilder {
    endpoint_reference: Option<String>,
    types: Vec<String>,
    scopes: Option<String>,
    x_addrs: Option<String>,
    metadata_version: Option<MetadataVersion>,
    error: Option<OnvifError>,
}

impl ProbeMatchBuilder {
    fn set_endpoint_reference(&mut self, epr: &str) {
        if self.error.is_some() {
            return;
        }
        match validate_epr(epr) {
            Ok(clean) => self.endpoint_reference = Some(clean),
            Err(e) => self.error = Some(e),
        }
    }

    fn apply(&mut self, name: &str, text: &str) {
        if self.error.is_some() {
            return;
        }
        match name {
            WSD_TYPES => self.types = text.split_whitespace().map(|s| s.to_string()).collect(),
            WSD_SCOPES => self.scopes = Some(text.to_string()),
            WSD_X_ADDRS => self.x_addrs = Some(text.to_string()),
            WSD_METADATA_VERSION => match parse_optional_metadata_version(text) {
                Ok(v) => self.metadata_version = v,
                Err(e) => self.error = Some(e),
            },
            _ => {}
        }
    }

    fn build(self, discovered_at: u64) -> OnvifResult<ProbeMatch> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let epr = validate_epr(&self.endpoint_reference.unwrap_or_default())?;
        Ok(ProbeMatch {
            endpoint_reference: super::types::EndpointReference(epr),
            types: self.types,
            scopes: self
                .scopes
                .as_ref()
                .map(|s| Scopes(s.split_whitespace().map(|x| x.to_string()).collect())),
            x_addrs: XAddrs::parse(&self.x_addrs.unwrap_or_default())?,
            metadata_version: self.metadata_version.unwrap_or(0),
            discovered_at,
        })
    }
}

#[derive(Default)]
struct HelloByeBuilder {
    endpoint_reference: Option<String>,
    types: Vec<String>,
    scopes: Option<String>,
    x_addrs: Option<String>,
    metadata_version: Option<MetadataVersion>,
    error: Option<OnvifError>,
}

impl HelloByeBuilder {
    fn set_endpoint_reference(&mut self, epr: &str) {
        if self.error.is_some() {
            return;
        }
        match validate_epr(epr) {
            Ok(clean) => self.endpoint_reference = Some(clean),
            Err(e) => self.error = Some(e),
        }
    }

    fn apply(&mut self, name: &str, text: &str) {
        if self.error.is_some() {
            return;
        }
        match name {
            WSD_TYPES => self.types = text.split_whitespace().map(|s| s.to_string()).collect(),
            WSD_SCOPES => self.scopes = Some(text.to_string()),
            WSD_X_ADDRS => self.x_addrs = Some(text.to_string()),
            WSD_METADATA_VERSION => match parse_optional_metadata_version(text) {
                Ok(v) => self.metadata_version = v,
                Err(e) => self.error = Some(e),
            },
            _ => {}
        }
    }

    fn build_hello(self, discovered_at: u64) -> OnvifResult<Hello> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let epr = validate_epr(&self.endpoint_reference.unwrap_or_default())?;
        Ok(Hello {
            endpoint_reference: super::types::EndpointReference(epr),
            types: self.types,
            scopes: self
                .scopes
                .as_ref()
                .map(|s| Scopes(s.split_whitespace().map(|x| x.to_string()).collect())),
            x_addrs: XAddrs::parse(&self.x_addrs.unwrap_or_default())?,
            metadata_version: self.metadata_version.unwrap_or(0),
            discovered_at,
        })
    }

    fn build_bye(self, discovered_at: u64) -> OnvifResult<Bye> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let epr = validate_epr(&self.endpoint_reference.unwrap_or_default())?;
        Ok(Bye {
            endpoint_reference: super::types::EndpointReference(epr),
            discovered_at,
        })
    }
}

#[derive(Default)]
struct ResolveMatchBuilder {
    endpoint_reference: Option<String>,
    x_addrs: Option<String>,
    metadata_version: Option<MetadataVersion>,
    error: Option<OnvifError>,
}

impl ResolveMatchBuilder {
    fn set_endpoint_reference(&mut self, epr: &str) {
        if self.error.is_some() {
            return;
        }
        match validate_epr(epr) {
            Ok(clean) => self.endpoint_reference = Some(clean),
            Err(e) => self.error = Some(e),
        }
    }

    fn apply(&mut self, name: &str, text: &str) {
        if self.error.is_some() {
            return;
        }
        match name {
            WSD_X_ADDRS => self.x_addrs = Some(text.to_string()),
            WSD_METADATA_VERSION => match parse_optional_metadata_version(text) {
                Ok(v) => self.metadata_version = v,
                Err(e) => self.error = Some(e),
            },
            _ => {}
        }
    }

    fn build(self, discovered_at: u64) -> OnvifResult<ResolveMatch> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let epr = validate_epr(&self.endpoint_reference.unwrap_or_default())?;
        Ok(ResolveMatch {
            endpoint_reference: super::types::EndpointReference(epr),
            x_addrs: XAddrs::parse(&self.x_addrs.unwrap_or_default())?,
            metadata_version: self.metadata_version.unwrap_or(0),
            discovered_at,
        })
    }
}

fn parse_optional_metadata_version(text: &str) -> OnvifResult<Option<MetadataVersion>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<MetadataVersion>().map(Some).map_err(|_| {
        OnvifError::invalid_field(format!("MetadataVersion is not a valid integer: {text}"))
    })
}
