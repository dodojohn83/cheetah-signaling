//! Legal and illegal state-transition table tests for the cascade upstream
//! registration state machine (`GB4-TST-002`).
//!
//! Each row drives a fresh [`Gb28181Cascade`] to a known starting state, applies
//! a single input, and asserts both the resulting state discriminant and the
//! class of output produced. Together the rows form an explicit transition
//! table for `Idle`/`Registering`/`Registered`/`Deregistering`/`Failed`,
//! including the illegal/ignored inputs that must be no-ops. All timing uses an
//! explicit monotonic `now`; no real clock, socket or test ordering is
//! involved.

#![allow(clippy::unwrap_used)]

use crate::cascade::tests::{
    build_200, build_401, challenge_ctx, config, password_provider, request_call_id_cseq,
};
use crate::cascade::{
    CascadeConfig, CascadeCredentialProvider, CascadeError, CascadeEvent, CascadeInput,
    CascadeOutput, Gb28181Cascade,
};
use crate::events::Gb28181Event;
use cheetah_gb28181_core::{HeaderName, HeaderValue, SipHeaders, SipMessage, StatusLine};

/// A coarse classification of the primary output produced by a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A SIP request (REGISTER, keepalive, etc.) was emitted.
    Request,
    /// A `CascadePlatformConnected` event was emitted.
    Connected,
    /// A `CascadePlatformDisconnected` event was emitted.
    Disconnected,
    /// No outputs were produced (an ignored/no-op transition).
    Empty,
}

/// The expected result of a single transition-table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// The transition succeeds with the given output class and resulting state.
    Ok(Class, &'static str),
    /// The transition returns an error (state is left unchanged).
    Err,
}

fn classify(outputs: &[CascadeOutput]) -> Class {
    if outputs.iter().any(|o| {
        matches!(
            o,
            CascadeOutput::EmitEvent(Gb28181Event::CascadePlatformConnected { .. })
        )
    }) {
        return Class::Connected;
    }
    if outputs.iter().any(|o| {
        matches!(
            o,
            CascadeOutput::EmitEvent(Gb28181Event::CascadePlatformDisconnected { .. })
        )
    }) {
        return Class::Disconnected;
    }
    if outputs
        .iter()
        .any(|o| matches!(o, CascadeOutput::SendRequest(_)))
    {
        return Class::Request;
    }
    Class::Empty
}

fn machine() -> Gb28181Cascade<impl CascadeCredentialProvider> {
    Gb28181Cascade::new(config(), password_provider()).unwrap()
}

fn machine_with(cfg: CascadeConfig) -> Gb28181Cascade<impl CascadeCredentialProvider> {
    Gb28181Cascade::new(cfg, password_provider()).unwrap()
}

fn register(
    cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>,
    now: u64,
) -> (String, String) {
    let outputs = cascade
        .process(CascadeInput {
            now,
            event: CascadeEvent::Register,
        })
        .unwrap();
    request_call_id_cseq(&outputs)
}

/// Drives a machine from `Idle` to `Registered` and returns the granted
/// call-id/cseq of the final `200 OK`.
fn to_registered(cascade: &mut Gb28181Cascade<impl CascadeCredentialProvider>) {
    let (call_id, cseq) = register(cascade, 1000);
    cascade
        .process(CascadeInput {
            now: 1001,
            event: CascadeEvent::Response(Box::new(build_200(3600, &call_id, &cseq))),
        })
        .unwrap();
    assert_eq!(cascade.state_label(), "Registered");
}

fn status_response(code: u16, reason: &str, call_id: &str, cseq: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::new(cseq.to_string()));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

