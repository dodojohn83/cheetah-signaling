//! Tenant repository for SQLite.

use cheetah_domain::Tenant;
use cheetah_signal_types::{ListCursor, Page, PageRequest, TenantId, UtcTimestamp};
use cheetah_storage_api::{StorageError, TenantRepository};
use sqlx::{FromRow, SqlitePool};

#[derive(FromRow)]
struct TenantRow {
    tenant_id: uuid::Uuid,
    name: String,
    created_at: String,
    updated_at: String,
    deleted: i32,
}

impl TryFrom<TenantRow> for Tenant {
    type Error = StorageError;

    fn try_from(row: TenantRow) -> Result<Self, Self::Error> {
        Ok(Self {
            tenant_id: TenantId::from_uuid(row.tenant_id),
            name: row.name,
            created_at: parse_ts(&row.created_at)?,
            updated_at: parse_ts(&row.updated_at)?,
            deleted: row.deleted != 0,
        })
    }
}

fn parse_ts(value: &str) -> Result<UtcTimestamp, StorageError> {
    UtcTimestamp::parse_rfc3339(value)
        .map_err(|e| StorageError::backend(format!("invalid tenant timestamp: {e}")))
}

fn format_ts(ts: UtcTimestamp) -> Result<String, StorageError> {
    ts.to_rfc3339()
        .map_err(|e| StorageError::backend(format!("failed to format tenant timestamp: {e}")))
}

/// SQLite tenant repository.
#[derive(Debug, Clone)]
pub struct SqliteTenantRepository {
    read_pool: SqlitePool,
    write_pool: SqlitePool,
}

impl SqliteTenantRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: SqlitePool, write_pool: SqlitePool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl TenantRepository for SqliteTenantRepository {
    async fn save(&mut self, tenant: &Tenant) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO tenants (tenant_id, name, created_at, updated_at, deleted)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(tenant_id) DO UPDATE SET
                name = EXCLUDED.name,
                updated_at = EXCLUDED.updated_at,
                deleted = EXCLUDED.deleted,
                created_at = COALESCE(tenants.created_at, EXCLUDED.created_at)",
        )
        .bind(tenant.tenant_id.as_uuid())
        .bind(&tenant.name)
        .bind(format_ts(tenant.created_at)?)
        .bind(format_ts(tenant.updated_at)?)
        .bind(if tenant.deleted { 1i32 } else { 0i32 })
        .execute(&self.write_pool)
        .await
        .map_err(StorageError::backend)?;
        Ok(())
    }

    async fn get(&self, tenant_id: TenantId) -> Result<Option<Tenant>, StorageError> {
        let row: Option<TenantRow> = sqlx::query_as::<sqlx::Sqlite, TenantRow>(
            "SELECT tenant_id, name, created_at, updated_at, deleted
             FROM tenants
             WHERE tenant_id = ? AND deleted = 0",
        )
        .bind(tenant_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(StorageError::backend)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn list(
        &self,
        name_prefix: Option<&str>,
        page: PageRequest,
    ) -> Result<Page<Tenant>, StorageError> {
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "SELECT tenant_id, name, created_at, updated_at, deleted
             FROM tenants
             WHERE deleted = 0",
        );

        if let Some(prefix) = name_prefix
            && !prefix.is_empty()
        {
            qb.push(" AND name LIKE ");
            qb.push_bind(format!("{prefix}%"));
        }

        if let Some(cursor_value) = &page.cursor {
            let cursor = ListCursor::decode(cursor_value)
                .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (updated_at, id) = cursor
                .parse()
                .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
            qb.push(" AND (updated_at, tenant_id) > (");
            qb.push_bind(format_ts(updated_at)?);
            qb.push(", ");
            qb.push_bind(id);
            qb.push(")");
        }

        let page_size = page.page_size_as_usize_clamped();
        qb.push(" ORDER BY updated_at, tenant_id LIMIT ");
        qb.push_bind(page.limit_plus_one());

        let rows: Vec<TenantRow> = qb
            .build_query_as::<TenantRow>()
            .fetch_all(&self.read_pool)
            .await
            .map_err(StorageError::backend)?;

        let mut tenants: Vec<Tenant> = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = tenants.len() > page_size;
        if has_more {
            tenants.truncate(page_size);
        }

        let next_cursor = if has_more {
            let last = tenants
                .last()
                .ok_or_else(|| StorageError::internal("empty page"))?;
            Some(
                ListCursor::new(last.updated_at, last.tenant_id.as_uuid())
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?
                    .encode()
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let mut result = Page::new(tenants);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }
}
