# GB4-SYS-007：有界合成容量 / 扩展 harness 报告

## 任务

`GB4-SYS-007`：建立有界合成容量/扩展 harness 与报告，覆盖 10 万 / 30 万 / 100 万在线设备画像，并清晰标注其开发规模等价物；输出容量指标；负载保持合成且无媒体负载，不隐含真实设备互操作。

## 范围与边界

- **合成/fake 证据**：容量证据来自 GB4-TST-004 的 fixed-shard simulator（虚拟时间、有界资源、payload-free）；延迟分位由 `cheetah-perf`（真实 SQLite/PostgreSQL）提供。
- **控制面**：simulator 只模拟 SIP 信令控制事件与 media control，**不产生 RTP/RTCP/PS/TS/ES 负载**；合成 vendor 画像**不构成真实设备互操作证据**。
- **规模映射**：本仓库 CI 邻近环境运行**开发规模**画像；10 万/30 万/100 万目标画像通过同一场景模板参数化放大（`device_count`/`shards`），其绝对容量声明必须绑定硬件、内核、commit、配置与场景文件后单独测量，本报告不做此绝对声明。

## 目标画像与开发规模等价物

| 画像 | device_count | 目的 | 本仓库运行 |
|------|-------------|------|-----------|
| release-100k | 100,000 | 发布容量阶梯第 1 级 | 参数化放大（需专用硬件测量） |
| release-300k | 300,000 | 发布容量阶梯第 2 级 | 参数化放大 |
| release-1m | 1,000,000 | 百万在线目标 | 参数化放大 |
| **capacity-dev** | **5,000** | 开发规模等价，确定性、快速 | ✅ `#[ignore]` 手动运行 |

开发规模场景与目标画像共享同一固定 shard 模型、错峰保活与控制路径步骤，仅规模不同；据此验证“资源随并发设备数线性有界、无每设备 task/timer、无无界队列”的结构性质，而非绝对 TPS。

## 实现

| 文件 | 说明 |
|------|------|
| `tools/gb28181-simulator/scenarios/capacity-dev.toml` | 开发规模容量场景（5000 设备、16 shard、UDP，600s 虚拟时长）。 |
| `tools/gb28181-simulator/tests/capacity.rs` | 容量 harness（`#[ignore]`）：收敛 + 资源有界 + 可复现，并打印指标 JSON。 |
| `crates/perf/cheetah-perf/tests/edge_baseline.rs`、`cluster_scale.rs` | 既有 P50/P95/P99 延迟与真实 DB 负载证据。 |

## 指标矩阵

| 指标 | 来源 | 本报告证据 |
|------|------|-----------|
| register / keepalive TPS | simulator outcomes / 虚拟时长 | ✅ 见下 |
| operation/message throughput | simulator message counts | ✅ |
| P50/P95/P99 延迟 | `cheetah-perf` `Summary` | ✅（perf 场景） |
| queue depth / timer lag | simulator `peak_scheduled_events` | ✅（有界） |
| CPU/RSS/network/FD | 需专用容量环境测量 | ⚠ 目标画像阶段 |
| ownership distribution | cluster 容量场景（放大阶段） | ⚠ 目标画像阶段 |
| DB/NATS load | 容器化容量阶段测量 | ⚠ 目标画像阶段 |
| reject/drop/dedupe rate | simulator fault counts | ✅ |
| recovery time | 见 GB4-SYS-006 chaos | ✅ |

## 开发规模运行结果

```bash
cargo test -p cheetah-gb28181-simulator --test capacity -- --ignored --nocapture
```

```json
CAPACITY {
  "scenario": "capacity-dev",
  "devices": 5000,
  "virtual_duration_s": 600.0,
  "register_tps": 8.33,
  "keepalive_tps": 66.61,
  "message_tps": 241.54,
  "shards": 16,
  "udp_sockets": 16,
  "tcp_pool": 256,
  "peak_scheduled_events": 15003,
  "total_events_processed": 189893,
  "drops": 0,
  "duplicates": 0,
  "parse_errors": 0
}
```

关键结论：5000 设备全部收敛注册；shard/socket/pool 计数固定；`peak_scheduled_events`（15003 ≈ 3×设备数）随并发设备数线性有界，证明无每设备 task/timer 爆炸、无无界 backlog；运行可复现（transcript hash 稳定）。

## 结论

容量 harness 在开发规模上确定性验证了结构性有界与收敛性质，并为 10 万/30 万/100 万目标画像提供了参数化放大入口。绝对容量与横向扩展的权威声明须在绑定硬件/配置的专用环境中测量，且明确区分于合成证据。
