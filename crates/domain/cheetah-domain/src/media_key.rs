//! Stable media key encoding for logical media streams.
//!
//! A `MediaKey` identifies a logical stream within a tenant using the encoding
//! described in `dev-docs/001_next_generation_signaling/05_media_plane_integration.md`:
//! the key value is `{tenant_id}/{app}/{stream_id}`, where `tenant_id` is the
//! canonical UUID string, `app` is the stream category (e.g. `live`, `playback`,
//! `download`, `talk`), and `stream_id` is the canonical UUID string of the
//! channel or media session that identifies the stream.

/// A stable media key scoped to a tenant.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MediaKey {
    /// Resource kind, e.g. `rtp`, `proxy`, `record`, `snapshot`, `playback`.
    pub kind: String,
    /// Encoded key value: `{tenant_id}/{app}/{stream_id}`.
    pub value: String,
}

impl MediaKey {
    /// Encodes a media key from its components.
    ///
    /// `kind` names the resource category as used by the media plane.
    /// `app` is the stream category such as `live`, `playback`, `download` or `talk`.
    /// `stream_id` is the canonical identifier of the channel or media session.
    pub fn encode(
        kind: impl Into<String>,
        tenant_id: impl std::fmt::Display,
        app: impl Into<String>,
        stream_id: impl std::fmt::Display,
    ) -> Self {
        let kind = kind.into();
        let app = app.into();
        let value = format!("{}/{}/{}", tenant_id, app, stream_id);
        Self { kind, value }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cheetah_signal_types::{MediaSessionId, TenantId};
    use std::str::FromStr;

    #[test]
    fn encode_live_key() {
        let tenant = TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap();
        let stream = MediaSessionId::from_str("22222222-2222-2222-2222-222222222222").unwrap();
        let key = MediaKey::encode("rtp", tenant, "live", stream);
        assert_eq!(key.kind, "rtp");
        assert_eq!(
            key.value,
            "11111111-1111-1111-1111-111111111111/live/22222222-2222-2222-2222-222222222222"
        );
    }

    #[test]
    fn encode_playback_key() {
        let tenant = TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap();
        let stream = MediaSessionId::from_str("33333333-3333-3333-3333-333333333333").unwrap();
        let key = MediaKey::encode("rtp", tenant, "playback", stream);
        assert_eq!(key.kind, "rtp");
        assert_eq!(
            key.value,
            "11111111-1111-1111-1111-111111111111/playback/33333333-3333-3333-3333-333333333333"
        );
    }
}
