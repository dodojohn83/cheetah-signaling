#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]

use super::*;
use cheetah_onvif_services::services::MediaDialect;
use cheetah_plugin_sdk::{CommandSource, DeviceSink, PluginName, ProtocolEvent, ResourceBudget};
use secrecy::{ExposeSecret, SecretString};

#[test]
fn parse_dialect_values() {
    assert_eq!(parse_dialect(Some("media1")), MediaDialect::Media1);
    assert_eq!(parse_dialect(Some("media2")), MediaDialect::Media2);
    assert_eq!(parse_dialect(Some("Media1")), MediaDialect::Media1);
    assert_eq!(parse_dialect(Some("MEDIA2")), MediaDialect::Media2);
    assert_eq!(parse_dialect(None), MediaDialect::Media2);
    assert_eq!(parse_dialect(Some("unknown")), MediaDialect::Media2);
}

#[test]
fn credentials_require_both_username_and_password() {
    use secrecy::SecretString;

    assert!(
        make_credentials(
            Some("admin"),
            Some(SecretString::from("secret".to_string())),
            false,
            0
        )
        .unwrap()
        .is_some()
    );
    assert!(
        make_credentials(Some("admin"), None, false, 0)
            .unwrap()
            .is_none()
    );
    assert!(
        make_credentials(
            None,
            Some(SecretString::from("secret".to_string())),
            false,
            0
        )
        .unwrap()
        .is_none()
    );
    assert!(
        make_credentials(
            Some(""),
            Some(SecretString::from("secret".to_string())),
            false,
            0
        )
        .unwrap()
        .is_none()
    );
    assert!(
        make_credentials(
            Some("admin"),
            Some(SecretString::from("".to_string())),
            false,
            0
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn credentials_reject_oversized_username_and_password() {
    use secrecy::SecretString;

    let oversized_username = "a".repeat(MAX_ONVIF_USERNAME_BYTES + 1);
    assert!(
        make_credentials(
            Some(&oversized_username),
            Some(SecretString::from("secret".to_string())),
            false,
            0
        )
        .is_err()
    );

    let oversized_password = "a".repeat(MAX_ONVIF_PASSWORD_BYTES + 1);
    assert!(
        make_credentials(
            Some("admin"),
            Some(SecretString::from(oversized_password)),
            false,
            0
        )
        .is_err()
    );
}

#[tokio::test]
async fn resolve_credentials_prefers_secret_provider_ref() {
    let ctx = FakeDriverContext::with_secrets(&[
        ("onvif.default.password", "default_secret"),
        ("per_device_password", "device_secret"),
    ]);
    let mut config = OnvifConfig::default();
    config.default_username = Some("admin".to_string());
    config.default_credentials_ref = Some("onvif.default.password".to_string());

    let creds = resolve_credentials(
        &ctx,
        &config,
        Some("device_user"),
        Some("per_device_password"),
        None,
        false,
        0,
    )
    .await
    .expect("resolve should succeed")
    .expect("credentials should be present");

    assert_eq!(creds.username, "device_user");
    assert_eq!(creds.password.expose_secret(), "device_secret");
}

#[tokio::test]
async fn resolve_credentials_falls_back_to_config_defaults() {
    let ctx = FakeDriverContext::with_secret("onvif.default.password", "fallback");
    let mut config = OnvifConfig::default();
    config.default_username = Some("admin".to_string());
    config.default_credentials_ref = Some("onvif.default.password".to_string());

    let creds = resolve_credentials(&ctx, &config, None, None, None, false, 0)
        .await
        .expect("resolve should succeed")
        .expect("credentials should be present");

    assert_eq!(creds.username, "admin");
    assert_eq!(creds.password.expose_secret(), "fallback");
}

#[tokio::test]
async fn resolve_credentials_returns_error_for_missing_secret() {
    let ctx = FakeDriverContext::with_secret("other", "value");
    let mut config = OnvifConfig::default();
    config.default_credentials_ref = Some("missing".to_string());

    let err = resolve_credentials(&ctx, &config, Some("admin"), None, None, false, 0)
        .await
        .expect_err("missing secret should error");
    assert!(err.to_string().contains("missing"));
}

#[tokio::test]
async fn resolve_credentials_prefers_inline_password_over_config_default() {
    let ctx = FakeDriverContext::with_secret("onvif.default.password", "default_secret");
    let mut config = OnvifConfig::default();
    config.default_username = Some("admin".to_string());
    config.default_credentials_ref = Some("onvif.default.password".to_string());

    let creds = resolve_credentials(
        &ctx,
        &config,
        Some("device_user"),
        None,
        Some("inline_secret"),
        false,
        0,
    )
    .await
    .expect("resolve should succeed")
    .expect("credentials should be present");

    assert_eq!(creds.username, "device_user");
    assert_eq!(creds.password.expose_secret(), "inline_secret");
}

struct FakeDeviceSink;

#[async_trait]
impl DeviceSink for FakeDeviceSink {
    async fn emit_event(&self, _event: ProtocolEvent) -> Result<(), PluginError> {
        Ok(())
    }
}

struct FakeCommandSource;

#[async_trait]
impl CommandSource for FakeCommandSource {
    async fn next_command(&self) -> Result<Option<DriverCommand>, PluginError> {
        Ok(None)
    }
}

struct FakeDriverContext {
    secrets: HashMap<String, SecretString>,
}

impl FakeDriverContext {
    fn with_secret(name: &str, value: &str) -> Self {
        let mut secrets = HashMap::new();
        secrets.insert(name.to_string(), SecretString::from(value.to_string()));
        Self { secrets }
    }

    fn with_secrets(pairs: &[(&str, &str)]) -> Self {
        let secrets = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), SecretString::from(v.to_string())))
            .collect();
        Self { secrets }
    }
}

