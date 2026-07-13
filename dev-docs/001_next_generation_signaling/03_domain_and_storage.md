# 03. 领域模型与存储

## 1. 身份模型

所有 ID 使用受校验 newtype。内部主 ID 使用 UUIDv7；wire 上使用固定字符串编码，数据库使用原生 UUID 或 16 字节等价表示。

```rust
TenantId
DeviceId
EndpointId
ChannelId
ProtocolSessionId
MediaSessionId
OperationId
NodeId
PluginId
```

外部协议身份单独建模：

```text
ProtocolIdentity {
  tenant_id,
  protocol,          // gb28181, onvif, ...
  authority,         // SIP realm/domain、ONVIF network zone 等
  external_id,
  device_id,
  attributes,
  revision
}
```

唯一约束为 `(tenant_id, protocol, authority, external_id)`。GB 设备编码、ONVIF EPR、MAC、序列号均不得直接替代 `DeviceId`。

## 2. 聚合与实体

### 2.1 Device

保存资产级状态：tenant、display name、kind、manufacturer/model、启用状态、标签、credential reference、desired policy、reported summary 和 revision。

`Device.online` 不是直接写入的布尔真相，而是由协议 presence 投影计算；API 返回 online 时同时返回 `reason`、`last_seen_at`、`expires_at` 和 `source`。

### 2.2 Endpoint

表示可访问地址：协议、transport、network zone、host/IP、port、path、TLS policy、priority。ONVIF 返回的 XAddr 在通过 SSRF 校验后才可成为 endpoint。

### 2.3 Channel

通道属于 Device，包含协议通道标识映射、名称、类型、父子层级、在线投影、能力与媒体提示。目录中的行政区域/组织节点也可表示为 channel tree node，但必须用 kind 区分是否可产生媒体。

### 2.4 Capability

能力使用稳定 key 和 typed value：live、playback、download、ptz、talk、alarm、snapshot、event、codec hints、transport 等。未知厂商能力进入 namespaced extension，不能伪装为标准 capability。

### 2.5 ProtocolSession

包含 owner node、owner epoch、transport、remote/local endpoint、状态、最后活动、deadline 和协议相关的受限 summary。完整 SIP message、SOAP body、密码或 token 不持久化。

### 2.6 MediaBinding

```text
MediaBinding {
  media_session_id,
  tenant_id,
  channel_id,
  purpose,              // live/playback/download/talk/snapshot
  media_node_id,
  media_key,
  media_handle_id,
  protocol_session_id,
  owner_epoch,
  state,
  created_at,
  deadline,
  last_error
}
```

### 2.7 Operation

所有可能等待设备、媒体或插件的命令都创建 Operation：

```text
Pending -> Running -> Succeeded
                   -> Failed
                   -> Cancelled
                   -> TimedOut
```

Operation 保存 command kind、target、principal、idempotency key、deadline、result reference、稳定错误和 revision。相同 tenant + principal + endpoint scope + idempotency key 必须返回原 Operation。

## 3. 统一命令与事件

公共命令包括：

- SyncDevice、RefreshCatalog、QueryStatus；
- StartLive、StopMediaSession；
- StartPlayback、SeekPlayback、SetPlaybackScale；
- StartDownload；
- PtzMove/PtzStop/Preset；
- StartTalk/StopTalk；
- TakeSnapshot；
- Create/RemovePlatformLink；
- Subscribe/UnsubscribeDeviceEvents。

领域事件包括：

- DeviceDiscovered、DeviceRegistered、DeviceOnlineChanged；
- ChannelUpserted/Removed/OnlineChanged；
- ProtocolSessionOpened/Closed；
- OperationStateChanged；
- MediaBindingStateChanged；
- AlarmReceived、PositionUpdated、DeviceEventReceived；
- PlatformLinkStateChanged；
- NodeStateChanged、PluginStateChanged。

事件必须包含 event ID、tenant、aggregate ref、aggregate sequence、occurred_at、correlation/causation ID、source 和 typed payload。事件不包含 secret、完整鉴权头或任意原始报文。

## 4. 状态所有权

一个设备在任一时刻最多有一个可执行控制命令的 owner：

```text
OwnershipRecord {
  tenant_id,
  device_id,
  node_id,
  protocol_session_id,
  epoch,
  acquired_at,
  last_confirmed_at
}
```

- 新注册使用 NATS KV CAS 创建或替换 owner。
- 成功 revision/epoch 作为 fencing token。
- 旧 owner 收到替换通知后关闭或降级自身 session。
- 所有设备副作用、数据库状态转换和媒体命令都携带 expected epoch。
- 被拒绝的 stale epoch 不可自动忽略为成功。

edge 的 `OwnershipPort` 使用内存实现，但保持相同语义和测试。

## 5. 权威数据与临时数据

PostgreSQL/SQLite 权威保存：

- tenants、principals/roles（或外部 subject 映射）；
- devices、protocol_identities、endpoints、channels、capabilities；
- platform_links、credential_refs；
- operations、media_bindings；
- outbox、inbox、webhook subscriptions/deliveries；
- audit records、schema/version metadata。

不在每次心跳写数据库。presence 变化、owner 变化和周期性汇总可异步投影；设备重注册和目录变更才写权威记录。

## 6. Repository 设计

领域层按聚合定义最小 trait，例如：

```rust
DeviceRepository
ChannelRepository
OperationRepository
MediaBindingRepository
PlatformLinkRepository
OutboxRepository
InboxRepository
AuditRepository
```

不得暴露 SQL row、transaction 或 backend error。application 需要跨 repository 原子性时使用 `UnitOfWork` port，明确列出允许的事务组合，避免万能 transaction closure 将 SQL 实现泄漏给领域层。

SQLite 和 PostgreSQL 分别维护迁移：

- schema 的逻辑字段和约束一致；
- SQL 方言、索引、分区、JSON/UUID 表示可不同；
- migration ID 和逻辑版本一致；
- 每次发布同时测试空库建库和上一受支持版本升级；
- cluster 使用 expand → backfill → switch → contract，禁止滚动升级期间先删除旧列。

## 7. Outbox/Inbox

创建 Operation 与写 outbox 必须在同一数据库事务提交。publisher 读取未发送记录，发布 Protobuf envelope，并在确认后标记发送；重复发布由 message ID 去重。

消费者在执行副作用前用 `(consumer_name, message_id)` 写 inbox。处理结果和 inbox 状态尽可能同事务提交。无法与外部设备/媒体节点形成事务时，依赖幂等键、fencing 和 reconciler。

不得将 NATS “exactly once”宣传为跨数据库、设备和媒体节点的端到端 exactly once。

## 8. Secret 与隐私

`SecretStore` 支持：

- edge：权限受限的加密文件或本机密钥封装；
- cluster：Vault/KMS 等 provider；
- 测试：内存 provider。

日志、事件、Operation result 和错误 details 不得出现 SIP 密码、ONVIF UsernameToken、Authorization、RTSP URI 中的 userinfo、Webhook secret 或私钥。设备抓包 fixture 入库前必须脱敏并保留可复现结构。
