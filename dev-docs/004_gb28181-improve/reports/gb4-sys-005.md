# GB4-SYS-005：GB28181 安全、过载和敏感信息泄漏测试报告

## 任务

`GB4-SYS-005`：完成 GB28181 安全、过载和敏感信息泄漏测试报告。

## 范围

本报告汇总当前 `dodojohn83/cheetah-signaling` 代码中已经实现并随 CI 运行的安全、过载与敏感信息保护测试。这些测试对应 `AGENTS.md` 与 `dev-docs/004_gb28181-improve/` 中关于凭据、Digest、准入、脱敏和 drain 的要求，为后续 real-device/NVR 互操作（`GB4-SYS-003`）和级联互操作（`GB4-SYS-004`）提供基线安全证据。

## 1. 安全测试

### 1.1 Digest 凭据与算法策略

来源：GB4-SEC-002 实现与测试（`crates/protocols/cheetah-gb28181-core` / `module` / `config`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `parse_rejects_md5_sess_algorithm` | `MD5-sess` 在解析期被拒绝 | pass |
| `parse_rejects_unknown_algorithm` | 未知 digest 算法在解析期被拒绝 | pass |
| `algorithm_downgrade_to_md5_is_rejected` | 服务端优先 SHA-256 时客户端回退 MD5 被判定为 `AlgorithmDowngrade` | pass |
| `unspecified_algorithm_against_sha256_is_rejected` | 客户端省略 algorithm 字段（隐含 MD5）对 SHA-256 challenge 被拒绝 | pass |
| `register_rejects_md5_downgrade_against_sha256_challenge_with_401` | module 层将降级映射为 `401` | pass |
| `challenge_optional_allowed_with_explicit_edge_profile` | 仅在显式 `system.profile = "edge"` 时允许 `challenge_optional` | pass |
| `challenge_optional_requires_explicit_edge_profile` | 推断 edge 但未显式设置时启动失败 | pass |
| `challenge_optional_rejected_in_cluster_profile` | cluster 模式下 `challenge_optional=true` 启动失败 | pass |

### 1.2 Replay / nonce 保护

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `replay_is_detected` | 重复 `(nonce, nc)` 被拒绝 | pass |
| `nc` 乱序拒绝 | `nc` 非严格递增被拒绝 | pass |
| `stale_nonce` | nonce 签名时间戳超过 TTL 被视为 `StaleNonce` | pass |
| `tampered_nonce` | 签名被篡改的 nonce 被拒绝 | pass |

### 1.3 限流与暴力破解

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `brute_force_source_is_rate_limited_with_429` | 同一来源认证失败超过阈值返回 `429` 并带 `Retry-After` | pass |
| `successful_auth_clears_rate_limit_failures` | 成功认证后失败计数清零 | pass |

### 1.4 凭据来源与 SecretStore

- 所有 digest 密码仅通过 `SecretStoreCredentialProvider::password_for` 从 `SecretStore` 按引用获取；
- `NotFound` 返回 `Ok(None)`，backend 故障返回 `CredentialError::Backend`，不会把“查不到”当作“无密码”放行；
- 审计确认 `cheetah-gb28181-module` 与 `apps/cheetah-signaling/src/assembly.rs` 不存在硬编码/回退明文口令路径；
- `SecretString`/`SecretSlice` 不实现可泄漏明文的 `Debug`/`Serialize`。

## 2. 过载保护测试

来源：GB4-OPS-003 / GB4-OPS-004 实现与测试（`crates/foundation/cheetah-signal-types`、`crates/runtime/cheetah-runtime-tokio`、`crates/application/cheetah-signal-application`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `token_bucket_allows_burst_then_refills` | 令牌桶在容量内允许突发并按速率恢复 | pass |
| `token_bucket_rejects_saturated_bucket` | 桶满后拒绝请求 | pass |
| `token_bucket_ignores_clock_regression` | 单调时钟回退按零处理，避免重复计票 | pass |
| `keyed_rate_limiter_lru_eviction_and_metrics` | per-key 限流器有 LRU 上限，超限 key 被淘汰并计数 | pass |
| `coalescer_merges_equivalent_events` | 同一 key 已 pending 事件被折叠 | pass |
| `coalescer_does_not_block_new_keys_when_full` | coalescer key 上限满时仍放行新 key | pass |
| `dead_letter_queue_bounded_fifo` | dead-letter 队列有界，满时丢弃最旧 | pass |
| `backlog_controller_hysteresis` | backlog high/low watermark 带滞回，避免抖动 | pass |
| `shed_low_priority_when_overloaded` | 过载时仅丢弃 `Priority::Low` 流量 | pass |
| `runtime_drain_rejects_new_work_and_empties_mailbox` | `Runtime::drain(deadline)` 停止新工作并清空 mailbox | pass |
| `lifecycle_recovery_system` | 崩溃后旧 owner epoch 命令被 fence，outbox 落库事件由恢复节点重放 | pass |

## 3. 敏感信息泄漏测试

来源：GB4-SEC-004 实现与测试（`crates/foundation/cheetah-signal-types/src/redaction.rs`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `redact_details_strips_authorization` | `Authorization` header 被替换为 `[REDACTED]` | pass |
| `redact_details_strips_proxy_and_www_authenticate` | `Proxy-Authorization`、`WWW-Authenticate` 被脱敏 | pass |
| `redact_details_strips_password_and_secret` | `password`、`secret`、`nonce`、`credentials`、`token`、`privateKey`、`key` 被脱敏 | pass |
| `redacted_display_is_redacted` | `Redacted<T>` 的 `Display`/`Debug` 输出 `[REDACTED]` | pass |
| `safe_details_passes_through_non_sensitive` | 非敏感字段原样保留 | pass |

`AuditEvent.details` 已从 `Option<String>` 改为 `Option<SafeDetails>`，强制所有审计事件构造点经过脱敏。

## 4. CI 运行结果

当前 `devin/gb4-base-all-v2` 基线（含 `GB4-SEC-002`、`GB4-OPS-003/004`、`GB4-SEC-004`、`GB4-COMP-001`、`GB4-ACC-004`、`GB4-CMD-001`、`GB4-SIP-006`）的 CI 全部通过：

- `fmt-check`：通过；
- `clippy`：通过；
- `deny`：通过；
- `nextest`：通过；
- `proto`：通过；
- `contract-baseline`：通过。

## 5. 结论与后续工作

- 安全、过载和敏感信息泄漏三大领域的核心测试已在 CI 中常态化运行，覆盖 Digest 算法降级、replay/nonce、限流、SecretStore、准入、coalescing、dead-letter、backlog、drain、崩溃恢复、redaction。
- 后续 `GB4-MED`、`GB4-CAS` 阶段新增的媒体协商、级联和事件处理必须继续复用 `SafeDetails`/`Redacted` 与 `TenantIngressAdmission`，避免引入新的敏感信息输出点。
- 随着 real-device/NVR 互操作（`GB4-SYS-003`）和级联互操作（`GB4-SYS-004`）推进，将补充抓包审计与 profile-driven 兼容 workaround 的安全基线证据。
