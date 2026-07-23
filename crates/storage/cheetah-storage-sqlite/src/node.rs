//! Cluster node repository for SQLite.

use cheetah_domain::{ClusterNode, NodeCapacity, NodeLoad};
use cheetah_signal_types::{ListCursor, NodeId, NodeInstanceId, Page, PageRequest, UtcTimestamp};
use cheetah_storage_api::{NodeRepository, StorageError};
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
struct NodeRow {
    node_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    zone: String,
    version: String,
    started_at: i64,
    lease_until: i64,
    updated_at: i64,
    draining: i32,
    contract_versions: Json<std::collections::HashMap<String, String>>,
    capacity: Json<NodeCapacity>,
    load: Json<NodeLoad>,
}

impl From<NodeRow> for ClusterNode {
    fn from(row: NodeRow) -> Self {
        let mut node = Self::new(
            row.node_id.into(),
            row.instance_id.into(),
            row.zone,
            row.version,
            from_millis(row.started_at),
        );
        node.lease_until = from_millis(row.lease_until);
        node.updated_at = from_millis(row.updated_at);
        node.draining = row.draining != 0;
        node.contract_versions = row.contract_versions.0;
        node.capacity = row.capacity.0;
        node.load = row.load.0;
        node
    }
}

/// SQLite cluster node repository.
#[derive(Debug, Clone)]
pub struct SqliteNodeRepository {
    read_pool: SqlitePool,
    write_pool: SqlitePool,
}

impl SqliteNodeRepository {
    /// Creates a new repository.
    pub const fn new(read_pool: SqlitePool, write_pool: SqlitePool) -> Self {
        Self {
            read_pool,
            write_pool,
        }
    }
}

#[async_trait::async_trait]
impl NodeRepository for SqliteNodeRepository {
    async fn register(&mut self, node: ClusterNode) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO cluster_nodes (
                node_id, instance_id, zone, version, started_at, lease_until, updated_at,
                draining, contract_versions, capacity, load
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(node_id) DO UPDATE SET
                instance_id = EXCLUDED.instance_id,
                zone = EXCLUDED.zone,
                version = EXCLUDED.version,
                started_at = EXCLUDED.started_at,
                lease_until = EXCLUDED.lease_until,
                updated_at = EXCLUDED.updated_at,
                draining = EXCLUDED.draining,
                contract_versions = EXCLUDED.contract_versions,
                capacity = EXCLUDED.capacity,
                load = EXCLUDED.load",
        )
        .bind(node.node_id.as_uuid())
        .bind(node.instance_id.as_uuid())
        .bind(&node.zone)
        .bind(&node.version)
        .bind(to_millis(node.started_at))
        .bind(to_millis(node.lease_until))
        .bind(to_millis(node.updated_at))
        .bind(if node.draining { 1i32 } else { 0i32 })
        .bind(Json(&node.contract_versions))
        .bind(Json(&node.capacity))
        .bind(Json(&node.load))
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(())
    }

    async fn heartbeat(
        &mut self,
        node_id: NodeId,
        instance_id: NodeInstanceId,
        lease_until: UtcTimestamp,
        updated_at: UtcTimestamp,
        load: NodeLoad,
    ) -> Result<Option<ClusterNode>, StorageError> {
        let result = sqlx::query(
            "UPDATE cluster_nodes
             SET lease_until = ?, updated_at = ?, load = ?
             WHERE node_id = ? AND instance_id = ?",
        )
        .bind(to_millis(lease_until))
        .bind(to_millis(updated_at))
        .bind(Json(load))
        .bind(node_id.as_uuid())
        .bind(instance_id.as_uuid())
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let row: Option<NodeRow> = sqlx::query_as::<sqlx::Sqlite, NodeRow>(
            "SELECT node_id, instance_id, zone, version, started_at, lease_until, updated_at,
                    draining, contract_versions, capacity, load
             FROM cluster_nodes
             WHERE node_id = ?",
        )
        .bind(node_id.as_uuid())
        .fetch_optional(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(Into::into))
    }

    async fn get(&self, node_id: NodeId) -> Result<Option<ClusterNode>, StorageError> {
        let row: Option<NodeRow> = sqlx::query_as::<sqlx::Sqlite, NodeRow>(
            "SELECT node_id, instance_id, zone, version, started_at, lease_until, updated_at,
                    draining, contract_versions, capacity, load
             FROM cluster_nodes
             WHERE node_id = ?",
        )
        .bind(node_id.as_uuid())
        .fetch_optional(&self.read_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;

        Ok(row.map(Into::into))
    }

    async fn list_alive(
        &self,
        now: UtcTimestamp,
        page: PageRequest,
    ) -> Result<Page<ClusterNode>, StorageError> {
        let mut qb: sqlx::QueryBuilder<'_, sqlx::Sqlite> = sqlx::QueryBuilder::new(
            "SELECT node_id, instance_id, zone, version, started_at, lease_until, updated_at,
                    draining, contract_versions, capacity, load
             FROM cluster_nodes
             WHERE lease_until > ",
        );
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

        let rows: Vec<NodeRow> = qb
            .build_query_as::<NodeRow>()
            .fetch_all(&self.read_pool)
            .await
            .map_err(|e| StorageError::backend(e.to_string()))?;

        let mut nodes: Vec<ClusterNode> = rows.into_iter().map(Into::into).collect();
        let has_more = nodes.len() > page_size;
        if has_more {
            nodes.truncate(page_size);
        }

        let next_cursor = if has_more {
            let last = nodes
                .last()
                .ok_or_else(|| StorageError::internal("empty page"))?;
            Some(
                ListCursor::new(last.updated_at, last.node_id.as_uuid())
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?
                    .encode()
                    .map_err(|e| StorageError::internal(format!("failed to encode cursor: {e}")))?,
            )
        } else {
            None
        };

        let mut result = Page::new(nodes);
        if let Some(cursor) = next_cursor {
            result = result.with_next_cursor(cursor);
        }
        Ok(result)
    }

    async fn mark_draining(
        &mut self,
        node_id: NodeId,
        instance_id: NodeInstanceId,
        updated_at: UtcTimestamp,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query(
            "UPDATE cluster_nodes SET draining = 1, updated_at = ? WHERE node_id = ? AND instance_id = ?",
        )
        .bind(to_millis(updated_at))
        .bind(node_id.as_uuid())
        .bind(instance_id.as_uuid())
        .execute(&self.write_pool)
        .await
        .map_err(|e| StorageError::backend(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }
}
