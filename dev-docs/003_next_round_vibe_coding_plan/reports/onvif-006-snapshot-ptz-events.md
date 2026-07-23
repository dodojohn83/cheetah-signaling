# ONVIF-006：Snapshot、PTZ 与事件 - 实现证据

## 完成项

- `OnvifHttpDriver` 新增 `get_ptz_presets`、`ptz_continuous_move`、`ptz_stop`、`create_pull_point_subscription`、`pull_messages`、`renew_pull_point_subscription`、`unsubscribe_pull_point` 方法，均复用 `acquire_device_permit` + 单一 `deadline` 模式，保证并发与超时语义一致。
- `OnvifTokioProtocolDriver::handle_command` 新增 `take_snapshot`、`ptz_get_presets`、`ptz_continuous_move`、`ptz_stop`、`create_pull_point_subscription`、`pull_messages`、`renew_pull_point_subscription`、`unsubscribe_pull_point` 命令映射。
- `take_snapshot` 返回的快照 URI 在作为 `onvif.snapshot_uri` 协议事件发射前通过 `redact_uri_userinfo` 脱敏 userinfo。
- `ptz_continuous_move` 强制调用方提供 `timeout_seconds`，速度分量在命令层通过 `clip_unit` 裁剪到 `[-1, 1]`；`continuous_move_request` 与 `move_with_vector`（RelativeMove/AbsoluteMove）在 `tptz:*Move` 元素上声明 `xmlns:tt="http://www.onvif.org/ver10/schema"`，确保速度/位置分量 `<tt:x>`/`<tt:y>` 前缀已绑定，符合命名空间敏感的 SOAP 解析器要求；`ptz_stop` 默认停止 pan/tilt 和 zoom。
- `create_pull_point_subscription` 返回的 `PullPointSubscription` 以 `onvif.pull_point_subscription` 事件向北向暴露（subscription reference 已脱敏），为 `pull_messages`/`renew`/`unsubscribe` 提供输入。
- `pull_messages` 通过 `message_limit` 限制单批返回通知数，超出即解析错误；返回的批量通知以单条 `onvif.notification` 事件发射，避免部分 emit 失败导致已消费通知丢失；空结果不再向北向事件总线发射，防止 idle long-poll 产生大量无价值消息；通知 topic 使用 `normalize_topic` 映射为稳定的北向事件类型。
- `renew_pull_point_subscription`/`unsubscribe_pull_point` 提供事件订阅生命周期管理，`unsubscribe` 作为显式清理路径释放设备端 pull-point 资源。
- 所有出站 SOAP 目标在 `OnvifHttpDriver::post_with_optional_auth` 中先通过 `validate_endpoint` 执行 `XAddrPolicy` SSRF 校验（scheme、端口、userinfo、目标网段），防止命令 payload 中的 `subscription_reference`、`ptz_endpoint`、`events_endpoint`、`media_endpoint` 等被恶意指向内网服务并泄漏凭据。
- `renew_request` 在 `tev:Renew` 元素上声明 `xmlns:wsnt="http://docs.oasis-open.org/wsn/b-2"`，修复生成的 `<wsnt:TerminationTime>` 前缀未绑定的问题，并补充单元测试防止回归。
- 测试 `FakeDriverContext` 中的 `unimplemented!()` 替换为稳定的 `PluginError::Unsupported` 错误，消除 `audit_architecture.py` 的 placeholder 告警。
- 无效的 `tenant_id` 在命令入口即通过 `parse_tenant_id` 返回 `PluginError::Driver` 错误，避免在执行设备副作用（如创建 pull-point subscription）后才发现无法归属事件。
- `parse_pull_messages_response` 对 `Source`/`Key`/`Data` 扩展片段做多字节安全截断：按字符边界截断到不超过 128 字节，并在总长度接近 512 字节时停止追加，避免非 ASCII 设备事件文本触发 UTF-8 切片 panic。

## 关键文件

- `crates/protocols/cheetah-onvif-driver-tokio/src/lib.rs`：驱动层方法。
- `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`：命令映射、事件发射与单元测试。
- `crates/protocols/cheetah-onvif-module/src/services/events.rs`：导出 `RENEW_ACTION`/`UNSUBSCRIBE_ACTION` 并复用 `renew_request`/`unsubscribe_request`；修复 `renew_request` 的 `wsnt` 命名空间声明。
- `crates/protocols/cheetah-onvif-module/src/services/ptz.rs`：PTZ 请求构造器，修复 `tt` 命名空间声明。
- `crates/protocols/cheetah-onvif-module/src/services/mod.rs`：同步导出事件相关 action 常量与请求构造器。
- `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`：任务追踪 ONVIF-006 全部勾选。

## 验证结果

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins --tests
cargo test --doc --workspace
cargo deny check
python3 scripts/audit_architecture.py
python3 scripts/verify_gb4_fixtures.py
```

全部通过。`audit_architecture.py` 仍报告两项已存在的依赖层违规（`cheetah-media-scheduler -> cheetah-media-client`、`cheetah-onvif-driver-tokio -> cheetah-onvif-module`）和三个 foundation 层 forbidden dependency 警告，均为既有架构问题，非本次改动引入；placeholder 与 panic 检查已清零。
