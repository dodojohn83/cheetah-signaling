# GB4-SYS-005：GB28181 安全、过载和敏感信息泄漏测试报告

## 任务

`GB4-SYS-005`：完成 GB28181 安全、过载和敏感信息泄漏测试报告。

## 范围

本报告汇总当前 `dodojohn83/cheetah-signaling` 代码中已经实现并随 CI 运行的安全、过载与敏感信息保护测试。这些测试对应 `AGENTS.md` 与 `dev-docs/004_gb28181-improve/` 中关于凭据、Digest、准入、脱敏和 drain 的要求，为后续 real-device/NVR 互操作（`GB4-SYS-003`）和级联互操作（`GB4-SYS-004`）提供基线安全证据。

## 1. 安全测试

### 1.1 Digest 凭据与算法策略

来源：GB4-SEC-002 实现与测试（`crates/protocols/cheetah-gb28181-core/tests/digest_validate_tests.rs`、`crates/protocols/cheetah-gb28181-module/tests/register_tests.rs`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `md5_disallowed_by_policy` | policy 为 SHA-256 时 `MD5` algorithm 被拒绝 | pass |
| `qop_downgrade_is_rejected` | `auth-int`/`auth` qop 降级被拒绝 | pass |
| `register_required_rejects_unknown_algorithm_with_400` | 未知 digest 算法在 register 处理阶段返回 `400` | pass |
| `challenge_optional_register_without_auth_succeeds` | `challenge_optional=true` 时未认证 REGISTER 被接受 | pass |
| `challenge_optional_rejects_invalid_credentials_with_401` | `challenge_optional=true` 但携带错误凭据仍返回 `401` | pass |
| `challenge_optional_rejects_credential_backend_error` | 凭据后端故障不被视为“无密码”放行 | pass |

> 注：`challenge_optional` 的 dev-only 策略由 `Gb28181Config` 显式字段控制；`apps/cheetah-signaling/src/assembly.rs` 启动路径会记录警告并标记 readiness insecure，无同名 `#[test]` 覆盖。

### 1.2 Replay / nonce 保护

来源：`crates/protocols/cheetah-gb28181-core/tests/digest_validate_tests.rs`。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `replay_is_detected` | 重复 `(nonce, nc)` 被拒绝 | pass |
| `out_of_order_nc_is_replay` | `nc` 非严格递增被视为 replay | pass |
| `same_nc_with_different_cnonce_is_replay` | 相同 `nc` 但不同 `cnonce` 被视为 replay | pass |
| `expired_nonce_is_stale` | nonce 签名时间戳超过 TTL 被视为 `StaleNonce` | pass |
| `tampered_nonce_fails_signature` | 签名被篡改的 nonce 被拒绝 | pass |

### 1.3 限流与暴力破解

来源：`crates/protocols/cheetah-gb28181-module/tests/register_tests.rs`。

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

来源：GB4-OPS-003 / GB4-OPS-004 实现与测试（`crates/foundation/cheetah-signal-types/src/admission/mod.rs`、`crates/runtime/cheetah-runtime-tokio/src/admission_policy.rs`、`crates/runtime/cheetah-runtime-tokio/tests/admission_test.rs`、`crates/application/cheetah-signal-application/tests/lifecycle_recovery_system.rs`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `token_bucket_enforces_capacity_and_refill` | 令牌桶在容量内允许突发并按速率恢复 | pass |
| `token_bucket_ignores_backwards_time` | 单调时钟回退按零处理，避免重复计票 | pass |
| `keyed_rate_limiter_bounds_keys_via_lru` | per-key 限流器有 LRU 上限，超限 key 被淘汰并计数 | pass |
| `coalescer_collapses_pending_and_releases` | 同一 key 已 pending 事件被折叠 | pass |
| `dead_letter_queue_is_bounded_and_redrivable` | dead-letter 队列有界，满时丢弃最旧 | pass |
| `backlog_controller_has_hysteresis` | backlog high/low watermark 带滞回，避免抖动 | pass |
| `traffic_class_priorities` | `Command` 优先级高于 `Background` | pass |
| `sheds_low_priority_when_overloaded` | 过载时仅丢弃 `Priority::Low` 流量 | pass |
| `admit_sheds_low_priority_then_redrives_after_recovery` | 运行时过载后低优被 drop，恢复后重投 | pass |
| `send_message_rejected_while_draining` | `Runtime::drain` 后新消息被拒绝 | pass |
| `drain_reports_clean_completion_for_idle_runtime` | 空载 drain 干净完成 | pass |
| `sqlite_startup_graceful_shutdown_and_crash_recovery` | 崩溃后旧 owner epoch 命令被 fence，outbox 落库事件由恢复节点重放 | pass |

## 3. 敏感信息泄漏测试

来源：GB4-SEC-004 实现与测试（`crates/foundation/cheetah-signal-types/src/redaction.rs`、`crates/plugin/cheetah-plugin-host/src/oob/log_sanitize.rs`）。

| 测试 | 验证点 | 状态 |
|------|--------|------|
| `redacts_authorization_header` | `Authorization`、`Proxy-Authorization`、`WWW-Authenticate` 等被替换为 `[REDACTED]` | pass |
| `keeps_innocent_lines` | 非敏感字段原样保留 | pass |
| `redacted_display_is_masked` | `Redacted<T>` 的 `Display`/`Debug` 输出 `[REDACTED]` | pass |

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
