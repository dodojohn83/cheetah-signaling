# 04. 设备接入、命令与事件闭环

## 1. 目标

把 REGISTER、保活、目录、查询、控制和通知从 in-memory access + logging sink 提升为 tenant-scoped、owner-fenced、可幂等恢复的 application 行为。所有外部输入必须先完成 tenant、identity、认证、大小和状态验证，再产生领域副作用。

## 2. ProtocolSession 模型

新增持久化 `ProtocolSession` 聚合，表达 REGISTER/binding 和临时控制会话，不与 Operation 或 MediaSession 混用。至少包含：

- `protocol_session_id`：UUIDv7；
- `tenant_id`、`device_id`、`ProtocolIdentity`；
- local listener/domain/realm/device ID；
- transport、observed source、Contact、advertised endpoint；
- REGISTER Call-ID、CSeq、expiry、last authenticated refresh；
- presence state、last keepalive、offline reason；
- owner node、owner epoch；
- compatibility profile ID/revision；
- aggregate revision、created/updated UTC time。

Repository 方法显式携带 tenant；更新带 revision；聚合更新和 outbox 同事务。SQLite/PostgreSQL 通过同一 contract suite。

## 3. REGISTER 与 Digest

### 3.1 处理顺序

1. listener/domain router 确定 tenant 和 local identity；
2. parser/transaction 验证 method、Via、To/From、Contact、Call-ID/CSeq、Expires；
3. rate limit 与 nonce/replay 检查；
4. 通过 SecretStore namespace 查询设备密码或租户 enrollment policy；
5. Digest 验证成功后解析/创建 Device 和 ProtocolSession；
6. 原子获取或刷新 owner epoch；
7. 同事务提交 session/device/presence 与 outbox；
8. 缓存 transaction response，重复 REGISTER 返回同一结果；
9. 发送 200 后异步发起必要的 DeviceInfo/Catalog bootstrap Operation。

### 3.2 行为表

| 场景 | 行为 |
| --- | --- |
| 无 Authorization 且 Required | 401 challenge，不创建 session/device |
| stale nonce | 401 `stale=true`，不计为密码错误 |
| replay/nc 回退 | 401/403 + security metric，不执行副作用 |
| unknown device | 按 tenant enrollment policy 拒绝或创建 pending device；生产无全局默认密码 |
| Expires=0 | 认证后注销，关闭 binding/presence，不删除 Device |
| duplicate branch/CSeq | 返回缓存响应，不重复 online/outbox |
| CSeq 增长续期 | 更新 expiry、endpoint 和 revision，不重复创建设备 |
| source 变化 | 只有认证续期后更新 observed route；必要时增加 session generation/owner epoch |
| expiry/keepalive timeout | timer wheel 产生 offline action；重复 timer 幂等 |
| credential backend timeout | 503/temporarily unavailable，可重试但不降级 ChallengeOptional |

`challenge_optional` 仅允许明确 dev profile；启动日志警告且 readiness 标记 insecure。cluster/production profile 配置该值时启动失败。

## 4. Presence 与保活

- Keepalive 的 From identity、body DeviceID、tenant session 必须一致；
- transaction duplicate 不重复更新 outbox；可以合并为最新 `last_seen`；
- offline 后的合法 Keepalive 只在 session 未过期且 owner epoch 当前时恢复 online；否则要求重新 REGISTER；
- 保活状态与 REGISTER expiry 分开建模，两个 timer 都由 FakeClock/时间轮驱动；
- registration/keepalive storm 按 source、tenant、listener 和 device 多级限流；
- presence outbox 可降采样/合并，但 Device authoritative state 不得丢失。

## 5. Catalog、DeviceInfo 与 DeviceStatus

### 5.1 Query Operation

Catalog/DeviceInfo/DeviceStatus 查询由 application 创建 Operation 和 typed Command。Command 包含 operation/step ID、message ID、idempotency key、deadline、tenant、device、owner epoch 和 protocol session generation。

core 生成 MESSAGE client transaction；200 只表示 SIP 接收，不等价于业务 query 完成。对应 XML Response 按 SN/DeviceID/CommandType 关联 OperationStep。

### 5.2 Catalog 聚合

聚合 key 为 `(tenant_id, device_id, command_type, SN, operation_id)`，具有：

- max active aggregations、max fragments、max items、max bytes、deadline；
- fragment message dedupe key；
- SumNum、DeviceList Num 和实际 item 数的独立诊断；
- duplicate、out-of-order、empty、missing、slow-first-fragment 行为；
- `Complete`、`Partial`、`Failed` 三种结果；
- stable internal ChannelId 与独立 ProtocolIdentity mapping；
- 同事务 channel upsert/delete policy、device revision 和 outbox。

超时有已验证片段时产生 Partial 并保留诊断；没有可信片段时 Failed。不得把缺片结果当作完整目录，也不得无限等待。

## 6. Typed 命令矩阵

| 类别 | 命令 | 副作用/重试语义 |
| --- | --- | --- |
| Query | Catalog、DeviceInfo、DeviceStatus、RecordInfo、PresetQuery、ConfigDownload | 可按 transaction policy 重发；业务 Response 幂等合并 |
| PTZ | pan/tilt/zoom、Stop、preset set/call/delete、HomePosition、DragZoom | 设备是否执行不明时 UnknownOutcome；连续移动必须有 Stop/deadline |
| DeviceControl | Guard、AlarmReset、Record、TeleBoot、IFrame、DeviceConfig | 危险控制不自动业务重试；明确失败可返回 Failed |
| Notify/Event | Alarm、MobilePosition、Catalog change、MediaStatus | 至少一次输入 + inbox dedupe；关键事件不静默 drop |
| Media | StartLive/Stop、Playback/Download、Talk/Broadcast、MANSRTSP control | 交由 [05](05_media_operations_and_reconciliation.md) Saga |

