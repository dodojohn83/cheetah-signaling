//! Idempotent inbox command consumer.

use cheetah_domain::{
    Clock, Command, DeviceOwnerResolver, DomainError, ProcessedMessageRecord,
    ProcessedMessageStatus, UnitOfWork,
};
use cheetah_message_api::{bus::RawCommandBus, mapper::decode_command, subject::command_subject};
use cheetah_signal_types::{DurationMs, NodeId, Result, SignalError};
use cheetah_storage_api::Storage;
use serde::Serialize;
use std::sync::Arc;
use tracing::{info, warn};

/// Inbox-level receipt for a command delivery.
///
/// This describes what happened to the message from the inbox perspective,
/// independent of the operation step outcome or transport dispatch status.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxReceipt {
    /// The command was accepted and will be dispatched.
    Accepted,
    /// The delivery was a duplicate of an already processed message.
    Duplicate,
    /// The command was rejected before dispatch.
    Rejected {
        /// Stable reason for rejection.
        reason: String,
    },
    /// The command was dead-lettered and will not be retried.
    DeadLetter {
        /// Stable reason the message was dead-lettered.
        reason: String,
    },
}

/// Status of the dispatch attempt for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandDispatch {
    /// The command was queued for dispatch.
    Queued,
    /// The command was sent to the downstream handler/driver.
    Sent,
    /// The command could not be sent due to a transport error.
    TransportFailed {
        /// Stable reason for the transport failure.
        reason: String,
    },
    /// The dispatch attempt timed out before an acknowledgment.
    TimedOut,
}

/// Outcome of the operation step that a command represents.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStepOutcome {
    /// The operation step completed successfully.
    Succeeded,
    /// The operation step failed with a stable reason.
    Failed {
        /// Stable reason for the failure.
        reason: String,
    },
    /// The final outcome is not yet known; the operation remains active.
    Unknown,
    /// The operation step was cancelled before completion.
    Cancelled,
}

/// Result of handling a single command inside the inbox.
///
/// The result separates three concerns that were previously conflated into a
/// single `ProcessedMessageStatus::Completed`:
///
/// 1. `receipt` - what the inbox should record as the delivery disposition.
/// 2. `dispatch` - what happened when the command was handed to the driver/plugin.
/// 3. `outcome` - the business outcome of the operation step, if known.
#[derive(Clone, Debug)]
pub struct CommandHandlerResult {
    /// Inbox-level receipt for the delivery.
    pub receipt: InboxReceipt,
    /// Optional dispatch status reported by the handler.
    pub dispatch: Option<CommandDispatch>,
    /// Optional operation step outcome reported by the handler.
    pub outcome: Option<OperationStepOutcome>,
    /// Optional JSON-encoded result payload.
    pub result_payload: Option<String>,
}

impl CommandHandlerResult {
    /// Returns a receipt of `Accepted` with the given dispatch status and
    /// operation step outcome.
    pub fn accepted(dispatch: CommandDispatch, outcome: OperationStepOutcome) -> Self {
        Self {
            receipt: InboxReceipt::Accepted,
            dispatch: Some(dispatch),
            outcome: Some(outcome),
            result_payload: None,
        }
    }

    /// Returns a receipt of `Rejected` with the given reason.
    pub fn rejected(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            receipt: InboxReceipt::Rejected {
                reason: reason.clone(),
            },
            dispatch: None,
            outcome: Some(OperationStepOutcome::Failed { reason }),
            result_payload: None,
        }
    }

    /// Returns a receipt of `DeadLetter` with the given reason.
    pub fn dead_letter(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            receipt: InboxReceipt::DeadLetter {
                reason: reason.clone(),
            },
            dispatch: None,
            outcome: Some(OperationStepOutcome::Failed { reason }),
            result_payload: None,
        }
    }

    /// Returns a receipt of `Duplicate` with no dispatch or outcome.
    pub fn duplicate() -> Self {
        Self {
            receipt: InboxReceipt::Duplicate,
            dispatch: None,
            outcome: None,
            result_payload: None,
        }
    }

    /// Attaches a JSON result payload to the result.
    pub fn with_payload(mut self, payload: String) -> Self {
        self.result_payload = Some(payload);
        self
    }
}

