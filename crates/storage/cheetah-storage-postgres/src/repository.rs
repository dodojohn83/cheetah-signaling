//! Repository and outbox implementations for the PostgreSQL unit of work.

use crate::error::sqlx_to_domain;
use crate::list;
use crate::unit_of_work::PostgresUnitOfWork;
use cheetah_domain::Protocol;
use cheetah_domain::{
    Channel, ChannelRepository, Device, DeviceLifecycle, DeviceRepository, DomainError,
    MediaBinding, MediaBindingRepository, MediaSession, MediaSessionRepository, Operation,
    OperationRepository, Outbox, OutboxEntry, ProcessedMessageRecord, ProcessedMessageRepository,
    ProcessedMessageStatus,
};
use cheetah_signal_types::{DeviceId, Event, MessageId, Page, PageRequest, TenantId, UtcTimestamp};
use sqlx::FromRow;
use sqlx::types::Json;
use time::OffsetDateTime;

use cheetah_domain::DomainEvent;

fn variant_name<T: serde::Serialize>(value: &T) -> Result<String, DomainError> {
    let json = serde_json::to_string(value).map_err(|e| DomainError::internal(e.to_string()))?;
    let trimmed = json.trim_matches('"');
    if trimmed.len() == json.len() - 2 {
        Ok(trimmed.to_string())
    } else {
        Err(DomainError::internal(
            "enum variant has unexpected JSON shape",
        ))
    }
}

fn result_type(result: &cheetah_domain::OperationResult) -> Result<String, DomainError> {
    let value = serde_json::to_value(result).map_err(|e| DomainError::internal(e.to_string()))?;
    value
        .get("type")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| DomainError::internal("missing result type"))
}

fn connectivity_kind(value: &cheetah_domain::Connectivity) -> Result<String, DomainError> {
    let value = serde_json::to_value(value).map_err(|e| DomainError::internal(e.to_string()))?;
    value
        .get("kind")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .ok_or_else(|| DomainError::internal("missing connectivity kind"))
}

fn concurrent_modification_error(expected: u64, found: u64) -> DomainError {
    DomainError::ConcurrentModification { expected, found }
}

#[derive(FromRow)]
struct DeviceRow {
    #[sqlx(rename = "data")]
    data: Json<Device>,
}

#[async_trait::async_trait]
impl DeviceRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> cheetah_domain::Result<Option<Device>> {
        let row: Option<DeviceRow> = sqlx::query_as::<sqlx::Postgres, DeviceRow>(
            "SELECT data FROM devices WHERE tenant_id = $1 AND device_id = $2 AND deleted = false",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_external_id(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        protocol: Protocol,
        external_id: cheetah_signal_types::ProtocolIdentity,
    ) -> cheetah_domain::Result<Option<Device>> {
        let row: Option<DeviceRow> = sqlx::query_as::<sqlx::Postgres, DeviceRow>(
            "SELECT data FROM devices WHERE tenant_id = $1 AND protocol = $2 AND external_id = $3 AND deleted = false",
        )
        .bind(tenant_id.as_uuid())
        .bind(variant_name(&protocol)?)
        .bind(external_id.as_str())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, device: &Device) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "INSERT INTO devices (
                tenant_id, device_id, protocol, external_id, authority, name, kind, lifecycle,
                connectivity_kind, owner_epoch, revision, created_at, updated_at, deleted, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT(device_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                protocol = EXCLUDED.protocol,
                external_id = EXCLUDED.external_id,
                authority = EXCLUDED.authority,
                name = EXCLUDED.name,
                kind = EXCLUDED.kind,
                lifecycle = EXCLUDED.lifecycle,
                connectivity_kind = EXCLUDED.connectivity_kind,
                owner_epoch = EXCLUDED.owner_epoch,
                revision = EXCLUDED.revision,
                created_at = COALESCE(devices.created_at, EXCLUDED.created_at),
                updated_at = EXCLUDED.updated_at,
                deleted = EXCLUDED.deleted,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE devices.revision = EXCLUDED.revision - 1",
        )
        .bind(device.tenant_id().as_uuid())
        .bind(device.device_id().as_uuid())
        .bind(variant_name(&device.protocol())?)
        .bind(device.external_id().as_str())
        .bind(device.authority())
        .bind(device.name())
        .bind(variant_name(&device.kind())?)
        .bind(variant_name(&device.lifecycle())?)
        .bind(connectivity_kind(&device.connectivity())?)
        .bind(device.owner_epoch().0 as i64)
        .bind(device.revision().0 as i64)
        .bind(device.created_at().as_offset())
        .bind(device.updated_at().as_offset())
        .bind(device.lifecycle() == DeviceLifecycle::Retired)
        .bind(Json(device))
        .bind(1i32)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> =
                sqlx::query_as("SELECT revision FROM devices WHERE device_id = $1")
                    .bind(device.device_id().as_uuid())
                    .fetch_optional(self.tx().await?.as_mut())
                    .await
                    .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(
                device.revision().0.saturating_sub(1),
                found,
            ));
        }
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        protocol: Option<String>,
        lifecycle: Option<String>,
        name_prefix: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> cheetah_domain::Result<Page<Device>> {
        list::devices(
            self.tx().await?.as_mut(),
            tenant_id,
            protocol,
            lifecycle,
            name_prefix,
            updated_after,
            page,
        )
        .await
    }
}

