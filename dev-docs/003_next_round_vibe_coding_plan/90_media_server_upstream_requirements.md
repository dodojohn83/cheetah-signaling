# `cheetah-media-server-rs` 上游开发要求

## 1. 文档用途

本文可独立转交媒体服务器开发团队。目标是在不把信令业务放回媒体进程的前提下，为`cheetah-signaling`提供稳定、可集群化、支持幂等和fencing的跨进程控制接口。

基线：`cheetah-media-server-rs-dev` commit `d41ecbec4764519939d2b720141f275886a9bd8c`。

## 2. 已具备能力

最新`cheetah-media-api`已提供typed Rust traits和models：

- media query/session control；
- RTP receiver/connect/sender/update/get/list/stop；
- pull/push proxy及RTSP pull实现；
- record、snapshot、playback；
- output URL resolver；
- capability report和进程内bounded event bus；
- stable media error基础。

这些能力可作为gRPC mapper的domain侧目标，不应重写engine、RTP、proxy、record或snapshot状态机。

## 3. 缺口结论

当前接口不能直接满足信令集群：

1. 没有与信令`cheetah.media.v1`一致的生产gRPC adapter。
2. 没有media node向signaling注册、heartbeat、load、drain和deregister的client。
3. `MediaRequestContext`缺少tenant、owner node/epoch、target instance epoch、Operation/MediaSession/Binding等强类型上下文。
4. 进程内event bus无跨进程订阅、持久cursor、重放和gap语义。
5. pull proxy只有`source_url`，缺少不暴露密码的credential handle。
6. snapshot只支持已有MediaKey，缺少ONVIF SnapshotUri受限fetch。
7. query使用page/page_size且部分model同时出现cursor，不能保证稳定对账分页。
8. 资源缺少signaling binding关联，旧owner/instance回调无法被协议层强制fence。
9. 幂等实现分散且部分使用idempotency key直接构造资源ID，缺少canonical request冲突检测和第一次结果重放契约。

## 4. P0 阻塞项

### UP-MEDIA-P0-001：消费共享契约

- 从signaling发布的固定`cheetah.media.v1` tag/descriptor生成代码。
- 不在媒体仓维护可独立修改的Proto副本。
- 实现typed mapper；禁止generated DTO进入engine/domain持久状态。
- 发布supported contract version和capability generation。

交付：锁定依赖、codegen脚本、descriptor checksum、compatibility test。

### UP-MEDIA-P0-002：gRPC server adapter

实现：

```text
MediaCapability
MediaQuery
MediaRtp
MediaProxy
MediaRecord
MediaSnapshot
MediaPlayback
MediaOutput
MediaControl
MediaEventStream
```

建议独立adapter crate，依赖`cheetah-media-api` ports，不允许gRPC handler直接访问engine内部对象。

每个RPC实现deadline、cancellation、auth、capability check、错误mapper、metrics和audit。

### UP-MEDIA-P0-003：Mutation context 与 fencing

扩展媒体domain request context，至少包含：

```text
tenant_id
request_id / correlation_id / message_id
idempotency_key
deadline
source_signaling_node_id / owner_epoch
target_media_node_id / target_media_node_instance_epoch
operation_id / operation_step_id
media_session_id / media_binding_id
contract_version / trace context
```

- 修改调用缺字段必须拒绝。
- target instance不匹配在副作用前返回StaleOwner。
- 维护每个受控资源接受的owner epoch；旧epoch不能update/stop新资源。
- deadline过期不启动task或分配端口。

### UP-MEDIA-P0-004：幂等与结果语义

- key scope固定为tenant + operation kind + idempotency key。
- 保存canonical request digest、resource handle和第一次结果，至少覆盖资源生命周期/重试窗口。
- 相同key不同request返回Conflict。
- crash恢复后重复请求不能创建第二个RTP/proxy/record/playback资源。
- 错误标记`NOT_APPLIED`、`APPLIED`或`UNKNOWN`，客户端只自动重试`NOT_APPLIED`。

### UP-MEDIA-P0-005：Node registry client

