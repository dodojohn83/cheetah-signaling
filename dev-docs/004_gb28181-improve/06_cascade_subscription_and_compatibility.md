# 06. 级联、订阅与非标准兼容

## 1. 目标

将现有 cascade 注册、保活、目录、subscription 和 bridge 状态机接入生产配置、transport、owner、repository 和 MediaPort，完成：

- 作为下级向一个或多个上级平台注册；
- 作为上级接入下级平台；
- tenant-scoped 目录共享、查询与变更通知；
- 点播/回放/控制的单 owner 桥接；
- 显式、可审计的 2016/vendor compatibility profile。

## 2. 平台模型

新增或完善 `GbPlatformLink` 聚合：

- tenant、platform ID、方向 upstream/downstream；
- local/remote ProtocolIdentity、realm/domain、transport endpoint；
- credential reference、auth policy、listener/network zone；
- desired/actual registration state、Call-ID/CSeq、expiry、last keepalive；
- owner node/epoch、link generation、revision；
- catalog mapping/filter policy、subscription limits；
- compatibility profile ID/revision；
- rate limit、deadline、retry/backoff 配置。

凭据只保存引用。一个 remote platform 可以有多个历史 generation，但同一 generation 只能有一个当前 owner 和有效 registration/dialog 集合。

## 3. 向上级注册与保活

- application 创建 PlatformLink desired state，owner shard 驱动 `GbCascadeMachine`；
- REGISTER client transaction 支持 401/407 challenge、stale、expiry refresh、注销和 bounded backoff+jitter；
- Call-ID/CSeq 和 authorization context 按 link generation 管理；
- keepalive MESSAGE 具有独立 transaction/deadline，连续失败达到阈值才改变 link health；
- DNS/endpoint 解析通过受限 outbound policy，禁止任意 redirect 和 DNS rebinding；
- credential 暂时不可用时不发送 unauthenticated refresh；
- owner 丢失、drain 或 shutdown 时停止新事务，能安全发送 Expires=0 时有界注销，否则依赖 expiry。

## 4. 下级平台接入

- listener/domain router 区分 device 与 downstream platform enrollment；
- Digest、REGISTER duplicate/expiry/endpoint 规则与设备接入使用同一 core 语义；
- platform ACL 决定可见 tenant resource、目录前缀、control/media capability；
- 下级平台身份与普通 Device/Channel identity 分离，禁止因 ID 相同覆盖设备聚合；
- 大规模下级目录和事件使用分页/outbox，不在 SIP handler 内直接查询全库。

## 5. Catalog 映射与通知

### 5.1 ID 与目录

- 内部 TenantId/DeviceId/ChannelId 始终保持 UUIDv7/newtype；
- 每个平台使用独立 external ProtocolIdentity mapping；
- mapping key 包含 tenant + platform link + internal resource + mapping revision；
- external ID 冲突显式失败或使用预配置映射，不能静默覆盖；
- virtual organization/directory 由 typed policy 生成，并有最大深度/节点/页数；
- filter/whitelist/tag/org-prefix 在 repository query 中 tenant-scoped 执行。

### 5.2 Catalog Query/Notify

- 上级 Query 先返回 SIP 200，再按 cursor 分页构造 bounded MESSAGE response；
- inconsistent provider cursor、重复页或超过 max pages 终止并诊断，不能死循环；
- empty catalog 发送 SumNum=0 的单个响应；
- change event 经 outbox 合并为 Catalog Notify；subscription 未建立时按 policy 缓存最新摘要或丢弃低价值中间态；
- 每个 MESSAGE 有 transaction、deadline、SN、idempotency/dedupe key。

## 6. SUBSCRIBE/NOTIFY

- subscription key 包含 tenant、platform link、event package、Call-ID/tags 和 generation；
- 支持 initial、refresh、Expires=0、expiry、capacity eviction 和 terminated notify；
- Event package 不支持返回 489；认证/ACL 不通过返回 401/403；
- refresh 保留 dialog/local tag，CSeq 单调；
- NOTIFY client transaction 具有 retransmission/deadline，错误或 timeout 按 policy 终止/降级；
- subscription table、pending notify、event buffer 都有上限；
- owner/link generation 变化时旧 subscription callback 不推进新状态。

v1 必选 event package：Catalog change、Alarm、MobilePosition。新增 package 必须通过 capability 暴露，不复用万能 payload。

## 7. Bridge Saga

### 7.1 上级点播

```text
upstream INVITE
  -> auth/ACL/channel mapping
  -> 100 Trying
  -> bridge Operation + owner fencing
  -> 下游设备/协议 StartLive 或已有 MediaSession
  -> MediaPort sender/receiver binding
  -> upstream 200 SDP
  -> ACK
  -> Active bridge
```

- 上游指定 SSRC 与内部/media SSRC 分离；映射由 MediaPort/Binding 管理；
- 上游 CANCEL、BYE、ACK timeout、下游失败、media failure 均触发有界补偿；
- CANCEL 后 media ready/late response 不得发送新的成功 200；
- Stop-before-ACK 在 ACK/timeout 后完成清理；
- 同一 external request/idempotency scope 不创建重复 bridge；
- 不允许上下游同时成为控制命令 owner；灰度/迁移期间只转发一次有效副作用。

