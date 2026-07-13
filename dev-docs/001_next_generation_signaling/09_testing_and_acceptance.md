# 09. 测试与验收

## 1. 测试分层

```text
Protocol core unit/property/fuzz
            ↓
Driver IO and fault tests
            ↓
Module/application contract tests
            ↓
Storage/bus/media/plugin adapter tests
            ↓
Edge and cluster end-to-end tests
            ↓
Real-device interop / chaos / capacity / soak
```

每个生产问题优先沉淀为脱敏 fixture 或状态机输入序列，再修复实现。只依赖在线真实设备、无法自动回放的问题不算完成回归闭环。

## 2. Protocol core

### 2.1 通用要求

- 纯测试驱动，不启动 socket/runtime；
- fake clock 精确推进 timer；
- 任意 input 后输出有界且状态可解释；
- malformed input 不 panic、不无限循环、不产生无界分配；
- serializer/parser roundtrip 覆盖允许的规范化差异；
- 同一命令重复输入验证幂等/transaction 行为；
- 状态机终态后不会意外复活旧 transaction/dialog。

### 2.2 GB28181

- SIP CRLF、LF、半包、粘包、重复 header、compact header、大小写；
- REGISTER challenge、nonce 到期、realm/URI 错误、重放、注销；
- UDP/TCP transaction timer、重传、重复 response；
- INVITE/ACK/CANCEL/BYE 正常和交叉竞态；
- Catalog 分片、乱序、重复、错误 SumNum、超时 partial；
- 2016/2022 XML、编码、namespace、非法字符兼容；
- SDP IP/port/SSRC/payload/TCP mode 组合；
- PTZ 控制字和 checksum；
- 上下级级联路由、hop limit、循环检测。

### 2.3 ONVIF

- WS-Discovery ProbeMatch 多 XAddr/EPR/namespace；
- SOAP 1.2、WS-Addressing、SOAP Fault；
- HTTP Digest 与 UsernameToken、clock skew；
- Media2 success、partial/broken、Media1 fallback；
- profile/video source/channel 合并；
- PTZ range/space 转换和 auto-stop；
- PullPoint create/pull/renew/unsubscribe、termination race；
- DTD/XXE、深层 XML、超大列表、非法 URI；
- redirect 和 DNS/IP SSRF policy。

## 3. Property test 与 fuzz

property tests 至少验证：

- parser 对任意切片边界结果一致；
- parse → serialize → parse 保持语义；
- transaction/dialog ID 和 timer 不冲突；
- 目录 merge 幂等、顺序无关；
- operation/ownership state transition 满足单调 revision；
- MediaKey 映射稳定且不同 tenant 不碰撞；
- Proto unknown field/enum 可转发或明确拒绝。

持续 fuzz target：SIP message、GB XML、GB SDP、ONVIF discovery、SOAP envelope、Digest header、REST JSON、Protobuf extension。CI 运行短 fuzz；定时任务运行长 fuzz 并保留 corpus。OOM/timeout 视为失败。

## 4. Driver 与网络测试

- 真实 UDP/TCP loopback，覆盖半包、粘包、乱序、丢包、重复、连接重置；
- slowloris、读写队列满、listener 过载和 graceful drain；
- 多 interface WS-Discovery、错误 source address 和 zone 隔离；
- HTTP connection reuse、TLS、redirect、DNS 变化、deadline/cancel；
- timer wheel 在大规模 timer 下的 lag 和取消；
- driver 退出后所有 socket/task/queue 可回收；
- ARM/aarch64 构建和至少一个真实 ARM smoke test。

## 5. Repository 与消息契约

同一 repository contract suite 分别运行 SQLite 和 PostgreSQL：

- CRUD、revision/ETag、分页稳定性；
- tenant 隔离和唯一约束；
- Operation 幂等键；
- owner epoch conditional write；
- outbox 同事务提交、重复 publish；
- inbox 去重、crash-before/after-ack；
- migration 空库、上一版本升级、rollback 不支持时的恢复说明；
- 时间、UUID、JSON extension 在两种 backend 中语义一致。

NATS integration 覆盖 JetStream redelivery、consumer restart、poison/dead-letter、KV CAS conflict、node lease expiry、watch gap 和 reconnect。

## 6. 插件 contract test kit

发布与 Proto 同版本的黑盒 test kit，所有插件必须通过：

- Hello/Welcome 版本和 capability negotiation；
- 不兼容 major、未知 frame 和 oversized message；
- credit/inflight 背压；
- command deadline、cancel、重复 command；
- seq/ack、断线重连和有限重放；
- health timeout、drain、graceful shutdown；
- tenant/zone/capability 越权拒绝；
- secret handle 只能读取获授权 secret，且不进入日志。

host 使用故障插件验证崩溃隔离、退避、熔断和资源回收。

## 7. Media Plane contract

使用 fake media node 和真实 `cheetah-media-server-rs` 各运行一套：

