# GB4-OPS-001 / GB4-OPS-002 完成报告

- 任务 ID：`GB4-OPS-001`、`GB4-OPS-002`
- 结论：完成
- 日期：2026-07-21
- 分支：`devin/gb4-ops-001`（基于 `origin/devin/gb4-arc-sip` / PR #173，目标分支 `main`）
- 仓库 commit：`dodojohn83/cheetah-signaling`（当前 PR HEAD）

## GB4-OPS-001：GB28181 运行时与应用指标

### 有界枚举（`cheetah-signal-types`）

新增 `gb_metrics` 模块，定义固定、有界的指标类别，确保 tenant/device/session ID 不会成为 Prometheus label：

- `GbCommandMethod`（`ptz`/`device_control`/`device_config`/`query`/`record_info`/`broadcast`/`other`）
- `GbCommandOutcome`（`dispatched`/`succeeded`/`failed`/`unknown`/`cancelled`）
- `GbDevicePresence`（`online`/`offline`）
- `GbMediaSessionState`（`pending`/`active`/`stopping`/`terminated`）

每个枚举提供 `ALL` 固定数组、`as_str()` 稳定 label 值与 `index()` 稠密索引（用于固定长度原子数组）。同时定义 `GbMetricsRecorder` port 与 `NoopGbMetricsRecorder`，使应用层记录指标时无需依赖具体运行时实现，且只能传入有界类别，永不接收标识符。

### `GbMetrics`（`cheetah-runtime-tokio`）

实现聚合器 `GbMetrics`，同时实现 `MetricsExporter`、`GbMetricsRecorder` 与 `RuntimeHealthSource`。暴露以下九个指标族（序列基数由分片数与固定枚举界定）：

| 指标 | 类型 | Label |
| --- | --- | --- |
| `gb28181_shard_mailbox_depth` | gauge | `shard`（受配置分片数限制）|
| `gb28181_active_actors` | gauge | 无 |
| `gb28181_timer_lag_seconds` | gauge | 无 |
| `gb28181_active_operations` | gauge | 无 |
| `gb28181_device_total` | gauge | `presence`（固定枚举）|
| `gb28181_command_total` | counter | `method`、`outcome`（固定枚举）|
| `gb28181_catalog_fragment_total` | counter | 无 |
| `gb28181_media_session_total` | gauge | `state`（固定枚举）|
| `gb28181_cascade_link_total` | gauge | 无 |

超出配置分片数的 mailbox 深度样本会被忽略，序列数保持有界。

### 运行时数据接入

- `RuntimeMetrics` 新增 `timer_lag_ms` 采样 gauge，`timer_wheel` 在每个 tick 记录超出配置 tick 周期的滞后；
- `admission` 新增 `shard_mailbox_depths()`，由有界 Tokio channel 的 `max_capacity - capacity` 推导占用；
- `Runtime::sample_gb_metrics()` 将运行时快照与分片深度注入 `GbMetrics`。

### 应用侧记录

`apps/cheetah-signaling/src/gb_event_sink.rs` 在处理 GB28181 事件时记录真实指标：

- `CatalogReceived` → `record_catalog_fragment()`；
- `DeviceControlResponseReceived` → `record_command(DeviceControl, outcome)`，outcome 由 `result` 字符串映射为有界结果。

其余 gauge（device presence / media session / cascade / active operations）通过 `GbMetricsRecorder` 暴露 setter，供后续在对应处理路径激活时按有界类别写入；本 PR 未伪造其数值。

## GB4-OPS-002：运行时健康与就绪

### `RuntimeHealth`（`cheetah-runtime-tokio::health`）

- `HealthThresholds`：mailbox 容量、降级 mailbox 占用比例、降级/不可用 timer lag 阈值；
- `HealthReason` 固定枚举：`MailboxPressure`/`MailboxSaturated`/`TimerLag`/`TimerLagCritical`；
- `RuntimeHealth` 快照：`ready`、`degraded`、`reasons`、`max_shard_mailbox_depth`、`active_actors`、`timer_lag_ms`；
- 评估逻辑：mailbox 满 → 不就绪；超降级比例 → 降级；timer lag 达临界 → 不就绪；超降级阈值 → 降级。

`RuntimeHealthSource` port 供 HTTP 层消费。健康输出只含有界计数，不枚举 device/tenant/session 或 secret。

### HTTP 端点（`cheetah-http-api`）

- `ApiState` 新增 `gb_metrics`（`MetricsExporter`）与 `runtime_health`（`RuntimeHealthSource`）可选字段及构造器方法；
- `/metrics` 追加 `gb_metrics` 的 Prometheus 文本；
- 新增 `/healthz`（liveness 别名）与 `/readyz`：`/readyz` 先执行既有依赖检查（draining / migration / required media node），再消费运行时健康：mailbox 饱和或 timer lag 临界 → `503 not_ready`，较低压力 → `200` 但携带 `degraded=true` 与有界 reasons；
- 保留既有 `/health/live` 与 `/health/ready` 兼容路由。

依赖检查逻辑抽取为共享的 `dependency_check`，`/health/ready` 与 `/readyz` 复用，避免重复实现。

### 架构方向

`cheetah-http-api`（第 2 层）依赖 `cheetah-runtime-tokio`（第 5 层）为向下依赖，`scripts/audit_architecture.py` 未新增违规。

## 验证

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test --workspace                                  # pass（唯一失败为既有 cheetah-message-nats 文档测试，已在 base 复现）
python3 scripts/audit_architecture.py                   # 无新增违规（剩余项均为既有、与本任务无关）
cargo deny check                                        # advisories/bans/licenses/sources ok
```

## 未运行项

- `cargo nextest`：当前环境未安装 `cargo-nextest`；已用 `cargo test --workspace` 覆盖。
- `buf format/lint`：`buf` 未安装；本 PR 未修改 `.proto`。

## 边界说明

- 未修改 `cheetah-gb28181-driver-tokio`。
- 未引入任何无界 channel、队列或高基数 label；所有 label 均来自固定枚举或受分片数限制。
- 健康响应不泄漏标识符或 secret。
- 未改变信令进程对媒体 payload 的处理边界。
