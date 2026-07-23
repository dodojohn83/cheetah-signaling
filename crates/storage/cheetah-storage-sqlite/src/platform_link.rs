//! Cascade platform link repository for SQLite.

use crate::error::sqlx_to_domain;
use cheetah_domain::{
    DomainError, GbPlatformLink, PlatformDirection, PlatformLinkRepository, Result,
};
use cheetah_signal_types::{
    ListCursor, Page, PageRequest, PlatformLinkId, ProtocolIdentity, Revision, TenantId,
    UtcTimestamp,
};
use cheetah_storage_api::stored_revision_as_u64;
use sqlx::types::Json;
use sqlx::{FromRow, SqlitePool};
fn to_millis(ts: UtcTimestamp) -> i64 {
    let offset = ts.as_offset();
    offset.unix_timestamp() * 1000 + i64::from(offset.nanosecond()) / 1_000_000
}

fn from_millis(ms: i64) -> UtcTimestamp {
    UtcTimestamp::from_epoch_millis_saturating(ms)
}

#[derive(FromRow)]
struct PlatformLinkRow {
    platform_link_id: uuid::Uuid,
    updated_at: i64,
    #[sqlx(rename = "data")]
    data: Json<GbPlatformLink>,
}

/// SQLite cascade platform link repository.
#[derive(Debug, Clone)]
pub struct SqlitePlatformLinkRepository {
    read_pool: SqlitePool,
    write_pool: SqlitePool,
}

