# ONVIF-006：v1 不支持的 imaging 写操作返回稳定 Unsupported

## 任务目标

ONVIF v1 信令控制面不支持 imaging 写操作。此类命令必须返回稳定的 `Unsupported` 错误，且不能产生任何 Operation 副作用或访问设备。

## 实现变更

1. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`
   - 在 `handle_command` 的 dispatch match 中显式列出 imaging 写命令：
     - `set_imaging_settings`
     - `set_focus_configuration`
     - `set_exposure`
     - `set_white_balance`
     - `set_backlight_compensation`
     - `set_wide_dynamic_range`
     - `set_defog`
     - `set_iris_filter`
     - `set_focus`
   - 这些命令直接返回 `PluginError::Unsupported`，不解析 payload、不构造 SOAP 请求、不创建 Operation。

2. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs` 测试
   - 新增 `imaging_write_commands_return_unsupported` 异步单元测试，覆盖 `set_imaging_settings` 等 5 个典型命令，断言均返回 `PluginError::Unsupported`。

3. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - 将 `v1不支持的imaging写操作保持稳定Unsupported且不产生Operation副作用` 标记完成。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy -p cheetah-onvif-driver-tokio --tests -- -D warnings`：通过
- `cargo test -p cheetah-onvif-driver-tokio`：通过
