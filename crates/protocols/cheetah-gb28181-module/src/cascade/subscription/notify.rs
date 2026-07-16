//! SIP NOTIFY request construction for active upstream subscriptions.

use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
};

use super::Subscription;
use crate::cascade::{CascadeCredentialProvider, CascadeError, Gb28181Cascade, validate_token};
use crate::xml::{build_alarm_notify, build_catalog_response, build_mobile_position_notify};

/// Builds an outbound `NOTIFY` request for the supplied subscription.
pub(super) fn build_notify<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    sub: &Subscription,
    cseq: u32,
    branch: &str,
    subscription_state: &str,
    _now: u64,
) -> Result<SipMessage, CascadeError> {
    validate_token(branch)?;
    validate_token(&sub.local_tag)?;
    validate_token(&sub.remote_tag)?;

    let body = build_notify_body(cascade, sub, cseq)?;
    let local_host = cascade.config.local_uri.host();
    let local_port = cascade.config.local_uri.port().unwrap_or(5060);

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", local_host, local_port, branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&cascade.config.local_uri, &sub.local_tag)?,
    );
    headers.append(
        HeaderName::To,
        HeaderValue::from_uri(&sub.remote_uri, &sub.remote_tag)?,
    );
    headers.append(HeaderName::CallId, HeaderValue::new(sub.call_id.clone()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Notify));
    headers.append(
        HeaderName::Other("Event".to_string()),
        HeaderValue::new(sub.event_package.clone()),
    );
    headers.append(
        HeaderName::Other("Subscription-State".to_string()),
        HeaderValue::new(subscription_state.to_string()),
    );
    headers.append(
        HeaderName::Contact,
        HeaderValue::contact_uri(&cascade.config.local_uri),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    if let Some(ua) = &cascade.config.user_agent {
        headers.append(HeaderName::UserAgent, HeaderValue::new(ua.clone()));
    }
    if !body.is_empty() {
        headers.append(
            HeaderName::ContentType,
            HeaderValue::new("Application/MANSCDP+xml"),
        );
        headers.append(
            HeaderName::ContentLength,
            HeaderValue::new(body.len().to_string()),
        );
    } else {
        headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    }

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Notify, sub.remote_uri.clone()),
        headers,
        body,
    })
}

fn build_notify_body<P: CascadeCredentialProvider>(
    cascade: &Gb28181Cascade<P>,
    sub: &Subscription,
    sn: u32,
) -> Result<Body, CascadeError> {
    let sn = sn.to_string();
    let device_id = cascade.platform_id().to_string();
    let xml = match sub.event_package.as_str() {
        "Catalog" => build_catalog_response(&sn, &device_id, 0, &[]).map_err(|e| {
            CascadeError::Internal(format!("failed to encode Catalog NOTIFY body: {e}"))
        })?,
        "Alarm" => {
            build_alarm_notify(&sn, &device_id, None, None, None, None, None).map_err(|e| {
                CascadeError::Internal(format!("failed to encode Alarm NOTIFY body: {e}"))
            })?
        }
        "MobilePosition" => {
            build_mobile_position_notify(&sn, &device_id, None, None, None, None, None, None)
                .map_err(|e| {
                    CascadeError::Internal(format!(
                        "failed to encode MobilePosition NOTIFY body: {e}"
                    ))
                })?
        }
        _ => String::new(),
    };
    Ok(xml.into_bytes())
}
