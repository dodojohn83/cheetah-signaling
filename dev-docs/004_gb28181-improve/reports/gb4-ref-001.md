# GB4-REF：参考实现能力、兼容与鲁棒性分析

- 任务：`GB4-REF-001`、`GB4-REF-002`、`GB4-REF-003`、`GB4-REF-004`
- 结论：`Partial`；参考清单、行为映射、fixture 校验工具已落地；真实设备/平台互操作报告需 `GB4-SYS` 阶段补全
- 分析日期：2026-07-21
- 仓库基线：`96c76efc9b6c5bdf4956ab6a4c100429d0e8e8da`

## 1. 参考项目清单（GB4-REF-001）

| 项目 | Commit | 许可证/来源结论 | 文件路径/用途 | 可借鉴的信令能力 | 明确禁止照搬 |
| --- | --- | --- | --- | --- | --- |
| `escoffier/sipsdk` | `4d906b32cdc0677be6663249712825eb57d1f870` | GitHub 未识别仓库许可证 | 仅行为和测试场景，不复制源码 | reSIProcate transaction/dialog；REGISTER/INVITE/MESSAGE/ACK/BYE/CANCEL/INFO/SUBSCRIBE/NOTIFY；in-dialog callback；MANSCDP/MANSRTSP 与 vendor MIME | 旧代码、空 factory、无仓库许可证；不得复制源码或依赖其宽松解析行为 |
| `648540858/wvp-GB28181-pro` | `642a9fce82cd22246be28a233c046d696a88f283` | MIT | 行为、结构和 clean-room fixture；复制前仍检查文件头 | UDP/TCP、设备注册、目录、状态、报警、位置、PTZ/预置位、回放下载、语音、级联、多上级、订阅和虚拟目录 | Spring/Redis/数据库直接耦合 handler；不能把 media server 内部对象带入 signaling |
| `chatop2020/AKStream` | `3620ff58316534cce7a1d806f8d31239bc92e2c9` | MIT | 行为、结构和 clean-room fixture；不引入 media plane | IPv4/IPv6、UDP/TCP、GB2016、ConfigDownload、Preset、Broadcast、MediaStatus、记录查询和 SIP client/server 双角色 | 每设备线程、Thread.Sleep/Abort、AutoResetEvent、static map、完整 SIP 日志、媒体节点直接调用 |
| `GB28181/GB28181.Solution` | `28f423ab11ac59c3f3d9590eb6a78bc4f8b460d3` | 仓库声明 MIT/BSD，README 警告部分依赖/代码可能为 GPL | 默认仅行为参考；逐文件许可批准后才可引用代码 | registrar queue、binding、duplicate register、strict realm、Contact/NAT、UDP/TCP/TLS、transaction engine | 仓库自述非生产就绪；许可证混合；包含 RTP/RTSP/audio 与每设备 monitor，不得进入控制面 |
| local `simple-media-server` | `bd68e28745a9863f68d6a496fc077d43b9bf99aa` | Mulan PSL v2 | `../simple-media-server` 或固定 commit；信令行为和 clean-room fixture；媒体实现禁止进入本仓库 | Keepalive/Catalog/DeviceInfo/PTZ/Preset/HomePosition/RecordInfo/Config/Alarm/MobilePosition/MediaStatus/Broadcast；source endpoint 更新 | 记录完整 XML、密码/digest 诊断、每设备 timer、信令媒体耦合和 RTP/PS 代码 |

## 2. 行为映射（GB4-REF-002）

