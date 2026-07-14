//! Webhook configuration and delivery aggregates.

use crate::{Clock, DomainError, IdGenerator, Result};
use cheetah_signal_types::{DeliveryId, EventId, Revision, TenantId, UtcTimestamp, WebhookId};

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
        match s {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "dead_letter" => Ok(Self::DeadLetter),
            _ => Err(DomainError::invalid_argument(format!(
                "unknown delivery status: {s}"
            ))),
        }
    }
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
        if secret_ref.is_empty() {
            return Err(DomainError::invalid_argument(
                "webhook secret_ref must not be empty",
            ));
        }
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
            self.url = url;
        }
        if let Some(secret_ref) = secret_ref {
            if secret_ref.is_empty() {
                return Err(DomainError::invalid_argument(
                    "webhook secret_ref must not be empty",
                ));
            }
            self.secret_ref = secret_ref;
        }
        if let Some(event_types) = event_types {
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
    ) -> Self {
        let now = clock.now_wall();
        Self {
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
        }
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
    ) {
        self.status = DeliveryStatus::Failed;
        self.last_error = Some(error);
        self.next_attempt_at = next_attempt_at;
        self.updated_at = clock.now_wall();
    }

    /// Moves the delivery to the dead-letter state.
    pub fn dead_letter(&mut self, clock: &dyn Clock, error: String) {
        self.status = DeliveryStatus::DeadLetter;
        self.last_error = Some(error);
        self.next_attempt_at = None;
        self.updated_at = clock.now_wall();
    }

    /// Sets the HMAC signature after the payload has been signed.
    pub fn set_signature(&mut self, signature: String) {
        self.signature = signature;
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
