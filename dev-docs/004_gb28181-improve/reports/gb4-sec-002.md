# GB4-SEC-002 完成报告

- 任务 ID：`GB4-SEC-002`
- 结论：完成
- 日期：2026-07-21
- 基线分支：`origin/devin/gb4-base-all`（combined base）
- 工作分支：`devin/gb4-sec-002`（目标 `main`，stacked on combined base）

## 0. 范围与与基线的关系

本任务的目标是“收口”GB28181 的 Digest 安全与 SecretStore 策略。combined base（含 `GB4-SIP-004`）已经提供：

- Sans-I/O core digest 校验（password/`now` 由调用方注入）；
- module 侧同步 port `CredentialProvider::password_for(&DeviceId)` 与 `SecretStoreCredentialProvider`；
- assembly 侧 `resolve_digest_secret`（SecretStore + hex + ≥32 字节）；
- 有界 `AuthRateLimiter`（认证前限流，默认 `10 / 60s / 65536`）；
- 有界 `DigestReplayCache`（同一 nonce `nc` 严格递增 + TTL 回收）与 stale nonce。

因此本报告聚焦 **本任务新增/收紧** 的部分，并说明既有能力如何满足 SEC-002 的验收点。

## 1. SecretStore 是唯一凭据来源（审计结论）

- per-device password 只经 `SecretStoreCredentialProvider::password_for` 解析：模板 `{device_id}` 替换后查 `SecretStore`，`NotFound → Ok(None)`，backend 故障 → `CredentialError::Backend`（不当作“无密码”放行）。
- node digest secret 只经 `resolve_digest_secret` 从 `SecretStore` 读取。
- 审计 `cheetah-gb28181-module` 与 `apps/cheetah-signaling/src/assembly.rs`：**不存在** 硬编码/回退明文口令路径。`CascadeConfig.catalog_inbound_digest_credential_ref` 注释中的 “fallback” 只是**引用键**回退（缺省时复用 `credential_ref` 这一 *reference*），不是明文口令回退，符合 AGENTS “凭据按引用获取、禁止明文 secret” 的约束。
- secret 类型仍使用 `SecretString`/`SecretSlice`（不实现可泄漏明文的 `Debug`/`Serialize`）。

## 2. Digest 算法白名单与降级拒绝（新增）

`DigestContext::validate`（`crates/protocols/cheetah-gb28181-core/src/sip/digest/context.rs`）在算法环节收紧为：

1. 缺省 `algorithm` 显式视为 MD5（RFC 2617），使白名单/降级检查统一生效；
2. `MD5 && !allow_md5` → `AlgorithmNotAllowed`（既有）；
3. **新增**：`algorithm != preferred_algorithm` → `AlgorithmDowngrade`。合规客户端总是回显服务端 challenge 中通告的算法（见 `DigestClient::authorize` 固定使用 `challenge.algorithm`），因此任何其它取值——包括“恰好被单独允许的更弱算法”，或对 SHA-256 challenge 省略算法（隐含 MD5）——都被判定为降级/篡改并拒绝。

- 支持 MD5 与 SHA-256，`qop=auth`；`auth-int` 仍拒绝（既有 `InvalidQop`）。
- `MD5-sess` 与未知算法在解析期即被拒：`DigestAlgorithm::parse` 只识别 `md5`/`sha-256`/`sha-512`，其它（含 `md5-sess`）→ `None` → `DigestResponse::parse` 返回 `UnknownAlgorithm`；module 对 `UnknownAlgorithm` 回 `400`。
- 新增错误变体 `DigestError::AlgorithmDowngrade`（含 `Display`）。module 在 Required 路径将其归入 `Err(_) → auth_failed`（`401`）。

## 3. Replay / nonce 加固（既有能力，验收确认）

