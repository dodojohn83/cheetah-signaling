# GB4-SYS-002：Cluster PostgreSQL/NATS + fake media 全 GB28181 纵向与接管测试报告

## 任务

`GB4-SYS-002`：构建集群系统测试，使用可销毁的 PostgreSQL 与 NATS 容器（`testcontainers-modules`）和 fake 媒体，覆盖 GB28181 接入/事件、协议会话持久化、NATS 命令路由、owner 获取/接管/epoch fencing、媒体协商与会话生命周期，以及 ownership 迁移后的恢复。

## 范围与边界

- **容器支撑证据**：PostgreSQL 与 NATS 均为一次性 testcontainers 容器，使用动态宿主端口，**不使用开发者数据库或公共基础设施**。
- **fake 媒体 + 控制面**：媒体经 `InMemoryMediaPort` 与 SIP/SDP 控制消息完成，**不涉及任何 RTP/RTCP/PS/TS/ES 负载**。
- **确定性**：确定性时钟/ID；owner lease 到期通过推进 fake 时间触发，不依赖真实 sleep。
- **不覆盖**：真实 media node 与真实平台互操作（属 `GB4-SYS-003/004`）。

## 实现

| 文件 | 说明 |
|------|------|
| `crates/testing/cheetah-gb-system-tests/tests/gb4_sys_002_cluster.rs` | 集群纵向 + 接管系统测试。 |

组件：两个 owner lease service（node A/B）、两条 `NatsBus`、共享的 PostgreSQL owner resolver、application `DeviceService`/`OperationService`/`MediaService`、`InMemoryMediaPort` 与 `ProtocolSessionLink`。

## 验证的行为

| 环节 | 验证点 |
|------|--------|
| 接入 | digest REGISTER/keepalive 成功 |
| 协议会话持久化 | GB28181 协议会话写入 PostgreSQL |
| owner 获取 | node A 以 `OwnerEpoch(1)` 获取 owner |
| NATS 命令路由 | PTZ 命令仅路由到当前 owner（node A） |
| owner 接管 | fake 时间推进使 lease 到期后，node B 以 `OwnerEpoch(2)` 接管 |
| epoch fencing | node A 以旧 epoch 的协议会话操作返回 `SessionLinkError::StaleOwner` |
| 接管后路由 | 后续命令仅被 node B 的订阅接收，旧 owner 不再收到 |
| 媒体 | fake INVITE/200/ACK/BYE 协商与持久化媒体会话生命周期 |
| 恢复 | ownership 迁移后持久化的 owner/会话/媒体状态一致 |

## 运行

```bash
# 需要本机可用 Docker（testcontainers 拉起一次性 PostgreSQL / NATS）
cargo test -p cheetah-gb-system-tests --test gb4_sys_002_cluster
```

结果：`1 passed`（动态端口，容器自动销毁）。

## 结论

集群纵向路径在容器化 PostgreSQL/NATS + fake 媒体下端到端通过，并证明 owner 接管、epoch fencing 与命令路由在真实存储/消息后端上的正确性。
