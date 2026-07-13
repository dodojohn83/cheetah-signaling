# 05. Media Plane 集成

## 1. 边界

`cheetah-signaling` 是媒体资源的申请者和生命周期编排者；`cheetah-media-server-rs` 是媒体 session、RTP port、拉流代理、录制、快照和输出 URL 的唯一执行者。

信令侧不得：

- 绑定 RTP/RTCP 端口；
- 解析 PS/TS/ES、维护 SSRC 收包状态；
- 拉取 RTSP、解码快照或产生播放协议 URL；
- 访问媒体 engine、codec cache、stream manager 内部对象；
- 把 GB/ONVIF 领域对象写入媒体核心。

媒体侧不得：

- 实现设备注册、目录、SOAP、SIP dialog 或平台级联；
- 把设备密码和完整协议请求保存为媒体 metadata；
- 绕过 owner epoch 执行来自旧信令 owner 的操作；
- 反向访问 signaling 数据库。

## 2. 共享契约

共享 Protobuf 是 `cheetah.media.v1`。它映射媒体计划中的 `cheetah-media-api`，但 wire DTO 与 Rust domain struct 分离。两侧各自实现显式 mapper 和 contract test。

服务分为：

### 2.1 signaling 提供

```text
MediaClusterRegistry.RegisterMediaNode
MediaClusterRegistry.HeartbeatMediaNode
MediaClusterRegistry.DrainMediaNode
```

媒体节点注册内容：node ID、instance epoch、advertised gRPC endpoint、network zones、public/private addresses、RTP port pools、capability versions、codec/transport 支持、capacity 和 build version。

注册返回 lease ID、lease TTL、heartbeat interval、cluster time 和接收的 API version。lease 过期后节点立即停止接受新调度。

### 2.2 media node 提供

```text
MediaCapability.GetCapabilities
MediaQuery.GetMedia / IsMediaOnline / ListSessions
MediaRtp.OpenReceiver / OpenSender / Update / Stop
MediaProxy.CreatePull / DeletePull
MediaRecord.Start / Stop / Query
MediaSnapshot.Take / Query
MediaControl.RequestKeyframe / CloseSession
MediaEventStream.Subscribe
```

所有 mutating RPC 必须包含：tenant、request/correlation ID、idempotency key、deadline、signaling owner node、owner epoch 和 target media node instance epoch。

## 3. MediaKey 与 metadata

`MediaKey` 使用媒体计划的 `{ vhost, app, stream, schema }`：

| 用途 | vhost | app | stream | schema |
| --- | --- | --- | --- | --- |
| 实时 | TenantId 稳定编码 | `live` | ChannelId | 创建时为空 |
| 回放 | TenantId 稳定编码 | `playback` | MediaSessionId | 创建时为空 |
| 下载 | TenantId 稳定编码 | `download` | MediaSessionId | 创建时为空 |
| 对讲 | TenantId 稳定编码 | `talk` | MediaSessionId | 创建时为空 |

TenantId 编码必须可逆、URL safe、大小写稳定，不能使用可修改 tenant name。ChannelId/MediaSessionId 使用 canonical UUID 字符串。

允许的 metadata 白名单：`device_id`、`channel_id`、`media_session_id`、`purpose`、`protocol`、`correlation_id`。禁止：密码、Authorization、完整 RTSP URI userinfo、SIP message、SOAP body、任意嵌套 JSON。

## 4. 媒体节点调度

候选节点过滤顺序：

1. lease 有效且未 drain；
2. API/capability version 兼容；
3. 覆盖设备 network zone 或明确可路由；
4. 支持所需 RTP transport、codec、proxy、record/snapshot 能力；
5. RTP port pool、session、CPU、内存等未超过 hard limit；
6. tenant placement policy 允许；
7. 优先已有同设备绑定或最低归一化负载。

调度开始前必须已有逻辑 MediaSession。调度结果形成短租约 reservation，持久化为 `MediaBinding(Reserved)` 后执行 RPC，成功后再推进为 Active；超时、Operation 取消或协议失败必须释放并终结该 binding。调度不能只依赖最终一致的 metrics，媒体节点仍需在创建 RPC 内原子检查容量。

同一 live ChannelId 默认复用在线媒体资源，而不是每个观看者重新 INVITE。复用由 application 的 live-session coordinator 决定，并受 tenant policy、源健康和 publisher lease 约束。

