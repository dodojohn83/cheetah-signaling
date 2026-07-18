// Concrete webhook repository implementation for a specific SQLx driver.
//
// The including module must define `pub(crate) type Db = ::sqlx::<Driver>;` before
// `include!`ing this file.

use ::cheetah_domain::{DomainError, Result, WebhookConfig, WebhookDelivery};
use ::cheetah_signal_types::{DeliveryId, Page, PageRequest, TenantId, UtcTimestamp, WebhookId};
use ::sqlx::{types::Json, QueryBuilder};

#[derive(::sqlx::FromRow)]
struct WebhookConfigRow {
    updated_at: ::time::OffsetDateTime,
    #[sqlx(rename = "id")]
    id: ::uuid::Uuid,
    data: Json<WebhookConfig>,
}

#[derive(::sqlx::FromRow)]
struct WebhookDeliveryRow {
    updated_at: ::time::OffsetDateTime,
    #[sqlx(rename = "id")]
    id: ::uuid::Uuid,
    data: Json<WebhookDelivery>,
}

fn decode_cursor(page: &PageRequest) -> Result<Option<::uuid::Uuid>> {
    match &page.cursor {
        None => Ok(None),
        Some(value) => {
            let cursor = ::cheetah_signal_types::ListCursor::decode(value)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (_ts, id) = cursor
                .parse()
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            Ok(Some(id))
        }
    }
}

fn to_config_page(rows: Vec<WebhookConfigRow>, page_size: u32) -> Result<Page<WebhookConfig>> {
    let page_size = page_size as usize;
    let next_cursor = if rows.len() > page_size {
        let last = &rows[page_size - 1];
        let ts = UtcTimestamp::from_offset(last.updated_at);
        Some(
            ::cheetah_signal_types::ListCursor::new(ts, last.id)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?
                .encode()
                .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
        )
    } else {
        None
    };
    let items: Vec<WebhookConfig> = rows
        .into_iter()
        .take(page_size)
        .map(|r| r.data.0)
        .collect();
    let mut page = Page::new(items);
    if let Some(cursor) = next_cursor {
        page = page.with_next_cursor(cursor);
    }
    Ok(page)
}

fn to_delivery_page(rows: Vec<WebhookDeliveryRow>, page_size: u32) -> Result<Page<WebhookDelivery>> {
    let page_size = page_size as usize;
    let next_cursor = if rows.len() > page_size {
        let last = &rows[page_size - 1];
        let ts = UtcTimestamp::from_offset(last.updated_at);
        Some(
            ::cheetah_signal_types::ListCursor::new(ts, last.id)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?
                .encode()
                .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
        )
    } else {
        None
    };
    let items: Vec<WebhookDelivery> = rows
        .into_iter()
        .take(page_size)
        .map(|r| r.data.0)
        .collect();
    let mut page = Page::new(items);
    if let Some(cursor) = next_cursor {
        page = page.with_next_cursor(cursor);
    }
    Ok(page)
}

