# GB4-SIP-004 / GB4-SIP-005 完成报告

- 任务 ID：`GB4-SIP-004`、`GB4-SIP-005`
- 结论：完成
- 日期：2026-07-21
- 基线分支：`origin/devin/gb4-sip-002`（PR #178）
- 工作分支：`devin/gb4-sip-004`（目标 `main`，stacked on #178）

## 1. GB4-SIP-004：凭据解析与 Digest 生产链路

### 1.1 凭据解析边界（output/input）

- core（`cheetah-gb28181-core`）不持有任何凭据 provider，也不做 SecretStore/socket/DB I/O：digest 校验通过纯函数 `DigestContext::validate(response, method, uri, password, replay_cache, now)` 完成，password 由调用方注入，`now` 由调用方注入。
- 凭据解析边界落在 module 的同步 port `CredentialProvider::password_for(&DeviceId) -> Result<Option<SecretString>, CredentialError>`。core 不引用该 port；module 的 `Gb28181Access` 持有它并在处理 REGISTER 时显式调用一次，得到脱敏的 `SecretString`（不实现可泄漏明文的 `Debug`/`Serialize`）。
- adapter（module/assembly）以 `SecretStoreCredentialProvider` 实现该 port，用 `SecretStore` 按引用解析 per-device password（模板 `{device_id}` 替换），`NotFound` 映射为 `Ok(None)`，backend 故障映射为 `CredentialError::Backend`（不当作“无密码”放行）。node digest secret 由 `resolve_digest_secret` 从 SecretStore 读取、hex 解码并强制 ≥32 字节。
- 该设计满足 “core 不持有异步 CredentialProvider” 与 “adapter 使用 SecretStore 查询、返回脱敏结果” 的约束：provider 是同步、Sans-I/O 的（不依赖 Tokio），SecretStore 查询在 module/adapter 侧完成，core 只接收注入的口令与时间。

### 1.2 Digest replay / stale / algorithm 生产链路

复用既有 core digest 模块，生产路径的校验顺序为：algorithm policy → realm → uri → qop 一致性 → nonce 签名与 TTL → 响应摘要常量时间比较 → replay 检查。要点：

- **算法降级拒绝**：`allow_md5` 默认 `false`、`preferred_algorithm` 默认 `SHA-256`；MD5 仅在显式开启时接受，`auth-int`（qop）被拒。
- **stale nonce**：nonce 内嵌时间戳，`now - timestamp > ttl`（默认 300s）返回 `StaleNonce`，module 回 401 + `stale=true`。
- **replay**：`DigestReplayCache`（`Gb28181Access` 中上限 1024 条）要求同一 nonce 的 `nc` 严格递增，并按 TTL 有界回收。

### 1.3 Per-source brute-force 限流（新增）

- 新增 `cheetah_gb28181_core::AuthRateLimiter`（`sip/digest/rate_limit.rs`）：按 source IP 在滑动窗口内统计认证失败次数，超过预算即在窗口内阻断；Sans-I/O、`now` 注入、确定性。
- **有界内存**：跟踪的 source 数量上限 `max_sources`，达到上限按 FIFO 淘汰最旧 source；窗口过期的 source 会被 `prune` 清除。`max_failures`/`max_sources`/`window_seconds` 任一为 0 时关闭限流。
- **认证前限流**：`Gb28181Access` 在解析/校验 digest **之前** 检查 `is_blocked`，命中即返回 `429 Too Many Requests` + `Retry-After`，避免被迫做昂贵哈希；符合 AGENTS “认证前限制更严格、按来源分级” 的要求。
- 失败在各拒绝分支通过 `record_failure` 记账；一次成功认证 `record_success` 立即清零该 source 的失败状态，合法设备不会被此前的坏尝试拖累。
- 配置经 `Gb28181DomainConfig::with_auth_rate_limit(max_failures, window_seconds, max_sources)` 注入，默认 `10 / 60s / 65536`。

### 1.4 GB4-SIP-004 测试

