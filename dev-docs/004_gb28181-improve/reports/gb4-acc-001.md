# GB4-ACC-001 完成报告

- 任务 ID：`GB4-ACC-001`
- 结论：完成
- 日期：2026-07-21
- 分支：`devin/gb4-acc-001`（基于 `origin/devin/gb4-arc-sip`，PR 目标 `main`，stacked on #173）

## 变更摘要

1. 新增领域聚合 `cheetah-domain::protocol_session::ProtocolSession`（`crates/domain/cheetah-domain/src/protocol_session.rs`）：
   - 复用既有 `ProtocolSessionId`（UUIDv7）、`ProtocolIdentity`、`Revision`、`OwnerEpoch`、`UtcTimestamp` 等受校验 newtype，未新增重复身份类型。
   - 字段全部私有，只能通过保持不变量并 bump `Revision` 的方法修改：`refresh_registration`、`record_keepalive`、`mark_offline`、`assign_owner`；只读 getter 暴露状态。
   - 建模内容：`tenant_id`、`device_id`、`protocol`、`ProtocolIdentity`、本地 listener 身份（`LocalIdentity`：listener/local_device_id/domain/realm）、`SipTransport`、端点信息（`SessionEndpoint`：observed_source/contact_uri/advertised_endpoint）、REGISTER 事务（`RegistrationInfo`：call_id/cseq/expires_secs）、`expiry_at`、`last_authenticated_at`、`PresenceState`、`last_keepalive_at`、`offline_reason`、owner node/epoch、`CompatibilityProfile`（profile_id/revision）、created/updated 时间与聚合 `revision`。
   - `refresh_registration` 拒绝 CSeq 回退，keepalive 不改写路由端点；字符串字段有长度上限，`new` 校验非空协议与非 nil 会话 ID。
   - 聚合不依赖 Tokio/SQLx/协议 wire 类型，未改动 `cheetah-gb28181-core`/`cheetah-gb28181-driver-tokio`。
2. 新增 repository port `cheetah-domain::ports::ProtocolSessionRepository`（返回 `DomainError`）：
   - 所有方法显式携带 `TenantId`；`get`/`get_by_device`/`get_by_identity` 按租户过滤。
   - `save` 使用 `WHERE revision = EXCLUDED.revision - 1` 乐观并发，零行更新转换为 `DomainError::ConcurrentModification`。
   - `delete` 带 revision 条件，冲突返回 `ConcurrentModification`，缺失返回 `NotFound`。
   - `list_expired` 用稳定 `ListCursor` 游标分页扫描已过期会话（运维 reaper 用途，跨租户，返回项仍带各自 `tenant_id`）。
3. `cheetah-storage-api::Storage` 新增 `protocol_session_repository()` 工厂方法。
4. 新增 SQLite/PostgreSQL 适配器 `SqliteProtocolSessionRepository`/`PostgresProtocolSessionRepository`，共享同一 port 语义：核心查询字段（tenant/device/protocol/identity/expiry/updated/revision）落列，完整聚合存 `data`（SQLite TEXT / PostgreSQL JSONB）。
5. 新增 append-only 迁移 `migrations/{sqlite,postgres}/0009__protocol_sessions.sql`（逻辑版本一致，后端专用 SQL），含 `(tenant,protocol,device)`、`(tenant,protocol,identity)` 唯一索引与 expiry/updated 索引。
6. 新增共享 contract 测试 `crates/testing/cheetah-storage-tests/src/contract/protocol_session.rs`，由 SQLite 与 PostgreSQL 两个入口经 `run_all` 共同执行；覆盖 create/get/get_by_device/get_by_identity、租户隔离、revision 成功与冲突、revision 条件删除与 NotFound、过期扫描与游标分页；迁移启动兼容性由 `run_all` 中的 `migration().run()` 覆盖。

## 验证

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test -p cheetah-storage-tests --test sqlite       # pass (含 protocol_session contract)
cargo test -p cheetah-storage-tests --test postgres     # pass (testcontainers PostgreSQL)
cargo test --workspace                                  # 见下方“已知无关失败”
python3 scripts/audit_architecture.py                   # 无新增违规
```

`scripts/audit_architecture.py` 的既有告警（`cheetah-media-scheduler`、`cheetah-onvif-driver-tokio` 层级违规、`cheetah-cluster-registry`/`cheetah-signal-contracts` forbidden dep、两处 `panic!`）均不在本任务范围，且不涉及本次改动文件。

## 已知无关失败

- `cargo test --workspace` 中 `cheetah-message-nats` 的一个 doctest 编译失败（`node_id`/`resolver` 未绑定）。该 crate 未被本任务改动，`git diff origin/devin/gb4-arc-sip -- crates/messaging/cheetah-message-nats` 为空，属于基线上已存在问题。

## 未运行项

- `cargo nextest`：当前环境未安装 `cargo-nextest`，改用 `cargo test --workspace` 与针对性 contract 测试覆盖。
- `buf format/lint`、`cargo deny`：本 PR 未修改 `.proto` 与依赖清单，未运行。

## 边界说明

- 本任务仅实现持久化聚合、双库迁移与 repository contract；REGISTER/注销/续期/keepalive/owner acquisition 的事务链路（含 outbox 同事务提交）属于 `GB4-ACC-002`，未在此实现。
- 未改动协议 core/driver 状态机，未处理任何媒体负载。
