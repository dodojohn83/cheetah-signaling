# 07. ONVIF 设计

## 1. 角色与 Profile

首版只实现 ONVIF client/controller，不把 Cheetah 模拟为 ONVIF Device。

优先级：

1. Profile T 与 Media2；
2. Media1 compatibility；
3. Profile S legacy compatibility，仅显式开启；
4. Profile G/M 在后续版本独立增加 capability。

Profile 声明来自设备响应和实际探测，不能因某个 scope 字符串就假定所有操作可用。每个 operation 单独记录 supported/unsupported/broken/unknown。

## 2. 架构拆分

### 2.1 core

- WS-Discovery Probe/ProbeMatch/Resolve codec；
- SOAP 1.2 envelope、WS-Addressing、WS-Security UsernameToken；
- Device、Media1、Media2、PTZ、Imaging、Events 所需 DTO；
- Digest challenge 状态和 service workflow 状态机；
- namespace-aware、流式、有界 XML codec；
- request/response correlation、fault mapping。

官方 WSDL/XSD revision、来源、license 和生成方式必须记录。生成 DTO 可提交仓库，但外层 service facade、校验和兼容 normalizer 必须手写并稳定，避免把巨量生成类型暴露给领域层。

### 2.2 driver-tokio

- 指定 interface/zone 的 UDP multicast discovery；
- HTTP/HTTPS connection pool、Digest、TLS、deadline；
- DNS/IP 校验、redirect policy；
- SOAP body streaming 与大小限制；
- PullPoint long poll 和 cancellation。

### 2.3 module

- discovery result 与 Device/Endpoint 合并；
- credential、capability 和 profile 同步；
- Media/PTZ/Event 领域映射；
- network-zone 调度、SSRF policy；
- 兼容 profile、周期轮询和事件去重。

## 3. WS-Discovery

multicast discovery 不能由中心集群跨三层网络假设可达。每个 network zone 部署 discovery-agent，配置允许的 interface、source address、scope filter 和探测窗口。

发现流程：

1. 生成唯一 MessageID，向标准 multicast endpoint 发送 Probe；
2. 在有界窗口接收 ProbeMatch，校验 XML 和 endpoint；
3. 规范化 EPR、Types、Scopes、XAddrs、MetadataVersion；
4. 对每个 XAddr 执行 scheme、DNS/IP、zone 和端口策略；
5. 以 EPR 为主候选身份，序列号/MAC/厂商信息作为合并信号；
6. 创建或更新 pending Device，后续鉴权成功才标记 managed。

相同设备返回多个 XAddr 时保存为有优先级 endpoints。EPR 变化、重复 MAC、克隆设备和 NAT 场景不得自动破坏性合并，需产生 merge candidate。

主动发现必须有 tenant/zone 级频率限制，避免 UDP 放大和网络风暴。

## 4. 鉴权与时间

首选 HTTPS + HTTP Digest。兼容设备可使用 WS-Security UsernameToken Digest；PasswordText 默认禁止。Profile S legacy 必须在设备/tenant policy 显式开启并产生安全审计。

UsernameToken 时间戳依赖设备/客户端时钟。首次连接先获取 `GetSystemDateAndTime`，计算有上限的 offset，仅用于该设备 SOAP security header；不能修改系统全局时钟。

TLS policy 支持：

- strict PKI；
- tenant 私有 CA；
- 首次指纹登记后 pin；
- 明确标记的 insecure HTTP legacy。

跳过证书校验不能作为全局默认值。

## 5. 设备与能力同步

标准同步流程：

```text
GetSystemDateAndTime
  -> GetDeviceInformation
  -> GetCapabilities / GetServices
  -> Media2 GetProfiles (fallback Media1 GetProfiles)
  -> PTZ/Imaging/Event service capabilities as advertised
  -> upsert endpoints, channels, profiles and capabilities
```

每个 service endpoint 独立校验。设备返回内网别名、错误 host 或不可达 XAddr 时，可按 compat policy 将 host 替换为已连接 endpoint，但必须记录修复原因，不能接受跨 zone 地址。

Profile token 是协议外部 ID，不是 ChannelId。通道合并优先使用 video source token，再结合 profile、encoder source 和厂商信息。多个 profile 可指向同一 Channel，作为不同 StreamProfile capability 保存。

同步使用 revision 和 diff，避免每次轮询重写全部通道。删除需连续多次缺失或显式确认，防止设备临时返回空列表导致资产清空。

## 6. Media2/Media1

Media2 是主路径；设备未声明、返回 ActionNotSupported 或已知 broken profile 时 fallback Media1。fallback 结果写入 capability diagnostics，不在每次请求重复试错。

首版规范化：

- video/audio source 与 encoder configuration；
- codec、resolution、framerate、bitrate、quality；
- profile token 与 channel mapping；
- RTP unicast、RTSP over TCP/HTTP transport hints；
- GetStreamUri/GetSnapshotUri；
- multicast 等非默认能力只声明，不自动启用。

stream URI 进入 media workflow 前移除日志中的 userinfo，并执行 SSRF/zone 校验。不得由 signaling 自行打开 RTSP 媒体连接。

## 7. PTZ 与 Imaging

PTZ 支持：GetConfigurations/GetStatus、ContinuousMove、RelativeMove、AbsoluteMove、Stop、Preset 查询/设置/跳转（以设备能力为准）。

统一 PTZ 命令归一到 `[-1.0, 1.0]` 速度/位置语义，再由 module 根据 spaces/ranges 转换。超范围拒绝或按显式 policy clamp，并返回 diagnostics。

ContinuousMove 必须带 timeout 或由服务端设置最大持续时间，超时自动 Stop。Operation 取消和 owner 丢失也要尽力 Stop。

Imaging v1 读取 settings/capability；写入曝光、焦距等高级设置不在默认范围，避免通用 API 在未知设备上造成不可逆配置变化。

## 8. Events

优先 PullPoint：

```text
CreatePullPointSubscription
  -> persist subscription summary
  -> PullMessages loop
  -> Renew before termination
  -> normalize notifications
  -> Unsubscribe on stop
```

subscription owner 由 lease 控制，同一 device/topic policy 只允许一个有效 worker。worker 故障后新 owner 创建新 subscription；不假设旧 subscription 可跨节点迁移。

事件规范化保存 topic、source/data simple items、property operation、UTC time、device/channel mapping 和厂商 extension。去重 key 由 device、topic、message identity/time 和 normalized payload hash 组成，窗口有上限。

PullMessages 的 MessageLimit、Timeout、续期提前量和失败退避必须可配置。空结果是正常状态；SOAP fault、鉴权失败和网络 timeout 分开统计。

## 9. 兼容与安全

ONVIF compat profile 集中处理：

- SOAP 1.1/1.2 content type 差异；
- WS-Addressing namespace/action 差异；
- Digest qop/algorithm/nonce quirks；
- clock skew；
- 错误或相对 XAddr、stream URI；
- XML namespace、空字段、非法枚举；
- Media2 部分实现和 Media1 fallback；
- PullPoint termination/renewal 异常。

XML 必须禁止 DTD、外部实体和无限 entity expansion，限制深度、属性、文本、列表和总响应体。redirect 每一跳重新做 SSRF 检查。连接设备返回的 URI 不能访问 loopback、link-local、云 metadata 或其他 tenant zone，除非部署策略明确允许。

## 10. 未来扩展

- Profile G：录像 job、recording/replay/search，与统一 Record/Playback 领域模型对接；
- Profile M：analytics metadata、规则和 MQTT 事件；
- TLS Configuration Add-on；
- ONVIF Device/relay facade 作为独立协议角色，不与 client module 混写。
