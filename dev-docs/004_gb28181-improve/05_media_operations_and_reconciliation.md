# 05. GB28181 媒体 Operation、Saga 与对账

## 1. 目标与边界

将现有 INVITE/ACK/CANCEL/BYE/INFO 状态机接入 application Operation 和版本化 MediaPort，完成 live、playback、download、talk 的控制面闭环。

本仓库只编排媒体资源：

- 可以生成/解析 SIP 与 SDP，调用 MediaPort，保存 MediaSession/Binding，等待媒体事件；
- 禁止绑定 RTP/RTCP 端口、接收/解析 PS/TS/ES、RTSP 拉流、codec、录制或播放 URL 生成；
- SSRC、RTP receiver/sender、stream detection 和 media node handle 的物理执行都属于 media server。

## 2. 四模型职责

| 模型 | GB28181 用途 |
| --- | --- |
| Operation | 一次 Start/Stop/Control 的可查询、取消、超时工作流；异步状态唯一权威来源 |
| Command | 派发到 owner GB runtime 的不可变 typed 指令 |
| MediaSession | 用户视角的 live/playback/download/talk 逻辑意图、desired state 和 generation |
| MediaBinding | 与具体 media node instance、MediaKey、receiver/sender handle 的物理关联 |

Start 成功后 Operation 终态，MediaSession 可继续 Active。Stop、seek、scale、pause、resume 必须创建新 Operation 引用原 MediaSession。

## 3. 公共上下文

每个媒体修改请求、SIP command 和回调必须携带或可关联：

- tenant、operation/step、command、media session/binding ID；
- request/correlation ID、idempotency key、deadline；
- device/channel ProtocolIdentity 与内部 typed ID；
- signaling owner node、owner epoch、session generation；
- media node ID、media node instance epoch、MediaKey/handle；
- compatibility profile revision。

缺少 fencing 或 deadline 的修改型 MediaPort 请求不得发出。

## 4. Live Saga

固定步骤：

1. 校验 Device/Channel capability、online ProtocolSession、owner epoch 和目标 transport policy；
2. 创建 Operation、MediaSession generation 和 pending step；
3. MediaPort `OpenRtpReceiver`，获得目标 media node、instance epoch、IP/port、SSRC/MediaKey/handle；
4. GB media machine 生成 INVITE/SDP 和 client transaction；
5. 处理 100/180/183、非 2xx、timeout、CANCEL 竞态和 200；
6. 校验 200 SDP 的 connection、media、payload、SSRC、setup/connection role；
7. 发送 ACK；
8. MediaPort `UpdateRtp` 提交协商结果；
9. 等待匹配 tenant/binding/media instance epoch 的 StreamOnline；
10. 原子推进 MediaBinding Active、MediaSession Active、Operation Succeeded。

补偿：

- Open 成功、INVITE 前失败：CloseRtpReceiver；
- INVITE pending 取消：CANCEL，等待 487/late 200；late 200 必须 ACK 后 BYE；
- 200/ACK 后 UpdateRtp 失败：BYE + Close；
- StreamOnline timeout：BYE + Close，Operation Failed/TimedOut；
- 设备先发媒体：StreamOnline 可暂存，但只有 dialog 和 binding fencing 一致后才能成功；
- duplicate 200：重发 ACK，不重复 Update/成功事件；
- outcome 无法确认：OperationStep UnknownOutcome，reconciler 查询两侧状态。

## 5. Stop Saga

- 根据 MediaSession generation 找到唯一有效 binding/dialog；
- pending INVITE 发送 CANCEL；active dialog 发送 BYE；
- SIP failure 不阻止有界 media cleanup，但记录 device-side UnknownOutcome；
- MediaPort Close 使用原 media node instance epoch 和幂等键；
- late StreamOnline、late 200、duplicate BYE/response 不能复活终态 binding；
- Stop 完成后 MediaSession Stopped，旧 binding terminal，不复用其 handle。

## 6. Playback 与 Download

- SDP `s=`、`t=`、`u=`、SSRC、downloadspeed 等由 typed encoder 生成；
- wall-clock recording range 与 monotonic operation deadline 分离；
- playback/download 使用独立 MediaSession purpose 和 generation；
- RecordInfo item 的 protocol record identity 不直接作为内部 session/SSRC ID；
- MANSRTSP INFO 支持 Play/Pause/Scale/Seek，使用 dialog CSeq 和新的 transaction branch；
- INFO 2xx/错误/timeout 映射控制 OperationStep；发送后无响应按命令安全性决定 UnknownOutcome；
- MediaStatus NotifyType=121 只终止匹配 dialog/generation 的 playback/download，不影响新 binding。

## 7. Talk 与 Broadcast

