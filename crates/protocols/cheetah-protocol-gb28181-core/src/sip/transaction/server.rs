//! RFC 3261 server transaction state machine.

use super::state_machine::{
    TransactionEvent, TransactionOutput, is_failure, is_provisional, is_success, request_method,
    response_code,
};
use super::timers::{TimerKind, TransactionConfig};
use crate::{Method, SipErrorKind, SipMessage};
use std::time::Duration;

/// Server transaction FSM for INVITE and non-INVITE requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerTransaction {
    config: TransactionConfig,
    request: SipMessage,
    is_invite: bool,
    state: ServerState,
    /// Most recent provisional response from the TU, used for retransmission.
    provisional_response: Option<SipMessage>,
    /// Final response from the TU, used for retransmission.
    final_response: Option<SipMessage>,
    /// Count of final-response retransmissions (timer G for INVITE).
    retransmit_count: u32,
    /// A CANCEL was received while the INVITE transaction was pending.
    cancelled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ServerState {
    /// non-INVITE server initial state.
    Trying,
    /// INVITE server initial state; also non-INVITE after 1xx.
    Proceeding,
    /// Final response (3xx-6xx for INVITE) sent; waiting for ACK or timer H.
    Completed,
    /// ACK received for a non-2xx INVITE final response.
    Confirmed,
    Terminated,
}

impl ServerTransaction {
    pub fn new(request: SipMessage, config: TransactionConfig) -> Self {
        let is_invite = request_method(&request) == Some(Method::Invite);
        let state = if is_invite {
            ServerState::Proceeding
        } else {
            ServerState::Trying
        };
        Self {
            config,
            request,
            is_invite,
            state,
            provisional_response: None,
            final_response: None,
            retransmit_count: 0,
            cancelled: false,
        }
    }

    pub fn is_terminated(&self) -> bool {
        matches!(self.state, ServerState::Terminated)
    }

    pub fn process(&mut self, event: TransactionEvent, now: Duration) -> Vec<TransactionOutput> {
        use ServerState::*;
        use TransactionEvent::*;

        match (&self.state, event) {
            (Proceeding, Request(req)) if self.is_invite => self.on_invite_request(req, now),
            (Completed, Request(req)) if self.is_invite => {
                self.on_invite_request_completed(req, now)
            }
            (Confirmed, Request(req)) if self.is_invite => self.on_invite_request_confirmed(req),
            (Proceeding | Completed, Response(resp)) if self.is_invite => {
                self.on_invite_tu_response(resp, now)
            }
            (Proceeding | Completed, Timer(TimerKind::G)) if self.is_invite => {
                self.on_invite_timer_g(now)
            }
            (Completed, Timer(TimerKind::H)) if self.is_invite => self.on_invite_timer_h(),
            (Confirmed, Timer(TimerKind::I)) if self.is_invite => self.terminate(),
            (Trying, Request(req)) if !self.is_invite => self.on_non_invite_request_trying(req),
            (Proceeding, Request(req)) if !self.is_invite => {
                self.on_non_invite_request_proceeding(req, now)
            }
            (Completed, Request(req)) if !self.is_invite => {
                self.on_non_invite_request_completed(req)
            }
            (Trying | Proceeding, Response(resp)) if !self.is_invite => {
                self.on_non_invite_tu_response(resp, now)
            }
            (Completed, Timer(TimerKind::J)) if !self.is_invite => self.terminate(),
            (_, TransportError) => self.terminate_with_failure(SipErrorKind::TransportFailure),
            _ => Vec::new(),
        }
    }

    /// Outputs to emit immediately after creating a server transaction: deliver
    /// the request to the TU.
    pub fn bootstrap(&mut self) -> Vec<TransactionOutput> {
        vec![TransactionOutput::Deliver(self.request.clone())]
    }

    fn on_invite_request(&mut self, req: SipMessage, now: Duration) -> Vec<TransactionOutput> {
        match request_method(&req) {
            Some(Method::Invite) => {
                if let Some(resp) = &self.provisional_response {
                    vec![TransactionOutput::SendMessage(resp.clone())]
                } else {
                    Vec::new()
                }
            }
            Some(Method::Ack) => {
                // ACK in Proceeding is unexpected; absorb it but inform TU.
                vec![TransactionOutput::Deliver(req)]
            }
            Some(Method::Cancel) => self.on_invite_cancel(req, now),
            _ => Vec::new(),
        }
    }

    fn on_invite_request_completed(
        &mut self,
        req: SipMessage,
        now: Duration,
    ) -> Vec<TransactionOutput> {
        match request_method(&req) {
            Some(Method::Invite) => {
                if let Some(resp) = &self.final_response {
                    let mut out = vec![TransactionOutput::SendMessage(resp.clone())];
                    if self.config.transport.is_reliable() {
                        // No timer G on reliable transports.
                    } else if !out.iter().any(|o| {
                        matches!(
                            o,
                            TransactionOutput::SetTimer {
                                kind: TimerKind::G,
                                ..
                            }
                        )
                    }) {
                        // Ensure timer G continues after a retransmission-triggered send.
                        self.retransmit_count += 1;
                        out.push(TransactionOutput::SetTimer {
                            kind: TimerKind::G,
                            deadline: now + self.config.timer_g(self.retransmit_count),
                        });
                    }
                    out
                } else {
                    Vec::new()
                }
            }
            Some(Method::Ack) => self.on_invite_ack(req, now),
            Some(Method::Cancel) => self.on_invite_cancel(req, now),
            _ => Vec::new(),
        }
    }

