//! Webhook configuration and delivery aggregates.

use crate::{Clock, DomainError, IdGenerator, Result};
use cheetah_signal_types::{DeliveryId, EventId, Revision, TenantId, UtcTimestamp, WebhookId};

/// Maximum byte length of a webhook target URL.
const MAX_WEBHOOK_URL_BYTES: usize = 2048;
/// Maximum byte length of a webhook secret reference.
const MAX_WEBHOOK_SECRET_REF_BYTES: usize = 256;
/// Maximum number of subscribed event types on a webhook.
const MAX_WEBHOOK_EVENT_TYPES: usize = 64;
/// Maximum byte length of a single webhook event type name.
pub const MAX_WEBHOOK_EVENT_TYPE_BYTES: usize = 128;
/// Maximum byte length of a webhook delivery payload.
const MAX_WEBHOOK_PAYLOAD_BYTES: usize = 1_048_576;
/// Maximum byte length of a webhook delivery last-error message.
const MAX_WEBHOOK_LAST_ERROR_BYTES: usize = 1024;

/// State of a webhook delivery attempt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// Waiting to be attempted.
    #[default]
    Pending,
    /// Currently in flight.
    InProgress,
    /// Delivered successfully.
    Succeeded,
    /// Failed but may be retried.
    Failed,
    /// Exhausted retries; manual replay required.
    DeadLetter,
}

impl std::fmt::Display for DeliveryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::DeadLetter => "dead_letter",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for DeliveryStatus {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self> {
        let status = if s.eq_ignore_ascii_case("pending") {
            Self::Pending
        } else if s.eq_ignore_ascii_case("in_progress") {
            Self::InProgress
        } else if s.eq_ignore_ascii_case("succeeded") {
            Self::Succeeded
        } else if s.eq_ignore_ascii_case("failed") {
            Self::Failed
        } else if s.eq_ignore_ascii_case("dead_letter") {
            Self::DeadLetter
        } else {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown delivery status: {display}"
            )));
        };
        Ok(status)
    }
}

/// Validates that a webhook target URL has an allowed scheme and a host that is
/// not obviously internal, loopback, link-local, multicast, or a well-known
/// cloud metadata endpoint. DNS-based SSRF checks are performed again at delivery
/// time because the resolution may change.
fn validate_webhook_url(url: &str) -> Result<()> {
    if url.len() > MAX_WEBHOOK_URL_BYTES {
        return Err(DomainError::invalid_argument("webhook url too long"));
    }
    let parsed = url::Url::parse(url)
        .map_err(|e| DomainError::invalid_argument(format!("invalid webhook url: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DomainError::invalid_argument(
            "webhook url scheme must be http or https",
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| DomainError::invalid_argument("webhook url must have a host"))?;
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".local")
        || host.eq_ignore_ascii_case("metadata")
        || host.eq_ignore_ascii_case("metadata.google.internal")
    {
        return Err(DomainError::invalid_argument(
            "webhook url host is not allowed",
        ));
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>()
        && is_disallowed_ip(&ip)
    {
        return Err(DomainError::invalid_argument(
            "webhook url points to a disallowed address",
        ));
    }
    Ok(())
}

fn is_disallowed_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        std::net::IpAddr::V6(v6) => is_disallowed_ipv6(v6),
    }
}

fn is_disallowed_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_private()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
}

fn is_disallowed_ipv6(v6: &std::net::Ipv6Addr) -> bool {
    if v6.is_loopback()
        || v6.is_multicast()
        || v6.is_unspecified()
        || v6.is_unicast_link_local()
        || v6.is_unique_local()
    {
        return true;
    }
    if let Some(v4) = v6.to_ipv4() {
        return is_disallowed_ipv4(&v4);
    }
    false
}

fn validate_secret_ref(secret_ref: &str) -> Result<()> {
    if secret_ref.is_empty() {
        return Err(DomainError::invalid_argument(
            "webhook secret_ref must not be empty",
        ));
    }
    if secret_ref.len() > MAX_WEBHOOK_SECRET_REF_BYTES {
        return Err(DomainError::invalid_argument("webhook secret_ref too long"));
    }
    Ok(())
}

/// Validates a single webhook event type name.
pub fn validate_event_type(event_type: &str) -> Result<()> {
    if event_type.is_empty() {
        return Err(DomainError::invalid_argument(
            "webhook event type must not be empty",
        ));
    }
    if event_type.len() > MAX_WEBHOOK_EVENT_TYPE_BYTES {
        return Err(DomainError::invalid_argument("webhook event type too long"));
    }
    Ok(())
}

fn validate_event_types(event_types: &[String]) -> Result<()> {
    if event_types.len() > MAX_WEBHOOK_EVENT_TYPES {
        return Err(DomainError::invalid_argument(
            "too many webhook event types",
        ));
    }
    for event_type in event_types {
        validate_event_type(event_type)?;
    }
    Ok(())
}

/// Maximum byte length of an HMAC-SHA256 hex signature.
const MAX_WEBHOOK_SIGNATURE_BYTES: usize = 256;

