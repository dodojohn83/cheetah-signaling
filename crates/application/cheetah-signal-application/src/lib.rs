//! Application services for Cheetah Signaling.

pub mod admission_control;
pub mod command_dispatcher;
pub mod device_service;
pub mod dto;
pub mod event_service;
pub mod inbox;
pub mod media_service;
pub(crate) mod media_service_callback;
pub(crate) mod media_service_command;
pub(crate) mod media_service_command_start;
pub(crate) mod media_service_helpers;
pub(crate) mod media_service_reconciliation;
pub(crate) mod media_service_reconnect;
pub(crate) mod media_service_start;
pub mod operation_reconciler;
pub mod operation_service;
pub mod outbox_relay;
pub mod owner_reconciler;
pub mod takeover_service;
pub mod webhook_service;

pub use admission_control::{IngressAdmission, IngressAdmissionConfig, TenantIngressAdmission};
pub use command_dispatcher::CommandDispatcher;
pub use device_service::DeviceService;
pub use dto::*;
pub use event_service::EventService;
pub use inbox::{
    CommandDispatch, CommandHandler, CommandHandlerResult, InboxReceipt, InboxService,
    OperationStepOutcome,
};
pub use media_service::MediaService;
pub use operation_reconciler::OperationReconciler;
pub use operation_service::OperationService;
pub use outbox_relay::OutboxRelay;
pub use owner_reconciler::{
    LocalDeviceSession, OwnerMissingReport, OwnerReconciler, OwnerReconciliationReport,
};
pub use takeover_service::{OwnerValidation, TakeoverResult, TakeoverService};
pub use webhook_service::{
    WebhookDeliveryConfig, WebhookHttpClient, WebhookHttpRequest, WebhookHttpResponse,
    WebhookService,
};

pub use cheetah_signal_types::{Result, SignalError};
