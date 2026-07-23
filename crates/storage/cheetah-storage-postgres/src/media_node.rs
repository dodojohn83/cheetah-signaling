//! Media node repository for PostgreSQL.

use cheetah_domain::{
    DomainEvent, MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus,
};
use cheetah_signal_types::{Event, ListCursor, NodeId, Page, PageRequest, UtcTimestamp};
use cheetah_storage_api::{MediaNodeRepository, StorageError};
use sqlx::types::Json;
use sqlx::{FromRow, PgConnection, PgPool};
use std::collections::BTreeMap;
use time::OffsetDateTime;

fn to_millis(ts: UtcTimestamp) -> i64 {
    let offset = ts.as_offset();
    offset.unix_timestamp() * 1000 + i64::from(offset.nanosecond()) / 1_000_000
}

fn from_millis(ms: i64) -> UtcTimestamp {
    UtcTimestamp::from_epoch_millis_saturating(ms)
}

fn status_to_string(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::Active => "active",
        NodeStatus::Draining => "draining",
        NodeStatus::Left => "left",
    }
}

fn parse_status(value: &str) -> Result<NodeStatus, StorageError> {
    match value {
        "active" => Ok(NodeStatus::Active),
        "draining" => Ok(NodeStatus::Draining),
        "left" => Ok(NodeStatus::Left),
        other => Err(StorageError::backend(format!(
            "unknown node status: {other}"
        ))),
    }
}

fn media_node_columns() -> &'static str {
    "node_id, instance_id, instance_epoch, zone, region, network_zones, labels,
     control_endpoint, media_addresses, capabilities, capacity, load, session_count,
     draining, status, last_heartbeat_at, lease_until, generation, contract_version, revision, updated_at"
}

