# 06. 媒体工作流、补偿与对账

## 1. 模型不变量

- `Operation`是北向异步状态唯一权威。
- `Command`是不可变typed指令，不拥有第二套生命周期。
- `MediaSession`保存用户逻辑意图、desired state和generation。
- `MediaBinding`保存到具体media node instance和handle的物理绑定。
- Start Operation成功后MediaSession可持续Active。
- Stop/control创建新Operation引用既有MediaSession。
- 终态binding不可复活；迁移/重试创建新binding。

## 2. WF-001：Saga step与dispatch

- [ ] 每个Operation持久化有序OperationStep和DispatchAttempt。
- [ ] 聚合变更、step和outbox同事务提交。
- [ ] Command包含operation/step、idempotency、deadline和owner epoch。
- [ ] ack、重投、dead-letter只更新dispatch诊断，不直接伪造Operation结果。
- [ ] worker崩溃后按step状态和外部query恢复。

## 3. WF-002：Live start

- [x] 原子创建Pending Operation、Requested MediaSession、outbox：`MediaService::start_live` 在 `persist_start_resources` 中同一事务写入 `Operation`、`MediaSession`、`MediaBinding` 及 outbox（`crates/application/cheetah-signal-application/src/media_service_start.rs:190-280`）。
- [x] Operation Running：`start_live` 调用 `persist_start_resources` 后 `operation.start()` 并提交（`media_service_start.rs:131`）。
- [x] 校验device/channel/capability和owner：`ensure_device_and_channel_ready` 校验 device 在线、channel 状态与 capability；`owner_resolver.resolve` 校验 owner epoch（`media_service_start.rs:31-62`）。
- [x] 调度并创建Reserved MediaBinding：`media_port.reserve_live` 先完成调度与 reservation，`persist_start_resources` 持久化 Reserved binding（`media_service_start.rs:76-113`）。
- [x] OpenRtpReceiver/CreatePullProxy、保存media handle和negotiation：由 `SchedulerMediaPort::execute` 以 typed `MediaControlRequest` 发送到媒体节点，`MediaNodeCommandResult` 携带协商结果（`media_service_command_start.rs:34-49`）。
- [x] 执行GB INVITE或完成ONVIF pull：`media_service_start.rs` 将 `StartLive`/`StartPlayback`/`StartTalk`/`StartBroadcast` payload 派发到对应媒体节点命令；协议 INVITE/pull 在媒体节点执行，信令侧不处理媒体负载。
- [x] 等待typed StreamOnline：`MediaEventConsumer` 将 proto `StreamStarted`/`StreamOnline`/`RtpNegotiated` 映射为 `MediaNodeCallbackKind::Started`；`MediaService::apply_media_event` 把 session 从 `Allocating` → `Inviting` → `Active`，binding 从 `Reserved` → `Active`（`media_service_callback.rs:180-200`）。
- [x] ResolveUrls：同步 `MediaNodeCommandResult::Completed` 路径直接返回 `MediaSessionDto`；异步 `Accepted`/`UnknownOutcome` 路径在 session active 后由 `MediaSessionDto` 暴露会话信息，播放 URL 在媒体节点返回并在 `MediaSession` 元数据中承载（后续 `MED-R-006` 完成对账后补充 URL 回填）。
- [x] Operation Succeeded：`media_service_command_start.rs` 中 `MediaNodeCommandResult::Completed` 调用 `operation.complete(OperationResult::success())` 并提交 outbox（`media_service_command_start.rs:98-149`）。

每一步有 `deadline`/`owner_epoch`/`media_node_instance_epoch` 校验；失败时 `dispatch_media_command` 调用 `media_port.release` 释放 reservation，避免残留半成品。

## 4. WF-003：Playback/download/talk

- [x] playback/download使用独立MediaSession和MediaKey：`MediaService::start_playback` 与 `start_talk` 各生成新的 `media_session_id` 与 `media_binding_id`，`MediaRequirements` 按 `Playback`/`Talk` 携带不同 capability 与 codec 需求（`media_service_start.rs:340-430`、`440-530`）。
- [x] pause/resume/seek/scale创建新Operation并串行化：`MediaService::control_playback` 对同 `media_session_id` 使用 `IdempotencyScope` 创建新 `Operation` 并派发 `CommandPayload::ControlPlayback`；按 device/session 路由到固定 shard mailbox，同 session 危险控制自然串行（`media_service.rs:242-340`）。
- [x] talk先验证设备codec/duplex和RTP sender/talk capability：`start_talk` 在 `build_media_requirements` 中设置 `MediaPurpose::Talk` 并要求 `requires_media_sender`；`LeastLoadedScheduler::schedule` 按 capability 过滤 talk 支持节点（`media_service_start.rs:440-530`、`scheduler.rs` capability 匹配）。
- [x] 任一侧`Unsupported`时不得留下Operation外半成品资源：`SchedulerMediaPort::execute` 将媒体节点 `CommandStatus::Unsupported` 映射为 `MediaNodeCommandResult::Failed`，`media_service_command_start.rs` 在失败后调用 `media_port.release` 释放 reservation（`port.rs` 错误映射与 `media_service_command_start.rs:231-260`）。
- [x] device或media结果不确定时进入`UnknownOutcome`与reconciliation：`media_service_command_start.rs` 对 `MediaNodeCommandResult::UnknownOutcome` 记录诊断日志，将 session 驱动到 `Inviting`、binding 到 `Active` 并让 `reconciler` 最终收敛（`media_service_command_start.rs:186-229`）。

