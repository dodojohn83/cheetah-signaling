//! Inbox service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::PtzDirection;
use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator,
    request_context as in_memory_request_context,
};
use cheetah_domain::{Command, CommandPayload, Operation, ProcessedMessageStatus, UnitOfWork};
use cheetah_message_api::bus::RawCommandBus;
use cheetah_message_api::mapper::encode_command;
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    CommandDispatch, CommandHandler, CommandHandlerResult, InboxService, OperationStepOutcome,
};
use cheetah_signal_types::{Clock, IdGenerator, ResourceId, ResourceKind, ResourceRef};
use cheetah_signal_types::{DurationMs, OwnerEpoch};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;

struct RecordingHandler {
    commands: tokio::sync::Mutex<Vec<Command>>,
}

#[async_trait::async_trait]
impl CommandHandler for RecordingHandler {
    async fn handle(
        &self,
        _uow: &mut dyn UnitOfWork,
        command: &Command,
    ) -> cheetah_signal_types::Result<CommandHandlerResult> {
        self.commands.lock().await.push(command.clone());
        Ok(
            CommandHandlerResult::accepted(CommandDispatch::Sent, OperationStepOutcome::Unknown)
                .with_payload(r#"{"ok":true}"#.to_string()),
        )
    }
}

async fn setup_inbox(
    path: &std::path::Path,
) -> (
    Arc<InboxService>,
    Arc<InProcessMessageBus>,
    Arc<RecordingHandler>,
    Arc<SqliteStorage>,
    cheetah_signal_types::TenantId,
    cheetah_signal_types::DeviceId,
) {
    let storage = Arc::new(SqliteStorage::new(path).await.unwrap());
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let command_bus = Arc::new(InProcessMessageBus::new(16, 16));
    let owner_resolver = Arc::new(InMemoryDeviceOwnerResolver::new());
    let command_handler: Arc<RecordingHandler> = Arc::new(RecordingHandler {
        commands: tokio::sync::Mutex::new(Vec::new()),
    });

    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let this_node = id_generator.generate_node_id();

    owner_resolver.set_owner(
        tenant_id,
        device_id,
        cheetah_domain::OwnerInfo {
            owner_node_id: this_node,
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let inbox = Arc::new(InboxService::new(
        storage.clone(),
        command_bus.clone(),
        owner_resolver.clone(),
        command_handler.clone(),
        clock,
        this_node,
        DurationMs::from_millis(60_000),
    ));

    (
        inbox,
        command_bus,
        command_handler,
        storage,
        tenant_id,
        device_id,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_service_processes_command_once_and_deduplicates() {
    let file_id = InMemoryIdGenerator::new()
        .generate_message_id()
        .as_uuid()
        .to_string();
    let path = std::env::temp_dir().join(format!("cheetah_inbox_test_{file_id}.db"));
    let (inbox, bus, handler, storage, tenant_id, device_id) = setup_inbox(&path).await;

    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let clock = Arc::new(InMemoryClock::new());
    let context = in_memory_request_context(tenant_id, id_generator.as_ref(), clock.as_ref());
    let channel_id = id_generator.generate_channel_id();
    let target = ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    };
    let (operation, _event) = Operation::new(
        id_generator.as_ref(),
        clock.as_ref(),
        &context,
        "inbox-test",
        device_id,
        target,
        CommandPayload::Ptz {
            channel_id,
            direction: PtzDirection::Stop,
            speed: 0.0,
        },
        None,
        OwnerEpoch::default(),
    )
    .unwrap();
    let command = operation.command().clone();
    let envelope = encode_command(&command).unwrap();
    let message_id = command.message_id();

    let handle = tokio::spawn({
        let inbox = inbox.clone();
        async move { inbox.run("", "inbox-test").await }
    });

    bus.send("", &envelope).await.unwrap();
    bus.send("", &envelope).await.unwrap();

    let mut attempts = 0;
    while handler.commands.lock().await.is_empty() && attempts < 50 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        attempts += 1;
    }

    handle.abort();
    let _ = handle.await;

    let commands = handler.commands.lock().await;
    assert_eq!(
        commands.len(),
        1,
        "handler must process the command exactly once"
    );
    drop(commands);

    let mut uow = storage.begin().await.unwrap();
    let record = uow
        .processed_message_repository()
        .find(tenant_id, message_id)
        .await
        .unwrap()
        .expect("processed message record must exist");
    uow.commit().await.unwrap();

    assert_eq!(record.status, ProcessedMessageStatus::Accepted);
    assert_eq!(record.result_payload.as_deref(), Some(r#"{"ok":true}"#));

    let _ = std::fs::remove_file(&path);
}
