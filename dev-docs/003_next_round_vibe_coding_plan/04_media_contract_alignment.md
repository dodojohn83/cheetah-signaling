# 04. 最新媒体接口对齐与 `cheetah.media.v1` 契约

## 1. 结论

`cheetah-media-server-rs` 最新 typed Rust API 已覆盖 query、session control、RTP receiver/sender、proxy、record、snapshot、playback、output URL和capability，但它是进程内 facade。信令现有Proto是另一套通用命令模型，真实媒体进程没有对应gRPC adapter，二者不能直接互操作。

跨进程唯一权威继续使用版本化 `cheetah.media.v1`：

- Proto source of truth位于本仓库 `proto/cheetah/media/v1`；
- 媒体仓库消费固定tag/descriptor，不复制后自行演化；
- Rust typed API属于媒体仓库内部domain contract；
- 两侧各自使用显式mapper，禁止wire DTO直接成为domain实体；
- 采用v1兼容扩展，旧generic RPC/message标记deprecated，不删除或复用字段号。

## 2. MED-C-001：请求上下文

所有修改型RPC共享typed `MediaMutationContext`，必须包含：

| 字段 | 语义 |
| --- | --- |
| `tenant_id` | 强制租户隔离 |
| `request_id`、`correlation_id` | 请求与跨系统关联 |
| `message_id` | inbox/事件去重 |
| `idempotency_key` | 同租户同操作幂等 |
| `deadline` | UTC绝对截止时间 |
| `source_signaling_node_id` | 当前信令owner节点 |
| `owner_epoch` | 设备/会话fencing |
| `target_media_node_id` | 稳定媒体节点ID |
| `target_media_node_instance_epoch` | 媒体进程实例fencing |
| `operation_id`、`operation_step_id` | Saga诊断 |
| `media_session_id`、`media_binding_id` | 逻辑/物理关联 |
| `contract_version` | 契约协商 |
| `traceparent`、`tracestate` | trace传播 |

校验规则：

- 修改请求缺少tenant、idempotency、deadline或epoch直接`InvalidArgument`。
- deadline已过不得产生副作用。
- 同一幂等键、相同canonical request返回第一次结果；参数不同返回`Conflict`。
- 旧owner或旧media instance返回`StaleOwner`，不能泄漏当前租户外状态。

## 3. MED-C-002：Typed 服务

新增或扩展以下RPC，禁止用`bytes payload`或无约束map表达核心命令：

```text
MediaCapability.GetCapabilities
MediaQuery.GetMedia / IsMediaOnline / ListSessions
MediaRtp.OpenReceiver / ConnectReceiver / OpenSender / Update / Get / List / Stop
MediaProxy.CreatePull / GetPull / ListPull / DeletePull
MediaRecord.Start / Stop / Get / ListTasks / ListFiles
MediaSnapshot.Take / Fetch / Get / List
MediaPlayback.Open / Get / List / Control / Stop
MediaOutput.ResolveUrls
MediaControl.RequestKeyframe / CloseSession
MediaEventStream.Subscribe
```

`Fetch`用于ONVIF SnapshotUri，输入只允许受限URL和credential handle，媒体节点自行执行SSRF与DNS rebinding防护。

## 4. MED-C-003：资源引用与状态

每个创建结果包含：

- opaque media handle；
- MediaKey；
- resource kind和typed state；
- resource generation/revision；
- media node ID与instance epoch；
- created/updated time；
- negotiation结果；
- capability/version；
-安全的last error；
- output URLs仅由`MediaOutput.ResolveUrls`生成。

RTP返回明确local advertised address、port、RTCP、SSRC policy、payload type、transport和TCP active/passive mode，禁止只返回自由格式SDP。

Proxy返回proxy handle、sanitized source、destination MediaKey、state和generation，不回显URL userinfo。

## 5. MED-C-004：能力和节点描述

能力按capability + version + operation + runtime state声明，至少包括：

- RTP receiver/sender/talk及UDP/TCP模式；
- PS/TS/ES封装和codec；
- RTSP/RTSPS pull；
- snapshot from stream / restricted fetch；
- record/playback/control；
- output schemas；
- hard capacity和network zones。

调用方必须先按version与operation筛选。`Unsupported`表示节点没有能力，`Unavailable`表示声明能力暂不可用。

