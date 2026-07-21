# GB4-SIP-006 完成报告

- 任务 ID：`GB4-SIP-006`
- 结论：完成
- 日期：2026-07-21
- 基线分支：`origin/devin/gb4-base-all`（combined base，`174b2d5 merge gb4-sec-004`）
- 工作分支：`devin/gb4-sip-006`（目标 `main`，stacked on combined base）

## 1. 目标

实现 endpoint route 模型、NAT/`rport` 策略与 source hijack regression：让设备发送目标来自认证建立的 route，而不是“当前输入 source”，从而抵御 NAT 变化与 keepalive/MESSAGE 源伪造。

## 2. Core：类型化 `EndpointRoute` 模型（`cheetah-gb28181-core`）

新增 `sip/endpoint.rs`（Sans-I/O，仅依赖 `std::net` 与既有 `SipUri`），并在 crate 根 re-export。

### 2.1 区分的 endpoint 语义

`EndpointRoute` 显式保留互不折叠的五类观测，符合设计 §8：

- `observed_source: SocketAddr`——最近一次被接受报文的传输源；
- `via_received_rport: Option<SocketAddr>`——由顶层 Via `received`/`rport` 参数推导的 RFC 3581 端点（NAT 映射后的公网地址）；
- `contact_uri: Option<SipUri>`——设备通告的 Contact URI；
- `advertised_endpoint: Option<SocketAddr>`——当 Contact host 为 IP 字面量时解析出的 socket 地址（域名在 Sans-I/O core 中不解析，返回 `None`）；
- `dialog_remote_target: Option<SipUri>`——对话内 remote target。

### 2.2 NAT/`rport` 选择策略

- `ViaRouteParams::parse` 解析顶层 Via 的 `received=`（`IpAddr`）与 `rport`（`Rport::Absent`/`Requested`/`Value(u16)`）；畸形 `rport` 值降级为 `Requested`（token 存在即表示对端要对称路由），畸形 `received` 忽略。
- `ViaRouteParams::resolved_endpoint(observed)`：仅当请求带 `rport` 时返回 `Some`——host 取 `received`（缺省用 observed IP），port 取显式 `rport=<port>`（缺省用 observed port）。
- `EndpointRoute::send_target()` 按 **`received:rport` → Contact host:port → observed source** 的优先级解析出对外/带外发送目标。
- `EndpointRoute::dialog_send_target()`：对话内请求优先用 `dialog_remote_target`（IP 字面量），否则回落 `send_target()`——即 in-dialog 请求使用 dialog route/target，不复用 REGISTER route。

### 2.3 source hijack 判定与更新上下文

- `EndpointRoute::is_unauthenticated_drift(source)`：当 `source` 既非 observed source 也非当前 send target 时为 `true`（潜在源劫持信号）。
- `RouteUpdateContext`（`AuthenticatedRegister`/`DialogTargetRefresh`/`CompatibilityProfile`/`UnauthenticatedKeepalive`/`UnauthenticatedRequest`）+ `may_change_route()`：仅认证 REGISTER、dialog target refresh、显式 compatibility profile 允许改变发送 route——普通 Keepalive/MESSAGE 不得改写 endpoint。

## 3. Module：注册状态使用 route（`cheetah-gb28181-module`）

- `Registration` 以类型化 `route: EndpointRoute` 取代此前分离的裸 `source`/`contact` 字段；新增 `Registration::source()` 返回 `route.send_target()`，作为事件与带外发送的权威地址。
- REGISTER 接受路径（`register_accepted`，仅在认证或 challenge-optional 接受时进入）用 `EndpointRoute::from_registration(observed_source, top_via, contact_uri)` 建立 route 并 `upsert`——这是唯一的 route 建立/迁移入口。
- **source hijack 修复**：`RegistrationTable::touch`（keepalive/MESSAGE 路径）不再执行 `reg.source = source`；只刷新 `last_seen`/在线状态，并返回 `TouchOutcome { was_offline, source_drift }`。`source_drift` 命中时记录 warning，报文仍按在线处理，但存储的 route **保持不变**。
- 新增只读访问器 `Gb28181Access::device_send_target(&DeviceId)` 与 `device_route(&DeviceId)`，供带外发送与测试查询解析后的目标。
- `tick()` 中过期/离线事件的 `source` 改用 `reg.source()`（认证建立的稳定地址），不再随最近一次 keepalive 漂移。

## 4. 测试

### 4.1 core 单元测试（`sip/endpoint.rs`，9 项）

Via 参数解析（flag/value/received、畸形降级）、`resolved_endpoint` 的 received/observed 优先级、`send_target` 的 rport→contact→observed 三级优先级、Contact 缺省端口 5060、IPv6 字面量 Contact、`dialog_send_target` 优先 dialog target、未认证 drift 判定、`RouteUpdateContext` 权限。

### 4.2 driver 集成测试（`tests/endpoint_routing.rs`，6 项）

驱动 driver 的 TU（`Gb28181Access`）并断言解析目标，另含一项真实 UDP socket 用例：

- `rport_policy_prefers_observed_source_over_private_contact`：Via/Contact 为私网地址但带 `;rport` → send target = 公网 observed source；
- `nat_rewrite_uses_public_contact_when_no_rport`：无 rport 时公网 Contact 端点胜出；
- `no_rport_and_unresolvable_contact_falls_back_to_observed_source`：域名 Contact 无法解析且无 rport → 回落 observed source；
- `authenticated_reregister_moves_endpoint`：认证 re-REGISTER 从新源迁移 route（endpoint drift 允许）；
- `keepalive_from_hijack_source_does_not_move_endpoint`：伪造源 keepalive 不改写 route，合法源 keepalive 亦保持；
- `register_response_is_routed_to_observed_source`：真实 UDP 端到端，401 响应回到 client 实际源（对称路由回归守卫）。

## 5. 架构决策

- endpoint/route 模型置于 core（纯类型 + 策略、无 I/O、`std::net` only），符合 core Sans-I/O 与分层约束；driver/module 复用同一模型，不引入并行实现。
- 域名 Contact 不在 core 做 DNS 解析（Sans-I/O），退化为 `advertised_endpoint = None` 并回落 observed source，保持确定性。
- keepalive/MESSAGE 仅更新在线状态、不改写 route，且非认证 drift 记录告警而非直接丢弃——避免误伤漫游设备的同时消除源劫持面；route 迁移只经认证 REGISTER。
- 未做响应 Via 的 `received`/`rport` 头改写（RFC 3581 服务端装饰）：现有 driver 已将响应发回 observed source，本工单聚焦于**存储 route 的 NAT/rport 策略**；Via 头装饰留待后续带外命令派发工单。

## 6. 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`（环境无 `cargo-nextest`；GB28181 三 crate 全绿。`cheetah-message-nats` 的 1 项 doctest 失败为基线既有、与本改动无关，已在干净基线复现）
- `python3 scripts/audit_architecture.py`（无新增 GB 依赖/分层违规，输出与基线一致）
