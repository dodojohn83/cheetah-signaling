//! Operation aggregate and lifecycle.

use crate::{Command, CommandPayload, DomainError, DomainEvent, IdempotencyScope};
use cheetah_signal_types::{
    Clock, Deadline, DeviceId, IdGenerator, OperationId, OwnerEpoch, Principal, RequestContext,
    ResourceRef, Revision, TenantId, UtcTimestamp,
};

/// Status of an operation.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationStatus {
    /// Operation is pending dispatch.
    #[default]
    Pending,
    /// Operation is running.
    Running,
    /// Operation completed successfully.
    Succeeded,
    /// Operation failed.
    Failed,
    /// Operation was cancelled.
    Cancelled,
    /// Operation timed out.
    TimedOut,
}

impl OperationStatus {
    /// Whether the status is a terminal state.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }
}

/// Result of an operation.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
#[non_exhaustive]
pub enum OperationResult {
    /// Operation completed successfully.
    Success,
    /// Operation failed with a stable code.
    Failure {
        /// Stable error code.
        code: String,
        /// Human readable message.
        message: String,
    },
}

impl OperationResult {
    /// Creates a successful result.
    pub fn success() -> Self {
        Self::Success
    }

    /// Creates a failure result.
    pub fn failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Failure {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Whether the result is success.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns the error code if this is a failure.
    pub fn error_code(&self) -> Option<&str> {
        if let Self::Failure { code, .. } = self {
            Some(code)
        } else {
            None
        }
    }

    /// Returns the error message if this is a failure.
    pub fn error_message(&self) -> Option<&str> {
        if let Self::Failure { message, .. } = self {
            Some(message)
        } else {
            None
        }
    }
}

/// Error attached to cancelled or timed out operations.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OperationError {
    code: String,
    message: String,
}

impl OperationError {
    /// Creates a new operation error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Error code for an operation that expired before dispatch.
    pub fn expired_before_dispatch() -> Self {
        Self::new(
            "expired_before_dispatch",
            "deadline expired before dispatch",
        )
    }

    /// Error code for a cancelled operation.
    pub fn cancelled() -> Self {
        Self::new("cancelled", "operation was cancelled")
    }

    /// Error code for a timed out operation.
    pub fn timeout() -> Self {
        Self::new("timeout", "operation timed out")
    }

