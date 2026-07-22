# GB4-ACC-003 完成报告

- 任务 ID：`GB4-ACC-003`
- 结论：完成
- 日期：2026-07-21
- 分支：`devin/gb4-acc-003`（基于 `origin/devin/gb4-base-all`，即合并 ARC/SIP/OPS/ACC-001/002/SEC-004 的组合基线；PR 目标 `main`，stacked on 该基线）

## 目标

完成 GB28181 接入的 listener→tenant 路由、body identity 一致性、protocol session 生成与 endpoint 安全校验：把 REGISTER/Keepalive/MESSAGE 的**不受信 wire 事实**在产生任何领域副作用之前，先经过 tenant 解析、身份一致性、端点安全与网络区校验，再交由 `GB4-ACC-002` 的持久化事务链路 `ProtocolSessionLink` 落库。

## 变更摘要

1. 新增模块层前置校验/路由组件 `cheetah-gb28181-module::ingress::AccessIngress`（`crates/protocols/cheetah-gb28181-module/src/ingress.rs`），位于 `ProtocolSessionLink` 之前：
   - **listener tenant 解析**：以 Request-URI / To 域名匹配配置的 `ListenerBinding`。未配置域名拒绝为 `404`（`UnconfiguredDomain`）；同一域名解析到多个 listener、或 Request-URI 与 To 域名解析到不同 listener，拒绝为 `403`（`AmbiguousDomain`）。tenant 与 `LocalIdentity` 一律取自解析到的 listener，**不信任调用方传入**，从而杜绝把设备塞进未认证租户。
   - **body identity 校验**：对 REGISTER/Keepalive/MESSAGE，MANSCDP body `DeviceID` 必须与 From（From 缺失时回退 To）URI user 部分一致，否则拒绝为 `403`（`BodyIdentityMismatch`）。REGISTER 允许无 body；Keepalive/MESSAGE 必须携带匹配的 `DeviceID`。
   - **endpoint 安全**：`authorize_endpoint_update` 表驱动决策——只有**认证 REGISTER** 或 **in-dialog target refresh** 可改写存储端点；Keepalive/MESSAGE 一律不得改写端点（`EndpointUpdateForbidden`）。`register`/`unregister` 要求 `authenticated=true`（否则 `AuthenticationRequired`/`401`）；`keepalive` 走 `ProtocolSessionLink::keepalive`，该路径本身从不改写 endpoint，因此异源 Keepalive 无法劫持设备路由。
   - **网络区校验**：`ListenerBinding` 可选携带 `NetworkZone`（CIDR）列表；配置后 observed source 必须落在某个区内，否则拒绝为 `403`（`SourceZoneRejected`）。`NetworkZone` 自带 IPv4/IPv6 CIDR 解析与按前缀位比较的 `contains`，不引入新依赖，越界前缀（v4>32、v6>128）与非法 CIDR 在构造期报错。
   - **protocol session 生成**：`AccessIngress` 由解析到的 listener（tenant + local identity）与调用方提供的受信设备/所有权事实（`DeviceBinding`：内部 `DeviceId`、`ProtocolIdentity`、transport、owner node/epoch、compatibility）组装可信 `SessionContext`，再调用 `ProtocolSessionLink` 落库；持久化与 outbox 仍经 `cheetah-domain::ProtocolSessionRepository` port。
   - `IngressError` 为稳定 enum，`sip_status()` 给出稳定 SIP 状态映射（404/403/401/500），不靠字符串判型；不记录任何原始 SIP/XML body。`IngressConfigError` 表达构造期配置错误。
2. 公共导出：`lib.rs` 新增 `AccessIngress`、`DeviceBinding`、`ListenerBinding`、`NetworkZone`、`RequestIdentity`、`IngressMethod`、`IngressError`、`IngressConfigError`。
3. 层次与依赖：仅新增 `cheetah-gb28181-module` 内部代码，复用 `GB4-ACC-002` 的 `ProtocolSessionLink` 与 `cheetah-domain` port（层 4 → 层 6 向下依赖，架构审计允许）；未引入任何存储/NATS/SQLx 依赖，未改动协议 core/driver 或媒体路径。

