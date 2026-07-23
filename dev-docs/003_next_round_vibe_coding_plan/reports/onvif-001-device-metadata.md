# ONVIF-001：endpoint、凭据引用、clock offset、capability revision 持久化

## 任务目标

ONVIF 设备探测结果中需要包含足够的 per-device 信息，供上层 `DeviceService` 在注册/更新设备时持久化：endpoint、凭据引用、clock offset、capabilities/services 刷新时间（作为 revision 标记）。

## 实现变更

1. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`
   - `OnvifTokioProtocolDriver::probe` 捕获 `get_system_date_and_time` 的 `SystemDateAndTime` 结果，计算设备 UTC 与本地 UTC 的差值，将 `onvif_clock_offset_seconds` 写入 `CapabilityDescriptor.metadata`。
   - 新增 `clock_offset_seconds` 与 `clock_offset_seconds_with_local` 辅助函数；单元测试使用固定 `local_utc` 验证 37 秒正偏移。
   - 将 `onvif_endpoint`、`onvif_default_credentials_ref`、`onvif_default_username` 一并写入 metadata。
   - 当 `get_services`/`get_capabilities` 成功时，写入 `onvif_services_fetched_at`/`onvif_capabilities_fetched_at`（RFC 3339 时间戳）作为可持久化的 revision 标记。

2. `crates/protocols/cheetah-onvif-driver-tokio/Cargo.toml`
   - 新增 `time` 依赖用于 `OffsetDateTime` 运算和格式化。

3. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - 将 ONVIF-001 的 endpoint/凭据/clock offset/revision 持久化项标记完成。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy -p cheetah-onvif-driver-tokio --tests -- -D warnings`：通过
- `cargo test -p cheetah-onvif-driver-tokio`：通过（含新增 clock offset 单元测试）

## 设计说明

- 持久化动作由上层 `DeviceService::register_or_update_device` 完成；`CapabilityDescriptor.metadata` 中的键值会作为设备 metadata 的一部分写入 repository，满足 per-device 持久化需求。
- 凭据引用仍按现有流程：命令级 `credentials_ref` 优先，缺省则使用 `OnvifConfig.default_credentials_ref`，并通过 `DriverContext::secret` 从 `SecretProvider` 解析出 `SecretString` 密码。
