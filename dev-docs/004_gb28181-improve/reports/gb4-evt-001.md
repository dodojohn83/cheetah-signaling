# GB4-EVT-001 完成报告

- 任务 ID：`GB4-EVT-001`
- 结论：完成
- 日期：2026-07-22
- 分支：`devin/gb4-evt-001`（基于 `origin/devin/gb4-base-all-v2`；PR 目标 `main`）

## 目标

将全部 GB28181 driver 事件接入 application handler，通过 repository/inbox/outbox 持久化或发布，删除 `gb_event_sink.rs` 中仅打印日志的 placeholder 分支。

## 变更摘要

1. `cheetah-domain` 新增 `DomainEvent::Gb28181EventReceived`（`crates/domain/cheetah-domain/src/event.rs`）：
   - 统一的 GB28181 协议事件 envelope，携带 `tenant_id`、`device_id`、`event_type`、`protocol`、`external_id`、payload 键值对；
   - payload 使用 `BTreeMap<String, String>`，复杂数组以 JSON 字符串形式保存；
   - 下游 consumer 可通过 `event_type` 与 payload 解析具体 GB28181 事件。

2. `apps/cheetah-signaling/src/gb_event_sink.rs` 替换全部 logging-only 分支：
   - `AlarmReceived`：解析 `sn`、`priority`、`method`、`alarm_type`、`time`、`info` 为 payload，解析 device 后写入设备聚合对应的 outbox；
   - `MobilePositionReceived`：解析 `sn`、`time`、`longitude`、`latitude`、`speed`、`direction`、`altitude` 为 payload，写入 outbox；
   - `DeviceControlResponseReceived`：记录 command 指标后，将 `sn`/`result` 写入 outbox；
   - `RecordInfoReceived`：聚合完成后将 `sn`、`sum_num`、`num`、`record_count` 以及序列化后的 `records` JSON 数组写入 outbox；
   - `MediaSessionStarted`/`MediaSessionStopped`/`MediaSessionFailed`：先更新对应 `MediaSession` 状态，再写入 outbox；
   - `CascadePlatformConnected`/`CascadePlatformDisconnected`/`CascadePlayRequested`/`CascadePlayStopped`：将级联事件字段写入 outbox。

3. `apps/cheetah-signaling/src/gb_event_sink.rs` 新增 `MediaSessionTransition` 与 `handle_media_session_event`：
   - 根据当前 `MediaSessionState` 合法推进到 `Active` 或 `Stopped`/`Failed`；
   - 每次状态迁移后立刻捕获当时的 `session.revision().0`，确保多个 `MediaSessionStateChanged` 事件拥有单调递增的 `aggregate_sequence`；
   - `Gb28181EventReceived` 信封与媒体状态变更事件在同一 UnitOfWork 中提交，避免状态持久化与事件丢失/重复之间的不一致。

4. `apps/cheetah-signaling/src/gb_event_sink.rs` 新增通用 `build_gb_event`/`append_gb_event`：
   - 能识别内部 `DeviceId` 时将事件挂到设备聚合；否则挂到合成 `ResourceKind::Event` 聚合，确保所有 GB28181 事件进入 outbox；
   - 使用 `Event` 手动构造，保留 `correlation_id`、`causation_id`、trace context 与 source node；
   - `build_gb_event` 复用于 `handle_media_session_event`，保持信封构造逻辑单一。

5. `process_event` 改为返回 `Result<(), SignalError>`，spawn 循环在出错时统一记录 warning。

## 架构

- 事件映射仍位于 `apps` 层（层 1），`cheetah-gb28181-module` 只负责业务到 `Gb28181Event` 的映射；
- `cheetah-domain` 新增的是通用 `DomainEvent` 变体，不引入 GB 协议细节；
- 媒体会话状态变更复用 `MediaSession` 聚合方法，不直接持久化原始 SDP/SSRC；
- 聚合修改与 outbox 在同一事务，遵循“持久化聚合 + outbox”原则。

## 测试

- 单元/集成测试：
  - `cargo test --workspace --lib --tests` 全绿；
  - `cheetah-domain` 媒体会话状态机测试未受影响；
  - `cheetah-signal-application` 设备/媒体/操作集成测试未受影响。
- 未新增针对 `Gb28181EventReceived` 的独立单元测试，因为当前 `gb_event_sink` 依赖 `ApiState` 与 storage，集成测试路径在现有注册/目录测试中间接覆盖；后续可补充针对 outbox payload 的 fixture 测试。

## 验证

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test --workspace --lib --tests                    # pass（跳过 cheetah-message-nats 预存 doctest 失败）
python3 scripts/verify_gb4_fixtures.py                  # pass
cargo deny check                                        # pass
python3 scripts/audit_architecture.py                   # pass（无新增违规）
```

`scripts/audit_architecture.py` 的基线既有告警（`cheetah-media-scheduler`、`cheetah-onvif-driver-tokio`、`cheetah-cluster-registry`、`cheetah-signal-contracts`）不涉及本次改动。

## 已知后续

- GB28181 事件去重（基于协议稳定字段 + transaction key 的 inbox/processed-message 语义）和 queue full 策略由 `GB4-EVT-002` 负责；当前实现已满足“所有事件进入 application handler/outbox”的最小闭环。
- 非媒体类 `Gb28181EventReceived` 使用 `aggregate_sequence: 0`，因为它们是通知信封而非聚合状态流的一部分；消费者应按 `occurred_at` 排序。