/// Overwrites `MediaNodeUpdated` payloads with the persisted node snapshot and
/// appends the events to the outbox in the current transaction.
#[allow(clippy::explicit_auto_deref)]
async fn append_outbox_events(
    conn: &mut PgConnection,
    events: &mut [Event<DomainEvent>],
    persisted: &MediaNode,
) -> Result<(), StorageError> {
    for event in events.iter_mut() {
        event.aggregate_sequence = persisted.revision;
        if let DomainEvent::MediaNodeUpdated { ref mut node } = event.payload {
            *node = persisted.clone();
        }

        sqlx::query(
            "INSERT INTO outbox_events (
                event_id, tenant_id, aggregate_ref, aggregate_sequence, payload, published,
                attempts, failed, next_attempt_at, error,
                occurred_at, correlation_id, causation_id, source
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (event_id) DO NOTHING",
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
        .execute(&mut *conn)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
    }
    Ok(())
}

#[derive(FromRow)]
struct MediaNodeRow {
    node_id: uuid::Uuid,
    instance_id: String,
    instance_epoch: i64,
    zone: String,
    region: String,
    network_zones: Json<Vec<String>>,
    labels: Json<BTreeMap<String, String>>,
    control_endpoint: String,
    media_addresses: Json<Vec<String>>,
    capabilities: Json<Vec<MediaCapability>>,
    capacity: Json<MediaNodeCapacity>,
    load: i64,
    session_count: i64,
    draining: bool,
    status: String,
    last_heartbeat_at: Option<i64>,
    lease_until: Option<i64>,
    generation: i64,
    contract_version: i64,
    revision: i64,
    updated_at: i64,
}

impl TryFrom<MediaNodeRow> for MediaNode {
    type Error = StorageError;

    fn try_from(row: MediaNodeRow) -> Result<Self, Self::Error> {
        let mut node = MediaNode {
            node_id: row.node_id.into(),
            instance_id: row.instance_id,
            instance_epoch: row.instance_epoch as u64,
            zone: row.zone,
            region: row.region,
            network_zones: row.network_zones.0,
            labels: row.labels.0,
            control_endpoint: row.control_endpoint,
            media_addresses: row.media_addresses.0,
            capabilities: row.capabilities.0,
            capacity: row.capacity.0,
            load: row.load as u64,
            session_count: row.session_count as u64,
            health: MediaNodeHealth::Healthy,
            draining: row.draining,
            status: parse_status(&row.status)?,
            last_heartbeat_at: row.last_heartbeat_at.map(from_millis),
            lease_until: row.lease_until.map(from_millis),
            generation: row.generation as u64,
            contract_version: row.contract_version as u32,
            revision: row.revision as u64,
        };
        node.recalc_health();
        Ok(node)
    }
}

/// PostgreSQL media node repository.
#[derive(Debug, Clone)]
pub struct PostgresMediaNodeRepository {
    read_pool: PgPool,
    write_pool: PgPool,
}

impl PostgresMediaNodeRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: PgPool, write_pool: PgPool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl MediaNodeRepository for PostgresMediaNodeRepository {
    async fn register(
        &mut self,
        node: MediaNode,
        mut events: Vec<Event<DomainEvent>>,
    ) -> Result<MediaNode, StorageError> {
        let updated_at = node.last_heartbeat_at.map_or(0, to_millis);

        let mut tx = self
            .write_pool
            .begin()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let existing: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT instance_id, instance_epoch, revision FROM media_nodes WHERE node_id = $1",
        )
        .bind(node.node_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let new_revision = existing
            .as_ref()
            .map_or(1, |(_instance_id, _epoch, revision)| revision + 1);

        if let Some((existing_instance_id, existing_epoch, existing_revision)) = existing.as_ref()
            && existing_instance_id != &node.instance_id
            && (node.instance_epoch as i64) < *existing_epoch
        {
            return Err(StorageError::concurrent_modification(
                *existing_revision as u64,
                node.revision,
            ));
        }

        match existing {
            None => {
                sqlx::query(&format!(
                    "INSERT INTO media_nodes ({}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)",
                    media_node_columns()
                ))
                .bind(node.node_id.as_uuid())
                .bind(&node.instance_id)
                .bind(node.instance_epoch as i64)
                .bind(&node.zone)
                .bind(&node.region)
                .bind(Json(&node.network_zones))
                .bind(Json(&node.labels))
                .bind(&node.control_endpoint)
                .bind(Json(&node.media_addresses))
                .bind(Json(&node.capabilities))
                .bind(Json(&node.capacity))
                .bind(node.load as i64)
                .bind(node.session_count as i64)
                .bind(node.draining)
                .bind(status_to_string(node.status))
                .bind(node.last_heartbeat_at.map(to_millis))
                .bind(node.lease_until.map(to_millis))
                .bind(node.generation as i64)
                .bind(node.contract_version as i64)
                .bind(new_revision)
                .bind(updated_at)
                .execute(&mut *tx)
                .await
                .map_err(|e| StorageError::backend(e.to_string()))?;
            }
            Some(_) => {
                sqlx::query(
                    "UPDATE media_nodes SET
                        instance_id = $1, instance_epoch = $2, zone = $3, region = $4,
                        network_zones = $5, labels = $6, control_endpoint = $7, media_addresses = $8,
                        capabilities = $9, capacity = $10, load = $11, session_count = $12, draining = $13,
                        status = $14, last_heartbeat_at = $15, lease_until = $16, generation = $17,
                        contract_version = $18, revision = $19, updated_at = $20
                     WHERE node_id = $21"
                )
                .bind(&node.instance_id)
                .bind(node.instance_epoch as i64)
                .bind(&node.zone)
                .bind(&node.region)
                .bind(Json(&node.network_zones))
                .bind(Json(&node.labels))
                .bind(&node.control_endpoint)
                .bind(Json(&node.media_addresses))
                .bind(Json(&node.capabilities))
                .bind(Json(&node.capacity))
                .bind(node.load as i64)
                .bind(node.session_count as i64)
                .bind(node.draining)
                .bind(status_to_string(node.status))
                .bind(node.last_heartbeat_at.map(to_millis))
                .bind(node.lease_until.map(to_millis))
                .bind(node.generation as i64)
                .bind(node.contract_version as i64)
                .bind(new_revision)
                .bind(updated_at)
                .bind(node.node_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(|e| StorageError::backend(e.to_string()))?;
            }
        }

        let row: MediaNodeRow = sqlx::query_as::<sqlx::Postgres, MediaNodeRow>(&format!(
            "SELECT {} FROM media_nodes WHERE node_id = $1",
            media_node_columns()
        ))
        .bind(node.node_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let persisted: MediaNode = row.try_into()?;
        append_outbox_events(&mut tx, &mut events, &persisted).await?;

        tx.commit()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(persisted)
    }

    async fn heartbeat(
        &mut self,
        node_id: NodeId,
        instance_id: String,
        lease_until: UtcTimestamp,
        updated_at: UtcTimestamp,
        load: u64,
        session_count: u64,
        mut events: Vec<Event<DomainEvent>>,
    ) -> Result<Option<MediaNode>, StorageError> {
        let mut tx = self
            .write_pool
            .begin()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let row: Option<MediaNodeRow> = sqlx::query_as::<sqlx::Postgres, MediaNodeRow>(&format!(
            "UPDATE media_nodes SET
                    lease_until = $1, last_heartbeat_at = $2, load = $3, session_count = $4,
                    updated_at = $5, revision = revision + 1
                 WHERE node_id = $6 AND instance_id = $7
                 RETURNING {}",
            media_node_columns()
        ))
        .bind(to_millis(lease_until))
        .bind(to_millis(updated_at))
        .bind(load as i64)
        .bind(session_count as i64)
        .bind(to_millis(updated_at))
        .bind(node_id.as_uuid())
        .bind(&instance_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let result = match row {
            Some(row) => {
                let persisted: MediaNode = row.try_into()?;
                append_outbox_events(&mut tx, &mut events, &persisted).await?;
                Some(persisted)
            }
            None => None,
        };

        tx.commit()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(result)
    }

    async fn get(&self, node_id: NodeId) -> Result<Option<MediaNode>, StorageError> {
        let row: Option<MediaNodeRow> = sqlx::query_as::<sqlx::Postgres, MediaNodeRow>(&format!(
            "SELECT {} FROM media_nodes WHERE node_id = $1",
            media_node_columns()
        ))
        .bind(node_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        row.map(TryInto::try_into).transpose()
    }

    async fn list_alive(
        &self,
        now: UtcTimestamp,
        page: PageRequest,
    ) -> Result<Page<MediaNode>, StorageError> {
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Postgres> = sqlx::QueryBuilder::new(&format!(
            "SELECT {} FROM media_nodes WHERE lease_until IS NOT NULL AND lease_until > ",
            media_node_columns()
        ));
        qb.push_bind(to_millis(now));

        if let Some(cursor_value) = &page.cursor {
            let cursor = ListCursor::decode(cursor_value)
                .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
            let (updated_at, id) = cursor
                .parse()
                .map_err(|e| StorageError::invalid_argument(format!("invalid cursor: {e}")))?;
            qb.push(" AND (updated_at, node_id) > (");
            qb.push_bind(to_millis(updated_at));
            qb.push(", ");
            qb.push_bind(id);
            qb.push(")");
        }

        let page_size = page.page_size_as_usize_clamped();
        qb.push(" ORDER BY updated_at, node_id LIMIT ");
        qb.push_bind(page.limit_plus_one());

        let rows: Vec<MediaNodeRow> = qb
            .build_query_as::<MediaNodeRow>()
            .fetch_all(&self.read_pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let has_more = rows.len() > page_size;
        let next_cursor = if has_more {
            let last = rows
                .get(page_size - 1)
                .ok_or_else(|| StorageError::internal("empty page"))?;
            Some(
                ListCursor::new(from_millis(last.updated_at), last.node_id)
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?
                    .encode()
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let nodes: Vec<MediaNode> = rows
            .into_iter()
            .take(page_size)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let mut result = Page::new(nodes);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }

    async fn set_draining(
        &mut self,
        node_id: NodeId,
        instance_id: String,
        draining: bool,
        updated_at: UtcTimestamp,
        mut events: Vec<Event<DomainEvent>>,
    ) -> Result<Option<MediaNode>, StorageError> {
        let status = if draining { "draining" } else { "active" };
        let mut tx = self
            .write_pool
            .begin()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let row: Option<MediaNodeRow> = sqlx::query_as::<sqlx::Postgres, MediaNodeRow>(
            &format!(
                "UPDATE media_nodes SET draining = $1, status = $2, updated_at = $3, revision = revision + 1
                 WHERE node_id = $4 AND instance_id = $5
                 RETURNING {}",
                media_node_columns()
            )
        )
        .bind(draining)
        .bind(status)
        .bind(to_millis(updated_at))
        .bind(node_id.as_uuid())
        .bind(&instance_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let result = match row {
            Some(row) => {
                let persisted: MediaNode = row.try_into()?;
                append_outbox_events(&mut tx, &mut events, &persisted).await?;
                Some(persisted)
            }
            None => None,
        };

        tx.commit()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(result)
    }

    async fn deregister(
        &mut self,
        node_id: NodeId,
        instance_id: String,
        updated_at: UtcTimestamp,
        lease_until: Option<UtcTimestamp>,
        mut events: Vec<Event<DomainEvent>>,
    ) -> Result<Option<MediaNode>, StorageError> {
        let mut tx = self
            .write_pool
            .begin()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let row: Option<MediaNodeRow> = sqlx::query_as::<sqlx::Postgres, MediaNodeRow>(
            &format!(
                "UPDATE media_nodes SET status = 'left', lease_until = $1, updated_at = $2, revision = revision + 1
                 WHERE node_id = $3 AND instance_id = $4
                 RETURNING {}",
                media_node_columns()
            )
        )
        .bind(lease_until.map(to_millis))
        .bind(to_millis(updated_at))
        .bind(node_id.as_uuid())
        .bind(&instance_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        let result = match row {
            Some(row) => {
                let persisted: MediaNode = row.try_into()?;
                append_outbox_events(&mut tx, &mut events, &persisted).await?;
                Some(persisted)
            }
            None => None,
        };

        tx.commit()
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(result)
    }
}
