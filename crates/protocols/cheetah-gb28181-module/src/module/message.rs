//! MESSAGE request/response handling for the GB28181 state machine.

use crate::{
    output::*,
    xml::{self, Gb28181Message},
};
use cheetah_gb28181_core::{HeaderName, HeaderValue, Method, SipHeaders, SipMessage, StatusLine};
use cheetah_signal_types::UtcTimestamp;
use std::net::SocketAddr;

use super::{Gb28181Module, MESSAGE_DEDUP_WINDOW_SECONDS, RecentMessage};

impl Gb28181Module {
    /// Handles a SIP MESSAGE request, parsing the XML body and producing the
    /// appropriate `Gb28181Output` events.
    pub(super) fn handle_message(
        &mut self,
        source: SocketAddr,
        message: SipMessage,
        now: UtcTimestamp,
    ) -> Vec<Gb28181Output> {
        self.prune_recent_messages(now.as_unix_seconds() as u64);
        if self.is_message_duplicate(&message) {
            return vec![Gb28181Output::SendMessage {
                endpoint: source,
                message: ok_response(&message),
            }];
        }
        self.record_message(&message, now.as_unix_seconds() as u64);

        let body = message.body();
        let envelope = match xml::parse_xml(body, &self.config) {
            Ok(envelope) => envelope,
            Err(e) => {
                return vec![
                    Gb28181Output::ProtocolError {
                        source: Some(source),
                        kind: "xml_parse_error".into(),
                        message: e.to_string(),
                    },
                    Gb28181Output::SendMessage {
                        endpoint: source,
                        message: simple_response(&message, 400, "Bad Request"),
                    },
                ];
            }
        };
        let msg = envelope.into_message();
        let mut outputs = Vec::new();

        match msg.cmd_type.as_str() {
            "Keepalive" => {
                outputs.push(Gb28181Output::Heartbeat(Gb28181Heartbeat {
                    status: msg.status.unwrap_or_else(|| "OK".into()),
                    received_at: now,
                }));
            }
            "Catalog" => {
                if let Some(catalog) = self.aggregate_catalog(&msg) {
                    outputs.push(Gb28181Output::Catalog(catalog));
                }
            }
            "DeviceInfo" => outputs.push(Gb28181Output::DeviceInfo(Gb28181DeviceInfo {
                device_id: msg.device_id.unwrap_or_default(),
                sn: msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
                name: msg.device_name,
                manufacturer: msg.manufacturer,
                model: msg.model,
                firmware: msg.firmware,
                max_camera: msg.max_camera,
                max_alarm: msg.max_alarm,
            })),
            "DeviceStatus" => outputs.push(Gb28181Output::DeviceStatus(Gb28181DeviceStatus {
                device_id: msg.device_id.unwrap_or_default(),
                sn: msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
                result: msg.result,
                online: msg.online,
                status: msg.status,
                encode: msg.encode,
                record: msg.record,
            })),
            "Alarm" => outputs.push(Gb28181Output::Alarm(Gb28181Alarm {
                device_id: msg.device_id.unwrap_or_default(),
                sn: msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
                priority: msg.alarm_priority,
                method: msg.alarm_method,
                alarm_type: msg.alarm_type,
                alarm_time: msg.alarm_time,
                info: msg.info,
            })),
            "MobilePosition" => {
                outputs.push(Gb28181Output::MobilePosition(Gb28181MobilePosition {
                    device_id: msg.device_id.unwrap_or_default(),
                    sn: msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
                    time: msg.time,
                    longitude: msg.longitude,
                    latitude: msg.latitude,
                    speed: msg.speed,
                    direction: msg.direction,
                    altitude: msg.altitude,
                }));
            }
            "DeviceControl" | "PTZ" => {
                if let Some(sn) = msg.sn.as_deref().and_then(|s| s.parse().ok())
                    && let Some(pending) = self.pending_commands.remove(&sn)
                {
                    outputs.push(Gb28181Output::CommandResponse {
                        command_id: pending.command_id,
                        sn,
                        result: msg.result.as_deref().map_or(Gb28181CommandResult::Ok, |r| {
                            if r.eq_ignore_ascii_case("ok") {
                                Gb28181CommandResult::Ok
                            } else {
                                Gb28181CommandResult::Error(r.into())
                            }
                        }),
                    });
                }
            }
            "RecordInfo" => {
                outputs.push(Gb28181Output::RecordInfo(record_info_from_msg(&msg)));
            }
            _ => {
                outputs.push(Gb28181Output::ProtocolError {
                    source: Some(source),
                    kind: "unknown_cmd_type".into(),
                    message: format!("unknown CmdType {}", msg.cmd_type),
                });
            }
        }

        outputs.push(Gb28181Output::SendMessage {
            endpoint: source,
            message: ok_response(&message),
        });
        outputs
    }

