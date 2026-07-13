# 18 集群、高可用与状态对账

## 1. 目标

实现无共享内存假设的多节点控制面。节点可独立故障、滚动升级和重新加入；设备连接、命令和媒体会话通过租约 epoch、幂等消息及周期对账收敛。

## 2. 节点模型与健康

`ClusterNode` 包含 `node_id`、`instance_id`、`zone`、`version`、`contract_versions`、`started_at`、`lease_until`、`capacity`、`load`、`draining`。

- liveness 只表示进程事件循环可运行。
- readiness 要求配置有效、数据库可写、所有权续租正常、关键监听器就绪。
- NATS 故障可按部署策略降低 readiness，但不得让健康端点阻塞。

## 3. 实现任务

### HA-001：节点租约

- [ ] 节点启动生成唯一 instance ID，注册固定 node ID。
- [ ] 心跳批量更新 load 和 lease，使用数据库时间避免主机漂移。
- [ ] 相同 node ID 新实例加入时旧 instance 被 fencing。
- [ ] 节点退出先标记 draining，再释放设备 owner。

### HA-002：设备分配

- [ ] 使用稳定哈希生成候选节点集，再考虑健康、zone、协议监听能力和负载。
- [ ] 已在线设备优先保持 owner，避免无收益迁移。
- [ ] 每轮迁移设置全局和每节点速率上限。
- [ ] owner 切换先增加 epoch，再通知新旧节点执行接管/清理。

### HA-003：故障接管

- [ ] 设备主动重连可立即在新节点竞争 owner。
- [ ] 无连接的长事务在 lease 过期后由新 owner 恢复或失败。
- [ ] SIP/ONVIF 网络会话不能透明迁移，按协议重建并向领域层报告间隙。
- [ ] 旧 owner 恢复后读取到高 epoch，必须丢弃本地 session。

### HA-004：Reconciler

分别实现：

- `OwnerReconciler`：数据库 owner 与本地 session。
- `CommandReconciler`：非终态命令与 deadline/owner。
- `MediaSessionReconciler`：数据库与媒体节点实际资源。
- `SubscriptionReconciler`：ONVIF subscription 与 owner。
- `OutboxReconciler`：长期未发布或卡死记录。

每个 reconciler 必须分页、限速、可取消、可重复执行。处理项使用 CAS/revision，不能用全表锁。

### HA-005：滚动升级

- [ ] 节点公布二进制版本和契约版本。
- [ ] 新旧相邻版本支持共同数据库 schema 和消息 envelope。
- [ ] drain 停止新 owner 分配，等待或转移现有工作。
- [ ] 超出兼容矩阵的节点拒绝加入并给出明确原因。

## 4. 故障注入矩阵

- [ ] 强杀 owner 节点、数据库主库切换、NATS 重启、媒体节点强杀。
- [ ] 节点与数据库网络分区，但仍可接收设备 UDP。
- [ ] 两节点时钟偏差，租约仍以数据库时间正确裁决。
- [ ] 滚动升级期间持续注册、PTZ、开停流和事件订阅。
- [ ] 反复抖动不导致无限迁移或消息风暴。

## 5. SLO 与验收

首版集群目标在性能基线中验证并冻结：

- 北向控制 API 月度可用性 99.95%，排除外部设备和媒体节点故障。
- gateway 故障在 15 秒内检测，新 owner 在 30 秒内接管；依赖设备主动重连的流程单独标注。
- 已接受配置/Operation 的 RPO 为 0，前提是 PostgreSQL 同步 HA 且 outbox 同事务提交成功。
- 已建立媒体流不因单个信令节点退出而中断；控制状态通过对账恢复。
- 任一资源对账最终达到数据库与外部实际状态一致，且没有跨租户处理。
- 所有接管、fencing、迁移和对账均有指标、事件和审计线索。
