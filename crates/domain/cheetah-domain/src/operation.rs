//! Operation aggregate and lifecycle.

use crate::{Command, CommandPayload, DomainError, DomainEvent, IdempotencyScope};
use cheetah_signal_types::{
    Clock, Deadline, DeviceId, IdGenerator, MessageId, OperationId, OwnerEpoch, Principal,
    RequestContext, ResourceRef, Revision, TenantId, UtcTimestamp,
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

impl std::fmt::Display for OperationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for OperationStatus {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let status = match s.to_lowercase().as_str() {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "timed_out" => Self::TimedOut,
            _ => {
                return Err(DomainError::invalid_argument(format!(
                    "unknown status: {s}"
                )));
            }
        };
        Ok(status)
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

/// Status of a single saga step within an operation.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OperationStepStatus {
    /// Step has not started.
    #[default]
    Pending,
    /// Step is currently running.
    Running,
    /// Step completed successfully.
    Succeeded,
    /// Step failed.
    Failed,
    /// Step was compensated.
    Compensated,
}

impl OperationStepStatus {
    /// Whether the step is in a terminal state.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Compensated)
    }
}

/// Outcome of a single dispatch attempt for a step command.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DispatchAttemptStatus {
    /// Attempt has been recorded but not yet sent.
    #[default]
    Pending,
    /// Command was dispatched.
    Sent,
    /// Positive acknowledgment received.
    Acked,
    /// Negative acknowledgment received.
    Nacked,
    /// Attempt timed out.
    TimedOut,
    /// Attempt was rejected to the dead letter queue.
    DeadLetter,
}

/// A single dispatch attempt for a saga step command.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DispatchAttempt {
    attempt_id: MessageId,
    status: DispatchAttemptStatus,
    sent_at: Option<UtcTimestamp>,
    acked_at: Option<UtcTimestamp>,
    error: Option<OperationError>,
}

impl DispatchAttempt {
    /// Creates a new pending dispatch attempt.
    pub fn new(attempt_id: MessageId) -> Self {
        Self {
            attempt_id,
            status: DispatchAttemptStatus::Pending,
            sent_at: None,
            acked_at: None,
            error: None,
        }
    }

    /// Marks the attempt as sent.
    pub fn mark_sent(&mut self, clock: &dyn Clock) {
        self.status = DispatchAttemptStatus::Sent;
        self.sent_at = Some(clock.now_wall());
    }

    /// Marks the attempt as positively acknowledged.
    pub fn mark_acked(&mut self, clock: &dyn Clock) {
        self.status = DispatchAttemptStatus::Acked;
        self.acked_at = Some(clock.now_wall());
    }

    /// Marks the attempt as negatively acknowledged with an error.
    pub fn mark_nacked(&mut self, error: OperationError, clock: &dyn Clock) {
        self.status = DispatchAttemptStatus::Nacked;
        self.error = Some(error);
        self.acked_at = Some(clock.now_wall());
    }

    /// Marks the attempt as timed out.
    pub fn mark_timed_out(&mut self, clock: &dyn Clock) {
        self.status = DispatchAttemptStatus::TimedOut;
        self.acked_at = Some(clock.now_wall());
    }

    /// Marks the attempt as dead-lettered.
    pub fn mark_dead_letter(&mut self, error: OperationError, clock: &dyn Clock) {
        self.status = DispatchAttemptStatus::DeadLetter;
        self.error = Some(error);
        self.acked_at = Some(clock.now_wall());
    }

    /// Returns the attempt identifier.
    pub fn attempt_id(&self) -> MessageId {
        self.attempt_id
    }

    /// Returns the current status of the attempt.
    pub fn status(&self) -> DispatchAttemptStatus {
        self.status
    }
}

/// A single step in an operation saga.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OperationStep {
    step_id: MessageId,
    command: Command,
    status: OperationStepStatus,
    result: Option<OperationResult>,
    attempts: Vec<DispatchAttempt>,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
}

impl OperationStep {
    /// Creates a new pending step for the given command.
    pub fn new(command: Command, clock: &dyn Clock) -> Self {
        Self {
            step_id: command.step_id().unwrap_or_else(|| command.command_id()),
            command,
            status: OperationStepStatus::Pending,
            result: None,
            attempts: Vec::new(),
            created_at: clock.now_wall(),
            updated_at: clock.now_wall(),
        }
    }

    /// The step identifier, equal to the step command id.
    pub fn step_id(&self) -> MessageId {
        self.step_id
    }

    /// The command associated with this step.
    pub fn command(&self) -> &Command {
        &self.command
    }

    /// Current step status.
    pub fn status(&self) -> OperationStepStatus {
        self.status
    }

    /// Step result, if terminal.
    pub fn result(&self) -> Option<OperationResult> {
        self.result.clone()
    }

    /// Dispatch attempts for this step.
    pub fn attempts(&self) -> &[DispatchAttempt] {
        &self.attempts
    }

    /// Adds a dispatch attempt to the step.
    pub fn add_attempt(&mut self, attempt: DispatchAttempt, clock: &dyn Clock) {
        self.attempts.push(attempt);
        self.updated_at = clock.now_wall();
    }

    fn attempt_mut(&mut self, attempt_id: MessageId) -> Option<&mut DispatchAttempt> {
        self.attempts
            .iter_mut()
            .find(|a| a.attempt_id == attempt_id)
    }

    /// Marks the step as running.
    pub fn start(&mut self, clock: &dyn Clock) {
        self.status = OperationStepStatus::Running;
        self.updated_at = clock.now_wall();
    }

    /// Completes the step with a result.
    pub fn complete(&mut self, result: OperationResult, clock: &dyn Clock) {
        self.status = if result.is_success() {
            OperationStepStatus::Succeeded
        } else {
            OperationStepStatus::Failed
        };
        self.result = Some(result);
        self.updated_at = clock.now_wall();
    }

