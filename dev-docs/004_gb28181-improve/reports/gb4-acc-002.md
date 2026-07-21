# GB4-ACC-002 完成报告

- 任务 ID：`GB4-ACC-002`
- 结论：完成
- 日期：2026-07-21
- 分支：`devin/gb4-acc-002`（基于 `origin/devin/gb4-base-acc-sip`，合并了 #161/#173/#177/#178；PR 目标 `main`，stacked on 该组合基线）

## 目标

完成 GB28181 REGISTER/注销/续期/expiry/keepalive/owner acquisition 的持久化事务链路，把 `cheetah-gb28181-module` 的接入行为接到 `cheetah-domain::ports::ProtocolSessionRepository`，使持久化 `ProtocolSession` 成为注册绑定的权威状态，而非仅靠 in-memory 注册表和下游事件。

## 变更摘要

1. 新增模块层事务链路 `cheetah-gb28181-module::session::ProtocolSessionLink`（`crates/protocols/cheetah-gb28181-module/src/session.rs`）：
   - 通过构造器注入 `Clock` 与 `IdGenerator`，无每设备常驻状态，`Clone` 廉价；全部权威状态存于 `ProtocolSession` 聚合。
   - `register`：认证 REGISTER。无既有会话时用 `ProtocolSession::new` 创建（分配 owner node/epoch、写入 Via/Contact/source 端点与 expiry）；已有会话时先做 owner fencing，再按需应用 owner takeover（node 变化或 epoch 增大时 `assign_owner`），随后 `refresh_registration` 更新 expiry/endpoint 并 bump revision，返回 `Created`/`Refreshed`。
   - `unregister`：显式注销（`Expires=0`）。删除注册绑定；无绑定时幂等返回 `None`；删除带 revision 条件。
   - `keepalive`：记录 `last_keepalive_at`、维持在线并 bump revision；对“无会话”返回 `NotRegistered`、对已过期返回 `Expired`（要求重新 REGISTER）、对旧 owner epoch 返回 `StaleOwner`。
   - `acquire_owner`：分片接管设备时更新 `owner_node_id`/`owner_epoch`，要求 epoch 严格递增，相等或更旧一律拒绝为 `StaleOwner`；无会话返回 `None`。
   - `reap_expired`：运维 reaper，用稳定游标分页有界扫描（`page_size`/`max_sessions` 双上限）已过期会话，将仍在线者 `mark_offline("expired")`；已 offline 跳过、并发冲突跳过而不中断整轮，返回本轮转 offline 数量。
   - `SessionLinkError` 为稳定 enum（`NotRegistered`/`Expired`/`StaleOwner`/`Repository(DomainError)`），不靠字符串判型；不记录任何原始 SIP/XML body。
   - `SessionContext` 承载由 application 解析的 tenant、内部 `DeviceId`、`ProtocolIdentity`、`LocalIdentity`、`SipTransport`、owner node/epoch 与 `CompatibilityProfile`——link 只消费这些受信输入，不自行做 listener 路由。
2. 模块依赖：`cheetah-gb28181-module` 新增对 `cheetah-domain` 的依赖（层 4 → 层 6，向下依赖，架构审计允许）；未引入任何存储/NATS/SQLx 依赖，未改动 `cheetah-gb28181-core`/`cheetah-gb28181-driver-tokio`。driver 仍只做 SIP↔`AccessInput`/`AccessOutput` 映射。
3. 测试适配器：`cheetah-domain` 的 `test-util` feature 新增确定性 `InMemoryProtocolSessionRepository`（`crates/domain/cheetah-domain/src/in_memory.rs`），复用既有 revision/租户/游标语义，供模块测试使用；生产路径不链接。
4. 运行期接线（expiry reaper）：
   - `apps/cheetah-signaling/src/workers.rs` 新增 `spawn_protocol_session_reaper_worker`，按固定间隔从 `Storage::protocol_session_repository()` 取仓储并调用 `ProtocolSessionLink::reap_expired`，取消令牌向下传播，重复 tick 幂等。
   - `apps/cheetah-signaling/src/assembly.rs` 在 GB28181 监听装配处按配置启动该 worker（间隔为 0 时不启动）。
   - `cheetah-signal-types::config::Gb28181Config` 新增有界可配置项 `session_reaper_interval_ms`/`session_reaper_batch_size`/`session_reaper_max_per_tick`（含默认值）。

## 测试

新增集成测试 `crates/protocols/cheetah-gb28181-module/tests/session_link.rs`（14 项，全部通过），使用 `FakeClock`/确定性 ID 与 `InMemoryProtocolSessionRepository`：

- REGISTER 创建会话并分配 owner；续期更新 expiry/endpoint/revision 且不新建会话；续期拒绝 CSeq 回退；
- endpoint 漂移在续期后更新 observed source；
- 显式注销删除绑定且幂等；
- keepalive 记录在线并 bump revision、无会话被拒、过期被拒、旧 owner epoch 被拒；
- owner 获取递增 epoch 并 fence 旧 epoch、无会话返回 `None`；REGISTER 期间 owner takeover 生效；
- reaper 将过期会话置 offline 且幂等、分页扫描多页、放过未过期会话。

## 验证

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test -p cheetah-gb28181-module --test session_link # pass (14/14)
cargo test --workspace                                  # 见下方“已知无关失败”
python3 scripts/audit_architecture.py                   # 无新增违规
```

`scripts/audit_architecture.py` 的既有告警（`cheetah-media-scheduler`、`cheetah-onvif-driver-tokio` 层级违规、`cheetah-cluster-registry`/`cheetah-signal-contracts` forbidden dep、两处 `panic!`）均不在本任务范围，且不涉及本次改动文件；新增的 `cheetah-gb28181-module -> cheetah-domain` 为向下依赖，未被判为违规，生产路径无 `todo!/unimplemented!/panic!` 与直连 SQL。

## 已知无关失败

- `cargo test --workspace` 中 `cheetah-message-nats` 的一个 doctest（源自 `README.md`，`node_id`/`resolver` 未绑定）编译失败。该 crate 未被本任务改动（`git status` 无该 crate 文件），基线 `origin/devin/gb4-base-acc-sip` 已存在，属既有问题。

## 未运行项

- `cargo nextest`：当前环境未安装 `cargo-nextest`，改用 `cargo test --workspace` 与针对性测试覆盖。
- `buf`、`cargo deny`：本 PR 未修改 `.proto` 与依赖策略，未运行。

## 边界说明

- 本任务实现 REGISTER/注销/续期/keepalive/owner acquisition/expiry 的持久化事务链路与运行期 reaper。link 消费 application 解析好的 `SessionContext`；listener→tenant、body identity 一致性、protocol session generation 与 endpoint 安全校验属 `GB4-ACC-003`，将 REGISTER/keepalive/注销 事件按新身份模型接入 application handler（去除 logging-only 分支、tenant/dedupe）属 `GB4-EVT-001`——两者未在本任务实现。因此生产 `gb_event_sink` 的注册/保活写入待 ACC-003 提供身份输入后接入本 link；本任务已把 expiry reaper 实际接入运行期。
- 未认证 REGISTER 的 401 challenge 与“不落库”语义由既有 `Gb28181Access` 状态机保持（挑战阶段不进入本 link 的持久化路径）。
- 未改动协议 core/driver 状态机，未处理任何媒体负载。