#[derive(FromRow)]
struct ChannelRow {
    #[sqlx(rename = "data")]
    data: Json<Channel>,
}

#[async_trait::async_trait]
impl ChannelRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        channel_id: cheetah_signal_types::ChannelId,
    ) -> cheetah_domain::Result<Option<Channel>> {
        let row: Option<ChannelRow> = sqlx::query_as::<sqlx::Postgres, ChannelRow>(
            "SELECT data FROM channels WHERE tenant_id = $1 AND device_id = $2 AND channel_id = $3 AND deleted = false",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(channel_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn list_by_device(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
    ) -> cheetah_domain::Result<Vec<Channel>> {
        let rows: Vec<ChannelRow> = sqlx::query_as::<sqlx::Postgres, ChannelRow>(
            "SELECT data FROM channels WHERE tenant_id = $1 AND device_id = $2 AND deleted = false ORDER BY channel_id",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .fetch_all(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(rows.into_iter().map(|r| r.data.0).collect())
    }

    async fn save(&mut self, channel: &Channel) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "INSERT INTO channels (
                tenant_id, device_id, channel_id, name, kind, enabled, status, revision, updated_at, deleted, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT(tenant_id, device_id, channel_id) DO UPDATE SET
                name = EXCLUDED.name,
                kind = EXCLUDED.kind,
                enabled = EXCLUDED.enabled,
                status = EXCLUDED.status,
                revision = EXCLUDED.revision,
                updated_at = EXCLUDED.updated_at,
                deleted = EXCLUDED.deleted,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE channels.revision = EXCLUDED.revision - 1",
        )
        .bind(channel.tenant_id().as_uuid())
        .bind(channel.device_id().as_uuid())
        .bind(channel.channel_id().as_uuid())
        .bind(channel.name())
        .bind(variant_name(&channel.kind())?)
        .bind(channel.enabled())
        .bind(variant_name(&channel.status())?)
        .bind(channel.revision().0 as i64)
        .bind(channel.updated_at().as_offset())
        .bind(false)
        .bind(Json(channel))
        .bind(1i32)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> = sqlx::query_as(
                "SELECT revision FROM channels WHERE tenant_id = $1 AND device_id = $2 AND channel_id = $3",
            )
            .bind(channel.tenant_id().as_uuid())
            .bind(channel.device_id().as_uuid())
            .bind(channel.channel_id().as_uuid())
            .fetch_optional(self.tx().await?.as_mut())
            .await
            .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(
                channel.revision().0.saturating_sub(1),
                found,
            ));
        }
        Ok(())
    }

    async fn remove(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        device_id: cheetah_signal_types::DeviceId,
        channel_id: cheetah_signal_types::ChannelId,
        expected_revision: cheetah_signal_types::Revision,
    ) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "DELETE FROM channels WHERE tenant_id = $1 AND device_id = $2 AND channel_id = $3 AND revision = $4",
        )
        .bind(tenant_id.as_uuid())
        .bind(device_id.as_uuid())
        .bind(channel_id.as_uuid())
        .bind(expected_revision.0 as i64)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;
        if result.rows_affected() != 1 {
            let found: Option<(i64,)> = sqlx::query_as(
                "SELECT revision FROM channels WHERE tenant_id = $1 AND device_id = $2 AND channel_id = $3",
            )
            .bind(tenant_id.as_uuid())
            .bind(device_id.as_uuid())
            .bind(channel_id.as_uuid())
            .fetch_optional(self.tx().await?.as_mut())
            .await
            .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(expected_revision.0, found));
        }
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        status: Option<String>,
        name_prefix: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> cheetah_domain::Result<Page<Channel>> {
        list::channels(
            self.tx().await?.as_mut(),
            tenant_id,
            device_id.as_uuid(),
            status,
            name_prefix,
            updated_after,
            page,
        )
        .await
    }
}

