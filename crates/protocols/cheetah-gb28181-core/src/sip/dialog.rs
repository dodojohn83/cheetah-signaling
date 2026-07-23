//! RFC 3261 dialog management.
//!
//! A `Dialog` represents a long-term peer-to-peer SIP relationship established by
//! a successful INVITE exchange. It holds only the protocol state required for
//! routing and sequencing (dialog id, route set, remote target, and CSeq
//! counters); media sessions are managed by the domain layer.

use super::error::{SipError, SipErrorKind};
use crate::{HeaderName, Method, SipMessage, SipUri};

/// Identifies a dialog by the tuple that uniquely distinguishes both peers.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct DialogId {
    /// `Call-ID` value.
    pub call_id: String,
    /// Tag chosen by the local UA.
    pub local_tag: String,
    /// Tag chosen by the remote UA.
    pub remote_tag: String,
}

/// Role of the UA that owns this dialog state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogRole {
    /// The dialog was created by an outgoing INVITE client transaction.
    Uac,
    /// The dialog was created by an incoming INVITE server transaction.
    Uas,
}

/// Lifecycle state of a dialog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogState {
    /// A 1xx response has been received/sent, but no final 2xx yet.
    Early,
    /// The dialog is established (2xx INVITE exchanged).
    Confirmed,
    /// The dialog has been torn down and should be discarded.
    Terminated,
}

/// Input delivered to a dialog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogEvent {
    /// An in-dialog or dialog-terminating request from the network.
    Request(SipMessage),
    /// A response to an in-dialog or dialog-terminating request.
    Response(SipMessage),
    /// A dialog-level timeout or expiration signal.
    Timer,
}

/// Output produced by a dialog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogOutput {
    /// Pass the message to the transaction user (TU) for application handling.
    Deliver(Box<SipMessage>),
    /// The dialog has ended and may be removed.
    Complete,
    /// The dialog reached an inconsistent state; the TU should be informed.
    Failure(SipErrorKind),
}

/// SIP dialog state machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dialog {
    id: DialogId,
    role: DialogRole,
    state: DialogState,
    route_set: Vec<SipUri>,
    remote_target: SipUri,
    local_cseq: u32,
    remote_cseq: Option<u32>,
}

impl Dialog {
    /// Creates a UAC dialog from the original INVITE and the 2xx final
    /// response.
    pub fn new_uac(invite: &SipMessage, final_response: &SipMessage) -> Result<Self, SipError> {
        let call_id = Self::require_str(invite, &HeaderName::CallId)?;
        let local_tag =
            extract_tag(require_header(invite, &HeaderName::From)?).ok_or_else(|| {
                SipError::new(
                    SipErrorKind::MissingRequiredHeader,
                    None,
                    "missing From tag",
                )
            })?;
        let remote_tag =
            extract_tag(require_header(final_response, &HeaderName::To)?).ok_or_else(|| {
                SipError::new(SipErrorKind::MissingRequiredHeader, None, "missing To tag")
            })?;

        let local_cseq = cseq_number(invite)?;
        // RFC 3261 §12.1.2: the UAC remote sequence number is empty until the
        // remote UA sends the first in-dialog request.
        let remote_cseq = None;

        let remote_target =
            extract_first_uri(require_header(final_response, &HeaderName::Contact)?).ok_or_else(
                || SipError::new(SipErrorKind::MissingRequiredHeader, None, "missing Contact"),
            )?;

        // UAC route set: Record-Route values from the response, reversed so that
        // the first hop for future requests is at the front of the vector.
        let route_set = extract_route_set(final_response, true)?;

        Ok(Self {
            id: DialogId {
                call_id: call_id.to_string(),
                local_tag: local_tag.to_string(),
                remote_tag: remote_tag.to_string(),
            },
            role: DialogRole::Uac,
            state: DialogState::Confirmed,
            route_set,
            remote_target,
            local_cseq,
            remote_cseq,
        })
    }