fn validate_last_error(error: &str) -> Result<()> {
    if error.len() > MAX_WEBHOOK_LAST_ERROR_BYTES {
        return Err(DomainError::invalid_argument(
            "webhook last error message too long",
        ));
    }
    Ok(())
}

/// A configured outbound webhook endpoint.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WebhookConfig {
    webhook_id: WebhookId,
    tenant_id: TenantId,
    url: String,
    secret_ref: String,
    event_types: Vec<String>,
    enabled: bool,
    revision: Revision,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl WebhookConfig {
    /// Creates a new webhook configuration.
    pub fn new(
        clock: &dyn Clock,
        id_generator: &dyn IdGenerator,
        tenant_id: TenantId,
        url: String,
        secret_ref: String,
        event_types: Vec<String>,
    ) -> Result<Self> {
        if url.is_empty() {
            return Err(DomainError::invalid_argument(
                "webhook url must not be empty",
            ));
        }
        validate_webhook_url(&url)?;
        validate_secret_ref(&secret_ref)?;
        validate_event_types(&event_types)?;
        let now = clock.now_wall();
        Ok(Self {
            webhook_id: id_generator.generate_webhook_id(),
            tenant_id,
            url,
            secret_ref,
            event_types,
            enabled: true,
            revision: Revision::default(),
            created_at: now,
            updated_at: now,
        })
    }

    /// Updates mutable fields and bumps the revision.
    pub fn update(
        &mut self,
        clock: &dyn Clock,
        url: Option<String>,
        secret_ref: Option<String>,
        event_types: Option<Vec<String>>,
        enabled: Option<bool>,
    ) -> Result<()> {
        if let Some(url) = url {
            if url.is_empty() {
                return Err(DomainError::invalid_argument(
                    "webhook url must not be empty",
                ));
            }
            validate_webhook_url(&url)?;
            self.url = url;
        }
        if let Some(secret_ref) = secret_ref {
            validate_secret_ref(&secret_ref)?;
            self.secret_ref = secret_ref;
        }
        if let Some(event_types) = event_types {
            validate_event_types(&event_types)?;
            self.event_types = event_types;
        }
        if let Some(enabled) = enabled {
            self.enabled = enabled;
        }
        self.bump_revision(clock);
        Ok(())
    }

    /// Returns true when the configuration is enabled and the event type matches.
    pub fn matches_event(&self, event_type: &str) -> bool {
        self.enabled
            && (self.event_types.is_empty() || self.event_types.iter().any(|t| t == event_type))
    }

    fn bump_revision(&mut self, clock: &dyn Clock) {
        self.revision.0 += 1;
        self.updated_at = clock.now_wall();
    }

    /// Webhook identifier.
    pub fn webhook_id(&self) -> WebhookId {
        self.webhook_id
    }

    /// Tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Target URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Secret reference.
    pub fn secret_ref(&self) -> &str {
        &self.secret_ref
    }

    /// Subscribed event types; empty means all events.
    pub fn event_types(&self) -> &[String] {
        &self.event_types
    }

    /// Whether the webhook is enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Optimistic concurrency revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Last update timestamp.
    pub fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }
}

