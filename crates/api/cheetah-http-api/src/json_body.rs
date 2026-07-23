//! JSON body extractor that maps rejections to RFC 9457 Problem Details.

use crate::HttpError;
use axum::{
    extract::{FromRequest, Request},
    http::header,
};
use cheetah_signal_types::{SignalError, SignalErrorKind};
use serde::de::DeserializeOwned;
use std::sync::Arc;

use crate::ApiState;

/// Maximum byte length of the `Content-Type` header accepted by [`JsonBody`].
const MAX_CONTENT_TYPE_BYTES: usize = 256;

/// Drop-in replacement for [`axum::Json`] that converts parse failures into
/// [`HttpError::InvalidJson`] (HTTP 400 + Problem Details) instead of Axum's
/// default plain-text 422 response.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonBody<T>(pub T);

impl<T> std::ops::Deref for JsonBody<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for JsonBody<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> FromRequest<Arc<ApiState>> for JsonBody<T>
where
    T: DeserializeOwned,
{
    type Rejection = HttpError;

    async fn from_request(req: Request, state: &Arc<ApiState>) -> Result<Self, Self::Rejection> {
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type.is_empty() && !is_json_content_type(content_type) {
            return Err(HttpError::Signal(SignalError::new(
                SignalErrorKind::InvalidArgument,
                "Content-Type must be application/json",
            )));
        }

        let bytes = axum::body::to_bytes(req.into_body(), state.config.request_body_limit_bytes)
            .await
            .map_err(|e| {
                HttpError::Signal(cheetah_signal_types::SignalError::new(
                    cheetah_signal_types::SignalErrorKind::InvalidArgument,
                    format!("failed to read request body: {e}"),
                ))
            })?;

        let value = serde_json::from_slice::<T>(&bytes).map_err(HttpError::from)?;
        Ok(Self(value))
    }
}

/// Returns `true` if `content_type` is `application/json` or any
/// `application/<subtype>+json` media type (e.g. `application/vnd.api+json`).
fn is_json_content_type(content_type: &str) -> bool {
    if content_type.len() > MAX_CONTENT_TYPE_BYTES {
        return false;
    }
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if media_type.eq_ignore_ascii_case("application/json") {
        return true;
    }
    let Some((ty, subtype)) = media_type.split_once('/') else {
        return false;
    };
    ty.eq_ignore_ascii_case("application")
        && (subtype.eq_ignore_ascii_case("json") || ends_with_ignore_ascii_case(subtype, "+json"))
}

/// Case-insensitive `str::ends_with` without allocating.
fn ends_with_ignore_ascii_case(value: &str, suffix: &str) -> bool {
    if value.len() < suffix.len() {
        return false;
    }
    value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_application_json() {
        assert!(is_json_content_type("application/json"));
    }

    #[test]
    fn accepts_json_with_charset() {
        assert!(is_json_content_type("application/json; charset=utf-8"));
    }

    #[test]
    fn accepts_json_vendor_suffix() {
        assert!(is_json_content_type("application/vnd.api+json"));
    }

    #[test]
    fn accepts_json_vendor_suffix_with_charset() {
        assert!(is_json_content_type(
            "application/vnd.api+json; charset=utf-8"
        ));
    }

    #[test]
    fn rejects_text_plain() {
        assert!(!is_json_content_type("text/plain"));
    }

    #[test]
    fn rejects_application_xml() {
        assert!(!is_json_content_type("application/xml"));
    }

    #[test]
    fn rejects_oversized_content_type() {
        assert!(!is_json_content_type(
            &("a/".to_string() + &"x".repeat(MAX_CONTENT_TYPE_BYTES))
        ));
    }
}
