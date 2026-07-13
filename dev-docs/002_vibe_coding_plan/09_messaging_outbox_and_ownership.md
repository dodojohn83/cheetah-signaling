# 09 消息总线、Outbox 与设备所有权

## 1. 目标

构建可替换的集群消息层，首选 NATS JetStream，同时保留进程内实现用于单机。系统采用“至少一次传递 + 幂等消费”，不宣称跨数据库与消息系统的 exactly-once。

## 2. 主题规范

主题必须版本化且包含租户边界：

```text
sig.v1.command.{tenant_bucket}.{owner_node}
sig.v1.event.{tenant_bucket}.{event_type}
sig.v1.owner.change.{tenant_bucket}
sig.v1.media.callback.{tenant_bucket}
```

不得直接把未转义的 tenant/device ID 放入主题。`tenant_bucket` 由固定哈希生成。消息体使用 `05` 定义的 Proto envelope。

## 3. 实现任务

### MSG-001：总线端口与实现

- [ ] 定义 `CommandBus`、`EventBus`、`Subscription`、`AckHandle`。
- [ ] 实现 `InProcessBus`，保持与集群端相同的序列化边界。
- [ ] 实现 `NatsJetStreamBus`，配置 durable consumer、ack deadline、max deliver 和 dead-letter subject。
- [ ] 生产者设置消息 ID，消费者基于 `message_id` 去重。

### MSG-002：Transactional Outbox

- [ ] 领域写事务同时插入 `outbox_events`。
- [ ] relay 通过批量抢占读取未发布事件，发布成功后标记。
- [ ] 进程在标记前崩溃会重复发布，消费者必须安全处理。
- [ ] 记录创建到发布延迟、重试次数、积压量和最老事件年龄。
- [ ] 超过上限的事件进入人工可重放的失败状态，不静默丢弃。

### MSG-003：Inbox/幂等消费

- [ ] 在处理副作用前写入或锁定 `processed_messages`。
- [ ] 重复消息返回之前结果或直接确认。
- [ ] 去重记录保留期大于消息最大重投窗口。
- [ ] 业务失败与不可反序列化消息使用不同死信原因。

### OWN-001：设备租约

`device_owners` 至少包含 `device_key`、`node_id`、`epoch`、`lease_until`、`updated_at`。获取所有权必须原子增加 epoch；只有持有匹配 epoch 的节点才能更新在线会话或完成命令。

- [ ] 单机模式使用本地 owner，仍生成 epoch。
- [ ] 集群模式通过数据库租约作为权威来源，NATS 只传播变化通知。
- [ ] 定期续租使用批量操作，续租失败立即降低 readiness 并停止接收新设备。
- [ ] 租约过期后新节点可接管，旧节点收到更高 epoch 后关闭旧会话。

### OWN-002：路由与转发

- [ ] `OwnerResolver` 优先读本地缓存，缓存项带 lease deadline。
- [ ] 非拥有者收到命令时转发一次；检测 hop count 防止循环。
- [ ] owner 不存在时触发受限抢占，避免所有节点同时竞争。
- [ ] 结果写入再次校验 epoch，陈旧结果记录指标后丢弃。

## 4. 故障场景测试

- [ ] 发布成功但 outbox 未标记：重复投递仅产生一次副作用。
- [ ] consumer 处理成功但 ack 丢失：重投被 inbox 去重。
- [ ] NATS 中断：本地协议连接继续维持，命令积压受限且可观测。
- [ ] 数据库短暂中断：不得自认为仍持有过期 lease。
- [ ] 网络分区后两个节点竞争：仅最高有效 epoch 可提交状态。
- [ ] owner 节点强杀：在租约窗口后其他节点接管。

## 5. 验收标准

- 每种消息都有 schema、超时、重试、死信和幂等策略。
- 不依赖 NATS 的单机模式能够运行完整业务。
- 所有权切换不会由旧节点覆盖新状态。
