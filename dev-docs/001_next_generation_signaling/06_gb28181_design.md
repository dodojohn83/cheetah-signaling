# 06. GB28181 设计

## 1. 标准与角色

主规范为 GB/T 28181-2022，兼容 GB/T 28181-2016 的实际设备行为。首版同时支持：

- SIP server：IPC、NVR、下级平台注册到 Cheetah；
- SIP client：Cheetah 向上级平台注册并共享目录；
- 级联桥：上级点播/控制映射到本地设备或下级平台。

标准版本、厂商、型号和 firmware 共同决定兼容 profile。兼容逻辑必须集中在 `compat` 层，不能散落在 parser、driver 和 API handler。

## 2. crate 边界

### 2.1 core

- 宽容但有界的 SIP parser/serializer；
- Via/From/To/Contact/Call-ID/CSeq/Route/Record-Route/Expires/Content-Type；
- Digest challenge/verification；
- INVITE client/server transaction、ACK、CANCEL、BYE；
- MESSAGE、SUBSCRIBE、NOTIFY transaction；
- GB XML command codec；
- SDP offer/answer、transport/SSRC/time range；
- registration、dialog、subscription 的 Sans-I/O 状态机。

core 不保存 Device 领域实体，只保存推进协议所需的 session state。所有 timeout 都输出 timer action，由 driver 注入时间。

### 2.2 driver-tokio

- UDP/TCP listener 与 outbound connection；
- TCP SIP framing 和半包/粘包；
- UDP response address、NAT observed address；
- 有界 send queue、transaction timer 驱动；
- socket option、连接限流和 metrics；
- 将 connection handle 与分片 routing key 关联。

### 2.3 module

- tenant/realm/DeviceId 解析和鉴权；
- 注册 owner CAS、presence 和设备目录同步；
- 领域命令与 GB XML/SDP 映射；
- 媒体 Saga、级联映射、权限和审计；
- 厂商兼容 profile 的选择和诊断。

## 3. 注册与保活

注册流程：

1. 根据 Request-URI、To domain、listener mapping 选择 tenant/realm；不允许只用 From ID 猜 tenant。
2. 校验报文限制和设备 ID 格式；未知设备按 tenant onboarding policy 拒绝、隔离或自动创建 pending asset。
3. 无/无效 Authorization 返回 401 challenge；nonce 包含时效和服务端签名，防止伪造。
4. 校验 Digest、nonce、realm、username、URI、qop/nc（设备支持时）和重放。
5. CAS 获取 ownership epoch；重复 REGISTER 同 Call-ID/CSeq 幂等，真正新 session 替换旧 owner。
6. 回复 Contact/Expires，并发布 DeviceRegistered/OnlineChanged。

`Expires: 0`、Contact expires=0 和管理员禁用均触发注销。保活 MESSAGE 成功只更新内存 presence；只有 online/offline 转换和汇总才持久化。

offline deadline 由 tenant/profile 配置，结合 REGISTER expiry、keepalive interval 和容忍次数计算。不得用全局固定 60 秒覆盖所有设备。

## 4. XML 命令

v1 支持：

| CmdType/场景 | 方向 | 领域映射 |
| --- | --- | --- |
| Keepalive | device → server | presence |
| Catalog | 双向 | Channel upsert/tree、级联目录 |
| DeviceInfo | query/response | Device reported info |
| DeviceStatus | query/response | reported state |
| DeviceControl | server → device | PTZ、guard、record 等受支持子命令 |
| RecordInfo | query/response | playback source list |
| Alarm | device → server | AlarmReceived |
| MobilePosition | notify/query | PositionUpdated |
| Broadcast | server → device | talk/broadcast setup |
| ConfigDownload/DeviceConfig | 可选能力 | 显式 capability，不作为首版强制 |

XML parser 必须 namespace tolerant、禁止 DTD/外部实体、限制深度/文本/条目数量，并保留原始字段位置用于安全诊断。未知字段可进入受限 compat diagnostics，不能整包存库。

目录响应可能分片、乱序、重复或 `SumNum` 不准确。module 使用 query SN + deadline 汇聚，按 protocol channel ID 幂等 upsert；达到上限、超时或显式完成时结束，并报告 partial result。

## 5. 实时点播

