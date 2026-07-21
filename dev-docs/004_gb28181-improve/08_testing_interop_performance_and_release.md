# 08. 测试、互操作、性能与发布门禁

## 1. 目标

建立从 pure state machine 到真实设备/平台/media node 的分层验收，证明标准路径、兼容 profile、失败恢复和百万设备资源模型，而不是仅证明 parser 或 simulator 能运行。

## 2. 测试分层

```text
unit / transition table
  -> parser/codec golden/property/fuzz
  -> core-driver-module contract
  -> repository/message/media contract
  -> deterministic simulator system test
  -> real media + reference peer interop
  -> real device/platform interop
  -> cluster chaos/capacity/soak/release
```

低层通过不能替代高层报告。真实测试不可作为默认单元测试依赖，必须使用显式 profile、deadline 和清理流程。

## 3. Core 与 Codec

### 3.1 SIP

- request/response、compact headers、unknown bounded headers、Content-Length；
- UDP datagram 与 TCP 任意 byte 切片、半包/粘包/多 message；
- Via/From/To/Contact/Call-ID/CSeq/Route/Record-Route/Subject/Event/Expires；
- duplicate/ambiguous Content-Length、obs-fold、CRLF/token injection；
- REGISTER/MESSAGE/INVITE/ACK/CANCEL/BYE/INFO/SUBSCRIBE/NOTIFY/OPTIONS golden；
- transaction Timer A/B/D/E/F/K、reliable transport、duplicate response/request；
- dialog route set、target refresh、CSeq、re-INVITE、late/repeated 2xx。

### 3.2 XML/SDP/MANSRTSP

- GB/T 28181-2022 主样本与 2016 compatibility 样本分目录；
- UTF-8/GB2312/GBK、声明不一致的 strict/profile 两组行为；
- Keepalive、Catalog、DeviceInfo、DeviceStatus、RecordInfo、Alarm、MobilePosition；
- PTZ/Preset/HomePosition/DragZoom/Guard/AlarmReset/Record/TeleBoot/IFrame；
- ConfigDownload/DeviceConfig/Broadcast/MediaStatus；
- live/playback/download/talk SDP 和 MANSRTSP Play/Pause/Scale/Seek；
- DTD/XXE、深度、节点、文本、item、extension、line/payload 上限；
- parser never-panic property 与 fuzz corpus regression。

## 4. 状态机与 Contract

| Suite | 必测行为 |
| --- | --- |
| Access | auth、register/unregister/refresh/expiry、keepalive、duplicate、owner takeover |
| Command | send/response/timeout/cancel/retry classification/UnknownOutcome/old epoch |
| Catalog | fragment/duplicate/reorder/missing/partial/crash/revision conflict |
| Media | every Saga step、late 200、CANCEL/BYE、early media、old media instance |
| Cascade | register/backoff、multi-upstream、catalog/subscription/bridge/loop/ACL |
| Repository | SQLite/PostgreSQL tenant/revision/transaction/outbox/migration/cursor |
| Message | at-least-once、inbox dedupe、ack loss、redelivery、DLQ |
| MediaPort | fake/real Open/Update/Close/event/fencing/idempotency/deadline |
| Architecture | Cargo metadata layer/forbidden dependency/feature graph |

所有时间测试使用 FakeClock/Tokio paused time；ID 和 jitter 使用确定性 generator/seed；不依赖固定公共端口或测试顺序。

## 5. Simulator 重构

当前 simulator 每设备创建 task/socket/timer，不适合作为容量工具。重构目标：

- 固定数量 shard task；每 shard 管理大量惰性 device state；
- UDP 设备可共享有限 socket/源端口策略，TCP 使用有界 connection pool；
- 所有 timer 进入时间轮，启动/保活按 seed 均匀错峰；
- scriptable scenario 描述 register、command、catalog、media SIP、subscription 和 cascade；
- 可注入 drop、duplicate、reorder、delay、malformed、disconnect、credential error、slow catalog、late 200、media early/late event；
- generic/标准 profile 与真实 vendor fixture profile 分离；合成的 dahua/hikvision 名称不作为互操作证据；
- 输出 seed、scenario、message counts、error/outcome、resource usage 和 transcript hash；
- simulator 不生成或发送真实媒体 payload，只模拟 media control event。

## 6. 互操作矩阵

### 6.1 设备/NVR

至少两类设备或 NVR，且组合覆盖：

- GB/T 28181-2022 与 2016；
- UDP 与 TCP，条件允许时 IPv4 与 IPv6；
- Digest Required、MD5 compatibility、注册/续期/注销/保活；
- Catalog/DeviceInfo/DeviceStatus/RecordInfo；
- PTZ/预置位/控制、Alarm、MobilePosition；
- live/playback/download/talk；
- endpoint/NAT、重启、断网、重复/迟到响应。

每份报告记录 manufacturer/model/firmware、网络拓扑、标准/profile、脱敏 semantic transcript 和不支持能力。

### 6.2 上下级平台

至少一个上级和一个下级组合，覆盖：

- REGISTER/Digest/keepalive/expiry；
- Catalog query/share/change Notify；
- Alarm/MobilePosition subscription；
- live/playback bridge、CANCEL/BYE/INFO；
- external ID mapping、virtual directory、ACL 和多 tenant 隔离；
- platform/signaling/media restart 与 owner migration。

WVP、AKStream 或 GB28181.Solution 可作为 reference peer，但 peer 间成功不等价于标准认证；报告必须标记 peer commit/config。

### 6.3 Real Media

