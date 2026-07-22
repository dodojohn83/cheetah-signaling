# 05. 媒体节点注册、调度、客户端与事件

## 1. 目标

在不处理任何媒体负载的前提下，把媒体节点纳入集群管理，并提供可被application使用的真实`MediaPort`。

## 2. MED-R-001：MediaClusterRegistry server

- [x] 启动独立内部gRPC listener并强制生产mTLS：`apps/cheetah-signaling/src/assembly.rs:1135-1172` 使用 `configure_grpc_tls` 配置 server TLS/optional mTLS，`MediaClusterRegistryService` 通过 `PeerIdentity` 扩展校验证书 identity。
- [x] Register验证证书identity与node ID，原子生成/推进instance epoch：`grpc.rs` `check_identity` 比对 mTLS identity 与 `node_id`；`InMemoryMediaNodeRegistry`/`PersistentMediaNodeRegistry` 在 `register` 中按 instance_id 是否相同递增 `instance_epoch` 和 `generation`。
- [x] 返回lease ID、TTL、heartbeat interval、cluster time和accepted contract version：`proto/cheetah/media/v1/media.proto` `MediaNodeInfo` 新增 `lease_id`、`lease_ttl_ms`、`heartbeat_interval_ms`、`cluster_time`、`accepted_contract_version`；`to_media_node_info` 在 register/heartbeat/drain/deregister 响应中填充这些字段。
- [x] Heartbeat带lease、instance epoch、load：`proto/cheetah/media/v1/media.proto` `MediaNodeHeartbeat` 新增 `lease_id` 与 `instance_epoch`；`MediaNodeRegistry::heartbeat` 扩展为接收 lease_id 与 instance_epoch 并在 `InMemory`/`Persistent` 实现中做 fencing；`load` 与 `session_count` 已存在。capacity 与 capability generation 的心跳携带将在后续调度任务中补充。
- [ ] Drain禁止新reservation但允许query/stop；Deregister保留保护窗口用于对账。
- [ ] lease过期立即移出候选，已有binding标记`NeedsVerification`。

## 3. MED-R-002：MediaNode repository

- [ ] 持久化稳定node、当前instance、endpoint、zone、addresses、capabilities、capacity、load、lease、drain和revision。
- [ ] 同node新instance必须fence旧instance；旧heartbeat不得延长新lease。
- [ ] 更新带revision条件并通过outbox发布node事件。
- [ ] SQLite/PostgreSQL执行相同contract。

## 4. MED-R-003：调度与reservation

过滤顺序固定：

1. contract/capability/operation兼容；
2. lease有效且非draining；
3. network zone可达；
4. transport/codec/port需求；
5. tenant placement；
6. hard capacity；
7. affinity和归一化负载评分。

- [x] 调度输入为不可变`MediaRequirements`：`MediaRequirements` 已新增 `contract_version` 字段，`matches_capability` 按 `cap.version >= requirements.contract_version` 过滤并在 `format_no_candidate_reason` 中输出 `contract_version` 与 `contract_version_mismatch`，评分通过 `contract_version_score` 优先精确版本（PR #227）。
- [ ] 同MediaSession generation重试优先原有效节点。
- [ ] 创建有TTL的reservation并持久化Reserved MediaBinding后才调用媒体。
- [ ] media RPC内再次原子检查容量，防止最终一致load超卖。
- [ ] 无候选返回逐规则reason summary，不泄漏其他tenant详情。

## 5. MED-R-004：Typed client

