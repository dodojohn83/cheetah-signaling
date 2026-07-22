# ONVIF-004：Stateful `OnvifTokioProtocolDriver` 与区分性 health

## 任务目标

让 `OnvifTokioProtocolDriver` 不再是零状态 lifecycle 对象，持有并复用 `OnvifHttpDriver`；同时让 `health` 报告能够区分 driver ready、凭据可用、队列饱和与依赖降级。

## 实现变更

1. `crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs`
   - `OnvifTokioProtocolDriver` 从 unit 类型改为持有 `Arc<Mutex<Option<OnvifHttpDriver>>>`。
   - 新增 `get_or_build_driver`：先尝试从缓存克隆已创建的 driver；未初始化时从 `DriverContext` 解析 `OnvifConfig` 并构建 `OnvifHttpDriver` 后写入缓存。
   - `start` 现在调用 `get_or_build_driver`，在启动期完成 driver 创建与缓存。
   - `handle_command`/`probe` 复用缓存的 driver，不再每次重建 HTTP client。
   - `health` 增强：
     - `driver_ready`：基于 `get_or_build_driver` 是否成功；
     - `credentials_available`：若配置了 `default_credentials_ref`，通过 `ctx.secret` 异步检查；
     - `queue_saturated`：`OnvifHttpDriver` 暴露 `is_request_queue_saturated()`，底层通过 `SoapClient` 的 `Arc<Semaphore>` 判断 `available_permits() == 0`；当并发请求打满时返回 `Degraded`；
     - `dependency_degraded`：`driver_result` 失败、凭据不可用或队列饱和时为 `1`；
     - `status` 综合上述指标返回 `Healthy`/`Degraded`/`Unhealthy`。

2. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - ONVIF-004 四项检查全部标记完成并更新说明。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy --workspace --all-targets -- -D warnings`：通过
- `cargo test --workspace --lib --bins --tests`：通过
- `cargo test --doc --workspace`：通过
- `cargo deny check`：通过
- `python3 scripts/audit_architecture.py`：通过
- `python3 scripts/verify_gb4_fixtures.py`：通过

## 备注

- `application port` 仍通过每次调用传入的 `DriverContext` 访问，不长期持有，符合当前 plugin SDK 接口。
- 队列饱和指标已通过 `OnvifHttpDriver::is_request_queue_saturated()` 实现，底层使用 `SoapClient` 的 `Arc<Semaphore>` 判断 `available_permits() == 0`。
