// Concrete paginated list implementation for a specific SQLx database driver.
//
// The including module must define `pub(crate) type Db = ::sqlx::<Driver>;` before
// `include!`ing this file.

use ::cheetah_domain::{Channel, Device, DomainError, MediaSession, Operation, Result};
use ::cheetah_signal_types::{ListCursor, Page, PageRequest, TenantId, UtcTimestamp};
use ::sqlx::{QueryBuilder, types::Json};
use ::time::OffsetDateTime;
use ::uuid::Uuid;

/// Row returned by paginated list queries. The SQL must alias the resource
/// identifier column to `id`.
#[derive(::sqlx::FromRow)]
struct ListRow<T> {
    updated_at: OffsetDateTime,
    #[sqlx(rename = "id")]
    id: Uuid,
    data: Json<T>,
}

fn to_page<T>(rows: Vec<ListRow<T>>, page_size: u32) -> Result<Page<T>> {
    let page_size = page_size as usize;
    let next_cursor = if rows.len() > page_size {
        let last = &rows[page_size - 1];
        let ts = UtcTimestamp::from_offset(last.updated_at);
        Some(
            ListCursor::new(ts, last.id)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?
                .encode()
                .map_err(|e| DomainError::internal(format!("failed to encode cursor: {e}")))?,
        )
    } else {
        None
    };

    let items: Vec<T> = rows.into_iter().take(page_size).map(|r| r.data.0).collect();
    let mut page = Page::new(items);
    if let Some(cursor) = next_cursor {
        page = page.with_next_cursor(cursor);
    }
    Ok(page)
}

fn decode_cursor(page: &PageRequest) -> Result<Option<Uuid>> {
    match &page.cursor {
        None => Ok(None),
        Some(value) => {
            let cursor = ListCursor::decode(value)
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (_ts, id) = cursor
                .parse()
                .map_err(|e| DomainError::invalid_argument(format!("invalid cursor: {e}")))?;
            Ok(Some(id))
        }
    }
}

fn push_string_filter(qb: &mut QueryBuilder<'_, Db>, column: &str, value: &Option<String>) {
    if let Some(value) = value {
        qb.push(" AND ");
        qb.push(column);
        qb.push(" = ");
        qb.push_bind(value.clone());
    }
}

fn push_name_prefix_filter(qb: &mut QueryBuilder<'_, Db>, column: &str, value: &Option<String>) {
    if let Some(value) = value {
        qb.push(" AND ");
        qb.push(column);
        qb.push(" LIKE ");
        qb.push_bind(format!("{value}%"));
    }
}

fn push_updated_after_filter(qb: &mut QueryBuilder<'_, Db>, value: &Option<UtcTimestamp>) {
    if let Some(value) = value {
        qb.push(" AND updated_at > ");
        qb.push_bind(value.as_offset());
    }
}

fn push_cursor_filter(qb: &mut QueryBuilder<'_, Db>, id_column: &str, cursor: Option<Uuid>) {
    if let Some(id) = cursor {
        qb.push(" AND ");
        qb.push(id_column);
        qb.push(" > ");
        qb.push_bind(id);
    }
}

pub(crate) async fn devices(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    protocol: Option<String>,
    lifecycle: Option<String>,
    name_prefix: Option<String>,
    updated_after: Option<UtcTimestamp>,
    page: PageRequest,
) -> Result<Page<Device>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, device_id AS id, data FROM devices WHERE deleted = ",
    );
    qb.push_bind(false);
    qb.push(" AND tenant_id = ");
    qb.push_bind(tenant_id.as_uuid());

    push_string_filter(&mut qb, "protocol", &protocol);
    push_string_filter(&mut qb, "lifecycle", &lifecycle);
    push_name_prefix_filter(&mut qb, "name", &name_prefix);
    push_updated_after_filter(&mut qb, &updated_after);
    push_cursor_filter(&mut qb, "device_id", cursor);

    qb.push(" ORDER BY device_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<ListRow<Device>> = qb
        .build_query_as::<ListRow<Device>>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_page(rows, page.page_size)
}

pub(crate) async fn channels(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    device_id: Uuid,
    status: Option<String>,
    name_prefix: Option<String>,
    updated_after: Option<UtcTimestamp>,
    page: PageRequest,
) -> Result<Page<Channel>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, channel_id AS id, data FROM channels WHERE deleted = ",
    );
    qb.push_bind(false);
    qb.push(" AND tenant_id = ");
    qb.push_bind(tenant_id.as_uuid());
    qb.push(" AND device_id = ");
    qb.push_bind(device_id);

    push_string_filter(&mut qb, "status", &status);
    push_name_prefix_filter(&mut qb, "name", &name_prefix);
    push_updated_after_filter(&mut qb, &updated_after);
    push_cursor_filter(&mut qb, "channel_id", cursor);

    qb.push(" ORDER BY channel_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<ListRow<Channel>> = qb
        .build_query_as::<ListRow<Channel>>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_page(rows, page.page_size)
}

pub(crate) async fn operations(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    device_id: Option<Uuid>,
    status: Option<String>,
    updated_after: Option<UtcTimestamp>,
    page: PageRequest,
) -> Result<Page<Operation>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, operation_id AS id, data FROM operations WHERE tenant_id = ",
    );
    qb.push_bind(tenant_id.as_uuid());

    if let Some(device_id) = device_id {
        qb.push(" AND device_id = ");
        qb.push_bind(device_id);
    }

    push_string_filter(&mut qb, "status", &status);
    push_updated_after_filter(&mut qb, &updated_after);
    push_cursor_filter(&mut qb, "operation_id", cursor);

    qb.push(" ORDER BY operation_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<ListRow<Operation>> = qb
        .build_query_as::<ListRow<Operation>>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_page(rows, page.page_size)
}

pub(crate) async fn media_sessions(
    conn: &mut <Db as ::sqlx::Database>::Connection,
    tenant_id: TenantId,
    device_id: Option<Uuid>,
    purpose: Option<String>,
    state: Option<String>,
    updated_after: Option<UtcTimestamp>,
    page: PageRequest,
) -> Result<Page<MediaSession>> {
    let cursor = decode_cursor(&page)?;
    let mut qb: QueryBuilder<'_, Db> = QueryBuilder::new(
        "SELECT updated_at, media_session_id AS id, data FROM media_sessions WHERE tenant_id = ",
    );
    qb.push_bind(tenant_id.as_uuid());

    if let Some(device_id) = device_id {
        qb.push(" AND device_id = ");
        qb.push_bind(device_id);
    }

    push_string_filter(&mut qb, "purpose", &purpose);
    push_string_filter(&mut qb, "state", &state);
    push_updated_after_filter(&mut qb, &updated_after);
    push_cursor_filter(&mut qb, "media_session_id", cursor);

    qb.push(" ORDER BY media_session_id LIMIT ");
    qb.push_bind((page.page_size + 1) as i64);

    let rows: Vec<ListRow<MediaSession>> = qb
        .build_query_as::<ListRow<MediaSession>>()
        .fetch_all(conn)
        .await
        .map_err(crate::error::sqlx_to_domain)?;

    to_page(rows, page.page_size)
}