- [x] 连接池key包含node ID、instance epoch、endpoint和TLS identity：`MediaControlClient::pool_key` 由 `{endpoint}\0{node_id}\0{instance_epoch}\0{tls_identity_digest}` 组成，`tls_identity_digest` 对 CA、客户端证书、密钥名和 `allow_insecure_http` 做 hash（PR #229）。
- [x] endpoint/证书变化废弃旧channel：`get_or_create_entry` 在插入新 channel 前，移除同一 `media_node_id` 但 key 不同的旧条目，避免 endpoint 或 TLS identity 变更后继续复用旧连接（PR #229）。
- [x] 每节点有bounded concurrency、connect timeout、request deadline和circuit breaker：`ChannelEntry` 持有 `per_node_concurrency` 的 `Semaphore`；`connect` 使用 `connect_timeout`；`execute`/`list_sessions` 用 `timeout(request_timeout_ms)` 包装 tonic 调用；`ChannelEntry` 维护 circuit breaker 状态并在连续失败达到阈值后冷却（PR #229）。
- [x] 只重试明确`NOT_APPLIED`的暂时错误；`UNKNOWN`交给query/reconciler：`is_retryable` 仅对 `Unavailable`/`DeadlineExceeded`/`ResourceExhausted`/`Aborted` 等暂时 gRPC 错误重试；`SchedulerMediaPort::execute` 将媒体节点返回的 `CommandStatus::Timeout` 映射为 `MediaNodeCommandResult::UnknownOutcome`，不伪造成功也不盲目重试副作用（PR #229）。
- [x] cancellation向tonic request和permit传播：`acquire_permit` 通过 `timeout` 获取 `OwnedSemaphorePermit`，future 被 tokio 取消时 permit 自动释放；tonic 请求本身在 future drop 时也会被取消（PR #229）。
- [x] client只接受typed request，不接收任意JSON/bytes：`MediaControlClient::execute` 接收 `MediaControlRequest`（typed `MediaCommand`），`list_sessions` 接收 `MediaListSessionsRequest`，不暴露原始 JSON/bytes 入口（PR #229）。

## 6. MED-R-005：Mapper 与 MediaPort

- [ ] domain newtype ↔ Proto显式转换，错误字段精确定位。
- [ ] MediaKey按001规则稳定编码tenant/app/stream。
- [ ] reserve_live/playback/talk只调度和创建reservation，不隐式执行协议步骤。
- [ ] execute校验node/instance/owner/deadline后调用typed operation。
- [ ] release重复调用安全；结果不存在视为已释放但记录对账。
- [ ] list_sessions严格tenant分页，畸形返回项导致节点contract violation而非静默跳过。

## 7. MED-R-006：Event consumer

- [ ] 每节点单独bounded subscription和resume cursor。
- [ ] inbox在副作用前按tenant+event ID去重。
- [ ] 校验node instance、binding、session generation和owner epoch。
- [ ] 旧事件只记diagnostic，不推进新binding。
- [ ] gap触发目标节点分页reconciliation，不假定丢失事件无关紧要。
- [ ] cursor与inbox提交顺序保证crash后可安全重放。

## 8. MED-R-007：Scheduler/registry reconciler

- [ ] lease过期检查所有绑定，不立即伪造session停止。
- [ ] draining节点按desired state迁移或有界等待自然结束。
- [ ] 媒体有资源、信令无binding：保护窗口后按idempotency/metadata复核再清理。
- [ ] 信令Active、媒体无资源：创建新binding或按policy失败，绝不复活旧终态binding。
- [ ] 同generation最多一个有效binding，数据库约束和application检查同时保证。

## 9. MED-R-008：观测和管理

低基数metrics：

- active/expired/draining media nodes；
- reservation success/reject/reason；
- RPC latency/error/outcome；
- event lag/gap/reconnect；
- reconciliation scanned/repaired/failed；
- per-node normalized load（node ID不作为无限动态label时需限制）。

审计记录register、drain、forced cleanup和manual reconciliation，不记录secret或source URL userinfo。

## 10. 测试与退出门禁

- 注册、重复注册、新instance替换、旧heartbeat、lease expiry和drain。
- 评分确定性、capacity race、no candidate reason和tenant placement。
- TLS identity mismatch、endpoint变更、timeout/cancel/circuit。
- event duplicate/out-of-order/gap/crash windows。
- scheduler和真实media contract均通过后，主应用才切换到`SchedulerMediaPort`。