    /// Creates a UAS dialog from the incoming INVITE and the local tag placed
    /// in the 2xx final response.
    pub fn new_uas(invite: &SipMessage, local_tag: impl Into<String>) -> Result<Self, SipError> {
        let call_id = Self::require_str(invite, &HeaderName::CallId)?;
        let local_tag = local_tag.into();
        let remote_tag =
            extract_tag(require_header(invite, &HeaderName::From)?).ok_or_else(|| {
                SipError::new(
                    SipErrorKind::MissingRequiredHeader,
                    None,
                    "missing From tag",
                )
            })?;

        // RFC 3261 §12.1.1: the UAS local sequence number is empty until it
        // sends the first in-dialog request.
        let local_cseq = 0;
        let remote_cseq = Some(cseq_number(invite)?);

        let remote_target = extract_first_uri(require_header(invite, &HeaderName::Contact)?)
            .ok_or_else(|| {
                SipError::new(SipErrorKind::MissingRequiredHeader, None, "missing Contact")
            })?;

        // UAS route set: Record-Route values from the request in received order.
        let route_set = extract_route_set(invite, false)?;

        Ok(Self {
            id: DialogId {
                call_id: call_id.to_string(),
                local_tag,
                remote_tag: remote_tag.to_string(),
            },
            role: DialogRole::Uas,
            state: DialogState::Confirmed,
            route_set,
            remote_target,
            local_cseq,
            remote_cseq,
        })
    }

    /// Returns the dialog identifier.
    pub fn id(&self) -> &DialogId {
        &self.id
    }

    /// Returns the UA role for this dialog.
    pub fn role(&self) -> DialogRole {
        self.role
    }

    /// Returns the current dialog state.
    pub fn state(&self) -> DialogState {
        self.state
    }

    /// Returns true when the dialog has reached the `Terminated` state.
    pub fn is_terminated(&self) -> bool {
        matches!(self.state, DialogState::Terminated)
    }

    /// Returns the route set for requests within this dialog.
    pub fn route_set(&self) -> &[SipUri] {
        &self.route_set
    }

    /// Returns the current remote target URI.
    pub fn remote_target(&self) -> &SipUri {
        &self.remote_target
    }

    /// Returns the highest local CSeq number used or seen.
    pub fn local_cseq(&self) -> u32 {
        self.local_cseq
    }

    /// Returns the highest remote CSeq number used or seen, if any.
    pub fn remote_cseq(&self) -> Option<u32> {
        self.remote_cseq
    }

    /// Allocates the next local CSeq number for a request originated in this
    /// dialog and returns it.
    pub fn next_local_cseq(&mut self) -> u32 {
        self.local_cseq += 1;
        self.local_cseq
    }

    /// Processes a dialog-level event.
    #[must_use]
    pub fn process(&mut self, event: DialogEvent) -> Vec<DialogOutput> {
        if self.is_terminated() {
            return Vec::new();
        }

        match event {
            DialogEvent::Request(req) => self.on_request(req),
            DialogEvent::Response(resp) => self.on_response(resp),
            DialogEvent::Timer => self.terminate(),
        }
    }

    fn on_request(&mut self, req: SipMessage) -> Vec<DialogOutput> {
        let method = match &req {
            SipMessage::Request { line, .. } => line.method.clone(),
            _ => return Vec::new(),
        };

        let (cseq, _) = match req.cseq() {
            Ok(c) => c,
            Err(e) => return vec![DialogOutput::Failure(e.kind)],
        };

        let remote_cseq = self.remote_cseq.unwrap_or(0);

        match method {
            Method::Bye => {
                if self.remote_cseq.is_some() && cseq <= remote_cseq {
                    return Vec::new();
                }
                self.remote_cseq = Some(cseq);
                self.state = DialogState::Terminated;
                vec![DialogOutput::Deliver(Box::new(req)), DialogOutput::Complete]
            }
            Method::Ack | Method::Cancel => {
                // ACK and CANCEL reuse the CSeq of the INVITE they target, so
                // they bypass the monotonic CSeq check and do not advance
                // remote_cseq. The transaction layer normally handles CANCEL,
                // but if it reaches the dialog it is delivered to the TU.
                vec![DialogOutput::Deliver(Box::new(req))]
            }
            Method::Invite | Method::Message | Method::Options => {
                if self.remote_cseq.is_some() && cseq <= remote_cseq {
                    // Out-of-order or retransmitted request inside the dialog.
                    // The transaction layer handles retransmissions; the dialog
                    // absorbs duplicates.
                    return Vec::new();
                }
                self.remote_cseq = Some(cseq);

                if method == Method::Invite {
                    // Re-INVITE may update the remote target.
                    if let Some(uri) = req
                        .headers()
                        .get(&HeaderName::Contact)
                        .and_then(|v| extract_first_uri(v.as_str()))
                    {
                        self.remote_target = uri;
                    }
                }

                vec![DialogOutput::Deliver(Box::new(req))]
            }
            _ => {
                if self.remote_cseq.is_some() && cseq <= remote_cseq {
                    return Vec::new();
                }
                self.remote_cseq = Some(cseq);
                vec![DialogOutput::Deliver(Box::new(req))]
            }
        }
    }

