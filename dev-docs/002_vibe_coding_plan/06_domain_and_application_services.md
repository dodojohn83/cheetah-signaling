# 06 领域模型与应用服务

## 1. 目标与权威模型

本章把 `001` 的统一领域模型变成可编译、可测试的代码。协议驱动不得直接写数据库、调用媒体服务器或拼装 HTTP 响应；所有业务动作必须通过应用服务和端口完成。

本阶段固定使用四个不可混淆的概念：

| 概念 | 职责 | 权威状态 |
| --- | --- | --- |
| `Operation` | 对外可查询、可取消、可超时的异步业务操作 | `Pending/Running/Succeeded/Failed/Cancelled/TimedOut` |
| `Command` | Operation/Saga 派发给 owner、协议驱动或插件的不可变指令 | 无独立业务状态机 |
| `MediaSession` | 用户视角的逻辑媒体意图和 desired state | `Requested/Allocating/Inviting/Active/Stopping/Stopped/Failed` |
| `MediaBinding` | MediaSession 与具体媒体节点资源的物理关联 | `Reserved/Active/Releasing/Released/Failed` |

`Operation` 是异步执行结果的唯一权威来源。禁止再实现 `Accepted -> Dispatched -> ...` 的 Command 聚合状态机。

交付物：

- `crates/domain/src/{device,channel,operation,command,event,media_session,media_binding}.rs`
- `crates/application/src/{device_service,operation_service,command_dispatcher,media_service,event_service}.rs`
- 四模型的关系约束、状态机、幂等规则、仓储端口和单元测试

## 2. 核心聚合与值对象

### 2.1 Device

`Device` 至少包含：`tenant_id`、`device_id`、`protocol`、`external_id`、`name`、`lifecycle`、`connectivity`、`owner_epoch`、`capabilities`、`metadata`、`created_at`、`updated_at`、`revision`。

状态拆分为两个正交维度：

- `DeviceLifecycle = Provisioning | Active | Suspended | Retired`
- `Connectivity = Unknown | Online | Offline { since, reason }`

禁止以单个 `status` 字段同时表达生命周期和在线状态。

### 2.2 Channel

`Channel` 使用 `(tenant_id, device_id, channel_id)` 唯一标识，包含 `kind`、`name`、`enabled`、`status`、`stream_profiles`、`ptz_capabilities`、`metadata`、`revision`。

### 2.3 Operation

`Operation` 状态机固定为：

```text
Pending -> Running -> Succeeded
                   -> Failed
                   -> Cancelled
                   -> TimedOut
```

- `Pending`：Operation 与 outbox 已在同一事务提交，但尚未开始执行。
- `Running`：至少一个 Saga step 或协议派发步骤已开始。
- `Succeeded/Failed/Cancelled/TimedOut`：不可逆终态；重复完成返回第一次结果。
- 投递前 deadline 到期也进入 `TimedOut`，错误码为 `expired_before_dispatch`。
- 执行中设备/媒体超时进入 `TimedOut`，使用步骤特定错误码。

Operation 必须包含 `operation_id`、`tenant_id`、`kind`、`target`、`principal`、`idempotency_scope`、`deadline`、`state`、`result_ref`、`error`、`created_at`、`updated_at`、`revision`。

### 2.4 Command

`Command` 是不可变 typed value/envelope，至少包含：

- `command_id`、`message_id`、`operation_id`、`tenant_id`
- `target`、typed payload、`idempotency_key`
- `deadline`、`expected_owner_epoch`
- `requested_by`、correlation/causation/trace context

Command 本身不提供 repository，不公开查询 URL，也不产生第二套业务终态。是否已发送、重试次数和 ack 等信息记录为 `OperationStep`/`DispatchAttempt`，用于恢复和诊断；其结果通过 CAS 推进 Operation。

### 2.5 MediaSession

`MediaSession` 独立于 Operation、ProtocolSession 和媒体节点内部 session，包含 `media_session_id`、tenant/device/channel、purpose、desired state、state、owner epoch、创建它的 operation ID、幂等作用域、deadline、error 和 revision。

状态机为：

```text
Requested -> Allocating -> Inviting -> Active -> Stopping -> Stopped
     |            |           |          |           |
     +------------+-----------+----------+----------> Failed
```

ONVIF pull/snapshot 等不需要设备邀请的工作流可以显式跳过 `Inviting`。`Stopped/Failed` 为终态；重建必须产生新 generation 或新 MediaSession，不得复活终态对象。

### 2.6 MediaBinding

`MediaBinding` 包含 `media_binding_id`、`media_session_id`、媒体节点/实例 epoch、MediaKey、media handle、protocol session、owner epoch、state、deadline、error 和 revision。

状态机为：

```text
Reserved -> Active -> Releasing -> Released
    |          |           |
    +----------+----------> Failed
```

一个 MediaSession 可以因重试或迁移拥有多个历史 binding，但同一 generation 最多一个非终态 binding。旧媒体节点 instance epoch 或旧 owner epoch 的回调不得推进 binding/session。

### 2.7 关系约束

```text
Operation
  ├── 0..N Command / OperationStep / DispatchAttempt
  └── 0..1 create-or-control MediaSession
                    └── 0..N MediaBinding
```

