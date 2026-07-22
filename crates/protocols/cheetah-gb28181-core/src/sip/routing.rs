//! Method routing for inbound SIP requests.
//!
//! GB28181 signaling uses a fixed subset of SIP methods. [`route_request`]
//! classifies an inbound request into a [`RequestRoute`] so a transport driver
//! can dispatch it to the correct handler and decide whether it participates in
//! a server transaction. The classification is pure and I/O-free; the driver
//! combines it with the [`TransactionManager`](crate::TransactionManager) to
//! realise retransmission, duplicate detection and deadlines.

use crate::{Method, SipMessage};

/// The dispatch category of an inbound SIP request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestRoute {
    /// `REGISTER` — device registration handled by the registrar/access handler.
    Register,
    /// `MESSAGE` — MANSCDP command/notify carried in the body; access handler.
    Message,
    /// `OPTIONS` — capability / keepalive probe.
    Options,
    /// `INVITE` — media session establishment; creates or refreshes a dialog.
    Invite,
    /// `ACK` — confirms an INVITE final response; absorbed by the transaction or
    /// delivered to the dialog. `ACK` never elicits a response of its own.
    Ack,
    /// `CANCEL` — cancels a pending INVITE server transaction.
    Cancel,
    /// `BYE` — terminates an established dialog.
    Bye,
    /// `INFO` — in-dialog information such as MANSRTSP playback control.
    Info,
    /// `SUBSCRIBE` — event subscription (catalog / alarm).
    Subscribe,
    /// `NOTIFY` — event notification within a subscription.
    Notify,
    /// Any method outside the GB28181 subset; the driver answers `501`.
    Unsupported,
}

impl RequestRoute {
    /// Classifies a SIP method into its route.
    pub fn from_method(method: &Method) -> Self {
        match method {
            Method::Register => RequestRoute::Register,
            Method::Message => RequestRoute::Message,
            Method::Options => RequestRoute::Options,
            Method::Invite => RequestRoute::Invite,
            Method::Ack => RequestRoute::Ack,
            Method::Cancel => RequestRoute::Cancel,
            Method::Bye => RequestRoute::Bye,
            Method::Info => RequestRoute::Info,
            Method::Subscribe => RequestRoute::Subscribe,
            Method::Notify => RequestRoute::Notify,
            Method::Other(_) => RequestRoute::Unsupported,
        }
    }

    /// True when the request is delivered to the transaction user for business
    /// handling. Every method except an orphaned `ACK` reaches the TU; `ACK` is
    /// delivered to the dialog layer rather than a request handler.
    pub fn delivers_to_tu(self) -> bool {
        !matches!(self, RequestRoute::Ack)
    }

    /// True when the method establishes or targets an INVITE dialog.
    pub fn is_dialog(self) -> bool {
        matches!(
            self,
            RequestRoute::Invite | RequestRoute::Ack | RequestRoute::Bye | RequestRoute::Info
        )
    }

    /// True when the request should be tracked by a server transaction.
    ///
    /// `ACK` is never a transaction-creating request (it either matches an
    /// existing INVITE server transaction or is delivered to the dialog).
    pub fn creates_server_transaction(self) -> bool {
        !matches!(self, RequestRoute::Ack)
    }
}

/// Classifies an inbound SIP message.
///
/// Returns `None` for responses, which are matched to client transactions
/// rather than routed by method.
pub fn route_request(message: &SipMessage) -> Option<RequestRoute> {
    match message {
        SipMessage::Request { line, .. } => Some(RequestRoute::from_method(&line.method)),
        SipMessage::Response { .. } => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{RequestLine, SipHeaders, SipMessage, SipUri, StatusLine};

    fn request(method: Method) -> SipMessage {
        SipMessage::Request {
            line: RequestLine::new(method, SipUri::parse("sip:d@example.com").unwrap()),
            headers: SipHeaders::new(),
            body: Vec::new(),
        }
    }

    #[test]
    fn golden_method_routing_table() {
        let cases = [
            (Method::Register, RequestRoute::Register),
            (Method::Message, RequestRoute::Message),
            (Method::Options, RequestRoute::Options),
            (Method::Invite, RequestRoute::Invite),
            (Method::Ack, RequestRoute::Ack),
            (Method::Cancel, RequestRoute::Cancel),
            (Method::Bye, RequestRoute::Bye),
            (Method::Info, RequestRoute::Info),
            (Method::Subscribe, RequestRoute::Subscribe),
            (Method::Notify, RequestRoute::Notify),
            (
                Method::Other("PUBLISH".to_string()),
                RequestRoute::Unsupported,
            ),
        ];
        for (method, expected) in cases {
            assert_eq!(route_request(&request(method)).unwrap(), expected);
        }
    }

    #[test]
    fn ack_is_not_delivered_as_request_but_is_dialog() {
        assert!(!RequestRoute::Ack.delivers_to_tu());
        assert!(!RequestRoute::Ack.creates_server_transaction());
        assert!(RequestRoute::Ack.is_dialog());
    }

    #[test]
    fn dialog_methods_are_classified() {
        for route in [
            RequestRoute::Invite,
            RequestRoute::Ack,
            RequestRoute::Bye,
            RequestRoute::Info,
        ] {
            assert!(route.is_dialog());
        }
        for route in [
            RequestRoute::Register,
            RequestRoute::Message,
            RequestRoute::Options,
            RequestRoute::Subscribe,
            RequestRoute::Notify,
            RequestRoute::Cancel,
        ] {
            assert!(!route.is_dialog());
        }
    }

    #[test]
    fn responses_are_not_routed_by_method() {
        let resp = SipMessage::Response {
            line: StatusLine::new(200, "OK"),
            headers: SipHeaders::new(),
            body: Vec::new(),
        };
        assert!(route_request(&resp).is_none());
    }
}
