//! Sans-I/O simulated GB28181 device state machine.
//!
//! A device consumes inputs (start, keepalive tick, inbound SIP message) and
//! produces outbound SIP messages plus semantic events.  It performs no I/O and
//! holds no timers of its own: the harness owns the timer wheel and transport.
//! Devices are stored densely inside shards, so there is no per-device task.

use crate::profile::ResolvedProfile;
use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage, SipUri, StatusLine,
};
use cheetah_gb28181_module::xml::{
    CatalogItem, build_catalog_response, build_keepalive, parse_catalog_query,
};
use secrecy::SecretString;

/// Semantic events emitted by a device for accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceEvent {
    /// The device transitioned to registered.
    Registered,
    /// A keepalive MESSAGE was emitted.
    KeepaliveSent,
    /// A catalog response MESSAGE was emitted.
    CatalogResponded,
    /// An INVITE was answered with a 200 (media control only).
    MediaInviteAnswered,
    /// A BYE was answered.
    MediaByeAnswered,
    /// A SIP error response was observed from the platform.
    ErrorObserved,
}

/// Inputs delivered to a device by the harness.
#[derive(Debug)]
pub enum DeviceInput<'a> {
    /// Begin registration.
    Start,
    /// Keepalive interval elapsed.
    KeepaliveTick,
    /// An inbound SIP message arrived from the platform.
    Inbound(&'a SipMessage),
}

/// The outputs of a single device step.
#[derive(Debug, Default)]
pub struct DeviceStep {
    /// Messages to send to the platform.
    pub messages: Vec<SipMessage>,
    /// Semantic events for accounting.
    pub events: Vec<DeviceEvent>,
}

/// A simulated device.
#[derive(Debug)]
pub struct Device {
    index: u32,
    device_id: String,
    password: SecretString,
    profile: ResolvedProfile,
    server_uri: Option<SipUri>,
    domain: String,
    local_port: u16,
    cseq: u32,
    call_id: String,
    from_tag: String,
    registered: bool,
    registering: bool,
}

impl Device {
    /// Creates a device with a stable, seed-independent identity.
    pub fn new(
        index: u32,
        device_id: String,
        password: SecretString,
        profile: ResolvedProfile,
        server: String,
        domain: String,
    ) -> Self {
        Self {
            index,
            call_id: format!("call-{device_id}"),
            from_tag: format!("tag-{device_id}"),
            device_id,
            password,
            profile,
            server_uri: SipUri::parse(format!("sip:{server}")).ok(),
            domain,
            local_port: 5000u16.wrapping_add((index % 20000) as u16),
            cseq: 1,
            registered: false,
            registering: false,
        }
    }

    /// Device identifier.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Whether the device is currently registered.
    pub fn registered(&self) -> bool {
        self.registered
    }

    /// Advances the device with a single input.
    pub fn step(&mut self, input: DeviceInput<'_>) -> DeviceStep {
        match input {
            DeviceInput::Start => self.start(),
            DeviceInput::KeepaliveTick => self.keepalive(),
            DeviceInput::Inbound(msg) => self.on_inbound(msg),
        }
    }

    fn start(&mut self) -> DeviceStep {
        self.registering = true;
        self.registered = false;
        DeviceStep {
            messages: self.build_register(None).into_iter().collect(),
            events: Vec::new(),
        }
    }

    fn keepalive(&mut self) -> DeviceStep {
        if !self.registered {
            return self.start();
        }
        let sn = self.next_cseq().to_string();
        match build_keepalive(&sn, &self.device_id, "OK") {
            Ok(xml) => match self.build_message(xml.into_bytes(), "application/MANSCDP+xml") {
                Some(msg) => DeviceStep {
                    messages: vec![msg],
                    events: vec![DeviceEvent::KeepaliveSent],
                },
                None => DeviceStep::default(),
            },
            Err(_) => DeviceStep::default(),
        }
    }