    fn on_response(&mut self, resp: SipMessage) -> Vec<DialogOutput> {
        let code = match &resp {
            SipMessage::Response { line, .. } => line.code,
            _ => return Vec::new(),
        };

        let (cseq, method) = match resp.cseq() {
            Ok(c) => c,
            Err(e) => return vec![DialogOutput::Failure(e.kind)],
        };

        if method == Method::Bye && (200..300).contains(&code) {
            self.state = DialogState::Terminated;
            return vec![
                DialogOutput::Deliver(Box::new(resp)),
                DialogOutput::Complete,
            ];
        }

        if cseq > self.local_cseq {
            self.local_cseq = cseq;
        }

        vec![DialogOutput::Deliver(Box::new(resp))]
    }

    fn terminate(&mut self) -> Vec<DialogOutput> {
        self.state = DialogState::Terminated;
        vec![DialogOutput::Complete]
    }

    fn require_str<'a>(msg: &'a SipMessage, name: &HeaderName) -> Result<&'a str, SipError> {
        require_header(msg, name)
    }
}

fn require_header<'a>(msg: &'a SipMessage, name: &HeaderName) -> Result<&'a str, SipError> {
    msg.headers()
        .get(name)
        .map(|v| v.as_str())
        .ok_or_else(|| SipError::new(SipErrorKind::MissingRequiredHeader, None, "missing header"))
}

fn cseq_number(msg: &SipMessage) -> Result<u32, SipError> {
    msg.cseq().map(|(n, _)| n)
}

pub(crate) fn extract_tag(value: &str) -> Option<&str> {
    let value = value.trim();
    const NEEDLE: &[u8] = b";tag=";
    for (i, window) in value.as_bytes().windows(NEEDLE.len()).enumerate() {
        if window
            .iter()
            .zip(NEEDLE)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            let start = i + NEEDLE.len();
            let rest = &value[start..];
            let end = rest
                .find(|c: char| c == ';' || c.is_whitespace())
                .unwrap_or(rest.len());
            let tag = &rest[..end];
            return Some(tag.trim_matches('"'));
        }
    }
    None
}

fn extract_route_set(msg: &SipMessage, reverse: bool) -> Result<Vec<SipUri>, SipError> {
    let mut uris = Vec::new();
    for value in msg.headers().get_all(&HeaderName::RecordRoute) {
        for token in split_route_entries(value.as_str()) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let Some(uri) = extract_first_uri(token) else {
                return Err(SipError::new(
                    SipErrorKind::InvalidHeader,
                    None,
                    "malformed Record-Route URI",
                ));
            };
            uris.push(uri);
        }
    }
    if reverse {
        uris.reverse();
    }
    Ok(uris)
}

/// Splits a comma-separated `Record-Route` header value without breaking on
/// commas that appear inside quoted display names or angle-bracketed URIs.
fn split_route_entries(value: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut in_angle = false;
    let mut chars = value.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            // Inside a quoted string, treat backslash as an escape for the
            // next character so escaped quotes do not toggle quote state.
            '\\' if in_quotes => {
                chars.next();
            }
            '"' if !in_angle => in_quotes = !in_quotes,
            '<' if !in_quotes => in_angle = true,
            '>' if !in_quotes && in_angle => in_angle = false,
            ',' if !in_quotes && !in_angle => {
                entries.push(&value[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    entries.push(&value[start..]);
    entries
}

fn extract_first_uri(value: &str) -> Option<SipUri> {
    let value = value.trim();

    // Angle-bracketed URI: take the contents of the first <> pair.
    if let Some(start) = value.find('<')
        && let Some(end) = value[start..].find('>')
    {
        let inner = &value[start + 1..start + end];
        return SipUri::parse(inner).ok();
    }

    // Bare addr-spec (possibly in a comma-separated list): split on commas
    // that are not inside quoted display names or angle brackets, then parse
    // the first URI-like token, stopping at the first unquoted ';'.
    for token in split_route_entries(value) {
        let token = token.trim();
        if token.starts_with("sip:") || token.starts_with("sips:") {
            let mut depth = 0_i32;
            let mut end = token.len();
            for (i, c) in token.char_indices() {
                match c {
                    '"' => depth = if depth == 0 { 1 } else { 0 },
                    ';' if depth == 0 => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            return SipUri::parse(&token[..end]).ok();
        }
    }

    None
}
