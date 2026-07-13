# 08 SQLite/PostgreSQL 存储实现

## 1. 目标

同一领域端口支持 SQLite 边缘单机和 PostgreSQL 集群部署。业务语义必须一致，但允许针对数据库能力使用不同查询实现。数据库名称统一使用 PostgreSQL，不在代码或文档中出现 `postgelsql`。

## 2. Crate 与目录

```text
crates/storage/
  src/{lib.rs,migrate.rs,repository.rs,tx.rs}
  src/sqlite/
  src/postgres/
  migrations/sqlite/
  migrations/postgres/
```

迁移文件只追加，不修改已发布版本。每个迁移具有相同逻辑版本号，并在两个后端分别实现。

## 3. 表与索引

首版至少建立：

- `tenants`
- `devices`
- `device_endpoints`
- `channels`
- `device_capabilities`
- `commands`
- `media_sessions`
- `device_owners`
- `outbox_events`
- `processed_messages`
- `plugin_instances`
- `audit_logs`

所有业务表包含 `tenant_id`。时间以 UTC 微秒或数据库时间戳存储并统一转换。JSON 字段必须有 schema 版本，不能用作核心查询字段的替代。

关键索引：外部设备 ID 唯一索引、在线状态查询索引、命令幂等唯一索引、outbox 未发布部分索引、owner lease 到期索引、媒体会话业务幂等索引。

## 4. 实现任务

### DB-001：迁移框架

- [ ] 启动时校验 schema 版本，生产环境默认只校验不自动升级。
- [ ] CLI 提供 `db migrate`、`db status`、`db validate`。
- [ ] 迁移必须可在空库执行，也可从上一正式版本升级。
- [ ] PostgreSQL 大表迁移采用分阶段兼容策略，禁止长时间独占锁。

### DB-002：SQLite 实现

- [ ] 启用 WAL、foreign_keys、busy_timeout，并配置合理同步级别。
- [ ] 单写者队列串行提交写事务，读连接使用独立池。
- [ ] 检测本地磁盘和文件权限；明确拒绝不受支持的网络文件系统部署。
- [ ] 提供在线备份命令和恢复校验。

### DB-003：PostgreSQL 实现

- [ ] 使用连接池，分别配置获取、执行和事务超时。
- [ ] owner、outbox 等抢占查询使用 `FOR UPDATE SKIP LOCKED`。
- [ ] 事务重试仅针对可识别的序列化冲突/死锁，且必须有次数上限和抖动。
- [ ] 每个连接设置 application name，慢查询可关联节点和 trace。

### DB-004：仓储与事务

- [ ] 实现 `DeviceRepository`、`ChannelRepository`、`CommandRepository`、`MediaSessionRepository`、`OwnerRepository`、`OutboxRepository`。
- [ ] `UnitOfWork` 保证聚合更新与 outbox 写入同一事务。
- [ ] 所有更新带 `revision` 条件，零行更新转换为并发冲突。
- [ ] 列表查询使用游标分页，不使用大 offset。

### DB-005：租户隔离与删除

- [ ] 每个仓储方法必须显式接收 `TenantId`。
- [ ] PostgreSQL 可选启用 RLS，应用层隔离仍保留。
- [ ] 软删除与物理清理分离；清理任务分批执行并可暂停。
- [ ] 审计表只追加，设置单独保留周期。

## 5. 一致性测试

建立同一套 repository contract tests，分别运行 SQLite 和 PostgreSQL：

- [ ] CRUD、并发 revision、唯一约束错误映射。
- [ ] 事务回滚不留下 outbox 孤儿。
- [ ] Unicode、最大长度、空值和时间精度一致。
- [ ] 游标分页不重复、不遗漏。
- [ ] 迁移前后数据兼容。

PostgreSQL 测试通过容器启动真实数据库；不得以 mock 替代 SQL 兼容验证。

## 6. 验收标准

- 切换数据库不改变应用服务和协议驱动代码。
- SQLite 单机崩溃恢复后通过完整性检查；PostgreSQL 节点故障时事务语义不破坏。
- 每条 SQL 有对应索引说明和慢查询基准。