    fn on_invite_request_confirmed(&mut self, req: SipMessage) -> Vec<TransactionOutput> {
        // Absorb additional ACKs and INVITE retransmissions after confirmation.
        if let Some(Method::Ack) = request_method(&req) {
            // Still pass ACK to TU for dialog processing.
            vec![TransactionOutput::Deliver(req)]
        } else {
            Vec::new()
        }
    }

    fn on_invite_tu_response(&mut self, resp: SipMessage, now: Duration) -> Vec<TransactionOutput> {
        let code = response_code(&resp).unwrap_or(0);
        let mut out = Vec::new();

        if is_provisional(code) {
            self.provisional_response = Some(resp.clone());
            self.state = ServerState::Proceeding;
            out.push(TransactionOutput::SendMessage(resp));
        } else if is_success(code) {
            out.push(TransactionOutput::SendMessage(resp));
            self.state = ServerState::Terminated;
            out.push(TransactionOutput::Complete);
        } else if is_failure(code) {
            self.final_response = Some(resp.clone());
            self.state = ServerState::Completed;
            out.push(TransactionOutput::SendMessage(resp));
            if !self.config.transport.is_reliable() {
                out.push(TransactionOutput::SetTimer {
                    kind: TimerKind::G,
                    deadline: now + self.config.timer_g(0),
                });
            }
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::H,
                deadline: now + self.config.timer_h(),
            });
        }
        out
    }

    fn on_invite_timer_g(&mut self, now: Duration) -> Vec<TransactionOutput> {
        if let Some(resp) = &self.final_response {
            self.retransmit_count += 1;
            vec![
                TransactionOutput::SendMessage(resp.clone()),
                TransactionOutput::SetTimer {
                    kind: TimerKind::G,
                    deadline: now + self.config.timer_g(self.retransmit_count),
                },
            ]
        } else {
            Vec::new()
        }
    }

    fn on_invite_timer_h(&mut self) -> Vec<TransactionOutput> {
        self.terminate_with_failure(SipErrorKind::TransactionTimeout)
    }

    fn on_invite_ack(&mut self, req: SipMessage, now: Duration) -> Vec<TransactionOutput> {
        let mut out = vec![TransactionOutput::Deliver(req)];
        self.state = ServerState::Confirmed;
        out.push(TransactionOutput::CancelTimer(TimerKind::G));
        out.push(TransactionOutput::CancelTimer(TimerKind::H));
        if self.config.timer_i().is_zero() {
            out.extend(self.terminate());
        } else {
            out.push(TransactionOutput::SetTimer {
                kind: TimerKind::I,
                deadline: now + self.config.timer_i(),
            });
        }
        out
    }

    fn on_invite_cancel(&mut self, req: SipMessage, _now: Duration) -> Vec<TransactionOutput> {
        self.cancelled = true;
        let mut out = vec![TransactionOutput::Deliver(req)];
        if self.final_response.is_none() {
            // The TU is responsible for generating a 487 final response. The
            // cancelled flag ensures no further provisional responses are sent.
            out.push(TransactionOutput::Failure(
                SipErrorKind::TransactionCancelled,
            ));
        }
        out
    }

    fn on_non_invite_request_trying(&mut self, req: SipMessage) -> Vec<TransactionOutput> {
        // Request retransmissions in Trying are silently discarded.
        if request_method(&req) == request_method(&self.request) {
            Vec::new()
        } else {
            // A stray/mismatched request is delivered to the TU.
            vec![TransactionOutput::Deliver(req)]
        }
    }

    fn on_non_invite_request_proceeding(
        &mut self,
        req: SipMessage,
        _now: Duration,
    ) -> Vec<TransactionOutput> {
        if request_method(&req) == request_method(&self.request) {
            if let Some(resp) = &self.provisional_response {
                vec![TransactionOutput::SendMessage(resp.clone())]
            } else {
                Vec::new()
            }
        } else {
            vec![TransactionOutput::Deliver(req)]
        }
    }

    fn on_non_invite_request_completed(&mut self, req: SipMessage) -> Vec<TransactionOutput> {
        if request_method(&req) == request_method(&self.request) {
            if let Some(resp) = &self.final_response {
                vec![TransactionOutput::SendMessage(resp.clone())]
            } else {
                Vec::new()
            }
        } else {
            vec![TransactionOutput::Deliver(req)]
        }
    }

    fn on_non_invite_tu_response(
        &mut self,
        resp: SipMessage,
        now: Duration,
    ) -> Vec<TransactionOutput> {
        let code = response_code(&resp).unwrap_or(0);
        let mut out = Vec::new();

        if is_provisional(code) {
            self.provisional_response = Some(resp.clone());
            self.state = ServerState::Proceeding;
            out.push(TransactionOutput::SendMessage(resp));
        } else if (200..700).contains(&code) {
            self.final_response = Some(resp.clone());
            self.state = ServerState::Completed;
            out.push(TransactionOutput::SendMessage(resp));
            if self.config.timer_j().is_zero() {
                self.state = ServerState::Terminated;
                out.push(TransactionOutput::Complete);
            } else {
                out.push(TransactionOutput::SetTimer {
                    kind: TimerKind::J,
                    deadline: now + self.config.timer_j(),
                });
            }
        }
        out
    }

    fn terminate(&mut self) -> Vec<TransactionOutput> {
        self.state = ServerState::Terminated;
        vec![TransactionOutput::Complete]
    }

    fn terminate_with_failure(&mut self, kind: SipErrorKind) -> Vec<TransactionOutput> {
        self.state = ServerState::Terminated;
        vec![
            TransactionOutput::Failure(kind),
            TransactionOutput::Complete,
        ]
    }
}
