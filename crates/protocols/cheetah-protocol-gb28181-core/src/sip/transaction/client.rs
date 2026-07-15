//! RFC 3261 client transaction state machine.

use super::state_machine::{
    TransactionEvent, TransactionOutput, is_failure, is_provisional, is_success, request_method,
    response_code,
};
use super::timers::{TimerKind, TransactionConfig};
use crate::{HeaderName, HeaderValue, Method, RequestLine, SipErrorKind, SipHeaders, SipMessage};
use std::time::Duration;

/// Client transaction FSM for INVITE and non-INVITE requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientTransaction {
    config: TransactionConfig,
    request: SipMessage,
    is_invite: bool,
    state: ClientState,
    /// Number of request retransmissions already sent (0 = original only).
    retransmit_count: u32,
    /// Final response stored for ACK retransmission on response retransmissions.
    final_response: Option<SipMessage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ClientState {
    /// INVITE client initial state.
    Calling,
    /// non-INVITE client initial state.
    Trying,
    Proceeding,
    Completed,
    Terminated,
}

impl ClientTransaction {
    pub fn new(request: SipMessage, config: TransactionConfig) -> Self {
        let is_invite = request_method(&request) == Some(Method::Invite);
        let state = if is_invite {
            ClientState::Calling
        } else {
            ClientState::Trying
        };
        Self {
            config,
            request,
            is_invite,
            state,
            retransmit_count: 0,
            final_response: None,
        }
    }

    pub fn is_terminated(&self) -> bool {
        matches!(self.state, ClientState::Terminated)
    }

    pub fn process(&mut self, event: TransactionEvent, now: Duration) -> Vec<TransactionOutput> {
        use ClientState::*;
        use TransactionEvent::*;

        match (&self.state, event) {
            (Calling, Timer(TimerKind::A)) if self.is_invite => self.on_invite_timer_a(now),
            (Calling | Proceeding, Timer(TimerKind::B)) if self.is_invite => {
                self.on_invite_timer_b()
            }
            (Completed, Timer(TimerKind::D)) if self.is_invite => self.terminate(),
            (Calling | Proceeding, Response(resp)) if self.is_invite => {
                self.on_invite_response(resp, now)
            }
            (Completed, Response(resp)) if self.is_invite => {
                self.on_invite_response_completed(resp)
            }
            (Trying, Timer(TimerKind::E)) if !self.is_invite => self.on_non_invite_timer_e(now),
            (Proceeding, Timer(TimerKind::E)) if !self.is_invite => {
                self.on_non_invite_timer_e_proceeding(now)
            }
            (Trying | Proceeding, Timer(TimerKind::F)) if !self.is_invite => {
                self.on_non_invite_timer_f()
            }
            (Completed, Timer(TimerKind::K)) if !self.is_invite => self.terminate(),
            (Trying | Proceeding, Response(resp)) if !self.is_invite => {
                self.on_non_invite_response(resp, now)
            }
            (Completed | Terminated, Response(_)) if !self.is_invite => Vec::new(),
            (_, TransportError) => self.terminate_with_failure(SipErrorKind::TransportFailure),
            _ => Vec::new(),
        }
    }

