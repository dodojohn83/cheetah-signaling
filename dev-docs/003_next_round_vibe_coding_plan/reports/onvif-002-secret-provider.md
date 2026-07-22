# ONVIF-002：WS-Security UsernameToken 使用 SecretProvider

## 任务目标

将 `cheetah-onvif-driver-tokio` 的 WS-Security UsernameToken 凭据解析从明文内联改为优先通过 `DriverContext::secret` 从 SecretProvider 获取，确保密码不进入日志或序列化输出。

## 实现变更

1. `crates/foundation/cheetah-signal-types/src/config.rs`
   - `OnvifConfig` 新增 `default_username: Option<String>` 与 `default_credentials_ref: Option<String>`。
   - 默认值均为 `None`，向后兼容。

2. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`
   - `EndpointCommand` / `MediaCommand` / `StreamUriCommand` / `SnapshotUriCommand` 新增 `credentials_ref: Option<String>`。
   - 新增 `resolve_credentials` 异步函数：
     - 命令级 `credentials_ref` 优先；
     - 若缺失，回退到 `OnvifConfig::default_credentials_ref`；
     - 通过 `ctx.secret(ref_name).await?` 获取 `SecretString`；
     - 与命令级或配置级 `username` 组合为 `DeviceCredentials`；
     - 保留 `password` 内联字段作为未配置 SecretProvider 时的降级路径。
   - `dispatch_command` 与 `probe` 现在显式调用 `resolve_credentials`；`get_system_date_and_time` 保持无认证。
   - `handle_command`、`probe`、`health` 复用同一份 `OnvifConfig`，避免重复解析。
   - 新增单元测试验证：
     - 命令级 `credentials_ref` 优先于配置默认值；
     - 配置默认值可作为 fallback；
     - SecretProvider 未找到引用时返回错误。

3. `config.example.toml`
   - `[onvif]` 段新增 `default_username` 和 `default_credentials_ref` 示例与注释。

4. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - ONVIF-002 WS-Security 项标记为完成，并更新 ONVIF-001 持久化说明。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy --workspace --all-targets -- -D warnings`：通过
- `cargo test --workspace --lib --bins --tests`：通过
- `cargo test --doc --workspace`：通过
- `cargo deny check`：通过（仅有预存 layer/forbidden dep 警告，与本改动无关）
- `python3 scripts/audit_architecture.py`：通过（`Test-fake todo!/unimplemented! hits` 仅在测试 fake 中）
- `python3 scripts/verify_gb4_fixtures.py`：通过

## 备注

- 架构审计中的 layer 违规 `cheetah-onvif-driver-tokio -> cheetah-onvif-module` 为既有问题（`MediaDialect` 依赖），未在本次任务范围内。
- `get_system_date_and_time` / probe 仍保持无认证；后续 ONVIF-003 取 `GetDeviceInformation` 等能力时，如需认证可在命令中携带 `credentials_ref`。