### 7.2 回放与 INFO

- upstream playback INVITE 映射到独立下游 playback MediaSession；
- MANSRTSP INFO 必须解析和重新编码 typed command，不透明转发任意 body；
- 上下游 response/outcome 分开记录；下游 UnknownOutcome 不伪造成上游成功；
- MediaStatus、BYE 和 media terminal event 只影响匹配 bridge generation。

## 8. Compatibility Profile

配置形态：

```toml
[[gb28181.compatibility_profiles]]
id = "vendor-model-firmware"
standard_version = "2016"
manufacturer = "..."
model = "..."
firmware = "..."
evidence_ref = "testdata/gb28181/...meta.toml"
```

profile 可声明的受控 override：

- charset fallback 与 XML declaration mismatch；
- MIME alias 到注册 typed message；
- Contact/source/rport route policy；
- 缺省/异常但无歧义的 header normalization；
- catalog count/fragment/Notify 行为；
- SDP SSRC、payload、TCP setup 和先发媒体策略；
- register/keepalive/response timeout；
- Broadcast/MediaStatus 或厂商 command capability。

禁止 override：

- 跳过 tenant/identity/Digest/owner epoch 校验；
- 放开 CRLF、DTD/XXE、body/深度/节点/连接/队列上限；
- 允许任意 SDP/URL 目标或 redirect；
- 透明转发未知 DeviceControl/MANSRTSP；
- 记录 secret 或完整原始报文；
- 改变同一公共 Operation/Command 的成功语义。

profile 匹配优先级固定为 exact firmware → model → manufacturer → standard generic；多个同优先级匹配视为配置错误。profile revision 固化到 ProtocolSession/PlatformLink，运行中的 dialog 不因热更新改变语义。

## 9. 首批兼容能力

- sipsdk MIME aliases：KSLP、KSPTZ、ALARM、KSSP、KSDU、cpim-pidf+xml、application/rtsp；
- WVP 场景：设备级密码、Catalog Notify、Alarm/MobilePosition subscription、multiple upstream、virtual directory/custom external ID；
- AKStream 场景：IPv6、GB2016、ConfigDownload、PresetQuery、Broadcast、MediaStatus；
- GB28181.Solution 场景：strict realm、duplicate REGISTER、Contact/NAT binding、minimum expiry、UDP/TCP route；
- simple-media-server 场景：DeviceInfo bootstrap、catalog timeout、source endpoint change、MediaStatus=121、Broadcast handshake。

这些只是 backlog，不代表默认全部启用。每项必须先有标准/设备证据、fixture 和 capability 行为，再进入具体 profile。

## 10. 实施任务

- [ ] `GB4-CAS-001`：实现 GbPlatformLink 持久化、双数据库 contract、owner/link generation 和配置加载。
- [ ] `GB4-CAS-002`：将 cascade REGISTER/keepalive/deregister 接入统一 client transaction、transport 和 SecretStore。
- [ ] `GB4-CAS-003`：完成 tenant-scoped external ID mapping、virtual directory、Catalog query/notify 和分页。
- [ ] `GB4-CAS-004`：完成 SUBSCRIBE/NOTIFY persistence、capacity、refresh/expiry 和 owner takeover。
- [ ] `GB4-CAS-005`：完成 live/playback bridge Saga、CANCEL/BYE/INFO、media binding 和双侧补偿。
- [ ] `GB4-CAS-006`：完成多上级、下级接入、ACL、loop/hop 和唯一 control owner 测试。
- [ ] `GB4-COMP-001`：实现 profile schema、exact match、revision pinning、capability 和配置校验。
- [ ] `GB4-COMP-002`：实现 charset/MIME/header/endpoint/catalog 首批受控 override。
- [ ] `GB4-COMP-003`：实现 SDP/Broadcast/MediaStatus 受控 override，并保持 MediaPort 网络边界。
- [ ] `GB4-COMP-004`：为每个 workaround 增加 provenance fixture、risk、regression 和 removal criteria。

## 11. 测试与退出门禁

- 上级注册覆盖 auth、stale、redirect rejection、credential outage、expiry、backoff、owner drain 和重启恢复。
- 多上级具有独立 Call-ID/CSeq/credential/owner/rate limit，任何一个失败不阻塞其他 link。
- Catalog mapping 覆盖 ID 冲突、tenant 越界、empty/large catalog、cursor inconsistency 和 notification storm。
- subscription 覆盖 initial/refresh/terminate/expire/evict、NOTIFY loss/error、旧 generation callback。
- bridge 覆盖 unknown channel、ACL、capacity、CANCEL-before-ready、late media ready、ACK timeout、BYE 和 media node restart。
- 每个 compatibility override 都有 strict-default rejection 和 profile-enabled acceptance 两组测试。
- 至少与一个真实上级和一个真实下级平台完成注册、目录、订阅、点播、回放和控制互操作报告。