状态机：

```text
Requested -> MediaReserved -> InviteSent -> Early
          -> DialogConfirmed -> WaitingMedia -> Streaming
          -> Stopping -> Stopped
          -> Failed
```

- 先申请媒体 receiver，再构造 SDP。
- Call-ID、From tag、branch、CSeq、Subject 和 SSRC 映射必须稳定且可审计。
- 200 SDP 需要校验 media address、port、transport、payload 和 SSRC；不信任设备返回地址可直接访问任意网络。
- ACK 后等待媒体 online event；SIP 成功不等同于播放成功。
- 用户停止、无人观看策略、设备 BYE、媒体 timeout 和 owner 丢失都进入统一终止路径。
- CANCEL 只用于未建立 dialog；已建立 dialog 使用 BYE。

同一 ChannelId 的 live source 默认复用。并发 StartLive 在 coordinator 上按 `(tenant, channel, source policy)` single-flight，调用者获得各自 subscriber/output session。

## 6. 回放与下载

- RecordInfo 查询是独立 Operation，按设备能力分页/分片汇聚。
- StartPlayback/Download 使用独立 MediaSessionId 与 MediaKey。
- 时间先保存为 UTC，module 按设备时区和 profile 转换；时钟偏移进入 diagnostics。
- pause/resume/scale/seek 仅在已建立的 compatible session 上开放；设备不支持时返回 Unsupported。
- 下载进度来自媒体节点和协议状态的组合，不从 SIP dialog 单独推断。

## 7. PTZ、告警与对讲

PTZ API 使用方向、速度、持续时间、preset 等具名字段。module 生成标准控制字并校验 checksum；持续移动命令必须有 server-side auto-stop deadline，客户端断线不能导致云台永久移动。

告警按 tenant/device 去重策略产生 typed event，保留 alarm method/type/status/time 等规范字段。原始 XML 只可在显式 debug capture 中短期、加密、脱敏保存。

对讲先协商设备能力和音频 codec，再申请媒体 RTP sender/双向 session。音频 packetization 和转码属于媒体面；Broadcast/MESSAGE/INVITE/dialog 属于信令面。

## 8. 平台级联

`PlatformLink` 定义上级或下级平台、SIP identity、realm、credential ref、local/remote endpoint、keepalive、catalog policy、ID mapping 和 event policy。

向上级：

- 独立 registration 状态机，指数退避并尊重 Retry-After/expiry；
- 按共享策略生成虚拟目录，不泄漏未授权 tenant/device；
- 处理上级 Catalog/Invite/DeviceControl/RecordInfo；
- 将本地 Alarm/Online change 映射为 Notify/Message。

下级平台：

- 作为特殊 Device/Platform 聚合接入；
- 目录项映射到内部 Device/Channel，并保留 source link；
- 防止多级目录环和 ID 冲突；
- 上级 INVITE 创建新的桥接 Operation，但媒体仍直接由媒体节点接收/发送。

级联路由必须有 hop count、visited platform IDs 和 policy deadline，避免环路。

## 9. 兼容策略

兼容 profile 以 `(standard version, manufacturer, model pattern, firmware range)` 匹配，内容包括：

- 行终止、重复 header、header 大小写；
- Digest 参数分隔、URI 差异；
- Contact/NAT 地址选择；
- XML namespace/encoding/非法字符修复；
- SDP transport、SSRC、payload 和 TCP mode 差异；
- 先媒体后应答、重复 response/BYE；
- Catalog SumNum/DeviceList 异常。

每个 quirk 必须有名称、风险、适用范围、默认开关、fixture 和回归测试。不能使用“兼容所有错误”的全局宽松模式。

## 10. 现有实现迁移

媒体仓库的现有 GB core 可提取 parser/SDP/fixture 思路，但其直接接受 REGISTER、内存设备表和媒体 engine module 不是目标架构。迁移时：

1. 用 capture/golden test 固定已验证兼容行为；
2. 在 signaling repo 重建完整 Sans-I/O core，不建立对媒体 crate 的运行时依赖；
3. 新 signaling + media gRPC 路径通过双跑测试；
4. 禁止两个生产 SIP listener 同时对同一设备宣告 owner；
5. 切流完成后移除媒体进程内信令入口。
