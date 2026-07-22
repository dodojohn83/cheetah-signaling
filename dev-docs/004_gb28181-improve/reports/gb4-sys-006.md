# GB4-SYS-006：确定性 Chaos / Rolling-Upgrade 覆盖报告

## 任务

`GB4-SYS-006`：增加确定性 chaos/rolling-upgrade 覆盖，涵盖节点 drain、owner 迁移、事务超时、服务重启；在合适处复用 GB4-TST-004 的确定性故障 DSL。

## 范围与边界

覆盖分为两个互补部分，分别对应帧级故障与生命周期故障：

1. **Simulator 网络故障（复用 GB4-TST-004 DSL）**：使用现有 fault DSL（`drop`/`delay`/`duplicate`/`sip_error`）注入丢包、慢速保活、重复目录响应与间歇 503，验证确定性、优雅降级/收敛与资源有界。
2. **Cluster 生命周期故障**：使用真实 PostgreSQL 容器，验证节点 drain、owner 迁移、事务超时（lease 到期）、服务重启恢复与有界收敛——这些是帧级 DSL **无法**表达的语义，因此不将生命周期语义错误地叠加到帧级故障变体上。

- **控制面**：simulator 只做 SIP 信令控制事件，不产生媒体负载；cluster 测试使用 fake/in-memory 组件。
- **确定性**：simulator 由 seed 驱动、cluster 由 fake 时钟驱动；无真实 sleep、无固定公共端口。

## 实现

| 文件 | 说明 |
|------|------|
| `tools/gb28181-simulator/scenarios/chaos-cluster.toml` | 复用故障 DSL 的确定性网络 chaos 场景（drop 0.05 / delay 0.3 / duplicate 0.05 / sip_error 503 0.1）。 |
| `tools/gb28181-simulator/tests/chaos.rs` | 网络 chaos 断言：确定性、故障注入、收敛、资源有界。 |
| `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_006_chaos.rs` | 容器化 cluster 生命周期 chaos 测试（drain/迁移/超时/重启）。 |

生命周期测试复用现有 `DrainingMigrationService`、`DeviceAssignmentService` 与 PostgreSQL owner/node 仓储。

## 验证的行为

### Simulator 网络故障（默认随 CI 运行）

| 断言 | 验证点 |
|------|--------|
| `chaos_run_is_deterministic` | 相同 seed/scenario 两次运行 transcript hash、counts、outcomes、fault_counts 完全一致 |
| `chaos_faults_are_injected` | drop/delay/duplicate/sip_error 均实际触发 |
| `chaos_converges_with_bounded_degradation` | 有界重试驱动全部设备最终注册收敛，catalog/invite/bye 完成，`parse_errors == 0` |
| `chaos_resources_stay_bounded` | shard 数固定（无每设备 task），peak scheduled events ≤ 设备数 × 8（无无界 backlog） |

### Cluster 生命周期故障（`#[ignore]` 之外，需 Docker）

| 断言 | 验证点 |
|------|--------|
| 节点 drain + 迁移 | node A 标记 draining 后，设备经 `DrainingMigrationService` 迁移至 node B |
| 有界 drain | `max_devices=1` 时首轮 `truncated=true` 迁移 1 台；续 drain 收敛（`truncated=false`） |
| 事务超时 | owner lease 到期后 assign 强制重新获取，epoch 严格递增（旧 epoch 被 fence） |
| 服务重启 | drop 所有句柄并在同一数据库上重开 `PostgresStorage`，收敛后的 ownership 恢复一致 |

## 运行

```bash
# Simulator 网络 chaos（随 workspace 测试运行）
cargo test -p cheetah-gb28181-simulator --test chaos

# Cluster 生命周期 chaos（需 Docker）
cargo test -p cheetah-gb-system-tests --test gb4_sys_006_chaos
```

均 `passed`。

## 结论

chaos 覆盖在确定性前提下同时验证了帧级网络故障下的收敛/有界性，以及集群 drain/迁移/超时/重启的生命周期正确性，无无限重试、无无界 backlog、无假成功。三节点真实 chaos（NATS/DB 注入、media node restart）仍属后续真实环境范畴。
