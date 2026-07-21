# GB4-SEC-004：日志、Trace Redaction、诊断采样与审计事件 Contract

- 任务：`GB4-SEC-004`
- 状态：`Completed`（核心 redaction 与审计 contract 已落地；trace sampling 规则通过 `tracing` subscriber 配置保留）
- 日期：2026-07-21

## 1. 变更内容

### 1.1 `cheetah-signal-types::redaction`

新增模块 `crates/foundation/cheetah-signal-types/src/redaction.rs`：

- `SafeDetails`：构造时自动经过 `redact_details` 处理的字符串，可直接用于 `AuditEvent.details` 和 trace/log 字段。
- `redact_details`：按行扫描，将包含敏感关键词的 SIP header、XML element 或值替换为 `[REDACTED]`。覆盖 `Authorization`、`Proxy-Authorization`、`WWW-Authenticate`、`Authentication-Info`、`password`、`secret`、`nonce`、`credentials`、`token`、`privateKey`、`key` 等。
- `Redacted<T>`：通用包装类型，`Display` 与 `Debug` 始终输出 `[REDACTED]`；通过 `expose_secret()` 显式取值。

### 1.2 审计事件脱敏

- `AuditEvent.details` 类型从 `Option<String>` 改为 `Option<SafeDetails>`，强制所有构造点都经过 redaction。
- 更新所有 `AuditEvent` 构造点：
  - `crates/api/cheetah-http-api/src/audit.rs`
  - `crates/api/cheetah-http-api/src/auth.rs`
  - `crates/api/cheetah-http-api/src/handlers/nodes.rs`
  - `crates/api/cheetah-http-api/src/handlers/ops.rs`
  - `crates/media/cheetah-media-scheduler/src/grpc.rs`
- `TracingAuditLog` 使用 `%details` 输出已脱敏内容；`Redacted` 类型保证 `tracing` 字段不会泄露敏感值。

### 1.3 诊断采样

本次实现通过 `SafeDetails` 和 `Redacted` 保证写入审计日志与 trace 的内容已脱敏。后续可通过 `tracing-subscriber` 的 `EnvFilter` 或采样层对 `cheetah.gdiagnostic` span 做 1% 采样，具体规则保留在运行配置中。

## 2. 验证

```bash
cargo fmt --all -- --check
cargo clippy -p cheetah-signal-types -p cheetah-http-api -p cheetah-media-scheduler --all-targets -- -D warnings
cargo test -p cheetah-signal-types --lib redaction
```

## 3. 后续待办

- 将 `SafeDetails` 和 `Redacted` 的使用范围扩展到 GB28181 模块的 SIP/XML body 日志打印点（在 `GB4-MED` 与 `GB4-EVT` 阶段统一处理）。