1. 查询 device/channel 和 media node 的 talk、codec、transport capability；
2. 创建 MediaSession/Binding；
3. MediaPort 打开 RTP sender/talk resource；
4. 对需要 Broadcast MESSAGE 的 profile 先完成 typed Broadcast handshake；
5. 发起 audio sendonly/sendrecv INVITE；
6. 校验远端 SDP codec/payload/transport，ACK 后更新 media sender；
7. 任一步失败按 sender + dialog 双侧补偿；
8. Stop/BYE、设备 BYE、media node restart 都收敛到 terminal binding。

不支持的 codec/transport 在副作用前返回稳定 Unsupported，不临时转码或回退未声明 codec。

## 8. SDP 与网络安全

- SDP parser/encoder 继续位于 core，所有字段有行数、长度、payload 数量和地址限制；
- 不信任设备 SDP 的任意目标地址；MediaPort 的 receiver/sender endpoint 是允许地址的权威来源；
- remote SDP address 与 observed device/network zone 不一致时按 strict policy 拒绝，只有有证据的 compatibility profile 可允许受限改写；
- 不把完整 SDP、Contact userinfo、媒体 credential 或私网拓扑写入普通日志；
- SSRC 冲突由 MediaPort/Binding 幂等处理，signaling 不扫描 media engine 或复制 SSRC allocator。

## 9. Reconciler

按 bounded cursor batch 扫描非终态 Operation、desired/actual 不一致 MediaSession 和可疑 binding：

- 验证 signaling owner epoch；非当前 owner 不执行副作用；
- 查询 media node lease/instance epoch 和 binding handle；
- 根据 ProtocolSession/dialog 持久化摘要判断是否可继续、补偿或 UnknownOutcome；
- media node instance 变化时旧 binding terminal，新 generation 创建新 binding；
- device/signaling/media crash window 均有幂等 step key；
- 循环具有 cancellation、batch 上限、退避、jitter、积压和最老年龄指标。

## 10. 依赖关系

| 本任务 | 必须依赖的 003 交付 |
| --- | --- |
| typed MediaPort request/event | MED-C-001..008 |
| real media registry/client/readiness | MED-R-001..008、UP-MEDIA-P0 |
| Operation/MediaSession/Binding Saga | WF-001..008 |
| owner/inbox/outbox/reconcile | ASM/PROD 对应任务 |

依赖未满足时：

- 可以完成 core state table、module mapper、fake MediaPort contract 和逐 step 故障测试；
- real media integration、StreamOnline 和 release interop 保持 `Blocked`；
- 不得用 fake 或固定事件把任务标记 Completed。

## 11. 实施任务

- [~] `GB4-MED-001`：live start/stop 状态表和 INVITE/ACK/BYE/CANCEL 会话机已在 `media` 模块实现；`GbMediaMachine` 迁入 core 并接入真实 transaction/dialog/timer driver 仍 `Blocked`（依赖 MED-R）。见 `reports/gb4-med-001-004.md`。
- [~] `GB4-MED-002`：typed GB command mapper（`media/mapper.rs`：`map_start`/`map_control`）已完成，把已 fencing 的控制面意图映射为 `MediaCommand`；OperationStep 与 MediaPort Open/Update/Close Saga 已在 application 层实现。driver 接线待 MED-R。
- [~] `GB4-MED-003`：UDP、TCP active/passive、SSRC/payload/setup 协商已在 SDP encoder 和 mapper 实现；compatibility profile 改写策略待 real 设备样本。
- [~] `GB4-MED-004`：playback/download/MANSRTSP（Play/Pause/Seek/Scale/Teardown）wire 生命周期与 golden fixture 已完成；RecordInfo→session 映射与 MediaStatus NotifyType=121 收敛待 driver 接线。
- [x] `GB4-MED-005`：完成 talk/Broadcast 与 media sender capability、双侧补偿。
- [x] `GB4-MED-006`：实现 late/duplicate/early media、CANCEL/BYE、timeout/cancel/UnknownOutcome 状态迁移。
- [x] `GB4-MED-007`：接入 reconciler，覆盖 signaling/device/media restart 和旧 epoch/instance callback。
- [x] `GB4-MED-008`：fake 与 real media node 运行同一 GB media contract/system suite。

## 12. 测试与退出门禁

- 每个 Saga step 注入失败、timeout、cancel、duplicate 和 crash-after-side-effect。
- INVITE 覆盖 provisional、non-2xx、late/repeated 200、ACK loss、CANCEL/487、device BYE 和 BYE response loss。
- StreamOnline/Offline 覆盖早到、迟到、重复、错误 tenant/binding/owner/media instance epoch。
- playback/download 覆盖时间边界、seek/scale 注入、INFO response、MediaStatus 和新旧 generation。
- talk 覆盖 codec unsupported、sender open/update/close failure、Broadcast timeout 和设备先 BYE。
- fake/real MediaPort contract 结果一致；real report 固定 media server commit/config。
- 抓包证明 signaling 只处理 SIP/SDP，不绑定 RTP/RTCP 或包含媒体 payload。
- terminal Operation 无 pending step；Stopped/Failed MediaSession 无有效 binding。