- register/heartbeat/capability/drain；
- media node instance epoch 更换；
- OpenRtpReceiver、Update、Stop 的幂等和 fencing；
- pull proxy、snapshot、record、keyframe；
- MediaEvent 重复、乱序、gap、旧 instance；
- GB live 成功及每一个 Saga step 的失败补偿；
- ONVIF pull 成功、URI/credential/zone 失败；
- signaling crash 后 reconciler 不创建重复 RTP/proxy；
- media node crash 后 Operation、MediaSession 和 MediaBinding 状态正确，旧 binding 回调不能覆盖新 generation。

媒体 contract 未通过前不能移除媒体仓库旧 GB listener。

## 8. 北向 API

OpenAPI golden 与 handler contract 覆盖：

- 成功、校验、401、403、404、409、429、timeout、unsupported、unavailable；
- RFC 9457 stable code、request ID、field violations；
- tenant path/token 不一致；
- `Idempotency-Key` 重复和 payload 冲突；
- ETag lost update；
- cursor 篡改、分页上限和稳定排序；
- Operation 202、轮询、取消、超时和结果；
- SSE resume、gap、慢消费者；
- Webhook 签名、重试、熔断、dead-letter、SSRF/DNS rebinding；
- secret、URI userinfo 和内部错误不出现在 response/log/event。

CI 对 OpenAPI 做 breaking change 检查。新增 required 字段、删除 enum/endpoint、改变错误语义均视为破坏性变更。

## 9. 真实设备互操作

建立版本化矩阵：厂商、型号、firmware、标准/Profile、transport、已验证功能、已知 quirk、fixture 和测试日期。

GB 至少覆盖：

- 海康/大华 IPC 与 NVR；
- UDP/TCP 注册和保活；
- 实时、回放、下载、PTZ、告警、对讲；
- 2016/2022 差异；
- 一个上级和一个下级平台级联；
- NAT、设备重启、重复注册和媒体 timeout。

ONVIF 至少覆盖：

- Profile T/Media2 设备；
- 只有 Media1/Profile S 的 legacy 设备；
- HTTP Digest、HTTPS/pinned cert、UsernameToken legacy；
- PTZ、PullPoint、snapshot、多个 profile/video source；
- 错误 XAddr/clock/namespace 等真实 quirk。

项目只能声称“兼容已测试型号”，不能因协议实现存在就声称 ONVIF conformant；正式 conformance 需要对应官方测试流程。

## 10. HA 与 chaos

自动故障矩阵：

| 故障 | 必须验证 |
| --- | --- |
| kill gateway | 15 秒内失效，30 秒内可新接管，旧 epoch 被拒绝 |
| network partition gateway↔NATS | 不取得新 owner，不产生 split-brain |
| kill workflow | Operation 被接管，无重复媒体副作用 |
| kill media node | binding unavailable，按 policy 重建或终止 |
| NATS restart/leader change | redelivery 可去重、KV CAS 正确 |
| PostgreSQL failover | 已确认事务不丢，新写按可用性返回明确结果 |
| plugin crash loop | 有界退避，不拖垮 host |
| clock jump | deadline 使用 monotonic 驱动，wire time 可诊断 |
| rolling upgrade | 当前/上一版本可共存，无 schema/wire break |

对任何无法确定是否已作用到设备的命令，结果必须是 `UnknownOutcome`/可诊断失败，不能伪造成功或盲目重复危险动作。

## 11. 百万设备容量测试

### 11.1 GB 场景

- 100 万注册在线设备；
- 60 秒保活，启动和保活均匀错峰；
- 注册续期按配置 expiry；
- UDP/TCP 比例、报文大小、Catalog/Alarm/Invite 比例参数化；
- 故障时模拟一批设备同时重注册，验证 admission control，避免惊群压垮集群。

### 11.2 ONVIF 场景

- 100 万纳管且 presence online 的资产；
- 轮询间隔分层并加入 jitter；
- PullPoint subscription 比例参数化；
- discovery 按 network zone 分批，禁止一次性 multicast 风暴；
- 模拟慢设备、timeout、TLS 和错误 XML。

### 11.3 混合与媒体控制

- GB/ONVIF 比例参数化，不只发布单一最佳场景；
- StartLive 等媒体控制并发按在线设备 1%、5%、10% 阶梯；
- 媒体数据由 fake media node 模拟，信令测试不传输媒体 payload；
- 输出不同 gateway/worker 数量下的吞吐、p50/p95/p99、CPU、RSS、网络、queue depth、timer lag、DB/NATS 负载。

通过条件：达到 100 万在线且无丢失权威状态、无无界队列、无持续内存增长；节点增加能提升容量。所有结果绑定硬件、系统参数、commit、配置和负载脚本。

## 12. Soak 与发布门禁

生产候选版本执行 72 小时混合 soak，期间包含设备抖动、节点滚动、媒体失败和 webhook 故障。验收：

- RSS 和对象数量在热身后无持续单调增长；
- timer/queue lag 回到稳态；
- outbox、consumer lag 和 dead-letter 可解释；
- 无 stale owner 成功执行；
- terminal Operation 无未完成 step；Stopped/Failed MediaSession 无未回收 binding；
- 日志中无 secret 或未限长原始报文。

提交最低门禁：format、clippy `-D warnings`、受影响 crate test、Proto/OpenAPI breaking check、migration test、dependency license/advisory check。共享领域、Proto、storage 或 ownership 变更需运行工作区 contract tests。
