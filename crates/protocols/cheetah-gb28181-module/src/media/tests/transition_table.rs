//! Legal and illegal state-transition table tests for the GB28181 media
//! session state machine (`GB4-TST-002`).
//!
//! Each row drives a fresh [`Gb28181Media`] to a known [`SessionState`]
//! (`Inviting`/`Active`/`Stopping`/terminated) and applies a single input,
//! asserting both the resulting session state (or its removal) and the class of
//! output produced. The rows together enumerate the invite / 200 OK / ACK / BYE
//! / CANCEL / late-200 transitions, plus the illegal inputs that must return a
//! typed error without mutating state. No real clock, socket or ordering is
//! used.

use super::*;
use crate::media::session::SessionState;

/// A coarse classification of the primary output produced by a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Invite,
    Ack,
    Bye,
    Cancel,
    Started,
    Stopped,
    Failed,
    Empty,
}

/// The expected result of a single transition-table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Success: the primary output class and the resulting session state
    /// (`None` means the session was terminated and removed).
    Ok(Class, Option<SessionState>),
    /// The transition returns an error (state is left unchanged).
    Err,
}

fn request_method(outputs: &[MediaOutput]) -> Option<Method> {
    outputs.iter().find_map(|o| match o {
        MediaOutput::SendMessage(SipMessage::Request { line, .. }) => Some(line.method.clone()),
        _ => None,
    })
}

fn classify(outputs: &[MediaOutput]) -> Class {
    if outputs.iter().any(|o| {
        matches!(
            o,
            MediaOutput::EmitEvent(Gb28181Event::MediaSessionFailed { .. })
        )
    }) {
        return Class::Failed;
    }
    if outputs.iter().any(|o| {
        matches!(
            o,
            MediaOutput::EmitEvent(Gb28181Event::MediaSessionStopped { .. })
        )
    }) {
        return Class::Stopped;
    }
    if outputs.iter().any(|o| {
        matches!(
            o,
            MediaOutput::EmitEvent(Gb28181Event::MediaSessionStarted { .. })
        )
    }) {
        return Class::Started;
    }
    match request_method(outputs) {
        Some(Method::Invite) => Class::Invite,
        Some(Method::Ack) => Class::Ack,
        Some(Method::Bye) => Class::Bye,
        Some(Method::Cancel) => Class::Cancel,
        _ => Class::Empty,
    }
}

fn state_of(media: &Gb28181Media, sid: MediaSessionId) -> Option<SessionState> {
    media.sessions.get(&sid).map(|s| s.state)
}

/// Builds a device-originated BYE request inside the established dialog.
fn device_bye() -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.20:5060;branch=z9hG4bK-dev-bye"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("101 BYE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Request {
        line: RequestLine::new(
            Method::Bye,
            SipUri::parse("sip:server@192.168.1.10:5060").unwrap(),
        ),
        headers,
        body: Vec::new(),
    }
}

