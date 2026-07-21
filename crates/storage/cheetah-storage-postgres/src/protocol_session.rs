//! Protocol session repository for PostgreSQL.

use crate::error::sqlx_to_domain;
use cheetah_domain::{DomainError, Protocol, ProtocolSession, ProtocolSessionRepository, Result};
use cheetah_signal_types::{
    DeviceId, ListCursor, Page, PageRequest, ProtocolIdentity, ProtocolSessionId, Revision,
    TenantId, UtcTimestamp,
};
use cheetah_storage_api::stored_revision_as_u64;
use sqlx::types::Json;
use sqlx::{FromRow, PgPool};
use time::{Duration, OffsetDateTime};

fn to_millis(ts: UtcTimestamp) -> i64 {
    let offset = ts.as_offset();
    offset.unix_timestamp() * 1000 + i64::from(offset.nanosecond()) / 1_000_000
}

fn from_millis(ms: i64) -> UtcTimestamp {
    UtcTimestamp::from_offset(OffsetDateTime::UNIX_EPOCH + Duration::milliseconds(ms))
}

#[derive(FromRow)]
struct ProtocolSessionRow {
    protocol_session_id: uuid::Uuid,
    updated_at: i64,
    #[sqlx(rename = "data")]
    data: Json<ProtocolSession>,
}

/// PostgreSQL protocol session repository.
#[derive(Debug, Clone)]
pub struct PostgresProtocolSessionRepository {
    read_pool: PgPool,
    write_pool: PgPool,
}