| 能力 | 来源 | 标准/Profile 归属 | Out-of-scope 原因 |
| --- | --- | --- | --- |
| UDP REGISTER/MESSAGE | sipsdk/WVP/AKStream/GB28181.Solution/simple-media-server | GB/T 28181-2022 第 5/6/7 章；`GB4-SIP`、`GB4-ACC` | 仅信令流程；RTP/RTCP/PS 解析不进入控制面 |
| TCP SIP | WVP/AKStream/GB28181.Solution/simple-media-server | GB/T 28181-2022 附录；`GB4-SIP-001` | 同上 |
| transaction/dialog 状态机 | sipsdk | RFC 3261；`GB4-SIP-002` | 仅状态机；媒体 payload 不处理 |
| 多 realm/tenant/listener | WVP/AKStream | 平台部署需求；`GB4-SIP-005` | 无 |
| OPTIONS 心跳 | core method 已存在 | GB/T 28181-2022 9.3；`GB4-SIP-003` | 无 |
| 主动 MESSAGE/query/control | WVP/AKStream | GB/T 28181-2022 第 9 章；`GB4-CMD` | 无 |
| INVITE/ACK/CANCEL/BYE/INFO | WVP/simple-media-server | GB/T 28181-2022 第 9 章 / MANSRTSP；`GB4-MED` | 媒体 SDP/SSRC 仅协商；不绑定 RTP 端口 |
| SUBSCRIBE/NOTIFY | WVP/GB28181.Solution | GB/T 28181-2022 第 9 章；`GB4-CAS` | 无 |
| Catalog/RecordInfo 分片聚合 | WVP/AKStream | GB/T 28181-2022 附录 A/B；`GB4-ACC-005` | 无 |
| PTZ/Preset/HomePosition/DragZoom | WVP/simple-media-server | GB/T 28181-2022 附录 C；`GB4-CMD-001` | 无 |
| Alarm/MobilePosition | WVP/simple-media-server | GB/T 28181-2022 附录 D/E；`GB4-EVT` | 无 |
| Broadcast/talk | WVP/AKStream | GB/T 28181-2022 9.13/9.14；`GB4-MED-005` | 媒体发送由 MediaPort 执行 |
| MediaStatus=121 终止回放 | simple-media-server | GB/T 28181-2022 9.11；`GB4-MED-004` | 无 |
| 每设备线程/timer/static map | AKStream/WVP | — | 明确排除：固定分片运行时和时间轮 |
| RTP/RTCP/PS/TS/ES 处理 | 全部参考项目 | — | 控制面禁止：由 MediaPort/媒体节点执行 |
| RTSP 拉流/转码/播放 URL | WVP | — | 控制面禁止：媒体服务器职责 |
| 完整 SIP/XML/SDP/密码日志 | 全部 | — | 安全规范禁止；仅脱敏诊断采样 |

## 3. Fixture 校验（GB4-REF-003）

新增 `scripts/verify_gb4_fixtures.py`，校验 `testdata/gb28181/**/*.meta.toml` 的必填字段：

- `source` ∈ `{synthetic, real-device, reference-peer}`
- `standard` ∈ `{GB/T 28181-2022, GB/T 28181-2016}`
- `profile`、`expected`、`desensitization`、`license`
- 可选字段：`source_project`、`source_commit`、`manufacturer`、`model`、`firmware`

执行结果：

```bash
python3 scripts/verify_gb4_fixtures.py
OK: 14 fixture data files and 7 metadata files validated.
```

当前 7 组 fixture 全部为 `synthetic` 或 `reference-peer` 来源，许可证使用 MIT-0，符合 clean-room 规则。无许可证未确认来源的 fixture。

## 4. 互操作报告（GB4-REF-004）

当前阶段尚未连接真实设备或平台；本项产出为**文字分析和 synthetic fixture**。真实互操作报告将在 `GB4-SYS-003/004` 阶段，在完成两类真实设备/平台联调后补充。

## 5. 吸收的鲁棒性原则

1. REGISTER、MESSAGE、INVITE、INFO、SUBSCRIBE 分别使用 transaction 状态，不共享隐式布尔状态。
2. Call-ID/CSeq/Via branch/SN 只用于协议关联，Operation/Command/MessageId 仍是内部权威身份。
3. observed source、Contact、Via received/rport、SDP media address 分开建模。
4. Catalog、RecordInfo、subscription 和 media dialog 都具有容量、deadline、清理和重复处理策略。
5. 设备或平台响应无法确认副作用时产生 `UnknownOutcome`，不得伪造成功或盲目重试。
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