## 5. GB28181 实时工作流

```text
Create Operation
  -> create MediaSession(Requested)
  -> resolve device owner and channel
  -> select media node
  -> persist MediaBinding(Reserved)
  -> OpenRtpReceiver(idempotency, expected owner epoch)
  -> send INVITE with returned address/port/SSRC policy
  -> receive 200, validate SDP, send ACK
  -> update RTP session if negotiated SSRC/transport differs
  -> wait MediaEvent(StreamOnline) until deadline
  -> MediaBinding Active
  -> MediaSession Active
  -> query output URLs
  -> Operation Succeeded
```

失败补偿按逆序执行：发送 BYE（dialog 已建立时）、StopRtpSession、终结 binding，并按 deadline/policy 终结或重试 MediaSession。补偿命令也使用原 idempotency key 派生键，重复执行必须安全。Operation 终态和 MediaSession 终态相互独立：Start Operation 成功后 MediaSession 正常保持 Active。

设备先发媒体后回 200、SSRC 与 SDP 不一致、TCP 主被动语义反转等 quirks 由 GB module 规范化；媒体节点只按显式 update API 改变 session。

## 6. GB 回放、下载与对讲

- 回放/下载为独立 MediaSessionId，不能覆盖 live MediaKey。
- 时间范围使用 UTC instant + 原始设备时区诊断信息；协议 module 负责转换 GB SDP 时间格式。
- pause/resume/seek/scale 是 Operation 子命令，先校验 session owner 和状态，再分别映射 SIP control 与媒体 playback control。
- 对讲根据设备能力申请 RTP sender 或双向 session。媒体节点处理 packetization，GB module 处理 INVITE/SDP/dialog。
- 任一侧失败后 reconciler 根据 desired state 关闭另一侧，不能留下无信令归属的 RTP session。

## 7. ONVIF 拉流与快照

ONVIF live：

```text
refresh profile/capability if stale
  -> GetStreamUri (Media2 first, Media1 fallback)
  -> validate scheme/host/zone and strip userinfo from logs
  -> issue short-lived credential handle
  -> select reachable media node
  -> CreatePullProxy(source + credential handle + MediaKey)
  -> wait StreamOnline
  -> return media output URLs
```

媒体节点通过受限 secret exchange 获取所需凭据；不得把明文密码写入 persistent request/event。来源 URI 只允许 RTSP/RTSPS 等 capability 声明的 scheme，并执行 DNS/IP SSRF 校验。

快照优先策略：

1. 已有在线 live stream 时调用媒体 `TakeSnapshot`；
2. 无 live stream 且设备支持 SnapshotUri 时，创建受限 snapshot fetch task；
3. 两者都不可用返回 `Unsupported`，不伪装成功。

## 8. 事件与对账

媒体事件包含 event ID、media node instance epoch、media session/handle、MediaKey、sequence、correlation ID 和 typed payload。signaling consumer 使用 event ID 去重，拒绝旧 node instance 的事件覆盖新 binding。

reconciler 周期检查：

- Running Operation 是否有对应 OperationStep、协议 session、MediaSession 和必要的 MediaBinding；
- Active MediaSession 是否有有效 Active MediaBinding，Active MediaBinding 的媒体 node lease/session 是否存在；
- 终态 Operation 是否有未完成步骤；Stopped/Failed MediaSession 是否残留 reservation、RTP/proxy/dialog；
- live 复用计数为零时是否应按 idle policy 关闭；
- drain/故障媒体节点的 binding 是否需要重建。

对账只修复到 MediaSession desired state，不能因旧 Operation 或 Command 重放而无条件重启用户已停止的媒体。

## 9. 媒体仓库迁移要求

`cheetah-media-server-rs` 需要：

1. 实现并冻结 `cheetah-media-api` domain ports；
2. 增加不泄漏 Tokio/Axum 的 gRPC adapter；
3. 增加 media node registry client、lease、capability 和 load heartbeat；
4. 所有 mutating media API 支持 idempotency、deadline、tenant 和 fencing；
5. 媒体事件支持 event ID、重放 cursor 或明确的 gap；
6. 新链路通过真实 GB contract test 后，停用媒体进程内 GB28181 listener/module；
7. 保留 RTP core/driver/module、PS demux 和兼容能力，它们仍属于 Media Plane。
