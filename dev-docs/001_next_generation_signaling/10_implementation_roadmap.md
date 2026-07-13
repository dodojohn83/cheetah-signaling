# 10. 实施路线图

## 1. 建议 workspace

目录按职责分组，避免协议增加后根目录扁平膨胀：

```text
apps/
  cheetah-signaling/
crates/
  foundation/
    cheetah-signal-types
    cheetah-signal-contracts
    cheetah-runtime-api
  domain/
    cheetah-device-domain
    cheetah-control-domain
  application/
    cheetah-signal-application
  protocols/
    gb28181/{core,driver-tokio,module,testing,fuzz}
    onvif/{core,driver-tokio,module,testing,fuzz}
  storage/
    api
    sqlite
    postgres
  messaging/
    api
    local
    nats
  cluster/
    ownership
    registry
  media/
    client
    scheduler
  plugin/
    sdk
    host
    testkit
  api/
    http
    grpc
```

初始实现可合并过小的 domain/application crate，前提是依赖方向和公共边界不变。不得为追求 crate 数量而产生循环依赖。

## 2. Phase 0：契约与工程基线

交付：

- Cargo workspace、Rust Edition 2024、toolchain/MSRV policy；
- CI、format/clippy/test、cargo-deny、SBOM 基线；
- common/control/plugin/media/cluster Proto 与 Buf breaking policy；
- 领域 ID、错误、Clock、ports 和 in-memory test doubles；
- 配置 schema、secret redaction 和 tracing 基线；
- OpenAPI skeleton 与 RFC 9457 错误。

完成条件：Proto 可以生成并被独立示例 client 消费；domain 不依赖 Tokio、Axum、tonic、SQLx 或 NATS；CI 能阻止 wire breaking change。

## 3. Phase 1：edge 控制内核

交付：

- Device/Channel/Operation/MediaSession/MediaBinding 聚合；
- 不可变 Command、application Operation handler、Operation state machine、Saga/reconciler；
- SQLite repository、migration、UnitOfWork、outbox/inbox；
- local bus、in-memory ownership、sharded worker 和 timer wheel；
- REST devices/channels/operations、SSE；
- edge `all` 进程装配和 ARM 构建。

完成条件：不启用任何协议也能用 fake adapter 完成设备、Operation、事件和崩溃恢复 contract test；所有队列和分页有上限。

## 4. Phase 2：Media Plane 契约

两仓库同步交付：

- `cheetah.media.v1` 冻结和生成 crate；
- signaling media registry/client/scheduler；
- media server gRPC adapter、node registration、capability、heartbeat；
- RTP/proxy/snapshot/record 的 idempotency、deadline、fencing；
- MediaEvent stream 和 fake/real contract suite；
- MediaKey 稳定映射与 network-zone placement。

完成条件：fake signaling 能驱动真实 media server 完成 open/online/query/stop；重复请求和旧 epoch 不产生第二个有效 session。

## 5. Phase 3：GB28181 设备接入

按顺序实现：

1. SIP parser/serializer、Digest、transaction、UDP/TCP driver；
2. REGISTER/注销/保活和 ownership；
3. GB XML、Catalog/Info/Status；
4. live INVITE/ACK/BYE + 媒体 Saga；
5. PTZ、Alarm、Position；
6. RecordInfo、回放、下载；
7. talk/broadcast；
8. 2016/2022 和厂商 compat fixtures。

完成条件：海康/大华至少各一个 IPC/NVR 完成生产核心集，故障补偿不残留 dialog/RTP binding；媒体数据不进入 signaling。

## 6. Phase 4：GB28181 平台级联

交付：

- PlatformLink 和上级 registration；
- 共享目录投影、ID mapping、SUBSCRIBE/NOTIFY；
- 上级点播、控制、RecordInfo 到本地 Operation；
- 下级平台接入和目录合并；
- alarm/presence 转发；
- hop limit、循环检测和权限隔离。

