# 02. 参考实现能力、兼容与鲁棒性分析

## 1. 分析方法

参考项目不作为架构模板，也不作为标准文本替代品。分析只提取：

- 已实际出现的 SIP method、MANSCDP/MANSRTSP 命令和级联场景；
- 设备、NVR、上下级平台常见的非标准差异；
- transaction、registration、catalog、media session 和恢复所需的状态；
- 需要转换为 fixture、simulator fault 和 interoperability case 的行为；
- 明确不能带入本仓库的媒体耦合、安全和资源治理反模式。

参考版本和许可证见 [90](90_reference_provenance_and_license.md)。

## 2. 能力矩阵

| 参考项目 | 值得吸收的信令能力 | 鲁棒性线索 | 禁止照搬 |
| --- | --- | --- | --- |
| sipsdk | reSIProcate transaction/dialog；REGISTER/INVITE/MESSAGE/ACK/BYE/CANCEL/INFO/SUBSCRIBE/NOTIFY；in-dialog callback；MANSCDP/MANSRTSP 与 vendor MIME | 将 invite early/confirmed/terminated、INFO、subscription、notify 分开回调 | 旧代码、空 factory、无仓库许可证；不得复制源码或依赖其宽松解析行为 |
| WVP | UDP/TCP、设备注册、目录、状态、报警、位置、PTZ/预置位、回放下载、语音、级联、多上级、订阅和虚拟目录 | 独立 request/response processor、catalog aggregation、invite session、SSRC、platform keepalive/registration manager | Spring/Redis/数据库直接耦合 handler；不能把 media server 内部对象带入 signaling |
| AKStream | IPv4/IPv6、UDP/TCP、GB2016、ConfigDownload、Preset、Broadcast、MediaStatus、记录查询和 SIP client/server 双角色 | NeedResponse/Call-ID 关联、keepalive lost/re-register、设备/通道远端 endpoint | 每设备线程、Thread.Sleep/Abort、AutoResetEvent、static map、完整 SIP 日志、媒体节点直接调用 |
| GB28181.Solution | registrar queue、binding、duplicate register、strict realm、Contact/NAT、UDP/TCP/TLS、transaction engine | queue 上限、minimum expiry、binding refresh、remote endpoint 变化和 NAT keepalive 场景 | 仓库自述非生产就绪；许可证混合；包含 RTP/RTSP/audio 与每设备 monitor，不得进入控制面 |
| simple-media-server | Keepalive/Catalog/DeviceInfo/PTZ/Preset/HomePosition/RecordInfo/Config/Alarm/MobilePosition/MediaStatus/Broadcast；source endpoint 更新 | 目录超时、设备重复接入、Call-ID/SSRC/media status 关联、上下级角色 | 记录完整 XML、密码/digest 诊断、每设备 timer、信令媒体耦合和 RTP/PS 代码 |

## 3. 标准能力差距

### 3.1 SIP 与接入

| 能力 | 当前状态 | 目标任务 |
| --- | --- | --- |
| UDP REGISTER/MESSAGE | `Partial`，已进生产但未接 transaction、owner、tenant router | GB4-SIP、GB4-ACC |
| TCP SIP | `Not Implemented` | GB4-SIP-001 |
| OPTIONS | core method 存在，生产 access 返回 501 | GB4-SIP-003 |
| 主动 MESSAGE/query/control | XML builder 局部存在，无生产 command sender | GB4-CMD |
| INVITE/ACK/CANCEL/BYE/INFO | module 状态机和测试存在，无生产路由 | GB4-MED |
| SUBSCRIBE/NOTIFY | cascade 测试存在，无生产路由 | GB4-CAS |
| transaction/dialog | core 测试存在，driver 未使用 | GB4-SIP-002 |
| 多 realm/tenant/listener | 只有单 domain/default tenant | GB4-SIP-005 |

### 3.2 MANSCDP/MANSRTSP

| 类别 | 当前状态 | 计划范围 |
| --- | --- | --- |
| Keepalive/Catalog/DeviceInfo/DeviceStatus | parser 存在，部分落库 | 完成 typed event、去重、事务和大目录行为 |
| Alarm/MobilePosition | parser 存在，只日志 | 持久化、outbox、subscription 与限流 |
| RecordInfo | parser 存在，只日志 | typed query/result、分片合并、Operation 终态 |
| PTZ | XML builder 存在，命令路径 Unsupported | typed command、transaction、UnknownOutcome |
| Preset/Guard/AlarmReset/Record/TeleBoot/IFrame/DragZoom/HomePosition | 不完整 | 按 2022/2016 能力表逐项实现和探测 |
| ConfigDownload/DeviceConfig | 不完整 | query/control 与受限 extension metadata |
| Broadcast/talk | media test 局部存在 | MediaPort sender + SIP dialog Saga |
| MediaStatus | 未进入生产 | playback/download 完成通知和幂等终态 |
| MANSRTSP | builder/test 存在 | INFO response、seek/scale/pause/play 状态机 |