    /// Stable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Human readable message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Long running operation aggregate.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Operation {
    operation_id: OperationId,
    tenant_id: TenantId,
    device_id: DeviceId,
    idempotency_scope: IdempotencyScope,
    principal: Principal,
    target: ResourceRef,
    command: Command,
    deadline: Option<Deadline>,
    expected_owner_epoch: OwnerEpoch,
    status: OperationStatus,
    result: Option<OperationResult>,
    error: Option<OperationError>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl Operation {
    /// Creates a new pending operation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id_generator: &dyn IdGenerator,
        clock: &dyn Clock,
        context: &RequestContext,
        idempotency_key: impl Into<String>,
        device_id: DeviceId,
        target: ResourceRef,
        payload: CommandPayload,
        deadline: Option<Deadline>,
        expected_owner_epoch: OwnerEpoch,
    ) -> crate::Result<(Self, DomainEvent)> {
        if device_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument("device_id must not be nil"));
        }
        let idempotency_key = idempotency_key.into();
        let idempotency_scope = IdempotencyScope::new(
            context.tenant_id,
            context.principal.id.clone(),
            target.clone(),
            idempotency_key,
        )?;
        let operation_id = id_generator.generate_operation_id();
        let command = Command::new(
            id_generator,
            operation_id,
            context.tenant_id,
            device_id,
            idempotency_scope.clone(),
            target.clone(),
            payload,
            deadline,
            expected_owner_epoch,
            context.principal.clone(),
            context.correlation_id,
            context.message_id,
            context.traceparent.clone(),
            context.tracestate.clone(),
        );
        let now = clock.now_wall();
        let operation = Self {
            operation_id,
            tenant_id: context.tenant_id,
            device_id,
            idempotency_scope,
            principal: context.principal.clone(),
            target,
            command,
            deadline,
            expected_owner_epoch,
            status: OperationStatus::Pending,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        };
        let event = DomainEvent::OperationSubmitted {
            operation_id,
            tenant_id: operation.tenant_id,
            device_id,
            idempotency_scope: Box::new(operation.idempotency_scope.clone()),
            command: Box::new(operation.command.clone()),
        };
        Ok((operation, event))
    }

    /// Transitions from `Pending` to `Running`.
    pub fn start(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Pending => {
                self.status = OperationStatus::Running;
                self.bump(clock);
                Ok(self.state_changed_event(OperationStatus::Pending))
            }
            OperationStatus::Running => Err(DomainError::invalid_transition(
                "Operation",
                format!("{:?}", self.status),
                "Running",
            )),
            s if s.is_terminal() => Err(DomainError::already_terminal(
                "Operation",
                format!("{:?}", s),
            )),
            _ => Err(DomainError::invalid_transition(
                "Operation",
                format!("{:?}", self.status),
                "Running",
            )),
        }
    }

    /// Completes the operation with a result.
    pub fn complete(
        &mut self,
        result: OperationResult,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Running => {
                let previous = self.status;
                self.status = if result.is_success() {
                    OperationStatus::Succeeded
                } else {
                    OperationStatus::Failed
                };
                self.result = Some(result);
                self.error = None;
                self.bump(clock);
                Ok(self.state_changed_event(previous))
            }
            OperationStatus::Succeeded | OperationStatus::Failed => {
                Err(DomainError::invalid_transition(
                    "Operation",
                    format!("{:?}", self.status),
                    if result.is_success() {
                        "Succeeded"
                    } else {
                        "Failed"
                    },
                ))
            }
            s if s.is_terminal() => Err(DomainError::already_terminal(
                "Operation",
                format!("{:?}", s),
            )),
            _ => Err(DomainError::invalid_transition(
                "Operation",
                format!("{:?}", self.status),
                if result.is_success() {
                    "Succeeded"
                } else {
                    "Failed"
                },
            )),
        }
    }

    /// Cancels the operation.
    pub fn cancel(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Pending | OperationStatus::Running => {
                let previous = self.status;
                self.status = OperationStatus::Cancelled;
                self.error = Some(OperationError::cancelled());
                self.bump(clock);
                Ok(self.state_changed_event(previous))
            }
            OperationStatus::Succeeded | OperationStatus::Failed => {
                Err(DomainError::invalid_transition(
                    "Operation",
                    format!("{:?}", self.status),
                    "Cancelled",
                ))
            }
            OperationStatus::Cancelled | OperationStatus::TimedOut => Err(
                DomainError::already_terminal("Operation", format!("{:?}", self.status)),
            ),
        }
    }

    /// Times out the operation.
    pub fn timeout(
        &mut self,
        error: OperationError,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Pending | OperationStatus::Running => {
                let previous = self.status;
                self.status = OperationStatus::TimedOut;
                self.error = Some(error);
                self.bump(clock);
                Ok(self.state_changed_event(previous))
            }
            OperationStatus::Succeeded | OperationStatus::Failed => {
                Err(DomainError::invalid_transition(
                    "Operation",
                    format!("{:?}", self.status),
                    "TimedOut",
                ))
            }
            OperationStatus::Cancelled | OperationStatus::TimedOut => Err(
                DomainError::already_terminal("Operation", format!("{:?}", self.status)),
            ),
        }
    }

    /// Expires the operation before dispatch.
    pub fn expire(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        self.timeout(OperationError::expired_before_dispatch(), clock)
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    fn state_changed_event(&self, previous_status: OperationStatus) -> DomainEvent {
        DomainEvent::OperationStateChanged {
            operation_id: self.operation_id,
            tenant_id: self.tenant_id,
            previous_status,
            status: self.status,
            result: self.result.clone(),
            error: self.error.clone(),
        }
    }

    /// Operation identifier.
    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    /// Tenant identifier.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Device identifier.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Idempotency scope.
    pub fn idempotency_scope(&self) -> &IdempotencyScope {
        &self.idempotency_scope
    }

    /// Principal that requested the operation.
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    /// Target resource.
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// Command describing the work.
    pub fn command(&self) -> &Command {
        &self.command
    }

    /// Human-readable kind of the operation.
    pub fn kind(&self) -> &'static str {
        self.command.kind()
    }

    /// Deadline.
    pub fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Expected owner epoch.
    pub fn expected_owner_epoch(&self) -> OwnerEpoch {
        self.expected_owner_epoch
    }

    /// Current status.
    pub fn status(&self) -> OperationStatus {
        self.status
    }

    /// Operation result.
    pub fn result(&self) -> Option<OperationResult> {
        self.result.clone()
    }

    /// Operation error.
    pub fn error(&self) -> Option<OperationError> {
        self.error.clone()
    }

    /// Creation timestamp.
    pub fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }

    /// Last update timestamp.
    pub fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }

    /// Revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Whether the operation is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}