## 6. MED-C-005：错误

Wire error稳定包含：

```text
code, safe_message, retryable, violations,
request_id, correlation_id, resource_ref,
outcome = NOT_APPLIED | APPLIED | UNKNOWN
```

最小code集合：

`INVALID_ARGUMENT`、`UNAUTHENTICATED`、`PERMISSION_DENIED`、`NOT_FOUND`、`CONFLICT`、`STALE_OWNER`、`BUSY`、`RATE_LIMITED`、`TIMEOUT`、`CANCELLED`、`UNAVAILABLE`、`UNSUPPORTED`、`VERSION_MISMATCH`、`UNKNOWN_OUTCOME`、`INTERNAL`。

内部source、URL凭据、文件路径和原始媒体/协议报文不得进入wire message。

## 7. MED-C-006：分页与对账查询

- 使用不透明、带版本和完整性校验的cursor；禁止同时暴露page number语义。
- 排序键固定且包含唯一ID，节点重启后cursor要么继续有效，要么明确`CursorExpired`。
- query显式tenant过滤，并支持按MediaSession、MediaBinding、handle、MediaKey、idempotency key检索。
- reconciler可分页枚举所有非终态资源，不依赖事件完整送达。

## 8. MED-C-007：事件

事件header包含：

```text
event_id, tenant_id, media_node_id, media_node_instance_epoch,
sequence, occurred_at, media_session_id, media_binding_id,
media_handle, media_key, correlation_id, trace context
```

typed payload至少覆盖resource state、stream online/offline、RTP timeout、proxy state、record/snapshot/playback完成和node lifecycle。

订阅请求包含tenant/filter/resume cursor/max batch。服务端明确：

- 至少一次投递；
-同event ID重复；
- sequence scope；
- cursor retention；
- gap通知；
- 慢消费者断开/降级策略；
- 重连和取消。

## 9. MED-C-008：兼容发布

- [x] 对旧字段设置deprecated注释，不改变原字段语义：`proto/cheetah/media/v1/media.proto` 中 `MediaEvent`/`MediaSessionEvent` 的旧字段已标 `[deprecated = true]`，不删除、不改变语义。
- [x] 新enum 0值为`UNSPECIFIED`：所有 proto enum（`MediaNodeStatus`、`ERROR_OUTCOME`、`SNAPSHOT_FORMAT`、`PLAYBACK_CONTROL`、`PTZ_DIRECTION`、`PRESET_ACTION`、`QUERY_KIND`、`DEVICE_CONTROL_KIND`、`DEVICE_STATUS`、`RESOURCE_KIND`、`COMMAND_STATUS`、`NODE_STATUS`）的 0 值均为 `*_UNSPECIFIED`。
- [x] 删除字段前reserved name/number；v1只做可选扩展：当前 v1 仅通过可选扩展和 `deprecated` 标记演进，尚无删除字段；后续删除字段时将使用 `reserved`。
- [x] 生成descriptor与breaking baseline：`scripts/generate_contract_baseline.sh` 生成 `descriptor.bin` 并执行 `buf breaking --against ".git#branch=origin/main"`；CI `contract-baseline` job 已纳入。
- [ ] old reader/new writer、new reader/old writer测试：待补充版本兼容性回归测试。
- [ ] 发布contract tag和checksum，媒体仓库只消费tag：需与 `cheetah-media-server-rs` 约定发布流程。
- [ ] 明确minimum/maximum supported contract version和滚动升级窗口：需在 proto 或配置中显式声明支持版本范围。

## 10. Contract tests

同一黑盒suite必须针对simulator和真实media adapter运行：

- 所有typed RPC的成功与validation。
- 幂等重复、payload conflict、deadline-before/after-dispatch。
- 旧owner epoch、旧media instance epoch。
- capacity full、draining、unsupported和version mismatch。
- event重复、乱序、gap、resume和慢消费者。
- cursor分页无重复/遗漏。
- secret/URL/internal error不泄漏。
- client crash、media crash和response丢失的UnknownOutcome。

## 11. 退出门禁

- Proto与最新媒体typed API逐操作有mapper矩阵。
- signaling simulator通过全部contract。
- 媒体上游P0完成后，真实adapter通过同一suite。
- 新生产路径不再调用generic `MediaControlPayload.bytes`。

