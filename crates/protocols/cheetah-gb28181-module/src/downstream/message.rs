//! `MESSAGE` handling and outbound command construction for lower platforms.

use crate::access::{
    add_or_replace_tag, build_message_response, copy_common_headers, device_from_address,
};
use crate::cascade::validate_token;
use crate::downstream::link::LinkTable;
use crate::downstream::{DownstreamCommand, DownstreamConfig, DownstreamError, DownstreamOutput};
use crate::events::{DevicePresence, Gb28181Event};
use crate::types::DeviceId;
use crate::xml::{
    XmlLimits, build_catalog_query, parse_alarm, parse_catalog, parse_device_control_response,
    parse_device_info, parse_device_status, parse_keepalive, parse_mobile_position,
    parse_record_info, parse_xml,
};
use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, StatusLine,
};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Handles an inbound `MESSAGE` request from a registered lower platform.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_message(
    config: &DownstreamConfig,
    links: &mut LinkTable,
    tag_counter: &AtomicU64,
    source: SocketAddr,
    now: u64,
    message: SipMessage,
) -> Vec<DownstreamOutput> {
    let SipMessage::Request { headers, body, .. } = &message else {
        return Vec::new();
    };

    let tag = next_tag(tag_counter);

    let Some(from) = headers.get(&HeaderName::From) else {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            tag,
        ))];
    };
    let Some(platform_id) = device_from_address(from.as_str()) else {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            tag,
        ))];
    };

    let Some(link) = links.get_mut(&platform_id) else {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            403,
            "Forbidden",
            tag,
        ))];
    };

    if link.source != source {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            403,
            "Forbidden",
            tag,
        ))];
    }

    let domain_id = config.domain_id().clone();

    // Parse the body once and validate CmdType and DeviceID before mutating
    // link state or dispatching to a typed parser.
    let root = match parse_xml(body, &XmlLimits::default()) {
        Ok(root) => root,
        Err(_) => {
            return vec![DownstreamOutput::SendResponse(build_error_response(
                &message,
                400,
                "Bad Request",
                tag,
            ))];
        }
    };

    let Some(cmd_type) = root.child_text("CmdType") else {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            tag,
        ))];
    };
    let Some(xml_device_id) = root.child_text("DeviceID") else {
        return vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            tag,
        ))];
    };
    let xml_device_id = xml_device_id.trim();

    // Platform-level messages must identify the platform itself. Device-level
    // notifications carry a sub-device/channel identifier and must be a valid
    // GB28181 device id, but need not equal the platform id.
    let device_id = match cmd_type.as_str() {
        "Keepalive" | "Catalog" | "DeviceInfo" => {
            if xml_device_id != platform_id.as_ref() {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
            platform_id.clone()
        }
        "DeviceStatus" | "Alarm" | "MobilePosition" | "RecordInfo" | "DeviceControl" => {
            match DeviceId::new(xml_device_id) {
                Some(id) => id,
                None => {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
            }
        }
        _other => {
            return vec![DownstreamOutput::SendResponse(build_error_response(
                &message,
                400,
                "Bad Request",
                tag,
            ))];
        }
    };

    let event = match cmd_type.as_str() {
        "Keepalive" => match parse_keepalive(body) {
            Ok(keepalive) => {
                if keepalive.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::Keepalive {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    status: keepalive.status,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "Catalog" => match parse_catalog(body) {
            Ok(catalog) => {
                if catalog.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::CatalogReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: catalog.sn,
                    sum_num: catalog.sum_num,
                    num: catalog.num,
                    items: catalog.items,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "DeviceInfo" => match parse_device_info(body) {
            Ok(info) => {
                if info.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::DeviceInfoReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: info.sn,
                    result: info.result,
                    manufacturer: info.manufacturer,
                    model: info.model,
                    firmware: info.firmware,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "DeviceStatus" => match parse_device_status(body) {
            Ok(status) => {
                if status.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::DeviceStatusReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: status.sn,
                    result: status.result,
                    online: status.online,
                    status: status.status,
                    reason: status.reason,
                    invalid_equip: status.invalid_equip,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "Alarm" => match parse_alarm(body) {
            Ok(alarm) => {
                if alarm.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::AlarmReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: alarm.sn,
                    priority: alarm.priority,
                    method: alarm.method,
                    alarm_type: alarm.alarm_type,
                    time: alarm.time,
                    info: alarm.info,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "MobilePosition" => match parse_mobile_position(body) {
            Ok(pos) => {
                if pos.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::MobilePositionReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: pos.sn,
                    time: pos.time,
                    longitude: pos.longitude,
                    latitude: pos.latitude,
                    speed: pos.speed,
                    direction: pos.direction,
                    altitude: pos.altitude,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "RecordInfo" => match parse_record_info(body) {
            Ok(info) => {
                if info.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::RecordInfoReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: info.sn,
                    name: info.name,
                    sum_num: info.sum_num,
                    num: info.num,
                    items: info.items,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        "DeviceControl" => match parse_device_control_response(body) {
            Ok(resp) => {
                if resp.device_id != device_id.as_ref() {
                    return vec![DownstreamOutput::SendResponse(build_error_response(
                        &message,
                        400,
                        "Bad Request",
                        tag,
                    ))];
                }
                Gb28181Event::DeviceControlResponseReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: resp.sn,
                    result: resp.result,
                }
            }
            Err(_) => {
                return vec![DownstreamOutput::SendResponse(build_error_response(
                    &message,
                    400,
                    "Bad Request",
                    tag,
                ))];
            }
        },
        _other => {
            return vec![DownstreamOutput::SendResponse(build_error_response(
                &message,
                400,
                "Bad Request",
                tag,
            ))];
        }
    };

    link.last_seen = now;
    let was_offline = link.offline;
    link.offline = false;

    let mut outputs = Vec::with_capacity(3);
    if was_offline {
        outputs.push(DownstreamOutput::EmitEvent(
            Gb28181Event::DevicePresenceChanged {
                domain_id,
                device_id: platform_id.clone(),
                source,
                presence: DevicePresence::Online,
            },
        ));
    }

    outputs.push(DownstreamOutput::EmitEvent(event));
    outputs.push(DownstreamOutput::SendResponse(build_message_response(
        &message,
        next_tag(tag_counter),
    )));
    outputs
}

/// Handles an outbound command to a registered lower platform.
pub(crate) fn handle_command(
    config: &DownstreamConfig,
    links: &mut LinkTable,
    tag_counter: &AtomicU64,
    _now: u64,
    command: DownstreamCommand,
) -> Result<Vec<DownstreamOutput>, DownstreamError> {
    match command {
        DownstreamCommand::QueryCatalog { platform_id, sn } => {
            let link = links
                .get_mut(&platform_id)
                .ok_or(DownstreamError::NotRegistered)?;
            let cseq = link.next_cseq;
            link.next_cseq = link.next_cseq.saturating_add(1);
            let branch = next_branch(tag_counter);
            let call_id = format!("query-{}", next_tag(tag_counter));
            let body = build_catalog_query(&sn, platform_id.as_ref())
                .map_err(|e| DownstreamError::Encode(e.to_string()))?;
            let request =
                build_message_request(config, link, cseq, &call_id, &branch, body.into_bytes())?;
            let destination = link.source;
            Ok(vec![DownstreamOutput::SendRequest(request, destination)])
        }
    }
}

fn build_message_request(
    config: &DownstreamConfig,
    link: &crate::downstream::link::PlatformLink,
    cseq: u32,
    call_id: &str,
    branch: &str,
    body: Body,
) -> Result<SipMessage, DownstreamError> {
    validate_token(call_id).map_err(|_| DownstreamError::InvalidToken)?;
    validate_token(branch).map_err(|_| DownstreamError::InvalidToken)?;
    validate_token(&link.local_tag).map_err(|_| DownstreamError::InvalidToken)?;

    let local_uri = config.local_uri();
    let local_host = local_uri.host();
    let local_port = local_uri.port().unwrap_or(5060);

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", local_host, local_port, branch)
            .map_err(|e| DownstreamError::Encode(e.to_string()))?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(local_uri, &link.local_tag)
            .map_err(|e| DownstreamError::Encode(e.to_string()))?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&link.contact));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Message));
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, link.contact.clone()),
        headers,
        body,
    })
}

fn build_error_response(request: &SipMessage, code: u16, reason: &str, tag: String) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

fn next_tag(tag_counter: &AtomicU64) -> String {
    let n = tag_counter.fetch_add(1, Ordering::Relaxed);
    format!("gb{n}")
}

fn next_branch(tag_counter: &AtomicU64) -> String {
    let n = tag_counter.fetch_add(1, Ordering::Relaxed);
    format!("z9hG4bK-{n}")
}
