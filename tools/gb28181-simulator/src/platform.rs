//! Sans-I/O in-memory platform (registrar) peer.
//!
//! The platform answers device REGISTER/keepalive/catalog traffic and, when a
//! scripted step fires, initiates catalog queries, media control INVITEs and
//! BYEs.  It never generates media payloads and holds no per-device tasks.

use cheetah_gb28181_core::{
    HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
};
use cheetah_gb28181_module::xml::{XmlElement, encode_xml};
use std::collections::HashMap;

/// Semantic events emitted by the platform for accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformEvent {
    /// A digest challenge (401) was sent.
    Challenged,
    /// A device registration was accepted (200).
    RegisterAccepted,
    /// A keepalive was acknowledged (200).
    KeepaliveAcked,
    /// A catalog response was received from a device.
    CatalogReceived,
    /// A SIP error status was injected in response to a device request.
    ErrorInjected,
}

/// Output of a platform step.
#[derive(Debug, Default)]
pub struct PlatformStep {
    /// Messages the platform sends.
    pub messages: Vec<SipMessage>,
    /// Semantic events for accounting.
    pub events: Vec<PlatformEvent>,
}

/// The in-memory platform peer.
#[derive(Debug)]
pub struct Platform {
    domain: String,
    registered: HashMap<String, bool>,
    sn: u32,
    cseq: u32,
}

impl Platform {
    /// Creates a platform with the given SIP domain (e.g. `platform`).
    pub fn new(domain: String) -> Self {
        Self {
            domain,
            registered: HashMap::new(),
            sn: 1,
            cseq: 1,
        }
    }

    /// Handles an inbound message from a device.
    ///
    /// `sip_error` optionally overrides a successful response with an injected
    /// SIP error status for register-class or other targeted requests.
    pub fn on_inbound(&mut self, msg: &SipMessage, sip_error: Option<u16>) -> PlatformStep {
        match msg {
            SipMessage::Request {
                line,
                headers,
                body,
            } => self.on_request(&line.method, headers, body, sip_error),
            SipMessage::Response { .. } => PlatformStep::default(),
        }
    }

    fn on_request(
        &mut self,
        method: &Method,
        headers: &SipHeaders,
        body: &[u8],
        sip_error: Option<u16>,
    ) -> PlatformStep {
        match method {
            Method::Register => self.on_register(headers, sip_error),
            Method::Message => self.on_message(headers, body, sip_error),
            _ => PlatformStep {
                messages: vec![response(405, "Method Not Allowed", headers)],
                events: Vec::new(),
            },
        }
    }

    fn on_register(&mut self, headers: &SipHeaders, sip_error: Option<u16>) -> PlatformStep {
        if headers.get(&HeaderName::Authorization).is_none() {
            let mut resp = response(401, "Unauthorized", headers);
            if let SipMessage::Response { headers, .. } = &mut resp {
                headers.append(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::new(
                        "Digest realm=\"3402000000\", nonce=\"0123456789abcdef\", algorithm=MD5",
                    ),
                );
            }
            return PlatformStep {
                messages: vec![resp],
                events: vec![PlatformEvent::Challenged],
            };
        }
        if let Some(code) = sip_error {
            return PlatformStep {
                messages: vec![response(code, "Injected Error", headers)],
                events: vec![PlatformEvent::ErrorInjected],
            };
        }
        if let Some(id) = header_user(headers, HeaderName::From) {
            self.registered.insert(id, true);
        }
        PlatformStep {
            messages: vec![response(200, "OK", headers)],
            events: vec![PlatformEvent::RegisterAccepted],
        }
    }

    fn on_message(
        &mut self,
        headers: &SipHeaders,
        body: &[u8],
        sip_error: Option<u16>,
    ) -> PlatformStep {
        if let Some(code) = sip_error {
            return PlatformStep {
                messages: vec![response(code, "Injected Error", headers)],
                events: vec![PlatformEvent::ErrorInjected],
            };
        }
        let text = String::from_utf8_lossy(body);
        let mut events = Vec::new();
        if text.contains("Keepalive") {
            events.push(PlatformEvent::KeepaliveAcked);
        } else if text.contains("<CmdType>Catalog") && text.contains("Response") {
            events.push(PlatformEvent::CatalogReceived);
        }
        PlatformStep {
            messages: vec![response(200, "OK", headers)],
            events,
        }
    }