impl SqlitePlatformLinkRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: SqlitePool, write_pool: SqlitePool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl PlatformLinkRepository for SqlitePlatformLinkRepository {
    async fn get(
        &self,
        tenant_id: TenantId,
        platform_link_id: PlatformLinkId,
    ) -> Result<Option<GbPlatformLink>> {
        let row: Option<PlatformLinkRow> = sqlx::query_as::<sqlx::Sqlite, PlatformLinkRow>(
            "SELECT platform_link_id, updated_at, data FROM platform_links
             WHERE tenant_id = ? AND platform_link_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(platform_link_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn get_by_remote_identity(
        &self,
        tenant_id: TenantId,
        direction: PlatformDirection,
        remote_identity: ProtocolIdentity,
    ) -> Result<Option<GbPlatformLink>> {
        let row: Option<PlatformLinkRow> = sqlx::query_as::<sqlx::Sqlite, PlatformLinkRow>(
            "SELECT platform_link_id, updated_at, data FROM platform_links
             WHERE tenant_id = ? AND direction = ? AND remote_identity = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(direction.to_string())
        .bind(remote_identity.as_str())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(sqlx_to_domain)?;

        Ok(row.map(|r| r.data.0))
    }

    async fn save(&mut self, link: &GbPlatformLink) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO platform_links (
                platform_link_id, tenant_id, direction, local_identity, remote_identity,
                desired_state, actual_state, owner_epoch, link_generation, revision,
                created_at, updated_at, data, schema_version
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(platform_link_id) DO UPDATE SET
                tenant_id = EXCLUDED.tenant_id,
                direction = EXCLUDED.direction,
                local_identity = EXCLUDED.local_identity,
                remote_identity = EXCLUDED.remote_identity,
                desired_state = EXCLUDED.desired_state,
                actual_state = EXCLUDED.actual_state,
                owner_epoch = EXCLUDED.owner_epoch,
                link_generation = EXCLUDED.link_generation,
                revision = EXCLUDED.revision,
                created_at = COALESCE(platform_links.created_at, EXCLUDED.created_at),
                updated_at = EXCLUDED.updated_at,
                data = EXCLUDED.data,
                schema_version = EXCLUDED.schema_version
            WHERE platform_links.revision = EXCLUDED.revision - 1",
        )
        .bind(link.platform_link_id().as_uuid())
        .bind(link.tenant_id().as_uuid())
        .bind(link.direction().to_string())
        .bind(link.identity().local.as_str())
        .bind(link.identity().remote.as_str())
        .bind(link.desired().to_string())
        .bind(link.actual().to_string())
        .bind(link.owner_epoch().0 as i64)
        .bind(link.generation() as i64)
        .bind(link.revision().0 as i64)
        .bind(to_millis(link.created_at()))
        .bind(to_millis(link.updated_at()))
        .bind(Json(link))
        .bind(1i32)
        .execute(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() != 1 {
            let found: Option<(i64,)> =
                sqlx::query_as("SELECT revision FROM platform_links WHERE platform_link_id = ?")
                    .bind(link.platform_link_id().as_uuid())
                    .fetch_optional(&self.write_pool)
                    .await
                    .map_err(sqlx_to_domain)?;
            let found = match found {
                Some((r,)) => stored_revision_as_u64(r)?,
                None => 0,
            };
            return Err(DomainError::ConcurrentModification {
                expected: link.revision().0.saturating_sub(1),
                found,
            });
        }
        Ok(())
    }

    async fn delete(
        &mut self,
        tenant_id: TenantId,
        platform_link_id: PlatformLinkId,
        expected_revision: Revision,
    ) -> Result<()> {
        let result = sqlx::query(
            "DELETE FROM platform_links
             WHERE tenant_id = ? AND platform_link_id = ? AND revision = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(platform_link_id.as_uuid())
        .bind(expected_revision.0 as i64)
        .execute(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        if result.rows_affected() == 1 {
            return Ok(());
        }

        let found: Option<(i64,)> = sqlx::query_as(
            "SELECT revision FROM platform_links WHERE tenant_id = ? AND platform_link_id = ?",
        )
        .bind(tenant_id.as_uuid())
        .bind(platform_link_id.as_uuid())
        .fetch_optional(&self.write_pool)
        .await
        .map_err(sqlx_to_domain)?;

        match found {
            Some((r,)) => Err(DomainError::ConcurrentModification {
                expected: expected_revision.0,
                found: stored_revision_as_u64(r)?,
            }),
            None => Err(DomainError::not_found(
                "platform_link",
                platform_link_id.as_uuid().to_string(),
            )),
        }
    }

    async fn list(
        &self,
        tenant_id: TenantId,
        direction: Option<PlatformDirection>,
        page: PageRequest,
    ) -> Result<Page<GbPlatformLink>> {
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "SELECT platform_link_id, updated_at, data FROM platform_links WHERE tenant_id = ",
        );
        qb.push_bind(tenant_id.as_uuid());

        if let Some(direction) = direction {
            qb.push(" AND direction = ");
            qb.push_bind(direction.to_string());
        }

        if let Some(cursor_value) = &page.cursor {
            let cursor = ListCursor::decode(cursor_value)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (updated_at, id) = cursor
                .parse()
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            qb.push(" AND (updated_at, platform_link_id) > (");
            qb.push_bind(to_millis(updated_at));
            qb.push(", ");
            qb.push_bind(id);
            qb.push(")");
        }

        let page_size = page.page_size_as_usize_clamped();
        qb.push(" ORDER BY updated_at, platform_link_id LIMIT ");
        qb.push_bind(page.limit_plus_one());

        let rows: Vec<PlatformLinkRow> = qb
            .build_query_as::<PlatformLinkRow>()
            .fetch_all(&self.read_pool)
            .await
            .map_err(sqlx_to_domain)?;

        let has_more = rows.len() > page_size;
        let next_cursor = if has_more {
            let last = rows
                .get(page_size - 1)
                .ok_or_else(|| DomainError::internal("empty page"))?;
            Some(
                ListCursor::new(from_millis(last.updated_at), last.platform_link_id)
                    .and_then(|c| c.encode())
                    .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let links: Vec<GbPlatformLink> =
            rows.into_iter().take(page_size).map(|r| r.data.0).collect();

        let mut result = Page::new(links);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }
}
