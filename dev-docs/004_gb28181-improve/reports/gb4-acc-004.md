# GB4-ACC-004 完成报告

- 任务 ID：`GB4-ACC-004`
- 结论：完成
- 日期：2026-07-22
- 分支：`devin/gb4-acc-004`（基于 `origin/devin/gb4-base-all-v2`；PR 目标 `main`）

## 目标

在 GB28181 设备上线（成功 REGISTER）后自动触发 `Catalog`、`DeviceInfo`、`DeviceStatus` 三个查询 Operation；使用已存在的 `CommandPayload::Query` 和 `OperationService`，保持六层架构、租户隔离、owner epoch 正确与稳定幂等键。

## 变更摘要

1. `cheetah-gb28181-module` 新增 `bootstrap` 模块（`crates/protocols/cheetah-gb28181-module/src/bootstrap.rs`）：
   - 提供 `bootstrap_query_kinds()` 返回 `[Catalog, DeviceInfo, DeviceStatus]`；
   - 提供 `bootstrap_query_payload(kind: QueryKind) -> CommandPayload`，构造带默认字段的 `CommandPayload::Query`；
   - 提供 `bootstrap_idempotency_key(tenant_id, device_id, owner_epoch, registration_sequence, kind) -> String`，键格式为 `gb28181:bootstrap:{tenant}:{device}:{owner_epoch}:{registration_sequence}:{kind}`，租户/设备/owner epoch/注册会话/查询类型共同决定唯一性。

2. `cheetah-domain` 为 `QueryKind` 增加 `as_str()` 稳定方法（`crates/domain/cheetah-domain/src/command.rs`），保证键字符串与 serde snake_case 一致，避免依赖 `Debug` 输出。

3. `cheetah-gb28181-module` 为 `DeviceRegistered` 事件增加 `registration_sequence`（`crates/protocols/cheetah-gb28181-module/src/events.rs`），并在 `RegistrationTable::upsert` 与 `Gb28181Access::register_accepted` 中分配/传递：
   - 新注册或从 offline 恢复时分配新的递增序列；
   - 普通续期保持原序列，避免刷新时误生成新 Operation；
   - 为下游 bootstrap 提供稳定的“protocol session generation”。

4. `apps/cheetah-signaling/src/gb_event_sink.rs` 在 `DeviceRegistered` 处理流程中，于 `DeviceService` 确保设备 online 后，调用 `OperationService::submit_operation` 提交三个查询 Operation：
   - 通过 `DeviceOwnerResolver` 解析当前 `owner_epoch`，回退到 `OwnerEpoch(0)`；
   - 使用 `ResourceRef { tenant_id, kind: Device, id: DeviceId }` 作为 target；
   - 每个查询使用独立 UOW 提交，`SubmitOperationRequest` 包含 30 秒 deadline、稳定 idempotency key 与 `expected_owner_epoch`；
   - `ensure_online` 返回 `Option<DeviceId>`，只有在成功定位或创建设备后才提交 bootstrap，刷新时不重复 mark online。

5. 架构：
   - 模块层（层 4）只负责生成 `CommandPayload` 与 idempotency key，不直接访问数据库/NATS；
   - 应用装配层（`apps/cheetah-signaling`）持有 `OperationService` 并提交 Operation，符合层 1/2 → 层 3 的向下依赖；
   - 未新增 `cheetah-gb28181-module -> cheetah-signal-application` 依赖，通过 `scripts/audit_architecture.py` 架构审计。

## 测试

- 单元测试（`crates/protocols/cheetah-gb28181-module/src/bootstrap.rs`，2 项）：
  - 三个 `bootstrap_query_payload` 与对应 `QueryKind` 匹配；
  - `bootstrap_idempotency_key` 对相同输入稳定，且随租户、设备、owner epoch、注册序列、查询类型变化而隔离。

- 既有集成测试全部通过：
  - `register_tests.rs`（40 项）：`DeviceRegistered` 事件模式兼容性；
  - `session_link.rs`（16 项）、`access_ingress.rs`（11 项）、`architecture.rs`（2 项）等未受影响。

## 验证

```text
cargo fmt --all -- --check                                    # pass
cargo clippy --workspace --all-targets -- -D warnings         # pass
cargo test --workspace --lib --bins                          # pass
cargo test -p cheetah-gb28181-module                          # pass
python3 scripts/audit_architecture.py                         # pass（no new violations）
```

`scripts/audit_architecture.py` 的基线既有告警（`cheetah-media-scheduler`、`cheetah-onvif-driver-tokio` 等）不涉及本次改动。

## 未运行项

- `cargo test --workspace` 未完整运行，因为基线 `cheetah-message-nats` doctest 存在无关编译失败（见 `gb4-acc-002.md` / `gb4-acc-003.md` 已知无关失败）。`--lib --bins` 与 `-p cheetah-gb28181-module` 已覆盖本任务相关代码。

## 边界说明

- 本任务实现 REGISTER 成功后的自动 bootstrap Operation 生成；后续 `GB4-ACC-005` 负责目录聚合与 channel 映射，`GB4-CMD-002` 负责把 Operation 路由到 GB client transaction 并移除 plugin placeholder。
- 当前 `Gb28181Access` 仍使用 in-memory `RegistrationTable`；`registration_sequence` 在该状态机内单调递增，足以支撑 idempotency key 在同进程/同分片场景下稳定。若未来接入 `ProtocolSessionLink` 持久化会话，序列可由 `ProtocolSession` 的 generation 替代。