- Start 操作通常创建 MediaSession；Stop/Seek/Scale 等 Operation 引用已有 MediaSession。
- Operation 终态不自动等于 MediaSession 终态，例如 StartLive Operation 成功后 MediaSession 仍为 Active。
- MediaBinding 失败不必立即终止 MediaSession；policy 可以在 deadline 内重新调度新 binding。
- MediaSession 已为 Stopped 时，reconciler 只能释放残留 binding，不能重新创建媒体资源。

## 3. 实现任务

### DOM-001：聚合和值对象

- [ ] 所有 ID 使用 foundation 强类型包装，不传播裸字符串。
- [ ] 聚合字段私有，通过构造函数和命令方法修改。
- [ ] 每个修改方法验证不变量并增加 revision。
- [ ] 时间由 `Clock` 注入，测试不得直接调用系统时钟。
- [ ] 元数据限制键数、键长和值长，避免协议输入导致无界内存。

### DOM-002：状态机

- [ ] 为 Device、Operation、MediaSession、MediaBinding 实现显式迁移函数。
- [ ] 非法迁移返回 `DomainError::InvalidTransition { entity, from, to }`。
- [ ] 状态迁移生成领域事件，但不直接发布。
- [ ] Command 保持不可变，不实现 CommandState。
- [ ] 用表驱动测试覆盖所有合法和非法迁移。

### APP-001：设备应用服务

实现 `register_or_update_device`、`mark_device_online`、`mark_device_offline`、`replace_channel_catalog`、`update_device_capabilities`、`retire_device`。

每个用例执行顺序固定为：鉴权上下文校验 → 输入规范化 → 加载聚合 → 执行业务方法 → 乐观并发保存 → 写入 outbox → 返回 DTO。

### APP-002：Operation 应用服务

- [ ] `submit_operation` 按 tenant + principal + endpoint scope + idempotency key 查重。
- [ ] 同一事务创建 Pending Operation 和 CommandEnvelope outbox。
- [ ] HTTP 只在事务提交后返回 `202 Accepted`、Operation URL 和可选 MediaSession reference。
- [ ] worker 开始执行时 CAS 为 Running；完成时 CAS 到唯一终态。
- [ ] deadline 到期由定时任务把非终态 Operation 置为 TimedOut。
- [ ] cancel 只记录 desired cancellation；正在进行的协议/媒体步骤执行有界补偿。

### APP-003：Command 派发器

- [ ] 根据 `DeviceOwnerResolver` 找到 owner，通过命令总线派发不可变 CommandEnvelope。
- [ ] 每次派发创建/更新 OperationStep 或 DispatchAttempt，不创建 Command 聚合。
- [ ] Command 重投使用相同 message/command ID 和幂等键。
- [ ] 协议结果必须带 operation ID、step ID 和 owner epoch。
- [ ] 旧 owner、重复或晚到结果不得覆盖 Operation 终态。

### APP-004：媒体应用服务

- [ ] `start_live`、`stop_live`、`start_playback`、`control_playback`、`start_talk` 创建或引用明确 MediaSession。
- [ ] 创建会话、创建 Operation、写 outbox 在允许的事务组合中原子提交。
- [ ] 媒体分配产生 MediaBinding；协议协商只推进 session/binding，不替代它们。
- [ ] 分配失败可在 deadline 内新建 binding 重试；旧 binding 保留为终态历史。
- [ ] 失败补偿逆序释放协议 dialog、媒体 handle 和 binding。
- [ ] 相同业务幂等作用域不得重复创建 MediaSession。

## 4. 仓储端口

必须提供：`OperationRepository`、`MediaSessionRepository`、`MediaBindingRepository`、`OperationStepRepository`（若步骤持久化）及对应 UnitOfWork 组合。

禁止提供承担业务权威状态的 `CommandRepository`。如果需要审计投递，使用只追加或 CAS 的 `OperationStepRepository/DispatchAttemptRepository`，并明确其清理周期。

## 5. 测试清单

- [ ] 属性测试：任意非法状态跳转失败且聚合不变。
- [ ] Operation 幂等测试：相同请求 100 次只产生一个 Operation 和一条逻辑 outbox command。
- [ ] Command 重投测试：重复 envelope 只产生一次有效副作用和一个 Operation 终态。
- [ ] 关系测试：Start 成功后 Operation 为 Succeeded 而 MediaSession 保持 Active。
- [ ] 迁移测试：MediaBinding 失败后可创建新 binding，旧 instance 回调不能覆盖新状态。
- [ ] 停止测试：Stopped MediaSession 的残留 binding 只能释放，不能被 reconciler 复活。
- [ ] 并发测试：两个 revision 同时保存，仅一个成功。
- [ ] 时间测试：虚拟时钟准确区分投递前过期和执行中超时错误码。
- [ ] DTO 测试：domain 与 Proto/HTTP mapper 往返不丢字段且不混用四类 ID。

## 6. 验收标准

- domain 不依赖 SQLx、Axum、Tonic、协议 crate 或具体消息中间件。
- 应用服务只依赖 port trait，可使用内存实现完整测试。
- 代码中不存在 `CommandState::Accepted/Dispatched/...` 或等价的第二套业务状态机。
- Operation、MediaSession、MediaBinding 的状态所有者和终态规则均有自动化测试。
