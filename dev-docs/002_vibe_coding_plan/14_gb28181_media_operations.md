# 14 GB28181 实时、回放与媒体操作

## 1. 目标

把统一媒体工作流映射为 GB28181 SIP/SDP 操作。信令服务器生成和解析 SDP、维护 INVITE dialog，并将媒体接收/发送端点来自 `media-control` 的预留结果；RTP/PS 数据不经过本进程。

## 2. SDP 模型

实现受限、强类型 SDP：session、connection、time、media、rtpmap、fmtp、setup、connection、ssrc/y 等 GB 相关属性。未知属性有限保留。

- [ ] 支持 UDP、TCP active/passive 的配置与能力协商。
- [ ] 支持 PS over RTP 的 payload/clock 描述。
- [ ] SSRC 作为字符串和数值双重校验，保留前导语义所需格式。
- [ ] IP 地址选择来自媒体节点公告端点，不使用信令监听地址推断。
- [ ] 解析限制行数、行长和总大小。

## 3. 实时预览

### GB-MED-001：StartLive

1. 应用服务原子创建 Pending `Operation`、Requested `MediaSession` 和 outbox。
2. worker 将 Operation 置为 Running，调度节点并创建 Reserved `MediaBinding`。
3. 通过 MediaBinding 预留媒体接收端口/传输模式。
4. 生成 subject 与 SDP，发送 INVITE。
5. 处理 provisional/final response，解析设备 SDP。
6. 发送 ACK，把协商结果确认给媒体节点。
7. 媒体节点上报流就绪后依次置 MediaBinding/MediaSession 为 Active，Operation 为 Succeeded。

- [ ] 每步写入 OperationStep、session/binding revision 和可恢复外部 handle。
- [ ] 设备 200 OK 但媒体确认失败时发送 BYE，释放资源并终结 binding；是否重试由 MediaSession deadline/policy 决定。
- [ ] 重复 start 使用业务幂等作用域返回原 Operation/MediaSession。
- [ ] INVITE 超时/CANCEL 竞争通过事务状态机裁决。

### GB-MED-002：StopLive

- [ ] StopLive 创建新的 Operation 并引用既有 MediaSession，将 desired state 设置为 Stopped。
- [ ] Active dialog 发送 BYE；尚在 INVITE 阶段发送 CANCEL。
- [ ] 无论设备是否响应，宽限期后释放媒体资源。
- [ ] 释放成功后 MediaBinding 为 Released、MediaSession 为 Stopped、Stop Operation 为 Succeeded。
- [ ] 重复 stop 返回原 Operation 或已停止结果，不创建重复副作用。
- [ ] 设备主动 BYE 时同步停止媒体会话。

## 4. 录像回放

### GB-MED-003：Playback

- [ ] 使用标准时间范围和 playback subject 生成 SDP。
- [ ] 媒体节点资源预留包含回放速率控制能力。
- [ ] 将设备返回的 RTP 参数传给媒体节点。
- [ ] 播放结束、设备 BYE、客户端 stop 均收敛到同一终止流程。

### GB-MED-004：Playback Control

- [ ] 实现 INFO/MANSRTSP 的 Play、Pause、Teardown、Scale、Range 子集。
- [ ] 串行化同一 dialog 的 CSeq 和控制命令。
- [ ] 应答与 operation/step/command ID 关联，乱序响应不覆盖 Operation、MediaSession 或 MediaBinding 新状态。
- [ ] 对不支持能力返回明确错误，不模拟成功。

## 5. 下载与语音对讲

### GB-MED-005：录像下载

- [ ] Download 使用独立 MediaSessionId/MediaKey，不覆盖 playback/live。
- [ ] UTC 时间范围按设备时区/profile 转换，原时区和时钟偏移只进入 diagnostics。
- [ ] 媒体节点提供下载接收、进度和完成事件；进度不得只由 SIP dialog 推断。
- [ ] 取消、设备完成、媒体失败和 deadline 到期进入同一终止/补偿流程。

### GB-MED-006：语音对讲

- [ ] 先探测设备音频 codec、采样率、通道数和 Broadcast/INVITE 能力。
- [ ] 向媒体节点申请 RTP sender 或双向 session，由媒体面完成 packetization/转码。
- [ ] 信令层处理 Broadcast MESSAGE、INVITE/SDP/dialog 及业务响应。
- [ ] 任一侧失败后 reconciler 关闭另一侧，不留下无信令归属的 RTP session。
- [ ] 不支持的 codec/模式返回稳定 `Unsupported`，不得创建半成品会话。

## 6. NAT 与地址策略

- [ ] 区分信令 observed address、设备 Contact、SDP connection 和媒体公告地址。
- [ ] 地址重写由显式策略决定，并在决策日志记录规则名。
- [ ] 不在信令层中继媒体以“解决”NAT。
- [ ] IPv4 首版完整支持，IPv6 类型和配置可表达并有解析测试。

## 7. 测试矩阵

- [ ] UDP/TCP active/TCP passive 三种媒体协商。
- [ ] 1xx、200、4xx、超时、晚到 200、重复 200。
- [ ] 客户端 stop 与设备主动 BYE 竞争。
- [ ] 媒体节点预留失败、确认失败、流就绪超时和回调重复。
- [ ] 下载完成/取消/断线及对讲 codec 协商、半开会话和双向失败。
- [ ] 服务重启后依据 dialog/session 信息对账清理。
- [ ] SDP corpus 与 fuzz 测试。

## 8. 验收标准

- 任一失败路径最终释放媒体资源且 Operation、MediaSession、MediaBinding 各有确定状态。
- SIP dialog、MediaSession 和 MediaBinding 通过强类型 ID 关联但生命周期不耦合。
- 抓包确认所有 RTP 目的地址均为媒体节点而非信令节点。