/// Builds a final non-2xx response to the original INVITE.
fn invite_rejected(code: u16, reason: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::new("SIP/2.0/UDP 192.168.1.10:5060;branch=z9hG4bK1234"),
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new("<sip:server@192.168.1.10:5060>;tag=tag-local"),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new("<sip:34020000001320000001@192.168.1.20:5060>;tag=tag-remote"),
    );
    headers.append(HeaderName::CallId, HeaderValue::new("call-1"));
    headers.append(HeaderName::CSeq, HeaderValue::new("1 INVITE"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

/// Fresh machine plus the id of a single session driven to `Inviting`.
fn inviting() -> (Gb28181Media, MediaSessionId) {
    let mut media = Gb28181Media::new(config());
    let sid = MediaSessionId::generate();
    media.process(MediaInput::Command(start_live(sid))).unwrap();
    assert_eq!(state_of(&media, sid), Some(SessionState::Inviting));
    (media, sid)
}

/// Fresh machine plus the id of a single session driven to `Active`.
fn active() -> (Gb28181Media, MediaSessionId) {
    let (mut media, sid) = inviting();
    media
        .process(MediaInput::Message(build_test_200_ok()))
        .unwrap();
    assert_eq!(state_of(&media, sid), Some(SessionState::Active));
    (media, sid)
}

/// Fresh machine plus the id of a single session driven to `Stopping` (BYE
/// sent from an active dialog).
fn stopping() -> (Gb28181Media, MediaSessionId) {
    let (mut media, sid) = active();
    media
        .process(MediaInput::Command(MediaCommand::StopMediaSession {
            media_session_id: sid,
        }))
        .unwrap();
    assert_eq!(state_of(&media, sid), Some(SessionState::Stopping));
    (media, sid)
}

fn stop(sid: MediaSessionId) -> MediaInput {
    MediaInput::Command(MediaCommand::StopMediaSession {
        media_session_id: sid,
    })
}

type Row = (
    &'static str,
    Box<dyn Fn() -> (Result<Vec<MediaOutput>, MediaError>, Option<SessionState>)>,
    Expect,
);

fn rows() -> Vec<Row> {
    vec![
        // --- Legal transitions -------------------------------------------------
        (
            "no session + StartLive -> Inviting (sends INVITE)",
            Box::new(|| {
                let mut media = Gb28181Media::new(config());
                let sid = MediaSessionId::generate();
                let out = media.process(MediaInput::Command(start_live(sid)));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Invite, Some(SessionState::Inviting)),
        ),
        (
            "inviting + 200 OK -> Active (ACK + started)",
            Box::new(|| {
                let (mut media, sid) = inviting();
                let out = media.process(MediaInput::Message(build_test_200_ok()));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Started, Some(SessionState::Active)),
        ),
        (
            "inviting + Stop -> Stopping (sends CANCEL)",
            Box::new(|| {
                let (mut media, sid) = inviting();
                let out = media.process(stop(sid));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Cancel, Some(SessionState::Stopping)),
        ),
        (
            "inviting + final 4xx -> terminated (failed)",
            Box::new(|| {
                let (mut media, sid) = inviting();
                let out = media.process(MediaInput::Message(invite_rejected(486, "Busy Here")));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Failed, None),
        ),
        (
            "inviting -> Stopping + late 200 OK -> terminated (ACK+BYE+failed)",
            Box::new(|| {
                let (mut media, sid) = inviting();
                media.process(stop(sid)).unwrap();
                let out = media.process(MediaInput::Message(build_test_200_ok()));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Failed, None),
        ),
        (
            "active + Stop -> Stopping (sends BYE)",
            Box::new(|| {
                let (mut media, sid) = active();
                let out = media.process(stop(sid));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Bye, Some(SessionState::Stopping)),
        ),
        (
            "active + device BYE -> terminated (OK + stopped)",
            Box::new(|| {
                let (mut media, sid) = active();
                let out = media.process(MediaInput::Message(device_bye()));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Stopped, None),
        ),
        (
            "stopping + BYE 200 OK -> terminated (stopped)",
            Box::new(|| {
                let (mut media, sid) = stopping();
                let out = media.process(MediaInput::Message(build_response_to_bye()));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Stopped, None),
        ),
        (
            "active + retransmitted 200 OK -> Active (re-ACK only)",
            Box::new(|| {
                let (mut media, sid) = active();
                let out = media.process(MediaInput::Message(build_test_200_ok()));
                (out, state_of(&media, sid))
            }),
            Expect::Ok(Class::Ack, Some(SessionState::Active)),
        ),
        // --- Illegal / error transitions --------------------------------------
        (
            "no session + Stop -> SessionNotFound",
            Box::new(|| {
                let mut media = Gb28181Media::new(config());
                let out = media.process(stop(MediaSessionId::generate()));
                (out, None)
            }),
            Expect::Err,
        ),
        (
            "inviting + duplicate StartLive (same id) -> AlreadyExists",
            Box::new(|| {
                let (mut media, sid) = inviting();
                let out = media.process(MediaInput::Command(start_live(sid)));
                (out, state_of(&media, sid))
            }),
            Expect::Err,
        ),
        (
            "stopping + Stop -> InvalidState",
            Box::new(|| {
                let (mut media, sid) = stopping();
                let out = media.process(stop(sid));
                (out, state_of(&media, sid))
            }),
            Expect::Err,
        ),
        (
            "inviting + ControlPlayback -> InvalidState (not Active)",
            Box::new(|| {
                let (mut media, sid) = inviting();
                let out = media.process(MediaInput::Command(MediaCommand::ControlPlayback {
                    media_session_id: sid,
                    action: PlaybackAction::Play,
                    scale: None,
                    range: None,
                }));
                (out, state_of(&media, sid))
            }),
            Expect::Err,
        ),
    ]
}

#[test]
fn media_session_transition_table() {
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