- 与固定 commit 的 `cheetah-media-server-rs` 运行同一 MediaPort contract；
- 验证 Open/Update/Close、StreamOnline/Offline、node instance epoch、restart 和 scheduling；
- signaling 抓包/端口审计证明没有媒体 payload 和 RTP/RTCP listener。

## 7. Chaos

三节点 cluster 至少注入：

- signaling owner crash、lease expiry、旧 owner 恢复；
- PostgreSQL 延迟/短断/连接池耗尽；
- NATS disconnect、ack loss、redelivery、consumer lag；
- media node restart、instance epoch 变化、event duplicate/reorder；
- SecretStore timeout；
- device/platform disconnect、register storm、slow TCP peer；
- rolling drain/upgrade 和上一版本共存。

通过条件：

- 15 秒内检测 gateway 故障，30 秒内可接管不依赖设备重连的工作；
- 旧 owner/旧 media instance 无有效副作用；
- inbox/outbox 可解释地重放/去重；
- Operation、ProtocolSession、MediaSession/Binding 和 PlatformLink 最终收敛；
- 无无限重试、无无界 backlog、无假成功。

## 8. 百万设备容量

规模逐级为 10 万、30 万、100 万在线设备：

- 60 秒 Keepalive，REGISTER refresh 按 expiry，启动/保活错峰；
- UDP/TCP、IPv4/IPv6、2016/2022、profile 比例参数化；
- Catalog/Alarm/command 比例和报文大小参数化；
- 10%/50% 设备在抖动窗口重注册；
- live 等媒体 Operation 按在线设备 1%/5%/10% 阶梯，media 使用 fake control node；
- 单 tenant、热点厂商/设备段和多 tenant 分布分别测试；
- 增加 gateway/shard 后验证近似水平扩展。

报告至少输出：register/keepalive TPS、command/Operation throughput、各阶段 P50/P95/P99、CPU/RSS/network/file descriptor、queue depth、timer lag、owner distribution、DB/NATS load、reject/drop/dedupe rate、failure recovery time。

通过条件：达到 100 万在线且无权威状态丢失、无每设备 task/timer、无无界队列/内存增长；所有容量声明绑定硬件、内核、commit、配置和场景文件。

## 9. Soak

开发阶段运行 24 小时；发布候选运行 72 小时混合 soak，包含：

- 设备抖动、注册续期、目录/报警和命令；
- 1%/5% media session churn；
- 节点滚动、media failure、DB/NATS/secret 短断；
- platform registration/subscription refresh；
- compatibility profile 命中与 strict reject。

验收：

- 热身后 RSS、对象、timer、connection、transaction、dialog 无持续单调增长；
- queue/timer/consumer lag 恢复稳态；
- outbox/inbox/dead-letter 可解释；
- 无 stale owner 成功副作用；
- terminal Operation 无 pending step；Stopped/Failed MediaSession 无有效 binding；
- ProtocolSession/PlatformLink expiry 和 subscription 清理正确；
- 日志/trace/audit 无 secret、完整原始报文或无限增长。

## 10. 发布命令

提交前至少运行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
buf format --diff --exit-code
buf lint
cargo deny check
python3 scripts/audit_architecture.py
```

按改动追加：

- Proto：Buf breaking、deterministic codegen、old-reader/new-writer；
- REST：OpenAPI snapshot/breaking/error matrix；
- SQL：SQLite/PostgreSQL migration/repository contract；
- GB：golden/property/fuzz、driver/module/simulator/system/interop；
- media/ownership：real contract、multi-node、fault injection/reconcile；
- feature/dependency：edge no-cluster tree、x86_64/aarch64 check。

## 11. 实施任务

- [ ] `GB4-TST-001`：扩充 SIP/XML/SDP/MANSRTSP golden、metadata、property 和 fuzz corpus。
- [ ] `GB4-TST-002`：建立 access/command/catalog/media/cascade 合法/非法迁移表测试。
- [ ] `GB4-TST-003`：建立 driver-core-module、repository/message/media/architecture contract suite。
- [ ] `GB4-TST-004`：重构 fixed-shard simulator 和 deterministic fault scenario DSL。
- [ ] `GB4-SYS-001`：完成 edge SQLite + fake media 全 GB vertical system test。
- [ ] `GB4-SYS-002`：完成 cluster PostgreSQL/NATS + real media 全 GB vertical system test。
- [ ] `GB4-SYS-003`：完成至少两类真实设备/NVR互操作报告。当前 `Blocked`，报告见 [reports/gb4-sys-003.md](reports/gb4-sys-003.md)。
- [ ] `GB4-SYS-004`：完成上级/下级平台级联互操作报告。
- [ ] `GB4-SYS-005`：完成安全、过载和敏感信息泄漏测试报告。
- [ ] `GB4-SYS-006`：完成三节点 chaos/rolling upgrade 报告。
- [ ] `GB4-SYS-007`：完成 10万/30万/100万容量与水平扩展报告。
- [ ] `GB4-SYS-008`：完成 24h development endurance 和 72h release soak 报告。
- [x] `GB4-SYS-009`：完成 x86_64/aarch64、SBOM/license/advisory、migration 和 release checklist。见 [reports/gb4-sys-009.md](reports/gb4-sys-009.md)。

## 12. 最终退出门禁

- 所有 `GB4-*` 任务为 Completed 或有批准的 v1 out-of-scope 替代任务；不存在未归属 Partial。
- fake、reference peer 和 real device/media 的证据明确区分。
- 所有 workaround 有 provenance、profile、风险和 regression。
- 公开 REST/Proto、配置和 migration 具有兼容策略。
- 完整质量门禁、互操作、chaos、100 万在线和 72 小时 soak 报告可复现。
- signaling Control Plane 边界通过依赖、端口和抓包三类审计。