/// Handles a decoded [`Command`] and returns a result payload.
#[async_trait::async_trait]
pub trait CommandHandler: Send + Sync {
    /// Handles the command within the provided unit of work.
    ///
    /// The handler is responsible for any aggregate side effects; the inbox
    /// service ensures the command is only processed once.
    async fn handle(
        &self,
        uow: &mut dyn UnitOfWork,
        command: &Command,
    ) -> Result<CommandHandlerResult>;
}

/// Idempotent consumer for inbound [`CommandEnvelope`] deliveries.
pub struct InboxService {
    storage: Arc<dyn Storage>,
    command_bus: Arc<dyn RawCommandBus>,
    owner_resolver: Arc<dyn DeviceOwnerResolver>,
    command_handler: Arc<dyn CommandHandler>,
    clock: Arc<dyn Clock>,
    this_node: NodeId,
    record_ttl: DurationMs,
}

impl std::fmt::Debug for InboxService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboxService")
            .field("this_node", &self.this_node)
            .field("record_ttl", &self.record_ttl)
            .finish_non_exhaustive()
    }
}

impl InboxService {
    /// Creates a new inbox service.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage: Arc<dyn Storage>,
        command_bus: Arc<dyn RawCommandBus>,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
        command_handler: Arc<dyn CommandHandler>,
        clock: Arc<dyn Clock>,
        this_node: NodeId,
        record_ttl: DurationMs,
    ) -> Self {
        Self {
            storage,
            command_bus,
            owner_resolver,
            command_handler,
            clock,
            this_node,
            record_ttl,
        }
    }

    /// Subscribes to `subject` and processes deliveries until the subscription
    /// is closed or an unrecoverable error occurs.
    pub async fn run(&self, subject: &str, consumer_group: &str) -> Result<()> {
        let mut subscription = self
            .command_bus
            .subscribe(subject, consumer_group)
            .await
            .map_err(|e| SignalError::from(DomainError::internal(e.to_string())))?;

        while let Some(delivery) = subscription
            .next()
            .await
            .map_err(|e| SignalError::from(DomainError::internal(e.to_string())))?
        {
            self.handle_delivery(delivery).await;
        }

        Ok(())
    }

    /// Processes a single delivery.
    async fn handle_delivery(
        &self,
        delivery: cheetah_message_api::Delivery<cheetah_message_api::CommandEnvelope>,
    ) {
        let command = match decode_command(&delivery.envelope) {
            Ok(command) => command,
            Err(e) => {
                warn!(error = %e, "failed to decode command envelope");
                let _ = delivery.ack.term(Some(&e.to_string())).await;
                return;
            }
        };

        let tenant_id = command.tenant_id();
        let device_id = command.device_id();

        match self.owner_resolver.resolve(tenant_id, device_id).await {
            Ok(Some(owner)) if owner.owner_node_id == self.this_node => {
                if owner.owner_epoch != command.expected_owner_epoch() {
                    warn!(
                        tenant_id = %tenant_id.as_uuid(),
                        device_id = %device_id.as_uuid(),
                        message_id = %command.message_id().as_uuid(),
                        "command received with stale owner epoch"
                    );
                    let _ = delivery.ack.term(Some("stale owner epoch")).await;
                    return;
                }
                self.process(tenant_id, command, delivery).await;
            }
            Ok(Some(owner)) => {
                // Forward to the current owner node.
                let subject = command_subject(tenant_id, owner.owner_node_id);
                if let Err(e) = self.command_bus.send(&subject, &delivery.envelope).await {
                    warn!(
                        tenant_id = %tenant_id.as_uuid(),
                        device_id = %device_id.as_uuid(),
                        error = %e,
                        "failed to forward command"
                    );
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                } else {
                    let _ = delivery.ack.ack().await;
                }
            }
            Ok(None) => {
                warn!(
                    tenant_id = %tenant_id.as_uuid(),
                    device_id = %device_id.as_uuid(),
                    "command received for device with no owner"
                );
                let _ = delivery.ack.nak(Some("no owner")).await;
            }
            Err(e) => {
                warn!(
                    tenant_id = %tenant_id.as_uuid(),
                    device_id = %device_id.as_uuid(),
                    error = %e,
                    "owner resolution failed"
                );
                let _ = delivery.ack.nak(Some(&e.to_string())).await;
            }
        }
    }

    async fn process(
        &self,
        tenant_id: cheetah_signal_types::TenantId,
        command: Command,
        delivery: cheetah_message_api::Delivery<cheetah_message_api::CommandEnvelope>,
    ) {
        let message_id = command.message_id();
        let now = self.clock.now_wall();
        let expires_at = now.checked_add(self.record_ttl);

        let record = ProcessedMessageRecord {
            tenant_id,
            message_id,
            idempotency_key: Some(command.idempotency_key().to_string()),
            status: ProcessedMessageStatus::Pending,
            result_payload: None,
            processed_at: now,
            expires_at,
        };

        let mut uow = match self.storage.begin().await {
            Ok(uow) => uow,
            Err(e) => {
                warn!(error = %e, "failed to begin unit of work");
                let _ = delivery.ack.nak(Some(&e.to_string())).await;
                return;
            }
        };

        let existing = match uow
            .processed_message_repository()
            .get_or_insert(record)
            .await
        {
            Ok(existing) => existing,
            Err(e) => {
                warn!(error = %e, "processed message repository error");
                let _ = uow.rollback().await;
                let _ = delivery.ack.nak(Some(&e.to_string())).await;
                return;
            }
        };

        if let Some(existing) = existing {
            if existing.status != ProcessedMessageStatus::Pending {
                let _ = uow.commit().await;
                let _ = delivery.ack.ack().await;
                return;
            }
            let _ = uow.rollback().await;
            let _ = delivery.ack.nak(Some("command already pending")).await;
            return;
        }

        match self.command_handler.handle(uow.as_mut(), &command).await {
            Ok(result) => {
                let status = processed_status_from_receipt(&result.receipt);
                let result_payload = result
                    .result_payload
                    .clone()
                    .or_else(|| build_result_payload(&result));
                let now = self.clock.now_wall();
                if let Err(e) = uow
                    .processed_message_repository()
                    .complete(tenant_id, message_id, status, result_payload, now)
                    .await
                {
                    warn!(error = %e, "failed to complete processed message record");
                    let _ = uow.rollback().await;
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                    return;
                }

                if let Err(e) = uow.commit().await {
                    warn!(error = %e, "failed to commit inbox processing");
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                    return;
                }

                info!(
                    tenant_id = %tenant_id.as_uuid(),
                    message_id = %message_id.as_uuid(),
                    ?status,
                    "command processed"
                );
                let _ = delivery.ack.ack().await;
            }
            Err(e) => {
                if e.is_retryable() {
                    let _ = uow.rollback().await;
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                    return;
                }

                let now = self.clock.now_wall();
                let payload = serde_json::json!({
                    "receipt": "rejected",
                    "reason": e.to_string(),
                })
                .to_string();
                if let Err(e2) = uow
                    .processed_message_repository()
                    .complete(
                        tenant_id,
                        message_id,
                        ProcessedMessageStatus::Failed,
                        Some(payload),
                        now,
                    )
                    .await
                {
                    warn!(error = %e2, "failed to mark processed message as failed");
                    let _ = uow.rollback().await;
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                    return;
                }

                if let Err(e2) = uow.commit().await {
                    warn!(error = %e2, "failed to commit failed status");
                    let _ = delivery.ack.nak(Some(&e.to_string())).await;
                    return;
                }

                let _ = delivery.ack.term(Some(&e.to_string())).await;
            }
        }
    }
}

fn processed_status_from_receipt(receipt: &InboxReceipt) -> ProcessedMessageStatus {
    match receipt {
        InboxReceipt::Accepted => ProcessedMessageStatus::Accepted,
        InboxReceipt::Duplicate => ProcessedMessageStatus::Duplicate,
        InboxReceipt::Rejected { .. } | InboxReceipt::DeadLetter { .. } => {
            ProcessedMessageStatus::Failed
        }
    }
}

fn build_result_payload(result: &CommandHandlerResult) -> Option<String> {
    #[derive(Serialize)]
    struct Payload<'a> {
        receipt: &'a InboxReceipt,
        #[serde(skip_serializing_if = "Option::is_none")]
        dispatch: Option<&'a CommandDispatch>,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<&'a OperationStepOutcome>,
    }

    let payload = Payload {
        receipt: &result.receipt,
        dispatch: result.dispatch.as_ref(),
        outcome: result.outcome.as_ref(),
    };
    serde_json::to_string(&payload).ok()
}
