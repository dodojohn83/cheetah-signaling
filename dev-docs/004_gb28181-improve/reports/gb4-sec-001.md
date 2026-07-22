# GB4-SEC-001 完成报告

- 任务 ID：`GB4-SEC-001`
- 结论：完成
- 日期：2026-07-22
- 基线分支：`origin/devin/gb4-base-all-v3`
- 工作分支：`devin/gb4-sec-001-003`（目标 `main`）

## 0. 范围

本任务更新 GB28181 威胁模型，把 [`07_security_observability_and_operations.md`](../07_security_observability_and_operations.md) §2 的威胁增量拆解为可追踪的风险条目，并把每条风险映射到**既有**或**本轮新增**的安全 regression。实现类风险（Contact/SDP/级联 endpoint、network zone、DNS 复验、redirect）的落地在 `GB4-SEC-003`，参见 [`gb4-sec-003.md`](./gb4-sec-003.md)。

术语：core = `cheetah-gb28181-core`，module = `cheetah-gb28181-module`，driver = `cheetah-gb28181-driver-tokio`，foundation = `cheetah-signal-types`。

## 1. Parser / 编解码风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| SIP start line/header/body/frame 超限、粘包/半包、模糊 Content-Length | core `SipParser` + `SipParserConfig` 增量解析、显式上限 | core `src/sip/parser.rs` 解析器单测；driver `tests/tcp_lifecycle.rs` framing/oversize |
| SIP header CRLF 注入 | core digest/URI/header 编码统一走类型化 encoder，`strip_crlf` 清洗 challenge | core `tests/digest_parse_tests.rs::challenge_header_round_trips_quoted_realm_and_strips_crlf`；module `cascade::validate_token`（`cascade/tests.rs` 拒绝注入 token） |
| SDP line/media/payload/attribute 超限、地址/端口注入 | core `parse_sdp` + `SdpParserConfig`；module 级联入站用有界 `UPSTREAM_SDP_CONFIG` | core `src/sdp` round-trip/bound 单测；module `cascade/tests/bridge.rs` 畸形 SDP → `400` |
| XML DTD/XXE、深度/节点/文本炸弹、charset 混淆 | module `xml` 解析禁用 DTD/外部实体，`XmlLimits` 限深度/节点/文本 | module `src/xml` 限制单测；`config` charset policy 单测 |
| SDP 编码非法字符注入 | core `sdp/encoder.rs` `validate_no_*` 校验 | core `src/sdp/encoder.rs` 单测 |

## 2. 认证 / 授权 / 凭据风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| 未认证 REGISTER/MESSAGE flood、暴力口令 | module `AuthRateLimiter`（解析 digest 前限流，生产不可关闭） | module `tests/register_tests.rs::brute_force_source_is_rate_limited_with_429`、`successful_auth_clears_rate_limit_failures` |
| nonce 重放、`nc` 回退 | core `DigestReplayCache`（`nc` 严格递增 + TTL） | core `tests/digest_validate_tests.rs`（replay/`nc` 相关）；SEC-002 报告 §5 |
| algorithm/qop downgrade（MD5 隐式降级、`auth-int`） | core `DigestContext::validate` 白名单 + `AlgorithmDowngrade` | core `tests/digest_validate_tests.rs::*`、module `tests/register_tests.rs::register_rejects_md5_downgrade_against_sha256_challenge_with_401` |
| stale/篡改 nonce | core nonce 内嵌签名时间戳 | core `tests/digest_validate_tests.rs::expired_nonce_is_stale`、`tampered_nonce_fails_signature` |
| 明文 secret 泄漏 | 凭据经 `SecretStore` 按引用获取；`SecretString`/`SecretBox` 不实现可泄漏 `Debug` | core `src/sip/uri.rs::password_is_split_and_round_tripped_but_redacted_in_debug`；SEC-002/SEC-004 |

## 3. Endpoint / source 劫持风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| Via/Contact/source/rport 欺骗改写发送目标 | core `EndpointRoute`（`received:rport` → Contact → observed source 优先级），module 只有认证 REGISTER 能改写 route | core `src/sip/endpoint.rs` route 单测；module `tests/access_ingress.rs::keepalive_from_new_source_does_not_hijack_endpoint` |
| 非认证 source drift | core `EndpointRoute::is_unauthenticated_drift` | core `src/sip/endpoint.rs` drift 单测 |
| Contact/SDP 私网/公网地址触发内部网络探测（SSRF） | **新增** module `EndpointPolicy`（network zone 分类 + 校验） | **新增** module `src/endpoint_policy.rs` 单测；`cascade/tests/bridge.rs` SDP zone 拒绝/放行 |
| advertised address 来自不可信 Host/Contact | **新增** `require_explicit_advertised_host` 强制显式配置 | **新增** module `src/endpoint_policy.rs::advertised_host_must_be_explicit`、`cascade/tests.rs::config_rejects_unspecified_local_advertised_host` |

