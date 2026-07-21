//! JSON body extractor that maps rejections to RFC 9457 Problem Details.

use crate::HttpError;
use axum::{
    extract::{FromRequest, Request},
    http::header,
};
use serde::de::DeserializeOwned;
use std::sync::Arc;

use crate::ApiState;

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
        if !content_type
            .split(';')
            .next()
            .unwrap_or("")
            .eq_ignore_ascii_case("application/json")
            && !content_type.is_empty()
            && !content_type
                .split(';')
                .next()
                .unwrap_or("")
                .eq_ignore_ascii_case("application/*+json")
        {
            return Err(HttpError::Signal(cheetah_signal_types::SignalError::new(
                cheetah_signal_types::SignalErrorKind::InvalidArgument,
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