    /// Compensates the step.
    pub fn compensate(&mut self, clock: &dyn Clock) {
        self.status = OperationStepStatus::Compensated;
        self.updated_at = clock.now_wall();
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
    /// Saga steps for this operation. Old rows may not contain steps; the
    /// initial step is synthesized from the top-level command when missing.
    #[serde(default)]
    steps: Vec<OperationStep>,
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
        let initial_step_id = id_generator.generate_message_id();
        let command = Command::new(
            id_generator,
            operation_id,
            Some(initial_step_id),
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
        let initial_step = OperationStep::new(command.clone(), clock);
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
            steps: vec![initial_step],
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

    /// Returns the currently active saga step, if any.
    fn current_step_mut(&mut self) -> Option<&mut OperationStep> {
        self.steps.iter_mut().rev().find(|s| {
            !matches!(
                s.status(),
                OperationStepStatus::Succeeded
                    | OperationStepStatus::Failed
                    | OperationStepStatus::Compensated
            )
        })
    }

    /// Transitions from `Pending` to `Running`.
    pub fn start(&mut self, clock: &dyn Clock) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Pending => {
                self.status = OperationStatus::Running;
                if let Some(step) = self.current_step_mut() {
                    step.start(clock);
                }
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
                self.result = Some(result.clone());
                self.error = None;
                if let Some(step) = self.current_step_mut() {
                    step.complete(result, clock);
                }
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
                if let Some(step) = self.current_step_mut() {
                    step.complete(
                        OperationResult::failure("cancelled", "operation cancelled"),
                        clock,
                    );
                }
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

    /// Fails a non-terminal operation with a structured error result.
    ///
    /// Works for `Pending` or `Running` operations. The result is set to a
    /// failure and the error is captured for observability.
    pub fn fail(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        clock: &dyn Clock,
    ) -> crate::Result<DomainEvent> {
        match self.status {
            OperationStatus::Pending | OperationStatus::Running => {
                let previous = self.status;
                self.status = OperationStatus::Failed;
                let code = code.into();
                let message = message.into();
                let result = OperationResult::failure(code.clone(), message.clone());
                self.result = Some(result.clone());
                self.error = Some(OperationError::new(code, message));
                if let Some(step) = self.current_step_mut() {
                    step.complete(result, clock);
                }
                self.bump(clock);
                Ok(self.state_changed_event(previous))
            }
            OperationStatus::Succeeded | OperationStatus::Failed => {
                Err(DomainError::invalid_transition(
                    "Operation",
                    format!("{:?}", self.status),
                    "Failed",
                ))
            }
            _ => Err(DomainError::already_terminal(
                "Operation",
                format!("{:?}", self.status),
            )),
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
                self.error = Some(error.clone());
                if let Some(step) = self.current_step_mut() {
                    step.complete(
                        OperationResult::failure(error.code(), error.message()),
                        clock,
                    );
                }
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

    /// Returns the saga steps for this operation.
    pub fn steps(&self) -> &[OperationStep] {
        &self.steps
    }

    /// Records a dispatch attempt against the requested step.
    pub fn record_dispatch_attempt(
        &mut self,
        step_id: MessageId,
        attempt: DispatchAttempt,
        clock: &dyn Clock,
    ) -> crate::Result<()> {
        let step = self
            .steps
            .iter_mut()
            .find(|s| s.step_id() == step_id)
            .ok_or_else(|| DomainError::not_found("operation step", step_id.to_string()))?;
        step.add_attempt(attempt, clock);
        self.bump(clock);
        Ok(())
    }

    fn step_mut(&mut self, step_id: MessageId) -> Option<&mut OperationStep> {
        self.steps.iter_mut().find(|s| s.step_id() == step_id)
    }

    /// Marks a dispatch attempt as sent.
    pub fn mark_dispatch_attempt_sent(
        &mut self,
        step_id: MessageId,
        attempt_id: MessageId,
        clock: &dyn Clock,
    ) -> crate::Result<()> {
        let step = self
            .step_mut(step_id)
            .ok_or_else(|| DomainError::not_found("operation step", step_id.to_string()))?;
        let attempt = step
            .attempt_mut(attempt_id)
            .ok_or_else(|| DomainError::not_found("dispatch attempt", attempt_id.to_string()))?;
        attempt.mark_sent(clock);
        self.bump(clock);
        Ok(())
    }

    /// Marks a dispatch attempt as acknowledged.
    pub fn mark_dispatch_attempt_acked(
        &mut self,
        step_id: MessageId,
        attempt_id: MessageId,
        clock: &dyn Clock,
    ) -> crate::Result<()> {
        let step = self
            .step_mut(step_id)
            .ok_or_else(|| DomainError::not_found("operation step", step_id.to_string()))?;
        let attempt = step
            .attempt_mut(attempt_id)
            .ok_or_else(|| DomainError::not_found("dispatch attempt", attempt_id.to_string()))?;
        attempt.mark_acked(clock);
        self.bump(clock);
        Ok(())
    }

    /// Marks a dispatch attempt as negatively acknowledged.
    pub fn mark_dispatch_attempt_nacked(
        &mut self,
        step_id: MessageId,
        attempt_id: MessageId,
        error: OperationError,
        clock: &dyn Clock,
    ) -> crate::Result<()> {
        let step = self
            .step_mut(step_id)
            .ok_or_else(|| DomainError::not_found("operation step", step_id.to_string()))?;
        let attempt = step
            .attempt_mut(attempt_id)
            .ok_or_else(|| DomainError::not_found("dispatch attempt", attempt_id.to_string()))?;
        attempt.mark_nacked(error, clock);
        self.bump(clock);
        Ok(())
    }
}