    fn on_inbound(&mut self, msg: &SipMessage) -> DeviceStep {
        match msg {
            SipMessage::Response { line, headers, .. } => self.on_response(line, headers),
            SipMessage::Request {
                line,
                headers,
                body,
            } => self.on_request(&line.method, headers, body),
        }
    }

    fn on_response(&mut self, line: &StatusLine, headers: &SipHeaders) -> DeviceStep {
        if line.code == 401 {
            if let Some(value) = headers.get(&HeaderName::WwwAuthenticate)
                && let Ok(challenge) = DigestChallenge::parse(value.as_str())
            {
                return DeviceStep {
                    messages: self.build_register(Some(challenge)).into_iter().collect(),
                    events: Vec::new(),
                };
            }
            return DeviceStep::default();
        }
        if (200..300).contains(&line.code) {
            if self.registering {
                self.registering = false;
                self.registered = true;
                return DeviceStep {
                    messages: Vec::new(),
                    events: vec![DeviceEvent::Registered],
                };
            }
            return DeviceStep::default();
        }
        // 3xx-6xx: registration failed, mark unregistered so keepalive retries.
        if self.registering {
            self.registering = false;
            self.registered = false;
        }
        DeviceStep {
            messages: Vec::new(),
            events: vec![DeviceEvent::ErrorObserved],
        }
    }

    fn on_request(&mut self, method: &Method, headers: &SipHeaders, body: &[u8]) -> DeviceStep {
        match method {
            Method::Message => self.on_message(headers, body),
            Method::Invite => self.on_invite(headers),
            Method::Bye => DeviceStep {
                messages: vec![self.build_response(200, "OK", headers, &[])],
                events: vec![DeviceEvent::MediaByeAnswered],
            },
            Method::Cancel => DeviceStep {
                messages: vec![self.build_response(200, "OK", headers, &[])],
                events: Vec::new(),
            },
            _ => DeviceStep {
                messages: vec![self.build_response(405, "Method Not Allowed", headers, &[])],
                events: Vec::new(),
            },
        }
    }

    fn on_message(&mut self, headers: &SipHeaders, body: &[u8]) -> DeviceStep {
        let mut step = DeviceStep {
            messages: vec![self.build_response(200, "OK", headers, &[])],
            events: Vec::new(),
        };
        if let Ok(query) = parse_catalog_query(body)
            && query.device_id == self.device_id
            && let Some(catalog) = self.build_catalog(&query.sn)
        {
            step.messages.push(catalog);
            step.events.push(DeviceEvent::CatalogResponded);
        }
        step
    }

    fn on_invite(&mut self, headers: &SipHeaders) -> DeviceStep {
        // Media control only: answer with a synthetic SDP offer/answer. No RTP,
        // RTCP, PS, TS or ES payload is produced or transmitted.
        let trying = self.build_response(100, "Trying", headers, &[]);
        let sdp = b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=Play\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 PS/90000\r\na=sendonly\r\n";
        let mut ok = self.build_response(200, "OK", headers, sdp);
        if let SipMessage::Response { headers, .. } = &mut ok {
            headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
        }
        DeviceStep {
            messages: vec![trying, ok],
            events: vec![DeviceEvent::MediaInviteAnswered],
        }
    }

    fn build_catalog(&mut self, sn: &str) -> Option<SipMessage> {
        let mut items = Vec::new();
        for i in 0..self.profile.catalog_items {
            items.push(CatalogItem {
                device_id: format!("{}{:02}", self.device_id, i),
                name: Some(format!("Channel {i}")),
                manufacturer: Some(self.profile.manufacturer.clone()),
                model: Some(self.profile.model.clone()),
                owner: None,
                civil_code: None,
                block: None,
                address: None,
                parental: Some("0".to_string()),
                parent_id: Some(self.device_id.clone()),
                safety_way: None,
                register_way: Some("1".to_string()),
                cert_num: None,
                certifiable: None,
                err_code: Some("0".to_string()),
                end_time: None,
                secrecy: Some("0".to_string()),
                ip_address: None,
                port: None,
                status: Some("ON".to_string()),
                longitude: None,
                latitude: None,
            });
        }
        let xml = build_catalog_response(sn, &self.device_id, items.len() as u32, &items).ok()?;
        self.build_message(xml.into_bytes(), "application/MANSCDP+xml")
    }