/// Runs one transition-table row: applies `event` and returns the produced
/// outputs together with the resulting state discriminant label.
type Row = (
    &'static str,
    Box<dyn Fn() -> (Result<Vec<CascadeOutput>, CascadeError>, &'static str)>,
    Expect,
);

fn rows() -> Vec<Row> {
    vec![
        // --- Legal transitions -------------------------------------------------
        (
            "idle + Register -> Registering (sends REGISTER)",
            Box::new(|| {
                let mut c = machine();
                let out = c.process(CascadeInput {
                    now: 1000,
                    event: CascadeEvent::Register,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Request, "Registering"),
        ),
        (
            "registering + 401 -> Registering (authenticated resend)",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let challenge = challenge_ctx().generate_challenge(1000).unwrap();
                let msg = build_401(&challenge.to_header_value(), &call_id, &cseq);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Request, "Registering"),
        ),
        (
            "registering + 200 -> Registered (connected)",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(build_200(3600, &call_id, &cseq))),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Connected, "Registered"),
        ),
        (
            "registering + 403 (retries left) -> Failed (backoff, no output)",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let msg = status_response(403, "Forbidden", &call_id, &cseq);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Failed"),
        ),
        (
            "registering + 403 (no retries) -> Idle (disconnected)",
            Box::new(|| {
                let mut cfg = config();
                cfg.max_retries = 0;
                let mut c = machine_with(cfg);
                let (call_id, cseq) = register(&mut c, 1000);
                let msg = status_response(403, "Forbidden", &call_id, &cseq);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Disconnected, "Idle"),
        ),
        (
            "registering + 302 (no retries) -> Idle (disconnected)",
            Box::new(|| {
                let mut cfg = config();
                cfg.max_retries = 0;
                let mut c = machine_with(cfg);
                let (call_id, cseq) = register(&mut c, 1000);
                let msg = status_response(302, "Moved Temporarily", &call_id, &cseq);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Disconnected, "Idle"),
        ),
        (
            "registering + 200 zero-expiry -> Idle (disconnected)",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(build_200(0, &call_id, &cseq))),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Disconnected, "Idle"),
        ),
        (
            "failed + Tick past backoff -> Registering (retry)",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let msg = status_response(403, "Forbidden", &call_id, &cseq);
                c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                })
                .unwrap();
                assert_eq!(c.state_label(), "Failed");
                // Advance well past any bounded backoff + jitter (<= max/1000 + jitter).
                let out = c.process(CascadeInput {
                    now: 1001 + 3600,
                    event: CascadeEvent::Tick,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Request, "Registering"),
        ),
        (
            "registered + Deregister -> Deregistering (sends REGISTER expires=0)",
            Box::new(|| {
                let mut c = machine();
                to_registered(&mut c);
                let out = c.process(CascadeInput {
                    now: 1002,
                    event: CascadeEvent::Deregister,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Request, "Deregistering"),
        ),
        (
            "deregistering + 200 -> Idle (disconnected)",
            Box::new(|| {
                let mut c = machine();
                to_registered(&mut c);
                let outputs = c
                    .process(CascadeInput {
                        now: 1002,
                        event: CascadeEvent::Deregister,
                    })
                    .unwrap();
                let (call_id, cseq) = request_call_id_cseq(&outputs);
                let out = c.process(CascadeInput {
                    now: 1003,
                    event: CascadeEvent::Response(Box::new(build_200(0, &call_id, &cseq))),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Disconnected, "Idle"),
        ),
        (
            "registered + Register -> Registering (explicit refresh)",
            Box::new(|| {
                let mut c = machine();
                to_registered(&mut c);
                let out = c.process(CascadeInput {
                    now: 1002,
                    event: CascadeEvent::Register,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Request, "Registering"),
        ),
        // --- Illegal / ignored transitions (must be no-ops) --------------------
        (
            "idle + Deregister -> Idle (nothing to do)",
            Box::new(|| {
                let mut c = machine();
                let out = c.process(CascadeInput {
                    now: 1000,
                    event: CascadeEvent::Deregister,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Idle"),
        ),
        (
            "idle + stray 200 response -> Idle (ignored)",
            Box::new(|| {
                let mut c = machine();
                let out = c.process(CascadeInput {
                    now: 1000,
                    event: CascadeEvent::Response(Box::new(build_200(
                        3600,
                        "call-x",
                        "1 REGISTER",
                    ))),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Idle"),
        ),
        (
            "registering + duplicate Register -> Registering (ignored)",
            Box::new(|| {
                let mut c = machine();
                register(&mut c, 1000);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Register,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Registering"),
        ),
        (
            "registering + Deregister -> Registering (ignored while in flight)",
            Box::new(|| {
                let mut c = machine();
                register(&mut c, 1000);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Deregister,
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Registering"),
        ),
        (
            "registering + response with mismatched call-id -> Registering (ignored)",
            Box::new(|| {
                let mut c = machine();
                register(&mut c, 1000);
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(build_200(
                        3600,
                        "unrelated-call-id",
                        "1 REGISTER",
                    ))),
                });
                (out, c.state_label())
            }),
            Expect::Ok(Class::Empty, "Registering"),
        ),
        // --- Error transition --------------------------------------------------
        (
            "registering + 200 with malformed Expires -> Err",
            Box::new(|| {
                let mut c = machine();
                let (call_id, cseq) = register(&mut c, 1000);
                let mut headers = SipHeaders::new();
                headers.append(HeaderName::CallId, HeaderValue::new(call_id));
                headers.append(HeaderName::CSeq, HeaderValue::new(cseq));
                headers.append(
                    HeaderName::Expires,
                    HeaderValue::new("not-a-number".to_string()),
                );
                let msg = SipMessage::Response {
                    line: StatusLine::new(200, "OK"),
                    headers,
                    body: Vec::new(),
                };
                let out = c.process(CascadeInput {
                    now: 1001,
                    event: CascadeEvent::Response(Box::new(msg)),
                });
                (out, c.state_label())
            }),
            Expect::Err,
        ),
    ]
}

#[test]
fn cascade_registration_transition_table() {
    for (name, run, expect) in rows() {
        let (result, state) = run();
        match expect {
            Expect::Ok(class, expected_state) => {
                let outputs = result
                    .unwrap_or_else(|e| panic!("[{name}] expected success, got error: {e:?}"));
                assert_eq!(
                    classify(&outputs),
                    class,
                    "[{name}] unexpected output class; outputs={outputs:?}"
                );
                assert_eq!(state, expected_state, "[{name}] unexpected resulting state");
            }
            Expect::Err => {
                assert!(
                    result.is_err(),
                    "[{name}] expected an error transition, got {result:?}"
                );
            }
        }
    }
}