    /// Handles a SIP response, usually a result for a previously sent MESSAGE.
    pub(super) fn handle_response(
        &mut self,
        _source: SocketAddr,
        message: SipMessage,
        _now: UtcTimestamp,
    ) -> Vec<Gb28181Output> {
        // INVITE/ACK/BYE responses are media-session handling and belong to WP-14.
        // MESSAGE responses may carry command results; attempt to parse the body.
        let code = match &message {
            SipMessage::Response { line, .. } => line.code,
            _ => return Vec::new(),
        };

        if code >= 300 {
            // Non-success final response: fail any pending command keyed by CSeq.
            if let Some((_, Method::Message)) = message.cseq()
                && let Some(sn) = extract_message_sn(&message)
                && let Some(pending) = self.pending_commands.remove(&sn)
            {
                return vec![Gb28181Output::CommandResponse {
                    command_id: pending.command_id,
                    sn,
                    result: Gb28181CommandResult::Error(format!("SIP {code}")),
                }];
            }
            return Vec::new();
        }

        let Ok(envelope) = xml::parse_xml(message.body(), &self.config) else {
            return Vec::new();
        };
        let msg = envelope.into_message();

        if let Some(sn) = msg.sn.as_deref().and_then(|s| s.parse().ok())
            && let Some(pending) = self.pending_commands.remove(&sn)
        {
            let result = if (200..300).contains(&code) {
                Gb28181CommandResult::Ok
            } else {
                Gb28181CommandResult::Error(format!("SIP {code}"))
            };
            return vec![Gb28181Output::CommandResponse {
                command_id: pending.command_id,
                sn,
                result,
            }];
        }

        Vec::new()
    }

    fn prune_recent_messages(&mut self, now_sec: u64) {
        while self.recent_messages.front().is_some_and(|entry| {
            entry.seen_at.saturating_add(MESSAGE_DEDUP_WINDOW_SECONDS) < now_sec
        }) {
            if let Some(entry) = self.recent_messages.pop_front() {
                self.recent_message_ids.remove(&(entry.call_id, entry.cseq));
            }
        }
    }

    fn is_message_duplicate(&self, message: &SipMessage) -> bool {
        let call_id = message.call_id().unwrap_or("");
        let cseq = message.cseq().map(|(n, _)| n).unwrap_or(0);
        self.recent_message_ids
            .contains(&(call_id.to_string(), cseq))
    }

    fn record_message(&mut self, message: &SipMessage, now_sec: u64) {
        let call_id = message.call_id().unwrap_or("").to_string();
        let cseq = message.cseq().map(|(n, _)| n).unwrap_or(0);
        if self.recent_message_ids.insert((call_id.clone(), cseq)) {
            self.recent_messages.push_back(RecentMessage {
                call_id,
                cseq,
                seen_at: now_sec,
            });
        }
    }
}

pub(super) fn extract_message_sn(message: &SipMessage) -> Option<u32> {
    let body = std::str::from_utf8(message.body()).ok()?;
    let sn_text = body.lines().find(|line| line.contains("<SN>"))?;
    let sn_text = sn_text.trim();
    let start = sn_text.find("<SN>").map(|i| i + "<SN>".len())?;
    let end = sn_text.rfind("</SN>")?;
    sn_text[start..end].trim().parse().ok()
}

pub(super) fn ok_response(request: &SipMessage) -> SipMessage {
    simple_response(request, 200, "OK")
}

pub(super) fn unauthorized_response(request: &SipMessage, www_auth: &str) -> SipMessage {
    let mut response = simple_response(request, 401, "Unauthorized");
    response
        .headers_mut()
        .append(HeaderName::WwwAuthenticate, HeaderValue::new(www_auth));
    response
}

pub(super) fn simple_response(request: &SipMessage, code: u16, reason: &str) -> SipMessage {
    let mut headers = SipHeaders::new();
    for name in [
        HeaderName::Via,
        HeaderName::From,
        HeaderName::To,
        HeaderName::CallId,
        HeaderName::CSeq,
    ] {
        for value in request.headers().get_all(&name) {
            headers.append(name.clone(), value.clone());
        }
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

pub(super) fn record_info_from_msg(msg: &Gb28181Message) -> Gb28181RecordInfo {
    let items = msg
        .record_list
        .as_ref()
        .map(|list| {
            list.item
                .iter()
                .map(|item| Gb28181RecordItem {
                    device_id: item.device_id.clone().unwrap_or_default(),
                    name: item.name.clone(),
                    file_path: item.file_path.clone(),
                    address: item.address.clone(),
                    start_time: item.start_time.clone(),
                    end_time: item.end_time.clone(),
                    secrecy: item.secrecy.clone(),
                    type_field: item.type_field.clone(),
                    recorder_id: item.recorder_id.clone(),
                    file_size: item.file_size,
                })
                .collect()
        })
        .unwrap_or_default();
    Gb28181RecordInfo {
        device_id: msg.device_id.clone().unwrap_or_default(),
        sn: msg.sn.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
        sum_num: msg.sum_num.unwrap_or(0),
        items,
        complete: true,
    }
}