    /// Builds a Catalog query MESSAGE for `device_id`.
    pub fn catalog_query(&mut self, device_id: &str) -> Option<SipMessage> {
        let sn = self.next_sn();
        let mut root = XmlElement {
            name: "Query".to_string(),
            ..XmlElement::default()
        };
        root.children.push(text_element("CmdType", "Catalog"));
        root.children.push(text_element("SN", &sn.to_string()));
        root.children.push(text_element("DeviceID", device_id));
        let xml = encode_xml(&root, true).ok()?;
        self.request(
            Method::Message,
            device_id,
            xml.into_bytes(),
            Some("application/MANSCDP+xml"),
        )
    }

    /// Builds a media-control INVITE for `device_id`.
    ///
    /// The SDP describes a control-plane offer only; the simulator never emits
    /// RTP, RTCP, PS, TS or ES media payloads.
    pub fn invite(&mut self, device_id: &str) -> Option<SipMessage> {
        let sdp = b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Play\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 PS/90000\r\na=recvonly\r\n";
        self.request(
            Method::Invite,
            device_id,
            sdp.to_vec(),
            Some("application/sdp"),
        )
    }

    /// Builds a BYE for `device_id`.
    pub fn bye(&mut self, device_id: &str) -> Option<SipMessage> {
        self.request(Method::Bye, device_id, Vec::new(), None)
    }

    fn request(
        &mut self,
        method: Method,
        device_id: &str,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Option<SipMessage> {
        let uri = SipUri::parse(format!("sip:{}@{}", device_id, self.domain)).ok()?;
        let cseq = self.next_cseq();
        let mut headers = SipHeaders::new();
        let branch = format!("z9hG4bKplat-{cseq}");
        if let Ok(via) = HeaderValue::via("UDP", "127.0.0.1", 5060, &branch) {
            headers.append(HeaderName::Via, via);
        }
        if let Ok(from_uri) = SipUri::parse(format!("sip:platform@{}", self.domain))
            && let Ok(from) = HeaderValue::from_uri(&from_uri, "platform-tag")
        {
            headers.append(HeaderName::From, from);
        }
        headers.append(HeaderName::To, HeaderValue::to_uri(&uri));
        headers.append(
            HeaderName::CallId,
            HeaderValue::new(format!("plat-{device_id}")),
        );
        headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, method.clone()));
        headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
        if let Some(content_type) = content_type {
            headers.append(HeaderName::ContentType, HeaderValue::new(content_type));
        }
        Some(SipMessage::Request {
            line: RequestLine::new(method, uri),
            headers,
            body,
        })
    }

    fn next_sn(&mut self) -> u32 {
        let sn = self.sn;
        self.sn += 1;
        sn
    }

    fn next_cseq(&mut self) -> u32 {
        let cseq = self.cseq;
        self.cseq += 1;
        cseq
    }
}

fn text_element(name: &str, text: &str) -> XmlElement {
    XmlElement {
        name: name.to_string(),
        text: text.to_string(),
        ..XmlElement::default()
    }
}

fn response(code: u16, reason: &str, request_headers: &SipHeaders) -> SipMessage {
    let mut headers = SipHeaders::new();
    for (name, value) in request_headers.iter() {
        if matches!(
            name,
            HeaderName::Via
                | HeaderName::From
                | HeaderName::To
                | HeaderName::CallId
                | HeaderName::CSeq
        ) {
            headers.append(name.clone(), value.clone());
        }
    }
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

fn header_user(headers: &SipHeaders, name: HeaderName) -> Option<String> {
    let value = headers.get(&name)?;
    let raw = value.as_str();
    let start = raw.find("sip:")? + 4;
    let rest = &raw[start..];
    let end = rest.find(['@', '>', ';', ' ']).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}