#[derive(FromRow)]
struct OperationRow {
    #[sqlx(rename = "data")]
    data: Json<Operation>,
}

#[async_trait::async_trait]
impl OperationRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        operation_id: cheetah_signal_types::OperationId,
    ) -> cheetah_domain::Result<Option<Operation>> {
        let row: Option<OperationRow> = sqlx::query_as::<sqlx::Postgres, OperationRow>(
            "SELECT data FROM operations WHERE tenant_id = $1 AND operation_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(operation_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_idempotency(
        &mut self,
        scope: &cheetah_domain::IdempotencyScope,
    ) -> cheetah_domain::Result<Option<Operation>> {
        let row: Option<OperationRow> = sqlx::query_as::<sqlx::Postgres, OperationRow>(
            "SELECT data FROM operations WHERE tenant_id = $1 AND principal_id = $2 AND idempotency_key = $3",
        )
        .bind(scope.tenant_id.as_uuid())
        .bind(&scope.principal_id)
        .bind(&scope.idempotency_key)
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, operation: &Operation) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "INSERT INTO operations (
                tenant_id, operation_id, device_id, principal_id, idempotency_key, status, result_type,
                revision, created_at, updated_at, deadline, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT(operation_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                device_id = EXCLUDED.device_id,
                principal_id = EXCLUDED.principal_id,
                idempotency_key = EXCLUDED.idempotency_key,
                status = EXCLUDED.status,
                result_type = EXCLUDED.result_type,
                revision = EXCLUDED.revision,
                created_at = EXCLUDED.created_at,
                updated_at = EXCLUDED.updated_at,
                deadline = EXCLUDED.deadline,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE operations.revision = EXCLUDED.revision - 1",
        )
        .bind(operation.tenant_id().as_uuid())
        .bind(operation.operation_id().as_uuid())
        .bind(operation.device_id().as_uuid())
        .bind(&operation.idempotency_scope().principal_id)
        .bind(&operation.idempotency_scope().idempotency_key)
        .bind(variant_name(&operation.status())?)
        .bind(operation.result().as_ref().map(result_type).transpose()?)
        .bind(operation.revision().0 as i64)
        .bind(operation.created_at().as_offset())
        .bind(operation.updated_at().as_offset())
        .bind(operation.deadline().map(|d| d.as_timestamp().as_offset()))
        .bind(Json(operation))
        .bind(1i32)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> =
                sqlx::query_as("SELECT revision FROM operations WHERE operation_id = $1")
                    .bind(operation.operation_id().as_uuid())
                    .fetch_optional(self.tx().await?.as_mut())
                    .await
                    .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(
                operation.revision().0.saturating_sub(1),
                found,
            ));
        }
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: Option<DeviceId>,
        status: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> cheetah_domain::Result<Page<Operation>> {
        list::operations(
            self.tx().await?.as_mut(),
            tenant_id,
            device_id.map(|d| d.as_uuid()),
            status,
            updated_after,
            page,
        )
        .await
    }
}

#[derive(FromRow)]
struct MediaSessionRow {
    #[sqlx(rename = "data")]
    data: Json<MediaSession>,
}

