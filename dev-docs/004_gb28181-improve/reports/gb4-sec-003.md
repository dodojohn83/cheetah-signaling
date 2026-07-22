# GB4-SEC-003 完成报告

- 任务 ID：`GB4-SEC-003`
- 结论：完成
- 日期：2026-07-22
- 基线分支：`origin/devin/gb4-base-all-v3`
- 工作分支：`devin/gb4-sec-001-003`（目标 `main`）

## 0. 范围

实现 [`07_security_observability_and_operations.md`](../07_security_observability_and_operations.md) §4.2「Endpoint 与出站」策略：

- 级联 remote endpoint 匹配允许的 scheme/transport/port/network zone；
- DNS 解析结果在 connect 前后复验，redirect 默认拒绝；
- Contact/SDP 中的私网/公网、IPv4/IPv6 地址按 network zone policy 校验；
- signaling 不直接连接 SDP media endpoint（保持由 `MediaPort` 决定）；
- advertised address 必须显式配置，禁止从不可信 Host/Contact 自动写入。

所有策略实现在 **GB28181 protocol module**（`cheetah-gb28181-module`），复用 foundation `cheetah_signal_types::net::is_internal_ip`，不进入 application/media 层，保持 core 的 Sans-I/O 边界（不做 DNS/socket I/O）。

## 1. 新增：`endpoint_policy` 模块

文件：`crates/protocols/cheetah-gb28181-module/src/endpoint_policy.rs`（纯函数，Sans-I/O）。

### 1.1 network zone 分类

```rust
pub enum NetworkZoneClass { Unspecified, Loopback, LinkLocal, Private, ReservedInternal, Public }
pub fn NetworkZoneClass::classify(ip: IpAddr) -> NetworkZoneClass
```

- 以 `cheetah_signal_types::net::is_internal_ip` 作为 public/internal 边界，额外细分 loopback/link-local/private/unspecified 子区；
- IPv4-mapped IPv6（`::ffff:a.b.c.d`）按内层 IPv4 分类，防止用 v6 包装绕过私网判定。

### 1.2 `EndpointPolicy`

```rust
EndpointPolicy::builder()
    .allow_scheme(..).allow_transport(..).allow_zone(..)
    .allow_ports(..).allow_cidr(..).build()
// 预设：
EndpointPolicy::public_sip()   // 仅公网 + sip/sips
EndpointPolicy::any_zone_sip() // 任意非 unspecified 区（专网/私网部署）
```

校验方法：

- `validate_sip_endpoint(&SipUri) -> EndpointHost`：校验 scheme、`transport=` 参数、port；host 为 IP 字面量时立即校验 zone/CIDR，返回 `EndpointHost::Ip`；host 为域名时返回 `EndpointHost::DomainName`，交由 driver 解析后复验（core 不做 DNS）。
- `validate_ip` / `validate_port`：zone + CIDR 允许表；port `0` 恒拒。
- `validate_sdp_connection(&SdpConnection, port)`：地址必须是良构 unicast IP，family 与 `IP4`/`IP6` 一致，通过 zone policy；含 `/`（multicast TTL/count 记法）或 unspecified 一律拒。
- `verify_resolved_addresses(&[IpAddr])`：DNS 复验；空集拒绝；**任一**地址违反策略即整体拒绝（防 rebinding / 混合解析）。
- `reject_redirect(status_code)`：3xx → `RedirectRejected`，其余透传。

### 1.3 advertised address

```rust
pub fn require_explicit_advertised_host(host: &str) -> Result<(), EndpointPolicyError>
```

拒绝空 host 与 unspecified（`0.0.0.0`/`::`）IP 字面量，确保 advertised（本端）地址来自显式配置而非 wildcard bind 或不可信 header。

错误统一为 `EndpointPolicyError`（`#[non_exhaustive]`，`thiserror`），调用方按变体分支，不依赖字符串。

## 2. 级联集成（`cascade/mod.rs`、`cascade/bridge.rs`）

- `CascadeConfig::with_options` 构造期：
  - `require_explicit_advertised_host(local_uri.host())` 校验本端 advertised host；
  - 用 `upstream_endpoint_policy(allow_internal_upstreams)` 校验上游 endpoint（`public_sip` / `any_zone_sip`），取代原先 ad-hoc 的 `is_internal_ip` 单点判断，同时覆盖 scheme/port/unspecified。行为兼容：`allow_internal_upstreams=false` 仍拒内网字面量。
- `CascadeConfig::verify_upstream_resolved_addresses(&[IpAddr])`：供 driver 在 DNS 解析后与 connect 后调用的复验钩子（Sans-I/O，状态机不自解析）。
- `CascadeConfig::with_sdp_endpoint_policy(EndpointPolicy)` + 新字段 `sdp_endpoint_policy: Option<EndpointPolicy>`：配置后，上游 INVITE 的 SDP 连接地址（session/media 级 `c=`）逐条按 zone policy 校验，越界回 `400` 且不 emit bridge 事件。默认 `None`，保持既有专网部署（设备常用 RFC1918 地址）行为不变。
- redirect：级联响应处理（`cascade/machine_response.rs`）既有「3xx 一律作为失败、绝不跟随」行为保留；`EndpointPolicy::reject_redirect` 提供统一可复用判定与显式单测。

## 3. 架构合规

- core 仍为 Sans-I/O：策略是纯函数，DNS 复验以「driver 解析 → 传回地址集」的注入形式表达（`verify_upstream_resolved_addresses`），module/driver 不在 core 里做 socket/DNS。
- 复用 foundation `is_internal_ip` 与既有 `NetworkZone`（CIDR），未复制网络分类逻辑。
- 未触碰 application/media 层；signaling 不连接 SDP media endpoint（媒体 endpoint 仍由 `MediaPort` 决定）。
- 无 `unwrap()/expect()` 进入生产路径；所有集合/上限保持有界；新增公共类型均有 rustdoc。

## 4. 测试

### 新增（module `src/endpoint_policy.rs` 单测）

- `classify_covers_zone_boundaries`、`ipv4_mapped_ipv6_is_classified_by_inner_v4`；
- `public_sip_policy_accepts_public_ip_literal_endpoint`、`public_sip_policy_rejects_internal_ip_literal_endpoint`、`domain_name_endpoint_defers_to_dns_reverification`；
- `scheme_and_transport_are_enforced`、`port_allow_list_is_enforced`、`cidr_allow_list_further_restricts_public_addresses`；
- `sdp_connection_validation_matches_family_and_zone`、`sdp_public_policy_rejects_internal_media_address`；
- `dns_reverification_rejects_any_internal_address`、`redirects_are_rejected`、`advertised_host_must_be_explicit`。

### 新增（module cascade 集成）

- `cascade/tests.rs::config_accepts_public_upstream_ip`、`config_rejects_upstream_with_unspecified_address`、`config_rejects_unspecified_local_advertised_host`、`verify_upstream_resolved_addresses_enforces_zone_policy`；
- `cascade/tests/bridge.rs::bridge_invite_rejected_when_sdp_address_violates_zone_policy`、`bridge_invite_accepted_when_sdp_address_matches_zone_policy`。

### 回归

- 既有 `config_rejects_internal_upstream_ip`、`config_allows_internal_upstream_ip_when_enabled` 及全部级联/bridge 用例通过（默认 `sdp_endpoint_policy=None` 不改变既有行为）。

## 5. 本地校验

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace --lib --bins --tests`：通过。
- `python3 scripts/audit_architecture.py`：通过。