impl PostgresProtocolSessionRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: PgPool, write_pool: PgPool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl ProtocolSessionRepository for PostgresProtocolSessionRepository {
    async fn get(
        &self,
        tenant_id: TenantId,
        protocol_session_id: ProtocolSessionId,
    ) -> Result<Option<ProtocolSession>> {
        let row: Option<ProtocolSessionRow> = sqlx::query_as::<sqlx::Postgres, ProtocolSessionRow>(
            "SELECT protocol_session_id, updated_at, data FROM protocol_sessions
             WHERE tenant_id = $1 AND protocol_session_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(protocol_session_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_device(
        &self,
        tenant_id: TenantId,
        protocol: Protocol,
        device_id: DeviceId,
    ) -> Result<Option<ProtocolSession>> {
        let row: Option<ProtocolSessionRow> = sqlx::query_as::<sqlx::Postgres, ProtocolSessionRow>(
            "SELECT protocol_session_id, updated_at, data FROM protocol_sessions
             WHERE tenant_id = $1 AND protocol = $2 AND device_id = $3",
        )
        .bind(tenant_id.as_uuid())
        .bind(protocol.to_string())
        .bind(device_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_identity(
        &self,
        tenant_id: TenantId,
        protocol: Protocol,
        protocol_identity: ProtocolIdentity,
    ) -> Result<Option<ProtocolSession>> {
        let row: Option<ProtocolSessionRow> = sqlx::query_as::<sqlx::Postgres, ProtocolSessionRow>(
            "SELECT protocol_session_id, updated_at, data FROM protocol_sessions
             WHERE tenant_id = $1 AND protocol = $2 AND protocol_identity = $3",
        )
        .bind(tenant_id.as_uuid())
        .bind(protocol.to_string())
        .bind(protocol_identity.as_str())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, session: &ProtocolSession) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO protocol_sessions (
                protocol_session_id, tenant_id, device_id, protocol, protocol_identity,
                presence, expiry_at, owner_epoch, revision, created_at, updated_at, data, schema_version
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT(protocol_session_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                device_id = EXCLUDED.device_id,
                protocol = EXCLUDED.protocol,
                protocol_identity = EXCLUDED.protocol_identity,
                presence = EXCLUDED.presence,
                expiry_at = EXCLUDED.expiry_at,
                owner_epoch = EXCLUDED.owner_epoch,
                revision = EXCLUDED.revision,
                created_at = COALESCE(protocol_sessions.created_at, EXCLUDED.created_at),
                updated_at = EXCLUDED.updated_at,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE protocol_sessions.revision = EXCLUDED.revision - 1",
        )
        .bind(session.protocol_session_id().as_uuid())
        .bind(session.tenant_id().as_uuid())
        .bind(session.device_id().as_uuid())
        .bind(session.protocol().to_string())
        .bind(session.protocol_identity().as_str())
        .bind(session.presence().to_string())
        .bind(to_millis(session.expiry_at()))
        .bind(session.owner_epoch().0 as i64)
        .bind(session.revision().0 as i64)
        .bind(to_millis(session.created_at()))
        .bind(to_millis(session.updated_at()))
        .bind(Json(session))
        .bind(1i64)
        .execute(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> = sqlx::query_as(
                "SELECT revision FROM protocol_sessions WHERE protocol_session_id = $1",
            )
            .bind(session.protocol_session_id().as_uuid())
            .fetch_optional(&self.write_pool)
            .await
            .map_err(sqlx_to_domain)?;
            let found = match found {
                Some((r,)) => stored_revision_as_u64(r)?,
                None => 0,
            };
            return Err(DomainError::ConcurrentModification {
                expected: session.revision().0.saturating_sub(1),
                found,
            });
        }
        Ok(())
    }

    async fn delete(
        &mut self,
        tenant_id: TenantId,
        protocol_session_id: ProtocolSessionId,
        expected_revision: Revision,
    ) -> Result<()> {
        let result = sqlx::query(
            "DELETE FROM protocol_sessions
             WHERE tenant_id = $1 AND protocol_session_id = $2 AND revision = $3",
        )
        .bind(tenant_id.as_uuid())
        .bind(protocol_session_id.as_uuid())
        .bind(expected_revision.0 as i64)
        .execute(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() == 1 {
            return Ok(());
        }

        let found: Option<(i64,)> = sqlx::query_as(
            "SELECT revision FROM protocol_sessions WHERE tenant_id = $1 AND protocol_session_id = $2",
        )
        .bind(tenant_id.as_uuid())
        .bind(protocol_session_id.as_uuid())
        .fetch_optional(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        match found {
            Some((r,)) => Err(DomainError::ConcurrentModification {
                expected: expected_revision.0,
                found: stored_revision_as_u64(r)?,
            }),
            None => Err(DomainError::not_found(
                "protocol_session",
                protocol_session_id.as_uuid().to_string(),
            )),
        }
    }

    async fn list_expired(
        &self,
        now: UtcTimestamp,
        page: PageRequest,
    ) -> Result<Page<ProtocolSession>> {
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Postgres> = sqlx::QueryBuilder::new(
            "SELECT protocol_session_id, updated_at, data FROM protocol_sessions WHERE expiry_at <= ",
        );
        qb.push_bind(to_millis(now));

        if let Some(cursor_value) = &page.cursor {
            let cursor = ListCursor::decode(cursor_value)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (updated_at, id) = cursor
                .parse()
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            qb.push(" AND (updated_at, protocol_session_id) > (");
            qb.push_bind(to_millis(updated_at));
            qb.push(", ");
            qb.push_bind(id);
            qb.push(")");
        }

        qb.push(" ORDER BY updated_at, protocol_session_id LIMIT ");
        qb.push_bind((page.page_size + 1) as i64);

        let rows: Vec<ProtocolSessionRow> = qb
            .build_query_as::<ProtocolSessionRow>()
            .fetch_all(&self.read_pool)
            .await
            .map_err(sqlx_to_domain)?;

        let page_size = page.page_size_as_usize();
        let has_more = rows.len() > page_size;
        let next_cursor = if has_more {
            let last = rows
                .get(page_size - 1)
                .ok_or_else(|| DomainError::internal("empty page"))?;
            Some(
                ListCursor::new(from_millis(last.updated_at), last.protocol_session_id)
                    .and_then(|c| c.encode())
                    .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let sessions: Vec<ProtocolSession> =
            rows.into_iter().take(page_size).map(|r| r.data.0).collect();

        let mut result = Page::new(sessions);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }
}