    /// Outputs to emit immediately after creating a client transaction.
    pub fn bootstrap(&mut self, now: Duration) -> Vec<TransactionOutput> {
        let mut out = Vec::new();
        out.push(TransactionOutput::SendMessage(self.request.clone()));
        if self.is_invite {
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::A,
                deadline: now + self.config.timer_a(0),
            });
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::B,
                deadline: now + self.config.timer_b(),
            });
        } else {
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::E,
                deadline: now + self.config.timer_e(0),
            });
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::F,
                deadline: now + self.config.timer_f(),
            });
        }
        out
    }

    fn on_invite_response(&mut self, resp: SipMessage, now: Duration) -> Vec<TransactionOutput> {
        let code = response_code(&resp).unwrap_or(0);
        let mut out = Vec::new();

        if is_provisional(code) {
            self.state = ClientState::Proceeding;
            out.push(TransactionOutput::CancelTimer(TimerKind::A));
            out.push(TransactionOutput::Deliver(resp));
        } else if is_success(code) {
            self.state = ClientState::Terminated;
            out.push(TransactionOutput::CancelTimer(TimerKind::A));
            out.push(TransactionOutput::CancelTimer(TimerKind::B));
            out.push(TransactionOutput::Deliver(resp));
            out.push(TransactionOutput::Complete);
        } else if is_failure(code) {
            self.final_response = Some(resp.clone());
            out.push(TransactionOutput::CancelTimer(TimerKind::A));
            out.push(TransactionOutput::CancelTimer(TimerKind::B));
            out.push(TransactionOutput::Deliver(resp));
            if let Some(final_response) = self.final_response.as_ref() {
                let ack = build_ack(&self.request, final_response);
                out.push(TransactionOutput::SendMessage(ack));
            }
            if self.config.timer_d().is_zero() {
                self.state = ClientState::Terminated;
                out.push(TransactionOutput::Complete);
            } else {
                self.state = ClientState::Completed;
                out.push(TransactionOutput::SetTimer {
                    kind: TimerKind::D,
                    deadline: now + self.config.timer_d(),
                });
            }
        }
        out
    }

    fn on_invite_response_completed(&mut self, resp: SipMessage) -> Vec<TransactionOutput> {
        let code = response_code(&resp).unwrap_or(0);
        if is_failure(code)
            && let Some(final_response) = self.final_response.as_ref()
        {
            let ack = build_ack(&self.request, final_response);
            return vec![TransactionOutput::SendMessage(ack)];
        }
        Vec::new()
    }

    fn on_invite_timer_a(&mut self, now: Duration) -> Vec<TransactionOutput> {
        if self.state != ClientState::Calling {
            return Vec::new();
        }
        self.retransmit_count += 1;
        vec![
            TransactionOutput::SendMessage(self.request.clone()),
            TransactionOutput::SetTimer {
                kind: TimerKind::A,
                deadline: now + self.config.timer_a(self.retransmit_count),
            },
        ]
    }

    fn on_invite_timer_b(&mut self) -> Vec<TransactionOutput> {
        self.terminate_with_failure(SipErrorKind::TransactionTimeout)
    }

    fn on_non_invite_response(
        &mut self,
        resp: SipMessage,
        now: Duration,
    ) -> Vec<TransactionOutput> {
        let code = response_code(&resp).unwrap_or(0);
        let mut out = Vec::new();

        if is_provisional(code) {
            self.state = ClientState::Proceeding;
            // After a provisional response, retransmissions continue at T2.
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::E,
                deadline: now + self.config.t2,
            });
            out.push(TransactionOutput::Deliver(resp));
        } else if (200..700).contains(&code) {
            self.state = ClientState::Completed;
            out.push(TransactionOutput::CancelTimer(TimerKind::E));
            out.push(TransactionOutput::CancelTimer(TimerKind::F));
            out.push(TransactionOutput::Deliver(resp));
            if self.config.timer_k().is_zero() {
                self.state = ClientState::Terminated;
                out.push(TransactionOutput::Complete);
            } else {
                out.push(TransactionOutput::SetTimer {
                    kind: TimerKind::K,
                    deadline: now + self.config.timer_k(),
                });
            }
        }
        out
    }

    fn on_non_invite_timer_e(&mut self, now: Duration) -> Vec<TransactionOutput> {
        if self.state != ClientState::Trying {
            return Vec::new();
        }
        self.retransmit_count += 1;
        vec![
            TransactionOutput::SendMessage(self.request.clone()),
            TransactionOutput::SetTimer {
                kind: TimerKind::E,
                deadline: now + self.config.timer_e(self.retransmit_count),
            },
        ]
    }

    fn on_non_invite_timer_e_proceeding(&mut self, now: Duration) -> Vec<TransactionOutput> {
        if self.state != ClientState::Proceeding {
            return Vec::new();
        }
        vec![
            TransactionOutput::SendMessage(self.request.clone()),
            TransactionOutput::SetTimer {
                kind: TimerKind::E,
                deadline: now + self.config.t2,
            },
        ]
    }

    fn on_non_invite_timer_f(&mut self) -> Vec<TransactionOutput> {
        self.terminate_with_failure(SipErrorKind::TransactionTimeout)
    }

    fn terminate(&mut self) -> Vec<TransactionOutput> {
        self.state = ClientState::Terminated;
        vec![TransactionOutput::Complete]
    }

    fn terminate_with_failure(&mut self, kind: SipErrorKind) -> Vec<TransactionOutput> {
        self.state = ClientState::Terminated;
        vec![
            TransactionOutput::Failure(kind),
            TransactionOutput::Complete,
        ]
    }
}

/// Builds an ACK request for a non-2xx final response to an INVITE.
///
/// Per RFC 3261 §17.1.1.3, the ACK contains the top Via of the original
/// request plus From, Call-ID, Route, Max-Forwards, the To from the response,
/// and a new CSeq with method ACK.
fn build_ack(invite: &SipMessage, response: &SipMessage) -> SipMessage {
    let (uri, mut headers) = match invite {
        SipMessage::Request { line, headers, .. } => {
            let mut h = SipHeaders::new();

            // The ACK MUST contain a single Via equal to the top Via of the
            // original request.
            if let Some(top_via) = headers.get_all(&HeaderName::Via).next() {
                h.append(HeaderName::Via, top_via.clone());
            }

            let allowed = [
                HeaderName::From,
                HeaderName::CallId,
                HeaderName::Route,
                HeaderName::MaxForwards,
            ];
            for (name, value) in headers.iter() {
                if allowed.contains(name) {
                    h.append(name.clone(), value.clone());
                }
            }
            (line.uri.clone(), h)
        }
        _ => unreachable!("client transaction stored a non-request"),
    };

    // To header from the response.
    if let Some(to) = response.headers().get(&HeaderName::To) {
        headers.append(HeaderName::To, to.clone());
    }

    // CSeq with the original sequence number and method ACK.
    let (seq, _) = invite.cseq().unwrap_or((0, Method::Invite));
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{seq} ACK")));

    SipMessage::Request {
        line: RequestLine::new(Method::Ack, uri),
        headers,
        body: Vec::new(),
    }
}