pub(crate) async fn get_webhook_config(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    webhook_id: WebhookId,
) -> Result<Option<WebhookConfig>> {
    let row: Option<(Json<WebhookConfig>,)> = sqlx::query_as(
        "SELECT data FROM webhook_configs WHERE tenant_id = $1 AND webhook_id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(webhook_id.as_uuid())
    .fetch_optional(conn)
    .await
    .map_err(crate::error::sqlx_to_domain)?;
    Ok(row.map(|r| r.0 .0))
}

pub(crate) async fn save_webhook_config(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    config: &WebhookConfig,
) -> Result<()> {
    let new_revision = i64::try_from(config.revision().0)
        .map_err(|_| DomainError::internal("webhook config revision overflow"))?;
    let previous_revision = i64::try_from(config.revision().0.saturating_sub(1))
        .map_err(|_| DomainError::internal("webhook config previous revision overflow"))?;
    let data = Json(config.clone());
    let event_types = Json(config.event_types().to_vec());

    let result = sqlx::query(
        "INSERT INTO webhook_configs (\
            tenant_id, webhook_id, url, secret_ref, event_types, enabled, \
            revision, updated_at, data, schema_version\
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1) \
         ON CONFLICT(webhook_id) DO UPDATE SET \
            tenant_id = EXCLUDED.tenant_id, \
            url = EXCLUDED.url, \
            secret_ref = EXCLUDED.secret_ref, \
            event_types = EXCLUDED.event_types, \
            enabled = EXCLUDED.enabled, \
            revision = EXCLUDED.revision, \
            updated_at = EXCLUDED.updated_at, \
            data = EXCLUDED.data, \
            schema_version = EXCLUDED.schema_version \
         WHERE webhook_configs.tenant_id = EXCLUDED.tenant_id \
            AND webhook_configs.revision = $10",
    )
    .bind(config.tenant_id().as_uuid())
    .bind(config.webhook_id().as_uuid())
    .bind(config.url())
    .bind(config.secret_ref())
    .bind(event_types)
    .bind(config.enabled())
    .bind(new_revision)
    .bind(config.updated_at().as_offset())
    .bind(data)
    .bind(previous_revision)
    .execute(&mut *conn)
    .await
    .map_err(crate::error::sqlx_to_domain)?;

    if result.rows_affected() != 1 {
        let placeholder = if IS_POSTGRES { "$1" } else { "?" };
        let query = format!("SELECT revision FROM webhook_configs WHERE webhook_id = {placeholder}");
        let found: Option<(i64,)> = sqlx::query_as(&query)
            .bind(config.webhook_id().as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(crate::error::sqlx_to_domain)?;
        let found = found
            .and_then(|(r,)| u64::try_from(r).ok())
            .unwrap_or(0);
        return Err(DomainError::ConcurrentModification {
            expected: config.revision().0.saturating_sub(1),
            found,
        });
    }
    Ok(())
}

pub(crate) async fn delete_webhook_config(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    webhook_id: WebhookId,
) -> Result<()> {
    sqlx::query("DELETE FROM webhook_configs WHERE tenant_id = $1 AND webhook_id = $2")
        .bind(tenant_id.as_uuid())
        .bind(webhook_id.as_uuid())
        .execute(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;
    Ok(())
}

pub(crate) async fn list_webhook_configs(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    enabled: Option<bool>,
    event_type: Option<String>,
    page: PageRequest,
) -> Result<Page<WebhookConfig>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, webhook_id AS id, data FROM webhook_configs WHERE tenant_id = ",
    );
    qb.push_bind(tenant_id.as_uuid());

    if let Some(enabled) = enabled {
        qb.push(" AND enabled = ");
        qb.push_bind(enabled);
    }

    if let Some(event_type) = event_type {
        if IS_POSTGRES {
            qb.push(" AND (event_types = '[]'::jsonb OR event_types @> ");
            qb.push_bind(Json(vec![event_type]));
            qb.push("::jsonb)");
        } else {
            qb.push(
                " AND (event_types = '[]' OR EXISTS (SELECT 1 FROM json_each(webhook_configs.event_types) WHERE value = ",
            );
            qb.push_bind(event_type);
            qb.push("))");
        }
    }

    if let Some(id) = cursor {
        qb.push(" AND webhook_id > ");
        qb.push_bind(id);
    }

    qb.push(" ORDER BY webhook_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<WebhookConfigRow> = qb
        .build_query_as::<WebhookConfigRow>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_config_page(rows, page.page_size)
}

pub(crate) async fn get_webhook_delivery(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    delivery_id: DeliveryId,
) -> Result<Option<WebhookDelivery>> {
    let row: Option<(Json<WebhookDelivery>,)> = sqlx::query_as(
        "SELECT data FROM webhook_deliveries WHERE tenant_id = $1 AND delivery_id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(delivery_id.as_uuid())
    .fetch_optional(conn)
    .await
    .map_err(crate::error::sqlx_to_domain)?;
    Ok(row.map(|r| r.0 .0))
}

pub(crate) async fn save_webhook_delivery(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    delivery: &WebhookDelivery,
) -> Result<()> {
    let data = Json(delivery.clone());
    sqlx::query(
        "INSERT INTO webhook_deliveries (tenant_id, delivery_id, webhook_id, event_id, status, \
         attempt_count, next_attempt_at, last_error, updated_at, data, schema_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 1) \
         ON CONFLICT (delivery_id) DO UPDATE SET \
         status = excluded.status, attempt_count = excluded.attempt_count, \
         next_attempt_at = excluded.next_attempt_at, last_error = excluded.last_error, \
         updated_at = excluded.updated_at, data = excluded.data, \
         schema_version = excluded.schema_version",
    )
    .bind(delivery.tenant_id().as_uuid())
    .bind(delivery.delivery_id().as_uuid())
    .bind(delivery.webhook_id().as_uuid())
    .bind(delivery.event_id().as_uuid())
    .bind(delivery.status().to_string())
    .bind(i64::from(delivery.attempt_count()))
    .bind(delivery.next_attempt_at().map(|t| t.as_offset()))
    .bind(delivery.last_error())
    .bind(delivery.updated_at().as_offset())
    .bind(data)
    .execute(&mut *conn)
    .await
    .map_err(crate::error::sqlx_to_domain)?;
    Ok(())
}

pub(crate) async fn list_webhook_deliveries(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    webhook_id: WebhookId,
    status: Option<String>,
    page: PageRequest,
) -> Result<Page<WebhookDelivery>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, delivery_id AS id, data FROM webhook_deliveries WHERE tenant_id = ",
    );
    qb.push_bind(tenant_id.as_uuid());
    qb.push(" AND webhook_id = ");
    qb.push_bind(webhook_id.as_uuid());

    if let Some(status) = &status {
        qb.push(" AND status = ");
        qb.push_bind(status.clone());
    }

    if let Some(id) = cursor {
        qb.push(" AND delivery_id > ");
        qb.push_bind(id);
    }

    qb.push(" ORDER BY delivery_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<WebhookDeliveryRow> = qb
        .build_query_as::<WebhookDeliveryRow>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_delivery_page(rows, page.page_size)
}

pub(crate) async fn pending_webhook_deliveries(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    now: UtcTimestamp,
    limit: usize,
) -> Result<Vec<WebhookDelivery>> {
    use ::cheetah_domain::DeliveryStatus;

    let status_pending = DeliveryStatus::Pending.to_string();
    let status_failed = DeliveryStatus::Failed.to_string();

    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT data FROM webhook_deliveries WHERE status IN (",
    );
    let mut separated = qb.separated(',');
    separated.push_bind(status_pending);
    separated.push_bind(status_failed);

    if IS_POSTGRES {
        qb.push(") AND (next_attempt_at IS NULL OR next_attempt_at <= ",
        );
        qb.push_bind(now.as_offset());
        qb.push(") ORDER BY next_attempt_at NULLS FIRST, delivery_id LIMIT ");
    } else {
        qb.push(") AND (next_attempt_at IS NULL OR julianday(next_attempt_at) <= julianday(");
        qb.push_bind(now.as_offset());
        qb.push(")) ORDER BY COALESCE(julianday(next_attempt_at), 0.0), delivery_id LIMIT ");
    }
    qb.push_bind(limit as i64);

    let rows: Vec<(Json<WebhookDelivery>,)> = qb
        .build_query_as::<(Json<WebhookDelivery>,)>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    Ok(rows.into_iter().map(|r| r.0 .0).collect())
}
