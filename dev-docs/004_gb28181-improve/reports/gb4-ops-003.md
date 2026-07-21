# GB4-OPS-003 / GB4-OPS-004 完成报告

- 任务 ID：`GB4-OPS-003`、`GB4-OPS-004`
- 结论：完成
- 日期：2026-07-21
- 分支：`devin/gb4-ops-003`（基于 `origin/devin/gb4-ops-001` / PR #179，目标分支 `main`，stacked）
- 仓库：`dodojohn83/cheetah-signaling`

## GB4-OPS-003：准入、优先级、coalescing、dead-letter 与 backlog recovery

### 有界准入原语（`cheetah-signal-types::admission`）

新增一组纯粹、确定性、无 I/O 且状态有界的准入原语，作为运行时与应用层的唯一权威实现：

- `TrafficClass`（`command`/`catalog`/`keepalive`/`position`/`alarm`/`location`/`other`，固定枚举，提供稳定 `as_str()` label、`priority()` 与 `is_coalescible()`）。
- `Priority`（`low`/`normal`/`high`）。
- `TokenBucket` / `TokenBucketConfig`：外部单调毫秒时钟驱动、饱和运算、milli-token 精度、时间回退按零处理。
- `KeyedRateLimiter<K>`：每 key 一个 token bucket，key 数量有上限，超限按 LRU 淘汰并计数。
- `Coalescer<K>`：同一 key 已 pending 时折叠后续等价事件；tracked key 有上限；超限的新 key 仍然放行（保工作而非丢工作）。
- `DeadLetterQueue<T>` + `DeadLetterEntry<T>` + `DeadLetterReason`（`rate_limited`/`overloaded`/`priority_shed`/`redrive_exhausted`）：有界 FIFO，满时丢弃最旧项，`drain(max)` 有界批量取出用于 redrive。
- `BacklogController` + `BacklogState`：high/low watermark 带滞回（hysteresis），`shed_low_priority()` 报告是否应在过载时丢弃低优先级工作，记录 overload/recovery 转换计数。

### 运行时集成（`cheetah-runtime-tokio`）

- 新增 `admission_policy` 模块，将上述原语组合为线程安全的 `AdmissionPolicy`（内部 `std::sync::Mutex`，临界区不跨 `.await`）。`AdmissionTicket { source_id, class, device_key }` 描述准入请求，`AdmissionOutcome` 描述结果。
- `AdmissionController::admit()` 执行完整策略：按聚合 mailbox 深度更新 backlog → 过载时对低优先级 `Priority::Low` 流量削减（shed）→ 按 `(source_id, class)` 令牌桶做 per-source/per-method 限流 → 对 `keepalive`/`position` 做 coalescing → 命中的正常流量入分片 mailbox；mailbox 满、限流、削减的消息落入有界 dead-letter 队列。
- `AdmissionController::redrive(max)` 仅在 backlog 恢复到 low watermark 以下后，有界批量把 dead-letter 消息重新投递回分片 mailbox；重投失败按尝试次数上限（`MAX_REDRIVE_ATTEMPTS = 8`）退避或最终丢弃，实现 backlog recovery。
- `AdmissionController::release_coalescible()` 在等价事件处理完成后释放 coalescing key。
- `RuntimeConfig` 新增 `AdmissionPolicyConfig`，所有界限（令牌桶容量/速率、key 上限、coalescer 上限、dead-letter 容量、backlog high/low watermark）均 `> 0` 且 `low <= high`，由 `validate()` 校验。
- `RuntimeMetrics` 新增聚合、无高基数 label 的计数器/gauge：`messages_rate_limited`、`messages_coalesced`、`messages_shed`、`messages_dead_lettered`、`messages_redriven`、`backlog_overload_transitions`，并在快照中导出。

### 应用集成（`cheetah-signal-application`）

- 新增 `admission_control` 模块，提供可注入、状态有界的 `TenantIngressAdmission<P>`：按 `(TenantId, TrafficClass)` 限流、按 `(TenantId, DeviceId, TrafficClass)` coalescing、并携带有界 dead-letter 队列与 redrive 取出接口，供协议/传输适配器在入站事件转化为 `Operation`/命令/仓储写入之前进行租户级准入。
- 复用同一套 `cheetah-signal-types::admission` 原语，避免概念重复实现。

## GB4-OPS-004：有界 startup/shutdown/drain 与 crash recovery 系统测试

### `Runtime::drain()`

- 新增公共 `Runtime::drain(deadline)`：先置 `draining` 标志（`AtomicBool`）停止接收新工作，再有界轮询聚合分片 mailbox 深度直到清空或超过 deadline，最后执行既有 `shutdown()`。返回 `DrainOutcome { drained_within_deadline, remaining_backlog }`。
- drain 后 `send_message()` 与 `admit()` 均以稳定错误 `RuntimeError::Draining` 拒绝新工作（`error.rs` 新增该变体）。
- drain 幂等：重复调用观察到空 backlog 后继续走 shutdown；`shutdown()` 对已取出的 join handle 安全（幂等）。

### crash recovery 系统测试

`crates/application/cheetah-signal-application/tests/lifecycle_recovery_system.rs` 使用真实 SQLite、进程内命令/事件总线与 in-memory fake media，覆盖：

1. **启动顺序**：按 `schema → bus → repository → ownership → media → services` 装配，node A 获取 lease（epoch 1）后成功派发命令，命令经总线交付（ready）。
2. **优雅关停 / drain**：关停前 `OutboxRelay` 把 pending outbox 事件排空，断言 outbox 已清空。
3. **crash recovery**：模拟 node A 在 commit outbox 事件后、发布前崩溃；node B 以全新缓存重启，重新获取 lease 使 owner epoch 原子递增到 2；携带旧 epoch（1）的命令在派发前被 fence 为 `STALE_OWNER`（`OperationStatus::Failed`）且不触达总线；崩溃前落库的 outbox 事件由恢复后的 relay 重放到事件总线。

## 校验结果

在 `dodojohn83/cheetah-signaling` 工作树运行：

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo nextest run --workspace`：通过（含新增 `cheetah-signal-types::admission` 单测、`admission_policy` 单测、运行时 `admission_test`、应用 `admission_control` 单测与 `lifecycle_recovery_system` 系统测试）。
- `python3 scripts/audit_architecture.py`：通过。

## 局限与后续

- 应用层 `TenantIngressAdmission` 作为可复用服务提供，尚未在具体协议/传输适配器中接线（由后续接入任务在装配层注入）。
- 运行时 redrive 与 backlog 观测依赖调用方（或后台巡检）周期性调用 `redrive()`；本任务未引入常驻 redrive task，以遵循「固定分片 worker、不新增每设备/每资源常驻 task」的约束。
- crash recovery 测试使用 `InMemoryClock`（冻结墙钟），因此 owner resolver 缓存的过期依赖「崩溃后新节点使用全新缓存」建模，而非 TTL 到期。
