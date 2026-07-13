# 06 领域模型与应用服务

## 1. 目标与交付物

本章把 `001` 中的统一设备模型变成可编译、可测试的领域代码。协议驱动不得直接写数据库、调用媒体服务器或拼装 HTTP 响应；所有业务动作必须通过应用服务和端口完成。

交付物：

- `crates/domain/src/{device,channel,session,command,event,media}.rs`
- `crates/application/src/{device_service,command_service,media_service,event_service}.rs`
- 领域状态机、幂等规则、命令生命周期及单元测试

## 2. 核心聚合

### 2.1 Device

`Device` 至少包含：`tenant_id`、`device_id`、`protocol`、`external_id`、`name`、`lifecycle`、`connectivity`、`owner_epoch`、`capabilities`、`metadata`、`created_at`、`updated_at`、`revision`。

状态拆分为两个正交维度：

- `DeviceLifecycle = Provisioning | Active | Suspended | Retired`
- `Connectivity = Unknown | Online | Offline { since, reason }`

禁止以单个 `status` 字段同时表达生命周期和在线状态。

### 2.2 Channel

`Channel` 使用 `(tenant_id, device_id, channel_id)` 唯一标识，包含 `kind`、`name`、`enabled`、`status`、`stream_profiles`、`ptz_capabilities`、`metadata`、`revision`。

### 2.3 Command

`Command` 状态机固定为：

```text
Accepted -> Dispatched -> Succeeded
                      \-> Failed
Accepted ----------------> Expired
Dispatched --------------> TimedOut
```

终态不可逆；重复完成必须返回第一次的结果。命令必须带 `command_id`、`idempotency_key`、`deadline`、`target`、`operation`、`requested_by`、`trace_context`。

### 2.4 MediaSession

`MediaSession` 独立于协议会话，状态为 `Requested | Allocating | Inviting | Active | Stopping | Stopped | Failed`。每次状态迁移记录原因和时间，失败必须保留可机器识别的错误码。

## 3. 实现任务

### DOM-001：实现聚合与值对象

- [ ] 所有 ID 使用 `foundation` 的强类型包装，不得在领域层传播裸字符串。
- [ ] 聚合字段私有，通过构造函数和命令方法修改。
- [ ] 每个修改方法验证不变量并增加 `revision`。
- [ ] 时间由 `Clock` 端口注入，测试不得直接调用系统时钟。
- [ ] 元数据限制键数、键长和值长，避免协议输入导致无界内存。

### DOM-002：实现状态机

- [ ] 为设备、命令、媒体会话实现显式迁移函数。
- [ ] 非法迁移返回 `DomainError::InvalidTransition { entity, from, to }`。
- [ ] 状态迁移生成领域事件，但不直接发布。
- [ ] 用表驱动测试覆盖每一条合法和非法迁移。

### APP-001：设备应用服务

实现以下用例：

- `register_or_update_device`
- `mark_device_online`
- `mark_device_offline`
- `replace_channel_catalog`
- `update_device_capabilities`
- `retire_device`

每个用例执行顺序固定为：鉴权上下文校验 → 输入规范化 → 加载聚合 → 执行业务方法 → 乐观并发保存 → 写入 outbox → 返回 DTO。

### APP-002：命令应用服务

- [ ] `submit_command` 先检查幂等键，再创建命令。
- [ ] 根据 `DeviceOwnerResolver` 找到拥有者节点并通过命令总线派发。
- [ ] 同步 HTTP 请求只等待“已接受”，长结果通过查询或事件获得。
- [ ] deadline 到期后由定时任务置为 `Expired/TimedOut`。
- [ ] 协议驱动回报结果时校验 `owner_epoch`，拒绝旧拥有者写入。

### APP-003：媒体应用服务

- [ ] `start_live`、`stop_live`、`start_playback`、`control_playback`、`start_talk` 使用统一模型。
- [ ] 媒体资源分配、协议协商、媒体服务器调用分别建模，失败时执行补偿。
- [ ] 相同业务幂等键不得重复创建媒体会话。

## 4. 测试清单

- [ ] 属性测试：任意非法状态跳转均失败且聚合不变。
- [ ] 幂等测试：相同请求执行 100 次只产生一个命令和一个 outbox 事件。
- [ ] 并发测试：两个 revision 同时保存，仅一个成功。
- [ ] 时间测试：虚拟时钟推进后命令准确超时。
- [ ] 序列化往返测试：领域 DTO 与 Proto/HTTP DTO 转换不丢字段。

## 5. 验收标准

- `domain` 不依赖 SQLx、Axum、Tonic、协议 crate 或具体消息中间件。
- 应用服务只依赖端口 trait，可使用内存实现完整测试。
- 所有状态迁移和失败分支有测试；不允许以布尔字段替代明确状态。
