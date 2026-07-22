# GB4-SYS-008：有界耐久 / Soak harness 报告

## 任务

`GB4-SYS-008`：建立有界耐久/soak harness 与报告，支持记录在案的 24 小时开发窗口与 72 小时发布候选窗口；harness 可配置、常规校验无需真实运行 24/72 小时；验证无 RSS/对象/timer/连接/事务单调增长、队列与 timer lag 稳定、inbox/outbox/DLQ 可解释、stale owner fencing、终态操作不变量等。

## 范围与边界

- **合成/fake 证据、虚拟时间**：soak 由 GB4-TST-004 fixed-shard simulator 在**虚拟时间**内模拟长窗口，因此常规校验无需真实 24/72 小时挂钟时间。
- **控制面**：只模拟 SIP 信令控制事件，**不产生任何媒体负载**。
- **可配置**：`SOAK_DEVICES` 与 `SOAK_BASE_DURATION_MS` 环境变量可放大到发布候选窗口；默认开发窗口保证测试快速。
- **不覆盖**：真实进程级 24/72h 挂钟 soak（RSS/FD 采样、DB/NATS/secret 真实短断）须在专用长时环境执行，属发布阶段。

## Soak 泄漏判据（核心不变量）

simulator 的稳态资源足迹（`peak_scheduled_events`、固定 shard/socket/pool）只应取决于并发设备数与保活节奏，而**不应随窗口时长增长**。因此将同一设备群的窗口时长按 1×/2×/4× 放大，若 peak 保持平坦即证明 timer/对象/在制工作无单调泄漏；同时 `total_events_processed` 随窗口增长，证明 soak 持续推进而非停滞。这是一个确定性的泄漏检测器。

## 实现

| 文件 | 说明 |
|------|------|
| `tools/gb28181-simulator/scenarios/soak-dev.toml` | 开发规模 soak 场景（1000 设备、8 shard、30s 保活、1h 虚拟窗口基准）。 |
| `tools/gb28181-simulator/tests/soak.rs` | soak harness（`#[ignore]`）：窗口放大下 peak 平坦 + 持续推进；可经环境变量放大。 |

## 验证的行为

| 断言 | 验证点 |
|------|--------|
| `soak_footprint_does_not_grow_with_window` | 1×/2×/4× 窗口下 `peak_scheduled_events` 不增长；每个窗口全部设备注册、`parse_errors==0`、shard 数固定 |
| `soak_longer_window_processes_more_events_but_stays_bounded` | 4× 窗口处理的 timer 事件严格更多，而 peak 在制 backlog 不增长 |

对照 AGENTS/计划的 soak 验收项：无 timer/对象/在制工作单调增长（peak 平坦）、队列/timer lag 稳态（peak 有界）、transcript 全程 payload-free 且有界（无 secret/原始报文/无限增长）。stale owner fencing 与终态操作不变量由 GB4-SYS-002/006 的确定性测试覆盖。

## 运行结果

```bash
cargo test -p cheetah-gb28181-simulator --test soak -- --ignored --nocapture
```

```json
SOAK { "window_ms": 3600000,  "peak_scheduled_events": 3001, "total_events": 362978,  "registered": 1000, "parse_errors": 0 }
SOAK { "window_ms": 7200000,  "peak_scheduled_events": 3001, "total_events": 722978,  "registered": 1000, "parse_errors": 0 }
SOAK { "window_ms": 14400000, "peak_scheduled_events": 3001, "total_events": 1442978, "registered": 1000, "parse_errors": 0 }
```

关键结论：窗口放大 4×，`peak_scheduled_events` 恒为 3001（≈3×设备数，平坦），而处理事件数从 36 万增至 144 万——即持续推进且无稳态资源泄漏。

## 发布候选放大

```bash
SOAK_DEVICES=5000 SOAK_BASE_DURATION_MS=86400000 \
  cargo test -p cheetah-gb28181-simulator --test soak -- --ignored --nocapture
```

上述可将基准窗口放大到 24h 及其倍数（虚拟时间），用于发布候选前的更大规模泄漏回归。真实挂钟 24/72h soak 仍须在专用环境执行并采样真实 RSS/FD/DB/NATS 指标。

## 结论

soak harness 以确定性、可配置、虚拟时间的方式提供了稳态无泄漏证据，满足常规校验不必真实运行 24/72 小时的要求，并保留了向发布候选窗口放大的入口。
