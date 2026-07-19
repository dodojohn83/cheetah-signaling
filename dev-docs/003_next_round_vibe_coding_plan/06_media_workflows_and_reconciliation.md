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

固定步骤：

1. 原子创建Pending Operation、Requested MediaSession、outbox；
2. Operation Running；
3. 校验device/channel/capability和owner；
4. 调度并创建Reserved MediaBinding；
5. OpenRtpReceiver或CreatePullProxy；
6. 保存media handle和negotiation；
7. 执行GB INVITE或完成ONVIF pull；
8. 等待typed StreamOnline；
9. binding Active、session Active；
10. ResolveUrls；
11. Operation Succeeded。

每一步规定deadline、重入条件和补偿。返回URL失败不能丢失已建立资源；按产品语义重试resolve或终止整个start，策略必须配置并测试。

## 4. WF-003：Playback/download/talk

- [ ] playback/download使用独立MediaSession和MediaKey。
- [ ] pause/resume/seek/scale创建新Operation，串行化同session危险控制。
- [ ] talk先验证设备codec/duplex和媒体RTP sender/talk capability，再创建任何资源。
- [ ] 任一侧`Unsupported`时不得留下Operation外半成品资源。
- [ ] device或media结果不确定时进入`UnknownOutcome`与reconciliation。

## 5. WF-004：Stop

1. 创建Stop Operation并把desired state设为Stopped；
2. 阻止新的start/control复用；
3. 停协议dialog/proxy/RTP；
4. 释放media handle和reservation；
5. binding终态；
6. session Stopped；
7. Operation Succeeded。

Stop重复请求按幂等键返回第一次Operation。资源已不存在视为补偿完成；权限、旧owner和错误tenant不能转换为成功。

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