## 4. Tenant 边界风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| realm/domain/DeviceID 混淆导致 tenant 越界 | module `AccessIngress`：tenant 由 listener 身份决定，body `DeviceID` 必须匹配 From/To | module `tests/access_ingress.rs::register_resolves_tenant_from_listener_domain`、`keepalive_across_tenant_is_not_registered` |
| 级联目录跨租户泄漏 | module `CatalogFilter`（tenant + whitelist + tag/org 前缀） | module `cascade/tests/catalog.rs::catalog_filter_respects_tenant_and_whitelist`、`catalog_filter_respects_tags_and_org_prefix` |

## 5. Control / PTZ / DeviceControl 风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| 旧 owner / stale epoch 命令推进状态 | module owner epoch fencing（副作用前后校验） | module `tests/session_link.rs::register_rejects_stale_owner_epoch`、`keepalive_rejects_stale_owner_epoch`、`acquire_owner_increments_epoch_and_fences_stale`；`tests/access_ingress.rs::keepalive_rejects_stale_owner_epoch` |
| 命令重放/重复 | Command 携带 request/idempotency/owner epoch（domain 契约） | domain 层 Operation/Command 契约测试（`cheetah-domain`） |

## 6. 级联（cascade）风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| 级联出站 endpoint 指向内网/非法 scheme/port（SSRF、redirect 跟随） | **新增** `EndpointPolicy` 校验 scheme/transport/port/zone；`reject_redirect` 默认拒 3xx；DNS 复验 | **新增** module `src/endpoint_policy.rs::*`；`cascade/tests.rs::config_accepts_public_upstream_ip`、`config_rejects_upstream_with_unspecified_address`、`verify_upstream_resolved_addresses_enforces_zone_policy`；既有 `cascade/machine_response.rs` 3xx→失败 |
| DNS rebinding（connect 前后地址变化） | **新增** `CascadeConfig::verify_upstream_resolved_addresses`（connect 前后复验，任一内网地址整体拒绝） | **新增** `cascade/tests.rs::verify_upstream_resolved_addresses_enforces_zone_policy` |
| subscription exhaustion | module 有界 `subscription_max_subscriptions` | module `cascade/tests/subscription.rs::*`（创建/续订/过期/清理/上限） |
| catalog 分页/条目洪水 | module `catalog_max_query_pages` / `catalog_max_items_per_packet` | module `cascade/tests/catalog.rs::catalog_query_respects_max_pages_cap` |
| bridge loop / 并发 bridge 耗尽 | module 有界 `media_bridge_max_sessions` + 事务/活跃超时 | module `cascade/tests/bridge.rs::bridge_max_sessions_returns_486` |
| ID collision | module 级联 local tag / call-id 由注入源生成 | module `cascade` tag/call-id 单测 |

## 7. Media callback 风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| media callback 伪造、旧 media node instance 推进新 binding | 修改型媒体请求携带 owner node/epoch、target media node instance epoch；旧 epoch 回调拒绝（domain `MediaSession`/`MediaBinding` fencing） | domain 层 media binding fencing 契约测试；module `cascade/tests/bridge.rs` bridge 生命周期（就绪/失败/清理）不接受越权推进 |
| signaling 直接连接 SDP media endpoint | 架构约束：媒体 endpoint 由 `MediaPort` 决定，signaling 不连接 | `scripts/audit_architecture.py`（依赖方向）；module 不含 RTP/RTCP 绑定 |

## 8. 信息泄漏风险

| 风险 | 控制点 | 测试映射 |
| --- | --- | --- |
| 日志/trace/error 泄漏 secret、完整 body、Contact/RTSP userinfo、高敏地址 | redaction / 诊断采样 / 审计契约（SEC-004） | 见 [`gb4-sec-004.md`](./gb4-sec-004.md) redaction 测试 |

## 9. 本地校验

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace --lib --bins --tests`：通过。
- `python3 scripts/audit_architecture.py`：通过。

## 10. 后续项

- media callback / owner epoch 的端到端 fencing 主要在 domain/application 层，本文件只做威胁映射，不在 protocol module 重复实现。
- 新增 profile / outbound endpoint / parser fallback 时必须回到本表补充风险与 regression（承接 §2 末尾要求）。