#[async_trait]
impl DriverContext for FakeDriverContext {
    fn plugin_name(&self) -> &PluginName {
        use std::sync::LazyLock;
        static NAME: LazyLock<PluginName> =
            LazyLock::new(|| PluginName::new("cheetah/test").unwrap());
        &NAME
    }

    fn config(&self) -> &serde_json::Value {
        use std::sync::LazyLock;
        static CONFIG: LazyLock<serde_json::Value> = LazyLock::new(|| serde_json::Value::Null);
        &CONFIG
    }

    fn budget(&self) -> &ResourceBudget {
        use std::sync::LazyLock;
        static BUDGET: LazyLock<ResourceBudget> = LazyLock::new(ResourceBudget::default);
        &BUDGET
    }

    fn monotonic_now(&self) -> cheetah_plugin_sdk::MonotonicSeconds {
        0
    }

    fn device_sink(&self) -> &dyn DeviceSink {
        static SINK: FakeDeviceSink = FakeDeviceSink;
        &SINK
    }

    fn command_source(&self) -> &dyn CommandSource {
        static SOURCE: FakeCommandSource = FakeCommandSource;
        &SOURCE
    }

    async fn secret(&self, name: &str) -> Result<Option<SecretString>, PluginError> {
        Ok(self.secrets.get(name).cloned())
    }

    async fn request_media_session(
        &self,
        _params: serde_json::Value,
        _timeout: DurationMs,
    ) -> Result<String, PluginError> {
        Err(PluginError::unsupported(
            "media session not available in tests",
        ))
    }

    async fn register_endpoint(
        &self,
        _protocol: &str,
        _address: &str,
    ) -> Result<String, PluginError> {
        Err(PluginError::unsupported(
            "endpoint registration not available in tests",
        ))
    }
}

#[tokio::test]
async fn imaging_write_commands_return_unsupported() {
    use cheetah_signal_types::UtcTimestamp;

    let driver = OnvifTokioProtocolDriver::new();
    let ctx = FakeDriverContext::with_secret("onvif.default.password", "secret");
    let deadline = UtcTimestamp::parse_rfc3339("9999-12-31T23:59:59Z").unwrap();

    for command_type in [
        "set_imaging_settings",
        "set_focus_configuration",
        "set_exposure",
        "set_white_balance",
        "set_focus",
    ] {
        let command = DriverCommand {
            command_type: command_type.to_string(),
            payload: serde_json::json!({}),
            idempotency_key: format!("test-{command_type}"),
            deadline,
        };
        let result = driver
            .handle_command(&ctx, command, DurationMs::from_millis(1_000))
            .await;
        assert!(
            matches!(result, Err(PluginError::Unsupported(_))),
            "{command_type} should be unsupported, got {result:?}"
        );
    }
}

#[test]
fn clock_offset_matches_device_minus_local_time() {
    use cheetah_onvif_core::services::system_date_time::{DateTime, SystemDateAndTime};

    let local = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
    let device = local + time::Duration::seconds(37);
    let system = SystemDateAndTime {
        date_time_type: "NTP".to_string(),
        daylight_savings: false,
        timezone: None,
        utc: DateTime {
            year: device.year(),
            month: device.month() as u8,
            day: device.day(),
            hour: device.hour(),
            minute: device.minute(),
            second: device.second(),
        },
        local: None,
    };

    let offset = clock_offset_seconds_with_local(&system, local).unwrap();
    assert_eq!(offset, 37);
}

#[test]
fn ptz_continuous_move_clips_velocity_components_to_unit_range() {
    let cmd = PtzContinuousMoveCommand {
        ptz_endpoint: "http://192.0.2.10/onvif/ptz".into(),
        profile_token: "profile1".into(),
        pan: 1.5,
        tilt: -2.0,
        zoom: 0.5,
        timeout_seconds: 5,
        timeout_ms: None,
        username: None,
        credentials_ref: None,
        password: None,
        password_text: false,
        clock_offset_seconds: 0,
    };
    let velocity = cmd.velocity();
    assert_eq!(velocity.pan, 1.0);
    assert_eq!(velocity.tilt, -1.0);
    assert_eq!(velocity.zoom, 0.5);
}

#[test]
fn parse_tenant_id_rejects_malformed_input() {
    assert!(parse_tenant_id(None).unwrap().is_none());
    assert!(parse_tenant_id(Some("")).unwrap().is_none());
    assert!(parse_tenant_id(Some("not-a-uuid")).is_err());
}
