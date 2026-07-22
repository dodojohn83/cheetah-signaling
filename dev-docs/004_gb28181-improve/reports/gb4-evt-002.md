# GB4-EVT-002: GB28181 事件优先级、合并、队列满和死信运维策略

## 目标

为 `cheetah-signaling` 的 GB28181 应用事件入站通道实现分类、合并、过载丢弃与死信重投策略，确保高优先级事件（命令响应、终端媒体事件、告警）在任何负载下都不被静默丢弃。

## 变更摘要

- 新增 `apps/cheetah-signaling/src/gb_event_admission.rs`
  - `GbApplicationEventSink`：在 `emit` 时完成事件分类、合并观测、背压检测和死信入队。
  - `spawn(...)`：保持原签名；启动后台 worker，使用有界 `tokio::sync::mpsc` 通道和 `DeadLetterQueue`。
- 拆分 `apps/cheetah-signaling/src/gb_event_sink.rs`
  - 原文件超过 800 行，按职责拆分为：
    - `gb_event_admission.rs`：准入控制与 worker 循环。
    - `gb_event_processing.rs`：事件到应用层调用的映射、媒体会话状态机、`build_context`、`event_source`、`control_outcome`。
    - `gb_event_sink.rs`：设备在线/离线、目录替换、bootstrap 查询、通用 outbox 辅助函数。
- 扩展 `cheetah_http_api::metrics::RequestMetrics`
  - `gb28181_events_admitted_total`
  - `gb28181_events_coalesced_total`
  - `gb28181_events_shed_total`
  - `gb28181_events_dead_lettered_total`
  - `gb28181_events_redriven_total`
  - `gb28181_events_redrive_exhausted_total`
  - 保留 `gb28181_events_dropped_total` 用于最终无法处理的场景。

## 事件分类

| `Gb28181Event` | `TrafficClass` | `Priority` | 可合并 |
| --- | --- | --- | --- |
| `DeviceControlResponseReceived` | `Command` | `High` | 否 |
| `MediaSessionStarted` / `Stopped` / `Failed` | `Command` | `High` | 否 |
| `CascadePlayRequested` / `Stopped` | `Command` | `High` | 否 |
| `CatalogReceived` | `Catalog` | `Normal` | 否 |
| `AlarmReceived` | `Alarm` | `Normal` | 否 |
| `RecordInfoReceived` / `DeviceInfoReceived` / `DeviceStatusReceived` / `ConfigReceived` / `PresetInfoReceived` / `Presence` / 其他 | `Other` | `Normal` | 否 |
| `Keepalive` | `Keepalive` | `Low` | 是，key：`tenant:device:Keepalive` |
| `MobilePositionReceived` | `Position` | `Low` | 是，key：`tenant:device:Position` |

合并键仅对 `Keepalive` 和 `MobilePositionReceived` 构造，使用 `tenant:device:<TrafficClass.as_str()>` 形式。`Coalescer` 在 worker 开始处理某 key 后标记该 key 为 pending，处理完成（无论成功与否）后 `release`，从而保证已合并事件在“最新一次”被处理完之前不会继续合并。

## 背压与丢弃策略

- `BacklogController` 以 `queue_depth * 4 / 5` 为高水位，`high / 2` 为低水位，带迟滞。
- 低水位线以下：正常接收。
- 高水位线以上：仅丢弃 `Priority::Low` 的 `Keepalive` / `Position` 事件，并记录 `gb28181_events_shed_total` + `gb28181_events_dropped_total`。
- `Command` / `Alarm` / `Catalog` / `Other` 等中高优先级事件即使在过载时也不被直接丢弃。

## 死信队列与重投

- 当高/普通优先级事件因 bounded channel 满而无法 `try_send` 时，写入 `DeadLetterQueue`。
- `DeadLetterQueue` 容量为 `max(queue_depth * 2, 256)`，有界。
- Worker 每 100ms 触发一次 redrive，每次最多尝试 `REDRIVE_BATCH_SIZE`（64）条。
- 每条事件最多重投 5 次；超过后记录 `gb28181_events_redrive_exhausted_total` + `gb28181_events_dropped_total` 并释放合并键。
- Worker 每次成功处理一个事件后也会立即 redrive，以便在 channel 空闲时尽快补回死信。

## 指标记录

- `admit` 成功：`gb28181_events_admitted_total`
- 被 `Coalescer` 合并：`gb28181_events_coalesced_total`
- 低优先级被过载丢弃：`gb28181_events_shed_total` + `gb28181_events_dropped_total`
- 高/普通优先级进入死信：`gb28181_events_dead_lettered_total`
- 从死信成功 redrive：`gb28181_events_redriven_total`
- 重投预算耗尽：`gb28181_events_redrive_exhausted_total` + `gb28181_events_dropped_total`

## 边界约束

- 未使用 `unbounded_channel`。
- `std::sync::Mutex` 用于 `AdmissionState`，且锁内不跨越 `.await`。
- 所有缓存/队列均有可配置上限：`mpsc::channel(queue_depth)`、`DeadLetterQueue(max(queue_depth * 2, 256))`、`Coalescer(max(queue_depth, 256))`。
- `spawn(...)` 签名保持与原实现一致。

## 测试与质量门禁

- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`：通过
- `cargo test --workspace --lib --bins --tests`：通过，包含 `cheetah_signal_types::admission` 下的 `coalescer_collapses_pending_and_releases`、`backlog_controller_has_hysteresis`、`dead_letter_queue_is_bounded_and_redrivable` 等用例

## 风险与后续工作

- 当前 `BacklogController` 以 channel 占用数（`pending`）近似真实积压。后续若引入 channel 内部 `len()` API 可更精确。
- `DeadLetterQueue` 在 `push` 时若队列满会挤出最旧条目，已被记录为 `dropped_total`。
- `emit` 使用同步 `Mutex`；若 `emit` 调用频率极高，可考虑在 sink 处增加无锁计数，但当前 `Coalescer` 需要稳定 key 跟踪，仍需要加锁。