- `DigestReplayCache` 容量有界（`Gb28181Access` 中 1024 条），同一 nonce 要求 `nc` 严格递增，按 TTL `prune` 回收（默认 300s）。
- nonce 内嵌签名时间戳，`now - ts > ttl` → `StaleNonce`，module 回 `401 + stale=true`。
- 重放的 `response`：由于 `qop=auth` 下 `nc` 必须严格递增，重复的 `(nonce, nc)`（即重放整条 `response`）被 `DigestReplayCache::check` 拒绝为 `ReplayDetected`；`nc` 乱序/回退同样拒绝。相关既有测试见 §5。

## 4. AuthRateLimiter 与 insecure profile 启动策略（新增）

### 4.1 AuthRateLimiter 生产强制

- `AuthRateLimiter` 在 `Gb28181Access::new` 中总是构造，且 apps 装配路径不暴露关闭它的配置项（`GbAccessSettings` 无 rate-limit 字段，始终采用 module 默认 `10 / 60s / 65536`）。因此生产（cluster）下限流不可被关闭。
- 限流在解析/校验 digest **之前** 执行（`is_blocked` 命中即 `429 + Retry-After`），成功认证 `record_success` 清零。

### 4.2 `challenge_optional` 启动策略

`SignalConfig::validate`（`crates/foundation/cheetah-signal-types/src/config.rs`）在推断 effective profile 后新增 `validate_gb28181_challenge_optional_policy`：

- 触发条件 `Gb28181Config::challenge_optional_requested()`：legacy `gb28181.challenge_optional` 或任一 `listeners[].challenge_optional` 为真；
- **cluster/production**（`inferred == Cluster`）下 `challenge_optional=true` → 启动失败；
- profile 非**显式** `edge`（含 profile 未设、由后端推断出的 edge）→ 启动失败，强制显式 opt-in；
- 仅当 `system.profile = "edge"` 显式设置时允许。

`config.example.toml` 的 `challenge_optional` 注释同步说明该策略。

## 5. 测试

### 新增

- core `tests/digest_parse_tests.rs`：`parse_rejects_md5_sess_algorithm`、`parse_rejects_unknown_algorithm`（均 → `UnknownAlgorithm`）。
- core `tests/digest_validate_tests.rs`：`algorithm_downgrade_to_md5_is_rejected`（`allow_md5=true` 但 preferred SHA-256、response MD5 → `AlgorithmDowngrade`）、`unspecified_algorithm_against_sha256_is_rejected`（省略算法对 SHA-256 challenge → `AlgorithmDowngrade`）。
- module `tests/register_tests.rs`：`register_rejects_md5_downgrade_against_sha256_challenge_with_401`（默认 SHA-256 策略下 MD5 应答 → `401`）。
- config `crates/foundation/cheetah-config/tests/config.rs`：`challenge_optional_allowed_with_explicit_edge_profile`、`challenge_optional_requires_explicit_edge_profile`（推断 edge 但未显式 → 拒绝）、`challenge_optional_rejected_in_cluster_profile`（cluster → 拒绝）。

### 既有（覆盖 SEC-002 验收点，回归通过）

- replay/nonce：`replay_is_detected`、`nc` 乱序拒绝、同 `nc` 不同 `cnonce` 拒绝、stale nonce、tampered nonce。
- 限流：`brute_force_source_is_rate_limited_with_429`、`successful_auth_clears_rate_limit_failures`。
- 算法策略：MD5 policy 拒绝、`auth-int` 拒绝、`register_required_rejects_unknown_algorithm_with_400`。

## 6. 本地校验

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo nextest run --workspace`（或 `cargo test --workspace`）：通过。
- `python3 scripts/audit_architecture.py`：通过。

## 7. 架构合规

- core 仍为 Sans-I/O：算法/降级检查是纯函数，`now`/password 注入，无新增 I/O 依赖。
- 秘钥解析仍位于 module/assembly 边界；core 只接收注入的口令与秘钥。
- 新增校验位于 foundation 配置层（启动期），不引入跨层依赖。
- 所有缓存/限流仍有界；外部输入不 panic；生产路径无 `unwrap()/expect()`。
