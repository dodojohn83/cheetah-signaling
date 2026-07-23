# ONVIF-003：workflow 可重入、可取消、设备级并发受限

## 任务目标

ONVIF 探测/取流/截图 workflow 需要具备：
- 可重入：同一 driver 实例能被多个调用者/Operation 并发调用；
- 可取消：每个阻塞阶段都有 timeout/deadline；
- 设备级并发受限：对同一 camera endpoint 的并发请求数有上限，避免压垮设备。

## 实现变更

1. `OnvifHttpDriver` 设计为 `Clone`、`&self` 且无跨请求可变状态，天然支持重入。

2. `OnvifHttpDriver` 新增 per-endpoint 信号量：
   - `per_device_concurrency: usize`（`OnvifConfig`/`DriverConfig` 可配，默认 2）。
   - `device_permits: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>`。
   - 私有异步方法 `acquire_device_permit(endpoint, timeout)` 在每次 `get_device_information`、`get_system_date_and_time`、`get_services`、`get_capabilities`、`get_profiles`、`get_stream_uri`、`get_snapshot_uri` 调用前获取 permit。
   - 信号量使用 `tokio::sync::Semaphore::acquire_owned()` 得到 `OwnedSemaphorePermit`，future 取消或超时时自动释放。

3. `soap_client.rs` 暴露 `request_timeout()`，使 `acquire_device_permit` 在 `timeout` 为 `None` 时使用客户端默认请求超时作为等待上限。

4. `OnvifConfig`/`DriverConfig` 新增 `per_device_concurrency`；`config.example.toml` 同步添加。

5. 新增单元测试 `per_device_concurrency_limits_concurrent_calls`（按 Devin Review 改为零超时、无真实 sleep 的确定性测试）：
   - 构造 `per_device_concurrency = 1` 的 driver；
   - 主任务获取 endpoint permit；
   - 同一任务以 `timeout = 0` 再次尝试获取同一 endpoint permit；
   - 断言立即返回 `DriverError::Timeout`。

6. 修复 Devin Review 指出的四个缺陷：
   - 所有公共方法现在将调用者传入的 `timeout` 转换为一个 `Instant` deadline，permit 等待与后续 HTTP 请求都通过 `resolve_timeout(deadline)` 复用同一个剩余时间，避免总耗时超过调用者预期两倍的 `timeout`。
   - `device_permits` 新增 `max_tracked_device_endpoints` 上限，并在每次插入前驱逐满足 `Arc::strong_count == 1` 且 `available_permits == per_device_concurrency` 的空闲 entry，防止长期运行进程无界增长。
   - `per_device_concurrency_limits_concurrent_calls` 不再依赖真实 `tokio::time::sleep`，而是使用 `#[tokio::test(flavor = "current_thread", start_paused = true)]` 与零超时完成确定性测试；为此 dev-dependencies 新增 `tokio/test-util`。
   - 在 `get_services`/`get_capabilities` 中，permit 等待或 deadline 超时不再直接 `?` 返回，而是先检查 `stale_services`/`stale_capabilities`，有则返回上次可用结果，保持方法文档中“刷新失败不丢失上次能力”的语义。
   - 新增单元测试 `idle_device_permits_are_evicted_when_map_exceeds_capacity`。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy --workspace --all-targets -- -D warnings`：通过
- `cargo test --workspace --lib --bins --tests`：通过
- `cargo test --doc --workspace`：通过
- `cargo deny check`：通过
- `scripts/audit_architecture.py`：通过
- `scripts/verify_gb4_fixtures.py`：通过

## 设计说明

- `OnvifHttpDriver` 本身不创建 `Operation`/`Saga`；重入/取消/并发能力通过 `&self` + per-endpoint Semaphore + `tokio::time::timeout` 提供，可被上层 `Operation`/`Saga` 复用。
- `per_device_concurrency` 上限同时保证缓存刷新和并发 workflow 不会向同一设备发送过多并发 SOAP 请求。
