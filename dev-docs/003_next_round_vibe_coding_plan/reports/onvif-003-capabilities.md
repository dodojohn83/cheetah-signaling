# ONVIF-003：Provision 与能力（部分完成）

## 任务目标

实现 `GetServices`/`GetCapabilities`/`GetDeviceInformation`/`GetSystemDateAndTime` 的 ONVIF 能力探测，并支持部分成功：即使凭据或服务能力请求失败，仍返回已获取的部分信息。

## 实现变更

1. `crates/protocols/cheetah-onvif-driver-tokio/src/lib.rs`
   - `OnvifHttpDriver` 新增 `get_services` 与 `get_capabilities` 方法。
   - 复用 `cheetah-onvif-module` 已有的请求构造与响应解析函数（`get_services_request`、`parse_get_services_response`、`get_capabilities_request`、`parse_get_capabilities_response`）。

2. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`
   - `EndpointCommand` 新增 `include_capabilities` 字段（默认 `false`），供 `get_services` 命令使用。
   - `dispatch_command` 新增 `get_services` 与 `get_capabilities` 命令分支。
   - `probe` 在 `get_system_date_and_time` 成功后，使用配置默认凭据尝试 `get_services`/`get_capabilities`；失败仅写入 `CapabilityDescriptor.metadata`，不导致 probe 失败，体现“部分成功”语义。
   - 新增 `services_to_json` 和 `capabilities_to_json` 辅助函数，将结果序列化为 JSON 字符串存入 descriptor metadata。

3. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - ONVIF-003 首项检查已标记完成；其余 workflow 重入/取消、capability TTL/ETag 刷新仍待后续实现。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy -p cheetah-onvif-driver-tokio --tests -- -D warnings`：通过
- `cargo test -p cheetah-onvif-driver-tokio`：通过

## 备注

- 本次仅完成 ONVIF-003 的“可部分成功”能力探测部分；`workflow 可重入/可取消/设备级并发受限` 和 `capability TTL/ETag 刷新` 尚未实现。
