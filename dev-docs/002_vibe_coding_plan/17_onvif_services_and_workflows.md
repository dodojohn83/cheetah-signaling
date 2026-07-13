# 17 ONVIF 服务与业务工作流

## 1. 目标

实现 ONVIF Device、Media/Media2、PTZ 和 Events 的首版工作流，并转换为统一设备、通道、命令和事件模型。设备返回的 token 视为不透明字符串，不解析其内部含义。

## 2. Device Management

### ONVIF-SVC-001：设备信息与时间

- [ ] GetDeviceInformation 映射 manufacturer/model/firmware/serial/hardware ID。
- [ ] GetSystemDateAndTime 映射时区、UTC 与漂移。
- [ ] GetHostname/GetNetworkInterfaces 仅用于诊断，敏感网络信息按权限返回。
- [ ] SystemReboot 创建异步 Operation，再派发 typed Command；设备断线不直接伪造成功或失败。

### ONVIF-SVC-002：纳管流程

候选审批 → 保存凭据引用 → 探测服务 → 拉取 profile/channel → 生成能力 → 标记 Active。任何步骤失败保存阶段和可重试原因；凭据本身不写入设备 metadata。

## 3. Media / Media2

### ONVIF-SVC-003：Profile 与 URI

- [ ] 优先使用 Media2，设备不支持时回退 Media1。
- [ ] 拉取 Profiles、VideoSourceConfigurations、VideoEncoderConfigurations。
- [ ] profile token 映射为稳定 Channel/Profile 标识并保留原 token。
- [ ] GetStreamUri 的 URI 在使用前执行协议和目标地址策略校验。
- [ ] URI 中 userinfo 在日志、API 和事件中删除或遮蔽。

### ONVIF-SVC-004：实时预览

ONVIF 通常提供 RTSP URI；信令层把 URI 及凭据引用交给媒体节点，由媒体节点拉流。工作流：创建 Operation/MediaSession → 调度支持 RTSP pull 的媒体节点 → 创建 MediaBinding → 获取短期 StreamUri → 创建媒体拉流任务 → 等待 ready → 激活 binding/session → 完成 Operation。

- [ ] StreamUri 获取尽量接近使用时刻，处理 InvalidAfterConnect/Reboot 等约束。
- [ ] 不把明文 URI 凭据持久化到 media session。
- [ ] stop 创建新的 Operation、设置 MediaSession desired state，并只控制 MediaBinding 对应媒体节点资源，不向设备伪造 RTSP 控制。

### ONVIF-SVC-005：快照

快照策略固定为：已有 live stream 时优先调用媒体节点 `TakeSnapshot`；否则获取设备 SnapshotUri 并让媒体节点执行受限 fetch；两者都不可用返回 `Unsupported`。

- [ ] SnapshotUri 与 StreamUri 使用相同的凭据脱敏、SSRF、DNS/IP 和 scheme 校验。
- [ ] 快照结果由媒体节点保存/返回，信令进程不解码图像负载。
- [ ] 请求设置最大响应大小、deadline、内容类型和租户存储策略。
- [ ] 重复请求是否复用结果由显式 cache key/TTL 决定。
- [ ] TakeSnapshot Operation、临时/复用 MediaSession 和 MediaBinding 的创建/清理规则必须显式，不得留下长期孤儿 binding。

## 4. PTZ

### ONVIF-SVC-006：PTZ 能力与命令

- [ ] 拉取 PTZ configurations、nodes、spaces 和 presets。
- [ ] ContinuousMove、RelativeMove、AbsoluteMove、Stop、GotoPreset 映射统一 PTZ 命令。
- [ ] 在发送前按设备 space 范围归一化/裁剪，无法映射则拒绝。
- [ ] ContinuousMove 必须自动安排 stop deadline，客户端断开不能导致持续运动。
- [ ] 对每设备串行化相互冲突的 PTZ 命令。

### ONVIF-SVC-007：Imaging 基础读取

- [ ] 读取 Imaging capabilities、GetImagingSettings 和 GetOptions。
- [ ] 映射亮度、色彩、对比度、曝光、聚焦等可安全表达的只读诊断值。
- [ ] v1 默认不写入曝光/焦距等可能不可逆设置；写请求返回 `Unsupported`。
- [ ] VideoSourceToken 与 Channel/Profile 映射有一致性检查。

## 5. Events

### ONVIF-SVC-008：PullPoint

- [ ] CreatePullPointSubscription，保存 endpoint、termination time 和 generation。
- [ ] 定期 PullMessages，设置 message limit、timeout 和 body 上限。
- [ ] 在到期前 Renew；失败时按策略重建订阅。
- [ ] 停机/退休设备时 Unsubscribe，失败交给过期机制。

### ONVIF-SVC-009：事件归一化

- [ ] 解析 Topic、Source、Key、Data、UtcTime、PropertyOperation。
- [ ] 映射移动侦测、数字输入、视频丢失等已知 topic。
- [ ] 未知 topic 转为 `vendor.onvif` 事件并限制扩展大小。
- [ ] 以设备、subscription generation、消息摘要去重。
- [ ] 事件经 outbox 发布，不能从拉取 task 直接推给 HTTP 客户端。

## 6. 故障恢复

- [ ] HTTP 连接失败、SOAP Fault、认证失败、设备忙分别分类。
- [ ] 只对幂等读取和明确安全操作自动重试。
- [ ] 设备 endpoint 变化触发重新探测，不直接覆盖正在使用的 endpoint。
- [ ] subscription 在 owner 迁移后由新 owner 重建，旧 generation 事件被拒绝。

## 7. 兼容与测试矩阵

建立 ONVIF 模拟服务，支持可配置：Media1/Media2、时钟漂移、WSSE 模式、SOAP Fault、慢响应、订阅过期、畸形扩展和 StreamUri 变化。

- [ ] 纳管全流程及部分能力失败。
- [ ] Media2 → Media1 回退。
- [ ] PTZ 坐标空间转换和自动 Stop。
- [ ] PullPoint 创建、拉取、续订、重建和去重。
- [ ] RTSP URI 安全处理与媒体节点联调。
- [ ] Snapshot 两级策略、SSRF/大小限制及 Imaging 只读映射。
- [ ] 至少两个真实厂商设备的脱敏 golden tests；未实测项明确标注。

## 8. 验收标准

- ONVIF handler 不依赖 Axum handler 或具体数据库。
- 设备凭据和带凭据 URI 不出现在持久化明文字段、日志或事件中。
- 单项服务失败不破坏已成功发现的其他能力。