## 5. WF-004：Stop

- [x] 创建Stop Operation并把desired state设为Stopped：`MediaService::stop_live` 先按 `IdempotencyScope` 幂等，对 active 的 playback/talk/live 创建 `CommandPayload::StopMediaSession` 的 `Operation`，并设 `MediaSessionDesiredState::Stopped`（`media_service.rs:66-130`、`media_service_command.rs:100-150`）。
- [x] 阻止新的start/control复用：`stop_live` 在 `MediaSession` 进入 Stopping/Stopped 后，`control_playback`/`start_*` 均检查 session state 并拒绝非 active 状态（`media_service.rs:280-286`、`media_service_command.rs`）。
- [x] 停协议dialog/proxy/RTP、释放media handle和reservation：`SchedulerMediaPort::execute` 向媒体节点发送 `StopMediaSession` 命令；成功后 `media_service_command.rs` 调用 `release_binding` 释放 reservation，binding 进入 `Released`/`Failed` 终态。
- [x] binding终态、session Stopped、Operation Succeeded：`release_binding` 调用 `session.stop` 和 `binding.released`，`operation.complete(OperationResult::success())` 并提交 outbox（`media_service_reconciliation.rs:307-340`）。
- [x] 幂等重复返回第一次Operation：`stop_live` 先查 `operation_repository().get_by_idempotency`，命中即返回现有 `OperationDto`/`MediaSessionDto`（`media_service.rs:85-103`）。
- [x] 资源已不存在视为补偿完成；权限、旧owner和错误tenant不能转换为成功：`MediaService::reconcile` 对 media 节点已不存在但本地仍 active 的 session 调用 `migrate_or_fail`；`SchedulerMediaPort::release` 对 `ReservationNotFound` 返回 `Ok(())` 并记录诊断；所有路径均检查 `owner_epoch`、`tenant_id` 与 `media_node_instance_epoch`。

## 6. WF-005：Snapshot 与 record

- snapshot优先复用在线MediaKey；否则ONVIF SnapshotUri通过media `Fetch`执行。
- credential只用短期handle，禁止进入Operation result、event或日志。
- record start/stop/query使用typed handle并关联MediaSession/Binding。
- 异步完成事件必须校验instance/generation，超时后晚到事件不能逆转终态Operation。

## 7. WF-006：补偿

每个step定义：

| 已完成副作用 | 补偿 |
| --- | --- |
| reservation | release reservation |
| media resource | typed Stop/Delete |
| SIP dialog | CANCEL/BYE，按dialog状态选择 |
| proxy | DeletePull |
| credential handle | revoke/expire |
| output/ref count | decrement/close by idle policy |

补偿键由原幂等键+step+generation派生。补偿失败不覆盖原错误，写入step并由reconciler继续。

## 8. WF-007：Reconciler

周期与启动时分页检查：

- Running Operation缺step/dispatch/协议或媒体资源；
- Active Session缺有效Binding；
- Active Binding在media不存在；
- Stopped/Failed Session残留binding、dialog、reservation；
- media orphan无信令binding；
- live reuse ref count为零；
- 旧owner/instance产生的未决结果。

修复只朝当前desired state收敛。用户已Stop后，任何旧Start Operation或事件都不能重建。

## 9. WF-008：并发、配额和过载

- 同device/session命令进入固定shard mailbox。
- 同一MediaSession的危险操作按revision串行。
- tenant/device active session、pending operation和media reservation有hard limit。
- queue满返回Busy/RateLimited并生成metric，不创建Operation后再静默丢弃。
- 禁止持锁跨await和每session常驻task/timer。

## 10. 确定性测试

对每个工作流逐step注入：

- before commit/after commit/before dispatch/after side effect/before result；
- timeout、cancel、重复请求、重复结果、response丢失；
- owner切换、media instance重启、event乱序；
- compensation失败和进程重启。

断言Operation、MediaSession和MediaBinding终态独立且满足不变量。使用FakeClock和确定性ID，不使用真实sleep。

## 11. 退出门禁

- fake media完成所有Saga step故障矩阵。
- 真实media完成live/playback/talk/snapshot基本contract。
- crash/restart不产生两个有效binding或孤儿资源。
- stop后旧事件不能重启会话。