## 4. 首批兼容 profile backlog

兼容 profile 默认关闭，必须以 `(standard_version, manufacturer, model, firmware)` 精确匹配，并附真实或 synthetic regression fixture。

| Profile 类别 | 受控差异 | 安全边界 |
| --- | --- | --- |
| charset | UTF-8、GB2312、GBK；XML 声明与实际编码不一致 | 仅在 profile 下 fallback；解码后 body/文本上限不变 |
| MIME | `Application/MANSCDP+xml` 大小写；KSLP/KSPTZ/ALARM/KSSP/KSDU/cpim-pidf+xml | alias 映射到已注册 typed message；未知控制不透明转发 |
| SIP header | Contact 缺省端口、异常但可识别 Via、缺少 rport、非 magic-cookie branch | 只允许可证明无歧义的规范化；不得放宽 CRLF/token 校验 |
| endpoint | Contact 私网地址、source 漂移、NAT received/rport | 端点变更需认证 REGISTER 或明确 profile；普通 MESSAGE 不得劫持 endpoint |
| catalog | SumNum/DeviceList Num 不一致、重复/乱序/慢首片、Notify Catalog | 有界聚合并返回 Complete/Partial/Failed，不无限等待 |
| identifiers | 非 UUID、前导零、平台自定义通道 ID、上下级 ID 冲突 | 使用 ProtocolIdentity/映射表，不覆盖内部 UUID |
| media SDP | 缺失 SSRC、TCP setup 差异、payload/rtpmap 变体、先发媒体 | 由能力/profile 显式启用；地址和 transport 仍受 MediaPort 策略 |
| lifecycle | 注册续期 Call-ID/CSeq 变化、重复 200、迟到 200、MediaStatus=121 | transaction/dialog/session generation 决定幂等行为 |

## 5. 需要吸收的鲁棒性原则

1. REGISTER、MESSAGE、INVITE、INFO、SUBSCRIBE 分别使用 transaction 状态，不在巨型 handler 中共享隐式布尔状态。
2. Call-ID/CSeq/Via branch/SN 只用于协议关联，Operation/Command/MessageId 仍是内部权威身份。
3. observed source、Contact、Via received/rport、SDP media address 分开建模。
4. Catalog、RecordInfo、subscription 和 media dialog 都具有容量、deadline、清理和重复处理策略。
5. 设备或平台响应无法确认副作用时产生 `UnknownOutcome`，不得伪造成功或盲目重试 PTZ/control。
6. 级联上下游命令使用唯一 owner 和显式 bridge Saga，禁止双写。
7. 所有 vendor 差异转化为 profile + fixture + test，不进入通用 parser 隐式放宽。

## 6. 明确排除的实现模式

- 每设备常驻线程、Tokio task、socket 或独立 sleep/timer；
- protocol module 直接访问 Redis、SQL、NATS、media client 或 plugin host；
- signal handler 内直接创建 RTP receiver、send RTP、RTSP pull、转码或播放 URL；
- static/global mutable device、session、SSRC 或 broadcast map；
- 使用 AutoResetEvent/阻塞等待设备响应；
- 完整 SIP/XML/SDP、Authorization、密码或 digest material 日志；
- 通过字符串拼接生成 SIP/XML/SDP；
- 以 Call-ID、SSRC 或 stream name 代替 tenant-scoped typed ID；
- 未知 vendor message 的无界保留或透明控制转发。

## 7. 参考分析任务

- [x] `GB4-REF-001`：为每个参考项目保存 commit、许可证、文件路径和所提取行为的清单（见 [reports/gb4-ref-001.md](reports/gb4-ref-001.md)）。
- [x] `GB4-REF-002`：把每项参考行为映射到标准条款、compatibility profile 或明确的 out-of-scope（见 [reports/gb4-ref-001.md](reports/gb4-ref-001.md) 第 2 节）。
- [x] `GB4-REF-003`：新增 `scripts/verify_gb4_fixtures.py` 校验所有 fixture metadata；当前 7 组 fixture 全部通过。
- [ ] `GB4-REF-004`：互操作报告说明哪些行为来自真实设备、参考 peer 或 simulator，不把 peer 间一致误称为标准一致；当前处于 `Blocked`，报告见 [reports/gb4-ref-004.md](reports/gb4-ref-004.md)，待 `GB4-SYS-003/004` 真实联调后补充。