## 测试

- 单元测试（`src/ingress.rs`，10 项）：IPv4/IPv6 CIDR 匹配、跨地址族不匹配、越界前缀/非法 CIDR 报错、空域名构造报错、body identity（From 命中、失配拒绝、Keepalive 必须带 body）、endpoint 更新授权表、SIP 状态映射稳定。
- 集成测试（`tests/access_ingress.rs`，11 项，使用 `FakeClock`/确定性 ID 与 `InMemoryProtocolSessionRepository`）：
  - listener 域名解析出租户且不泄漏到其他租户；
  - 未认证 REGISTER 被拒（`401`，不落库）；
  - 未配置域名 `404`、同名 listener 歧义 `403`、Request-URI 与 To 域名分歧 `403`；
  - Keepalive body identity 失配 `403`；
  - **跨租户** Keepalive 解析到无会话的租户 → `NotRegistered`（租户隔离）；
  - **端点劫持**：异源 Keepalive 不改写存储 endpoint（回归断言 observed source 不变）；
  - **stale owner**：低 epoch Keepalive 被 fence（`StaleOwner{current:5,got:4}`）；
  - allowed zone 外的 source 在落库前被拒 `403`；
  - 经 ingress 的显式注销删除绑定。

## 验证

```text
cargo fmt --all -- --check                                    # pass
cargo clippy --workspace --all-targets -- -D warnings         # pass
cargo test -p cheetah-gb28181-module                          # pass（lib 10 + access_ingress 11 + session_link 16 等全绿）
cargo test --workspace                                        # 见下方“已知无关失败”
python3 scripts/audit_architecture.py                         # 无新增违规
```

`scripts/audit_architecture.py` 的既有告警（`cheetah-media-scheduler`/`cheetah-onvif-driver-tokio` 层级违规、`cheetah-cluster-registry`/`cheetah-signal-contracts` forbidden dep、`cheetah-onvif-driver-tokio`/`cheetah-storage-api` 两处 `panic!`）均不在本任务范围，且不涉及本次改动文件；新增的 `cheetah-gb28181-module -> cheetah-domain` 为既有向下依赖，未被判为违规。生产路径无 `todo!/unimplemented!/panic!` 与直连 SQL。

## 已知无关失败

- `cargo test --workspace` 中 `cheetah-message-nats` 的一个 doctest（源自 `README.md` 经 `#![doc = include_str!("../README.md")]` 引入，`node_id`/`resolver` 未绑定、`await` 不在 async 上下文）编译失败。该 crate 未被本任务改动（`git diff origin/devin/gb4-base-all -- crates/messaging/cheetah-message-nats` 为空），属基线既有问题（见 `gb4-acc-002.md`）。

## 未运行项

- `cargo nextest`：当前环境未安装 `cargo-nextest`，改用 `cargo test --workspace` 与针对性测试覆盖。
- `buf`、`cargo deny`：本 PR 未修改 `.proto` 与依赖策略，未运行。

## 边界说明

- 本任务实现接入前置校验与 protocol session 生成；tenant/身份/端点/网络区校验后交由 `GB4-ACC-002` 的 `ProtocolSessionLink` 落库。REGISTER 未认证的 401 challenge 与“不落库”语义仍由既有 `Gb28181Access` 状态机在挑战阶段保持，未认证请求不进入 ingress 的持久化路径。
- 把 REGISTER/Keepalive/MESSAGE 事件按新身份模型接入 application handler（去除 logging-only 分支）属 `GB4-EVT-001`；Catalog/DeviceInfo/DeviceStatus bootstrap/query Operation 属 `GB4-ACC-004`，均未在本任务实现。
- 未改动协议 core/driver 状态机，未处理任何媒体负载。