    fn next_cseq(&mut self) -> u32 {
        let cseq = self.cseq;
        self.cseq += 1;
        cseq
    }

    fn common_headers(&self, cseq: u32, method: Method, branch_tag: &str) -> SipHeaders {
        let mut headers = SipHeaders::new();
        let branch = format!("z9hG4bK{branch_tag}-{}-{cseq}", self.index);
        if let Ok(via) = HeaderValue::via("UDP", "127.0.0.1", self.local_port, &branch) {
            headers.append(HeaderName::Via, via);
        }
        if let Ok(from_uri) = SipUri::parse(format!("sip:{}@{}", self.device_id, self.domain))
            && let Ok(from) = HeaderValue::from_uri(&from_uri, &self.from_tag)
        {
            headers.append(HeaderName::From, from);
        }
        if let Ok(to_uri) = SipUri::parse(format!("sip:{}@{}", self.device_id, self.domain)) {
            headers.append(HeaderName::To, HeaderValue::to_uri(&to_uri));
        }
        headers.append(HeaderName::CallId, HeaderValue::new(self.call_id.clone()));
        headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, method));
        headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
        headers.append(HeaderName::UserAgent, HeaderValue::new("CheetahGBSim/1.0"));
        headers
    }

    fn build_register(&mut self, challenge: Option<DigestChallenge>) -> Option<SipMessage> {
        let server_uri = self.server_uri.clone()?;
        let cseq = self.next_cseq();
        let mut headers = self.common_headers(cseq, Method::Register, "reg");
        if let Ok(contact_uri) = SipUri::parse(format!(
            "sip:{}@127.0.0.1:{}",
            self.device_id, self.local_port
        )) {
            headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
        }
        headers.append(HeaderName::Expires, HeaderValue::new("3600"));

        if let Some(challenge) = challenge
            && let Ok(cnonce) = DigestClient::derive_cnonce(&self.password, &self.call_id)
        {
            let mut client = DigestClient::new().allow_md5(true);
            if let Ok(response) = client.authorize(
                &self.device_id,
                &self.password,
                "REGISTER",
                &server_uri.encode(),
                &challenge,
                &cnonce,
            ) {
                headers.append(
                    HeaderName::Authorization,
                    HeaderValue::new(response.to_header_value()),
                );
            }
        }

        Some(SipMessage::Request {
            line: RequestLine::new(Method::Register, server_uri),
            headers,
            body: Vec::new(),
        })
    }

    fn build_message(&mut self, body: Vec<u8>, content_type: &str) -> Option<SipMessage> {
        let server_uri = self.server_uri.clone()?;
        let cseq = self.next_cseq();
        let mut headers = self.common_headers(cseq, Method::Message, "msg");
        if let Ok(contact_uri) = SipUri::parse(format!(
            "sip:{}@127.0.0.1:{}",
            self.device_id, self.local_port
        )) {
            headers.append(HeaderName::Contact, HeaderValue::contact_uri(&contact_uri));
        }
        headers.append(HeaderName::ContentType, HeaderValue::new(content_type));
        Some(SipMessage::Request {
            line: RequestLine::new(Method::Message, server_uri),
            headers,
            body,
        })
    }

    fn build_response(
        &self,
        code: u16,
        reason: &str,
        request_headers: &SipHeaders,
        body: &[u8],
    ) -> SipMessage {
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
            body: body.to_vec(),
        }
    }
}