#[async_trait::async_trait]
impl MediaSessionRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        media_session_id: cheetah_signal_types::MediaSessionId,
    ) -> cheetah_domain::Result<Option<MediaSession>> {
        let row: Option<MediaSessionRow> = sqlx::query_as::<sqlx::Postgres, MediaSessionRow>(
            "SELECT data FROM media_sessions WHERE tenant_id = $1 AND media_session_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(media_session_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_idempotency(
        &mut self,
        scope: &cheetah_domain::IdempotencyScope,
    ) -> cheetah_domain::Result<Option<MediaSession>> {
        let row: Option<MediaSessionRow> = sqlx::query_as::<sqlx::Postgres, MediaSessionRow>(
            "SELECT data FROM media_sessions WHERE tenant_id = $1 AND principal_id = $2 AND idempotency_key = $3",
        )
        .bind(scope.tenant_id.as_uuid())
        .bind(&scope.principal_id)
        .bind(&scope.idempotency_key)
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, session: &MediaSession) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "INSERT INTO media_sessions (
                tenant_id, media_session_id, device_id, channel_id, operation_id, principal_id,
                idempotency_key, purpose, state, desired_state, revision, created_at, updated_at, deadline, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT(media_session_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                device_id = EXCLUDED.device_id,
                channel_id = EXCLUDED.channel_id,
                operation_id = EXCLUDED.operation_id,
                principal_id = EXCLUDED.principal_id,
                idempotency_key = EXCLUDED.idempotency_key,
                purpose = EXCLUDED.purpose,
                state = EXCLUDED.state,
                desired_state = EXCLUDED.desired_state,
                revision = EXCLUDED.revision,
                created_at = EXCLUDED.created_at,
                updated_at = EXCLUDED.updated_at,
                deadline = EXCLUDED.deadline,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE media_sessions.revision = EXCLUDED.revision - 1",
        )
        .bind(session.tenant_id().as_uuid())
        .bind(session.media_session_id().as_uuid())
        .bind(session.device_id().as_uuid())
        .bind(session.channel_id().as_uuid())
        .bind(session.operation_id().as_uuid())
        .bind(&session.idempotency_scope().principal_id)
        .bind(&session.idempotency_scope().idempotency_key)
        .bind(variant_name(&session.purpose())?)
        .bind(variant_name(&session.state())?)
        .bind(variant_name(&session.desired_state())?)
        .bind(session.revision().0 as i64)
        .bind(session.created_at().as_offset())
        .bind(session.updated_at().as_offset())
        .bind(session.deadline().map(|d| d.as_timestamp().as_offset()))
        .bind(Json(session))
        .bind(1i32)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> =
                sqlx::query_as("SELECT revision FROM media_sessions WHERE media_session_id = $1")
                    .bind(session.media_session_id().as_uuid())
                    .fetch_optional(self.tx().await?.as_mut())
                    .await
                    .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(
                session.revision().0.saturating_sub(1),
                found,
            ));
        }
        Ok(())
    }

    async fn list(
        &mut self,
        tenant_id: TenantId,
        device_id: Option<DeviceId>,
        purpose: Option<String>,
        state: Option<String>,
        updated_after: Option<UtcTimestamp>,
        page: PageRequest,
    ) -> cheetah_domain::Result<Page<MediaSession>> {
        list::media_sessions(
            self.tx().await?.as_mut(),
            tenant_id,
            device_id.map(|d| d.as_uuid()),
            purpose,
            state,
            updated_after,
            page,
        )
        .await
    }
}

#[derive(FromRow)]
struct MediaBindingRow {
    #[sqlx(rename = "data")]
    data: Json<MediaBinding>,
}