新增 domain/REST/Proto payload 必须 typed、兼容扩展。Proto enum 0 为 `*_UNSPECIFIED`，禁止用 `Any` 或 JSON 表达核心命令。

## 7. Command Outcome

Command 保持不可变指令，不增加第二套业务生命周期。区分：

```text
Inbox receipt: accepted / duplicate / rejected / dead-lettered
DispatchAttempt: queued / sent / transport-failed / timed-out
OperationStep outcome: succeeded / failed / unknown-outcome / cancelled
```

- SIP 2xx 对 DeviceControl 可以表示设备接受，但具体动作结果仍按命令语义决定；
- network error 在确认发送前发生可标 Failed/retryable；
- 发送后无最终响应标 UnknownOutcome；
- Unsupported 在 dispatch 前返回稳定 Unsupported，不创建假成功；
- duplicate command 返回首次确定 outcome 或当前 OperationStep，不重复副作用；
- 旧 owner epoch 在发送前和接收 response/event 时均拒绝。

修改 `CommandHandlerResult` 或等价接口，使 inbox ack 不再强制用 `ProcessedMessageStatus::Completed` 表达未知业务结果。

## 8. Event Adapter

将 core event 映射为 typed application input：

- `DeviceRegistered/Unregistered/PresenceChanged`；
- `CatalogFragmentReceived`；
- `DeviceInfoReceived/DeviceStatusReceived`；
- `RecordInfoFragmentReceived/PresetInfoReceived/ConfigReceived`；
- `AlarmReceived/MobilePositionReceived/CatalogChanged`；
- `DeviceControlResponded`；
- `MediaStatusReceived`；
- cascade registration/subscription/bridge event。

每个 event 携带 tenant、ProtocolIdentity、ProtocolSessionId、owner epoch、transaction/message dedupe key、received UTC/monotonic time、listener ID 和 compatibility profile revision。

禁止 sink：

- 使用全局 default tenant；
- 为每次重传生成不稳定 MessageId；
- queue full 时丢弃 Alarm、command result 或 terminal media event；
- 只记录“handler not wired”日志后返回成功；
- 在 assembly 内完成目录 business mapping。

## 9. 事务和 Outbox

- Device/ProtocolSession/Channel 聚合修改与 outbox 在同一 UnitOfWork；
- 大目录按 bounded batch 提交，每批带 operation/aggregation revision，可在 crash 后续跑；
- inbox 在任何 application 副作用前去重；
- Alarm/MobilePosition 等事件使用协议稳定字段 + transaction key 生成 dedupe scope，不依赖随机 ID；
- handler 不在数据库事务中等待 SIP、MediaPort、NATS 或设备；
- owner acquisition/CAS 与 session generation 写入必须防止旧 owner 回调推进状态。

## 10. 实施任务

- [x] `GB4-ACC-001`：实现持久化 ProtocolSession、双数据库 migration 和 repository contract。详见 `dev-docs/004_gb28181-improve/reports/gb4-acc-001.md`。
- [ ] `GB4-ACC-002`：完成 REGISTER/注销/续期/expiry/keepalive/owner acquisition 事务链路。
- [ ] `GB4-ACC-003`：完成 listener tenant、body identity、protocol session generation 和 endpoint 安全校验。
- [ ] `GB4-ACC-004`：完成 Catalog/DeviceInfo/DeviceStatus bootstrap/query Operation。
- [ ] `GB4-ACC-005`：完成 bounded Catalog/RecordInfo aggregation 和 channel mapping。
- [ ] `GB4-CMD-001`：为 query、PTZ、preset、DeviceControl 新增 typed domain/REST/Proto payload 和 capability。
- [ ] `GB4-CMD-002`：将 application Command 直接路由到 GB shard/client transaction，移除 plugin placeholder。
- [ ] `GB4-CMD-003`：分离 inbox receipt、dispatch attempt 和 OperationStep outcome，正确实现 UnknownOutcome。
- [ ] `GB4-EVT-001`：将全部 GB event 接入 application handler、repository/inbox/outbox；删除 logging-only 分支。
- [ ] `GB4-EVT-002`：实现事件优先级、coalescing、queue full 和 dead-letter 运维策略。

## 11. 测试与退出门禁

- REGISTER 覆盖 401、stale、replay、错误密码、注销、duplicate、续期、endpoint 漂移、expiry 和 owner takeover。
- tenant/domain/body identity 越界全部拒绝，且数据库/日志无跨租户数据。
- Catalog 覆盖 zero/huge SumNum、重复/乱序/缺片、超时、崩溃恢复和 revision conflict。
- 每类命令覆盖 success、explicit failure、timeout、cancel、duplicate、queue full、old epoch 和 UnknownOutcome。
- Alarm/位置/控制响应/MediaStatus 在 queue saturation 下不静默丢失。
- SQLite/PostgreSQL contract 覆盖事务、outbox、revision、tenant 和 migration rollback/startup failure。
- production 不再出现 `handler not wired`、固定 accepted/unknown payload 或随机 retransmission dedupe ID。