完成条件：一上级 + 一下级的三级拓扑能完成目录、live、playback、PTZ 和事件；跨 tenant 通道不可见。

## 7. Phase 5：ONVIF

按顺序实现：

1. XML/SOAP/WS-Addressing/Discovery core 与 driver；
2. zone discovery-agent、Device/Capabilities；
3. Media2 profile/URI 与 Media1 fallback；
4. media pull proxy Saga 与 snapshot；
5. PTZ、Imaging read；
6. PullPoint events；
7. HTTPS/Digest/UsernameToken legacy 和 compat fixtures。

完成条件：Profile T/Media2 与 legacy Media1 真实设备均完成发现、同步、live、PTZ、事件和快照；所有 URI 经过 SSRF/zone 校验。

## 8. Phase 6：cluster 与 HA

交付：

- PostgreSQL repository/migrations；
- NATS local port 的 cluster 实现、JetStream streams/consumers；
- NATS KV node lease、ownership CAS、watcher/janitor；
- role-based process assembly、L4 readiness；
- outbox publisher、distributed reconciler；
- drain、rolling upgrade 和 chaos suite；
- PostgreSQL/NATS 部署模板和告警规则。

完成条件：gateway kill 后满足 15/30 秒目标，旧 epoch 对数据库和媒体均被拒绝；当前/上一版本可滚动共存。

## 9. Phase 7：插件 SDK

交付：

- PluginRuntime 双向 gRPC/UDS；
- host 进程监督、capability、config、credit/backpressure；
- Rust SDK 和语言无关 Proto 文档；
- contract test kit、故障插件和示例协议插件；
- tenant/zone/secret/media capability sandbox。

完成条件：插件崩溃/失联不会影响内置 GB/ONVIF；兼容 major 重连不丢已接受 command 的可查询结果。

## 10. Phase 8：百万设备与发布

交付：

- 分布式 GB/ONVIF simulator；
- 1M online、混合 Operation、重注册风暴、72h soak 报告；
- CPU/RSS/网络/DB/NATS/queue/timer flamegraph 与容量模型；
- 内核/socket/NATS/PostgreSQL 推荐参数；
- 安全测试、依赖审计、SBOM、恢复演练；
- edge tarball/systemd 示例与 cluster 容器/Kubernetes 示例。

完成条件：满足 [09_testing_and_acceptance.md](09_testing_and_acceptance.md) 的容量、HA、互操作和安全门禁，且结果可由固定脚本复现。

## 11. 媒体旧 GB module 迁移

迁移采用可回退步骤：

1. 从旧实现提取脱敏 fixture、已知 quirk 和真实设备基线；
2. media API/gRPC 与 signaling GB 新链路先在隔离端口运行；
3. mirror 非副作用事件验证目录/presence 映射；
4. 按 tenant/device allowlist 切换 SIP owner，禁止双写控制命令；
5. 对比在线、点播成功率、首帧、残留 RTP session 和错误分布；
6. 达到观察窗口后关闭旧 listener；
7. 保留一个发布周期回滚配置，但回滚时仍保证唯一 owner；
8. 最终从 media app 装配中移除 GB signaling module，保留 RTP/PS 能力。

## 12. v1 完成定义

v1 只有同时满足以下条件才可发布：

- GB 设备接入、级联与 ONVIF 生产核心集完成；
- edge 和 cluster 均通过相同 domain/media contract；
- 真实设备矩阵与 fixture 入库；
- media plane 完全解耦，signaling 无媒体 payload 热路径；
- HA fencing、outbox/inbox、reconciler 通过 chaos；
- 100 万在线和 72 小时 soak 报告通过；
- REST/OpenAPI、Proto、migration 有 breaking gate；
- 安全审计、SSRF/XXE、secret redaction、RBAC 通过测试；
- 部署、升级、备份、回滚、故障诊断文档齐全。

未完成 capability 必须在 capability API 和文档中返回 Unsupported；不得以空实现、HTTP 200 或虚假成功占位。