/// A single webhook delivery attempt.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WebhookDelivery {
    delivery_id: DeliveryId,
    tenant_id: TenantId,
    webhook_id: WebhookId,
    event_id: EventId,
    payload: Vec<u8>,
    signature: String,
    status: DeliveryStatus,
    attempt_count: u32,
    next_attempt_at: Option<UtcTimestamp>,
    last_error: Option<String>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl WebhookDelivery {
    /// Creates a pending delivery for an event payload.
    pub fn new(
        clock: &dyn Clock,
        id_generator: &dyn IdGenerator,
        tenant_id: TenantId,
        webhook_id: WebhookId,
        event_id: EventId,
        payload: Vec<u8>,
    ) -> Result<Self> {
        if payload.len() > MAX_WEBHOOK_PAYLOAD_BYTES {
            return Err(DomainError::invalid_argument("webhook payload too large"));
        }
        let now = clock.now_wall();
        Ok(Self {
            delivery_id: id_generator.generate_delivery_id(),
            tenant_id,
            webhook_id,
            event_id,
            payload,
            signature: String::new(),
            status: DeliveryStatus::Pending,
            attempt_count: 0,
            next_attempt_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// Starts a delivery attempt, bumping the counter.
    pub fn start_attempt(&mut self, clock: &dyn Clock) {
        self.attempt_count += 1;
        self.status = DeliveryStatus::InProgress;
        self.updated_at = clock.now_wall();
    }

    /// Records a successful delivery.
    pub fn succeed(&mut self, clock: &dyn Clock) {
        self.status = DeliveryStatus::Succeeded;
        self.next_attempt_at = None;
        self.last_error = None;
        self.updated_at = clock.now_wall();
    }

    /// Records a failed attempt and schedules the next retry.
    pub fn fail(
        &mut self,
        clock: &dyn Clock,
        error: String,
        next_attempt_at: Option<UtcTimestamp>,
    ) -> Result<()> {
        validate_last_error(&error)?;
        self.status = DeliveryStatus::Failed;
        self.last_error = Some(error);
        self.next_attempt_at = next_attempt_at;
        self.updated_at = clock.now_wall();
        Ok(())
    }

    /// Moves the delivery to the dead-letter state.
    pub fn dead_letter(&mut self, clock: &dyn Clock, error: String) -> Result<()> {
        validate_last_error(&error)?;
        self.status = DeliveryStatus::DeadLetter;
        self.last_error = Some(error);
        self.next_attempt_at = None;
        self.updated_at = clock.now_wall();
        Ok(())
    }

    /// Sets the HMAC signature after the payload has been signed.
    pub fn set_signature(&mut self, signature: String) -> Result<()> {
        if signature.len() > MAX_WEBHOOK_SIGNATURE_BYTES {
            return Err(DomainError::invalid_argument("webhook signature too long"));
        }
        self.signature = signature;
        Ok(())
    }

    /// Delivery identifier.
    pub fn delivery_id(&self) -> DeliveryId {
        self.delivery_id
    }

    /// Tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Webhook identifier.
    pub fn webhook_id(&self) -> WebhookId {
        self.webhook_id
    }

    /// Event identifier.
    pub fn event_id(&self) -> EventId {
        self.event_id
    }

    /// Payload bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// HMAC signature.
    pub fn signature(&self) -> &str {
        &self.signature
    }

    /// Current status.
    pub fn status(&self) -> DeliveryStatus {
        self.status
    }

    /// Number of attempts made.
    pub fn attempt_count(&self) -> u32 {
        self.attempt_count
    }

    /// Scheduled next attempt, if any.
    pub fn next_attempt_at(&self) -> Option<UtcTimestamp> {
        self.next_attempt_at
    }

    /// Last recorded error.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Last update timestamp.
    pub fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }
}

/// Computes an HMAC-SHA256 hex signature over a canonical payload.
///
/// The signed material is `{delivery_id}|{event_id}|{timestamp}|{body}`
/// so the signature binds the delivery, event and timestamp to the body.
pub fn sign_webhook_payload(
    secret: &str,
    delivery_id: &str,
    event_id: &str,
    timestamp: &str,
    body: &[u8],
) -> Result<String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| DomainError::internal(format!("invalid hmac key: {e}")))?;
    mac.update(delivery_id.as_bytes());
    mac.update(b"|");
    mac.update(event_id.as_bytes());
    mac.update(b"|");
    mac.update(timestamp.as_bytes());
    mac.update(b"|");
    mac.update(body);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::IdGenerator;

    #[test]
    fn webhook_config_rejects_oversized_url() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let url = format!("http://example.com/{}", "x".repeat(2048));
        let result = WebhookConfig::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            url,
            "ref".to_string(),
            vec![],
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn webhook_config_rejects_oversized_secret_ref() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let result = WebhookConfig::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            "http://example.com/hook".to_string(),
            "x".repeat(257),
            vec![],
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn webhook_config_rejects_too_many_event_types() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let event_types: Vec<String> = (0..65).map(|i| format!("event.{i}")).collect();
        let result = WebhookConfig::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            "http://example.com/hook".to_string(),
            "ref".to_string(),
            event_types,
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn validate_event_type_rejects_empty_and_oversized() {
        assert!(matches!(
            validate_event_type(""),
            Err(DomainError::InvalidArgument { .. })
        ));
        assert!(validate_event_type(&"x".repeat(MAX_WEBHOOK_EVENT_TYPE_BYTES)).is_ok());
        assert!(matches!(
            validate_event_type(&"x".repeat(MAX_WEBHOOK_EVENT_TYPE_BYTES + 1)),
            Err(DomainError::InvalidArgument { .. })
        ));
    }

    #[test]
    fn webhook_config_rejects_oversized_event_type() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let result = WebhookConfig::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            "http://example.com/hook".to_string(),
            "ref".to_string(),
            vec!["x".repeat(MAX_WEBHOOK_EVENT_TYPE_BYTES + 1)],
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn webhook_delivery_rejects_oversized_payload() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let result = WebhookDelivery::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            ids.generate_webhook_id(),
            ids.generate_event_id(),
            vec![0u8; MAX_WEBHOOK_PAYLOAD_BYTES + 1],
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn webhook_delivery_rejects_oversized_error() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let mut delivery = match WebhookDelivery::new(
            &clock,
            &ids,
            ids.generate_tenant_id(),
            ids.generate_webhook_id(),
            ids.generate_event_id(),
            Vec::new(),
        ) {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        };
        let result = delivery.fail(&clock, "x".repeat(MAX_WEBHOOK_LAST_ERROR_BYTES + 1), None);
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn delivery_status_from_str_is_case_insensitive_and_bounds_error() {
        let parsed = "In_Progress".parse::<DeliveryStatus>();
        assert!(matches!(parsed, Ok(DeliveryStatus::InProgress)));

        let result = "x".repeat(1024).parse::<DeliveryStatus>();
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }
}