#[async_trait::async_trait]
impl MediaBindingRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        media_binding_id: cheetah_signal_types::MediaBindingId,
    ) -> cheetah_domain::Result<Option<MediaBinding>> {
        let row: Option<MediaBindingRow> = sqlx::query_as::<sqlx::Postgres, MediaBindingRow>(
            "SELECT data FROM media_bindings WHERE tenant_id = $1 AND media_binding_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(media_binding_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_media_session(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        media_session_id: cheetah_signal_types::MediaSessionId,
    ) -> cheetah_domain::Result<Option<MediaBinding>> {
        let row: Option<MediaBindingRow> = sqlx::query_as::<sqlx::Postgres, MediaBindingRow>(
            "SELECT data FROM media_bindings WHERE tenant_id = $1 AND media_session_id = $2 AND state NOT IN ('released', 'failed') ORDER BY created_at DESC LIMIT 1",
        )
        .bind(tenant_id.as_uuid())
        .bind(media_session_id.as_uuid())
        .fetch_optional(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, binding: &MediaBinding) -> cheetah_domain::Result<()> {
        let result = sqlx::query(
            "INSERT INTO media_bindings (
                tenant_id, media_binding_id, media_session_id, channel_id, media_node_id, owner_epoch,
                state, revision, created_at, updated_at, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT(media_binding_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                media_session_id = EXCLUDED.media_session_id,
                channel_id = EXCLUDED.channel_id,
                media_node_id = EXCLUDED.media_node_id,
                owner_epoch = EXCLUDED.owner_epoch,
                state = EXCLUDED.state,
                revision = EXCLUDED.revision,
                created_at = EXCLUDED.created_at,
                updated_at = EXCLUDED.updated_at,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE media_bindings.revision = EXCLUDED.revision - 1",
        )
        .bind(binding.tenant_id().as_uuid())
        .bind(binding.media_binding_id().as_uuid())
        .bind(binding.media_session_id().as_uuid())
        .bind(binding.channel_id().as_uuid())
        .bind(binding.media_node_id().as_uuid())
        .bind(binding.owner_epoch().0 as i64)
        .bind(variant_name(&binding.state())?)
        .bind(binding.revision().0 as i64)
        .bind(binding.created_at().as_offset())
        .bind(binding.updated_at().as_offset())
        .bind(Json(binding))
        .bind(1i32)
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> =
                sqlx::query_as("SELECT revision FROM media_bindings WHERE media_binding_id = $1")
                    .bind(binding.media_binding_id().as_uuid())
                    .fetch_optional(self.tx().await?.as_mut())
                    .await
                    .map_err(sqlx_to_domain)?;
            let found = found.and_then(|(r,)| u64::try_from(r).ok()).unwrap_or(0);
            return Err(concurrent_modification_error(
                binding.revision().0.saturating_sub(1),
                found,
            ));
        }
        Ok(())
    }
}

#[derive(FromRow)]
struct OutboxRow {
    #[sqlx(rename = "payload")]
    payload: Json<Event<DomainEvent>>,
    published: bool,
    attempts: i64,
    failed: bool,
    next_attempt_at: Option<OffsetDateTime>,
    error: Option<String>,
}

#[async_trait::async_trait]
impl Outbox for PostgresUnitOfWork {
    async fn append(&mut self, event: Event<DomainEvent>) -> cheetah_domain::Result<()> {
        sqlx::query(
            "INSERT INTO outbox_events (
                event_id, tenant_id, aggregate_ref, aggregate_sequence, payload, published,
                attempts, failed, next_attempt_at, error,
                occurred_at, correlation_id, causation_id, source
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(event.event_id.as_uuid())
        .bind(event.tenant_id.as_uuid())
        .bind(Json(&event.aggregate_ref))
        .bind(event.aggregate_sequence as i64)
        .bind(Json(&event))
        .bind(false)
        .bind(0i64)
        .bind(false)
        .bind(Option::<OffsetDateTime>::None)
        .bind(Option::<String>::None)
        .bind(event.occurred_at.as_offset())
        .bind(event.correlation_id.as_uuid())
        .bind(event.causation_id.as_uuid())
        .bind(event.source.as_uuid())
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;
        Ok(())
    }

    async fn pending(
        &mut self,
        now: cheetah_signal_types::UtcTimestamp,
        limit: usize,
    ) -> cheetah_domain::Result<Vec<OutboxEntry>> {
        let rows: Vec<OutboxRow> = sqlx::query_as::<sqlx::Postgres, OutboxRow>(
            "SELECT payload, published, attempts, failed, next_attempt_at, error
             FROM outbox_events
             WHERE published = false AND failed = false AND (next_attempt_at IS NULL OR next_attempt_at <= $2)
             ORDER BY occurred_at ASC LIMIT $1",
        )
        .bind(limit as i64)
        .bind(now.as_offset())
        .fetch_all(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;

        Ok(rows
            .into_iter()
            .map(|r| OutboxEntry {
                event: r.payload.0,
                published: r.published,
                attempts: r.attempts as u32,
                failed: r.failed,
                error: r.error,
                next_attempt_at: r
                    .next_attempt_at
                    .map(cheetah_signal_types::UtcTimestamp::from_offset),
            })
            .collect())
    }

    async fn mark_published(
        &mut self,
        event_id: cheetah_signal_types::EventId,
    ) -> cheetah_domain::Result<()> {
        sqlx::query("UPDATE outbox_events SET published = true WHERE event_id = $1")
            .bind(event_id.as_uuid())
            .execute(self.tx().await?.as_mut())
            .await
            .map_err(sqlx_to_domain)?;
        Ok(())
    }

    async fn mark_failed(
        &mut self,
        event_id: cheetah_signal_types::EventId,
        attempts: u32,
        failed: bool,
        error: Option<String>,
        next_attempt_at: Option<cheetah_signal_types::UtcTimestamp>,
    ) -> cheetah_domain::Result<()> {
        sqlx::query(
            "UPDATE outbox_events
             SET attempts = $1, failed = $2, error = $3, next_attempt_at = $4
             WHERE event_id = $5",
        )
        .bind(attempts as i64)
        .bind(failed)
        .bind(error)
        .bind(next_attempt_at.map(|t| t.as_offset()))
        .bind(event_id.as_uuid())
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;
        Ok(())
    }
}

#[derive(FromRow)]
struct ProcessedMessageRow {
    tenant_id: uuid::Uuid,
    message_id: uuid::Uuid,
    idempotency_key: Option<String>,
    status: String,
    result_payload: Option<String>,
    processed_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
}

fn processed_status_to_string(status: ProcessedMessageStatus) -> &'static str {
    match status {
        ProcessedMessageStatus::Pending => "pending",
        ProcessedMessageStatus::Completed => "completed",
        ProcessedMessageStatus::Failed => "failed",
        ProcessedMessageStatus::Duplicate => "duplicate",
    }
}

fn processed_status_from_string(status: &str) -> Result<ProcessedMessageStatus, DomainError> {
    match status {
        "pending" => Ok(ProcessedMessageStatus::Pending),
        "completed" => Ok(ProcessedMessageStatus::Completed),
        "failed" => Ok(ProcessedMessageStatus::Failed),
        "duplicate" => Ok(ProcessedMessageStatus::Duplicate),
        _ => Err(DomainError::internal(format!(
            "unknown processed message status: {status}"
        ))),
    }
}

fn processed_row_to_record(
    row: ProcessedMessageRow,
) -> Result<ProcessedMessageRecord, DomainError> {
    Ok(ProcessedMessageRecord {
        tenant_id: row.tenant_id.into(),
        message_id: row.message_id.into(),
        idempotency_key: row.idempotency_key,
        status: processed_status_from_string(&row.status)?,
        result_payload: row.result_payload,
        processed_at: UtcTimestamp::from_offset(row.processed_at),
        expires_at: row.expires_at.map(UtcTimestamp::from_offset),
    })
}

#[async_trait::async_trait]
impl ProcessedMessageRepository for PostgresUnitOfWork {
    async fn find(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        message_id: MessageId,
    ) -> cheetah_domain::Result<Option<ProcessedMessageRecord>> {
        let row: Option<ProcessedMessageRow> =
            sqlx::query_as::<sqlx::Postgres, ProcessedMessageRow>(
                "SELECT tenant_id, message_id, idempotency_key, status, result_payload, processed_at, expires_at
                 FROM processed_messages WHERE tenant_id = $1 AND message_id = $2",
            )
            .bind(tenant_id.as_uuid())
            .bind(message_id.as_uuid())
            .fetch_optional(self.tx().await?.as_mut())
            .await
            .map_err(sqlx_to_domain)?;

        row.map(processed_row_to_record).transpose()
    }

    async fn get_or_insert(
        &mut self,
        record: ProcessedMessageRecord,
    ) -> cheetah_domain::Result<Option<ProcessedMessageRecord>> {
        let inserted: Option<ProcessedMessageRow> =
            sqlx::query_as::<sqlx::Postgres, ProcessedMessageRow>(
                "INSERT INTO processed_messages (
                    tenant_id, message_id, idempotency_key, status, result_payload, processed_at, expires_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT(tenant_id, message_id) DO NOTHING
                RETURNING tenant_id, message_id, idempotency_key, status, result_payload, processed_at, expires_at",
            )
            .bind(record.tenant_id.as_uuid())
            .bind(record.message_id.as_uuid())
            .bind(record.idempotency_key)
            .bind(processed_status_to_string(record.status))
            .bind(record.result_payload)
            .bind(record.processed_at.as_offset())
            .bind(record.expires_at.map(|t| t.as_offset()))
            .fetch_optional(self.tx().await?.as_mut())
            .await
            .map_err(sqlx_to_domain)?;

        if inserted.is_some() {
            Ok(None)
        } else {
            self.find(record.tenant_id, record.message_id).await
        }
    }

    async fn complete(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        message_id: MessageId,
        status: ProcessedMessageStatus,
        result_payload: Option<String>,
        processed_at: UtcTimestamp,
    ) -> cheetah_domain::Result<()> {
        sqlx::query(
            "UPDATE processed_messages
             SET status = $1, result_payload = $2, processed_at = $3
             WHERE tenant_id = $4 AND message_id = $5",
        )
        .bind(processed_status_to_string(status))
        .bind(result_payload)
        .bind(processed_at.as_offset())
        .bind(tenant_id.as_uuid())
        .bind(message_id.as_uuid())
        .execute(self.tx().await?.as_mut())
        .await
        .map_err(sqlx_to_domain)?;
        Ok(())
    }
}
