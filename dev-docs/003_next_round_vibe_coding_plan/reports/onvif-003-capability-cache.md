# ONVIF-003：Capability TTL/Stale Fallback

## 任务目标

为 `GetCapabilities`/`GetServices` 添加按 endpoint 的 TTL 缓存，并在刷新失败时返回上次可用能力，而不是直接失败。

## 实现变更

1. `crates/protocols/cheetah-onvif-driver-tokio/src/capability_cache.rs`
   - 新增 `CapabilityCache`：按 endpoint 缓存 `capabilities` 和 `services`。
   - `capabilities` 与 `services` 各自拥有独立的 `fetched_at` 时间戳，避免一方刷新影响另一方的新鲜度。
   - 缓存设置固定 `capacity`（默认 1024），插入新 endpoint 时先清理已过期条目，容量仍满则淘汰最久未见的 endpoint。
   - `get_*` 在 TTL 内命中缓存直接返回克隆。
   - `stale_*` 在刷新失败时返回上一次缓存，保证不丢失已知能力。

2. `crates/protocols/cheetah-onvif-driver-tokio/src/lib.rs`
   - `OnvifHttpDriver` 新增 `capability_cache` 与 `capability_ttl` 字段。
   - `get_services`/`get_capabilities` 先查缓存，命中则直接返回；未命中则发起 SOAP 请求，成功后写入缓存；失败时回退到 stale 缓存，无缓存才返回错误。

3. `crates/protocols/cheetah-onvif-driver-tokio/src/config.rs`
   - `DriverConfig` 新增 `capability_ttl`（`Duration`）与 `capability_cache_capacity`（`usize`）。

4. `crates/foundation/cheetah-signal-types/src/config.rs`
   - `OnvifConfig` 新增 `capability_ttl_ms`（`DurationMs`）和 `capability_cache_capacity`（`usize`），默认 300_000 毫秒 / 1024；零值 TTL 禁用缓存。

5. `config.example.toml`
   - 增加 `onvif.capability_ttl_ms` 和 `onvif.capability_cache_capacity` 示例。

6. `dev-docs/003_next_round_vibe_coding_plan/07_gb28181_and_onvif_vertical_completion.md`
   - 将 `capability TTL/ETag或revision过期后刷新，失败不删除上次可用能力` 标记完成。

## 验证结果

- `cargo fmt --all`：通过
- `cargo clippy -p cheetah-onvif-driver-tokio --tests -- -D warnings`：通过
- `cargo test -p cheetah-onvif-driver-tokio`：通过

后续需补充完整 workspace 验证。
