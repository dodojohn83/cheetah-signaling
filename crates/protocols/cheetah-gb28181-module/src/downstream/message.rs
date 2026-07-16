//! `MESSAGE` handling and outbound command construction for lower platforms.

use crate::access::{
    add_or_replace_tag, build_message_response, copy_common_headers, device_from_address,
};
use crate::downstream::link::LinkTable;
use crate::downstream::{DownstreamCommand, DownstreamConfig, DownstreamError, DownstreamOutput};
use crate::events::{DevicePresence, Gb28181Event};
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
) -> Result<Vec<DownstreamOutput>, DownstreamError> {
    let SipMessage::Request { headers, body, .. } = &message else {
        return Ok(Vec::new());
    };

    let Some(from) = headers.get(&HeaderName::From) else {
        return Ok(vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            next_tag(tag_counter),
        ))]);
    };
    let Some(platform_id) = device_from_address(from.as_str()) else {
        return Ok(vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            400,
            "Bad Request",
            next_tag(tag_counter),
        ))]);
    };

    let Some(link) = links.get_mut(&platform_id) else {
        return Ok(vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            403,
            "Forbidden",
            next_tag(tag_counter),
        ))]);
    };

    if link.source != source {
        return Ok(vec![DownstreamOutput::SendResponse(build_error_response(
            &message,
            403,
            "Forbidden",
            next_tag(tag_counter),
        ))]);
    }

    let domain_id = config.domain_id().clone();

    // Parse and validate the body before mutating link state so that a
    // malformed or unrecognized message does not silently clear the offline
    // flag and lose the subsequent Online presence event.
    let root = parse_xml(body, &XmlLimits::default()).map_err(DownstreamError::Access)?;
    let cmd_type = root.child_text("CmdType").ok_or_else(|| {
        DownstreamError::Access(crate::error::AccessError::InvalidXml(
            "missing CmdType".to_string(),
        ))
    })?;

    let event = match cmd_type.as_str() {
        "Keepalive" => {
            let keepalive = parse_keepalive(body).map_err(DownstreamError::Access)?;
            Gb28181Event::Keepalive {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                status: keepalive.status,
            }
        }
        "Catalog" => {
            let catalog = parse_catalog(body).map_err(DownstreamError::Access)?;
            Gb28181Event::CatalogReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: catalog.sn,
                sum_num: catalog.sum_num,
                num: catalog.num,
                items: catalog.items,
            }
        }
        "DeviceInfo" => {
            let info = parse_device_info(body).map_err(DownstreamError::Access)?;
            Gb28181Event::DeviceInfoReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: info.sn,
                result: info.result,
                manufacturer: info.manufacturer,
                model: info.model,
                firmware: info.firmware,
            }
        }
        "DeviceStatus" => {
            let status = parse_device_status(body).map_err(DownstreamError::Access)?;
            Gb28181Event::DeviceStatusReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: status.sn,
                result: status.result,
                online: status.online,
                status: status.status,
                reason: status.reason,
                invalid_equip: status.invalid_equip,
            }
        }
        "Alarm" => {
            let alarm = parse_alarm(body).map_err(DownstreamError::Access)?;
            Gb28181Event::AlarmReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: alarm.sn,
                priority: alarm.priority,
                method: alarm.method,
                alarm_type: alarm.alarm_type,
                time: alarm.time,
                info: alarm.info,
            }
        }
        "MobilePosition" => {
            let pos = parse_mobile_position(body).map_err(DownstreamError::Access)?;
            Gb28181Event::MobilePositionReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
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
        "RecordInfo" => {
            let info = parse_record_info(body).map_err(DownstreamError::Access)?;
            Gb28181Event::RecordInfoReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: info.sn,
                name: info.name,
                sum_num: info.sum_num,
                num: info.num,
                items: info.items,
            }
        }
        "DeviceControl" => {
            let resp = parse_device_control_response(body).map_err(DownstreamError::Access)?;
            Gb28181Event::DeviceControlResponseReceived {
                domain_id: domain_id.clone(),
                device_id: platform_id.clone(),
                source,
                sn: resp.sn,
                result: resp.result,
            }
        }
        _other => {
            return Ok(vec![DownstreamOutput::SendResponse(build_error_response(
                &message,
                400,
                "Bad Request",
                next_tag(tag_counter),
            ))]);
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
    Ok(outputs)
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