- 启动完成后注册node ID、instance ID/epoch、gRPC endpoint、network zone、advertised media addresses、capability、capacity和build version。
- 按server返回interval续租并上报load。
- drain时停止接受新create，保留query/stop。
- shutdown有界deregister。
- registry不可达时按lease规则自我降级，不能继续无限接受旧owner命令。

### UP-MEDIA-P0-006：Replayable event stream

- wire event包含event ID、tenant、node/instance、sequence、MediaSession/Binding、handle、MediaKey、correlation和typed payload。
- 支持resume cursor、retention和明确gap事件。
- 每subscriber bounded；慢消费者不能阻塞media pipeline。
- 至少一次投递，重复event ID安全。
- event stream断开不影响媒体处理。

### UP-MEDIA-P0-007：对账查询

- 所有RTP/proxy/record/snapshot/playback资源支持Get/List。
- stable opaque cursor，不使用大offset。
- filter支持tenant、MediaSession、MediaBinding、handle、MediaKey、idempotency key和state。
- 返回node instance epoch、resource generation和安全last error。
- orphan清理通过typed Stop/Delete，不提供绕过tenant/fencing的后门。

### UP-MEDIA-P0-008：凭据与受限fetch

- PullProxyRequest使用sanitized source URL + credential handle，禁止持久化/返回userinfo。
- media通过授权SecretExchange按tenant/resource读取短期凭据。
- 新增Snapshot Fetch：URL、credential handle、destination/storage policy和限制。
- 对RTSP/RTSPS/HTTP(S)执行scheme、port、redirect、DNS rebinding和网段策略。
- audit/log/error不包含密码、Authorization、完整URL userinfo或内部文件路径。

## 5. P1 完整性

### UP-MEDIA-P1-001：Typed operation覆盖

- RTP返回advertised address/port、RTCP、SSRC policy、payload、transport、TCP mode。
- Proxy实现create/get/list/delete和state events。
- Record/Snapshot/Playback具有typed generation/state和完成事件。
- Output resolver只使用配置public endpoint，不信任Host header。
- request keyframe/close session使用明确资源引用。

### UP-MEDIA-P1-002：容量与过载

- create在内部原子申请session/port/bandwidth permit。
- hard limit与capability/load heartbeat一致。
- 队列/任务/缓存/事件subscriber均有上限。
- overload返回Busy/RateLimited和retry hint，不接受后静默丢弃。

### UP-MEDIA-P1-003：mTLS 与授权

- gRPC server生产强制mTLS，signaling node identity与声明source ID匹配。
- tenant/resource scope在provider层复核。
- 证书轮换不需要重启全部media session。

## 6. P2 运维与迁移

- fault injection：before/after side effect、response loss、event loss/gap、slow RPC、instance restart。
- admin drain和diagnostic endpoint受mTLS/scope/audit保护。
- 旧媒体进程内GB listener有显式disable/unique-owner开关。
- 新链路通过真实GB contract和观察窗口后才移除旧listener。
- 提供x86_64/aarch64构建、容器、SBOM和升级说明。

## 7. 上游 Contract Suite

媒体仓提交以下测试，并允许signaling CI调用：

- fake/domain mapper和真实engine provider运行同一RPC contract。
- register/heartbeat/drain/instance replacement。
- RTP receiver/sender/talk、proxy、snapshot stream/fetch、record/playback、URL。
- idempotency重复与冲突、deadline、cancel、old owner/instance。
- capacity race、draining、unsupported、version mismatch。
- event duplicate/order/gap/resume/slow subscriber。
- crash-after-side-effect-before-response和重启对账。
- tenant越界、mTLS identity mismatch、secret/log泄漏。

测试不得依赖公网、真实私有设备或固定公共端口。

## 8. 上游完成定义

- P0所有项合入并发布固定contract tag。
- 真实media进程可注册到signaling simulator并保持lease。
- signaling统一media contract suite全部通过。
- crash/重复请求不产生第二个有效资源。
- old owner/instance无法修改新资源。
- event gap可被检测并通过分页query收敛。
- 不把SIP、SOAP、设备目录或信令数据库访问引入媒体仓。