- core 单元测试（`rate_limit.rs`）：超预算阻断、窗口过期重置、成功清零、per-source 隔离、source 数量有界、0 预算关闭。
- core `tests/digest_validate_tests.rs`（既有）：MD5/SHA-256 with-qop、stale、tampered nonce、replay、MD5 policy 拒绝、`auth-int` 拒绝、realm/uri mismatch、`nc` 乱序、qop 降级。
- module `tests/register_tests.rs`（新增）：`brute_force_source_is_rate_limited_with_429`（3 次失败后第 4 次返回 429 + Retry-After，且不影响其它 source）、`successful_auth_clears_rate_limit_failures`（成功认证清零计数）。

## 2. GB4-SIP-005：多 listener 路由与旧配置兼容窗口

### 2.1 显式 listener 配置

- 新增 `cheetah_signal_types::config::Gb28181ListenerConfig`：`id`、`tenant_id`、`local_device_id`、`realm`、`domain`、`udp_bind`、`tcp_bind`、`digest_secret_ref`、`device_password_ref`、`challenge_optional`。
- `Gb28181Config` 增加 `listeners: Vec<Gb28181ListenerConfig>`（`serde(default)` + `deny_unknown_fields`），非空时取代旧单 listener 字段。

### 2.2 校验（拒绝歧义）

`Gb28181Config::validate()`（由 `SignalConfig::validate` 调用）强制：

- 新旧混用（`listeners` 非空且任一 legacy 字段非默认）→ 拒绝；
- 每个 listener 必须有非空 `id`/`domain`/`realm`/`local_device_id`/`tenant_id`/`digest_secret_ref`，且至少绑定 udp 或 tcp 之一；
- `id`、`domain`、`realm`、udp bind、tcp bind 唯一——重复即视为歧义拒绝，保证 domain/realm 唯一解析到 tenant；
- cluster 不允许隐式 default tenant：显式 listener 的 `tenant_id` 必填。

### 2.3 旧配置兼容窗口

- `resolve_listeners()`：有显式 `listeners` 时原样返回（`legacy=false`）；否则当 `sip_port > 0` 时把 `sip_port/sip_domain/default_tenant_id/digest_secret_ref/device_password_ref/challenge_optional` 转换为单个合成 listener（`id="legacy"`，`legacy=true`）；都未配置时返回空。
- assembly 在 `legacy=true` 时输出**弃用日志**。`default_tenant_id` 缺省时合成 listener 的 `tenant_id` 为空串，sink 接收 `None`，保持旧的“无 tenant 丢弃事件”语义。

### 2.4 assembly 多 listener 构造

- `apps/cheetah-signaling/src/assembly.rs`：改为遍历 `resolve_listeners()`，每个 listener 构造独立的 `GbAccessSettings`→`build_access`、独立 `DriverConfig`（按 `udp_bind`/`tcp_bind` 绑定）、独立 `gb_event_sink`（携带该 listener 的 tenant）、独立 `Gb28181UdpDriver` 与 worker。
- 路由按 listener 隔离：每个 driver 只服务自己的 bind 地址，从而 realm/domain/tenant 在 socket 层面天然唯一映射；`tenant_id` 非法 UUID 直接启动失败。
- `SignalingRuntime.gb28181_addr` 保留为首个绑定地址以兼容既有测试。

### 2.5 GB4-SIP-005 测试

- `cheetah-signal-types` config 单元测试：空配置合法、legacy 转单 listener 且带弃用标记、新旧混用拒绝、重复 domain/realm/udp bind 拒绝、无 bind 拒绝、无 tenant 拒绝、多 distinct listener 合法。

## 3. 架构决策

- 不把 SecretStore 或异步 provider 下沉进 core；凭据解析保持在 module/adapter 的同步 Sans-I/O port，core 只接收注入口令与时间。
- 限流器置于 core（纯算法、时间注入、有界），由 module 在认证前调用，符合分层与 “bounded state / 认证前更严格” 约束。
- 多 listener 采用 “每 listener 一个 driver/access/sink” 的装配方式（driver 已支持多 bind），避免在 core/driver 引入新的 listener 路由抽象，改动最小且路由显式。

## 4. 验证

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace`（或 `cargo test --workspace`）
- `python3 scripts/audit_architecture.py`
