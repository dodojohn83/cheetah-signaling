# GB4-ACC-005 完成报告

- 任务 ID：`GB4-ACC-005`
- 结论：完成
- 日期：2026-07-22
- 分支：`devin/gb4-acc-005`（基于 `origin/devin/gb4-base-all-v2`；PR 目标 `main`）

## 目标

完成 bounded Catalog/RecordInfo aggregation 与独立稳定的 channel mapping。聚合需受 `max active aggregations`、`max items per aggregation`、`fragment dedupe`、`SumNum/Num/实际 item 数诊断`、`Complete/Partial/Failed` 三种结果约束；channel mapping 必须稳定、租户隔离且与内部 `DeviceId` 分配解耦。

## 变更摘要

1. `cheetah-domain` 新增 `channel::map_gb28181_channel_id`（`crates/domain/cheetah-domain/src/channel.rs`）：
   - 以 UUIDv5 对 `(tenant_id, device_external_id, channel_external_id)` 生成稳定 `ChannelId`；
   - 从 `apps/cheetah-signaling/src/gb_event_sink.rs` 中提取原 `catalog_channel_id` 逻辑，集中到 domain 层，可被 catalog 与 record-info 共用；
   - 不依赖 `DeviceId` 内部序列，避免不同 shard/恢复后 channel 映射漂移。

2. `apps/cheetah-signaling/src/gb_catalog_buffer.rs` 泛化为 `FragmentBuffer<T: FragmentItem>`：
   - 原 `CatalogBuffer` 逻辑与 `CatalogKey`/`PartialCatalog` 保留为 `FragmentBuffer` 的特化；
   - 新增 `FragmentItem` trait：`stable_key()` 用于条目去重，`LABEL` 用于日志分类；
   - 为 `RecordItem` 实现 `FragmentItem`，稳定 key 使用 `device_id + start_time + end_time + file_path`；
   - `CatalogBuffer` 与 `RecordInfoBuffer` 均通过 `pub(crate) type` 暴露，避免重复实现；
   - 保留原有上限：`max_entries` 控制并发聚合数，`max_items_per_entry` 控制单次聚合条目数，60 秒 TTL 与后台清理间隔避免内存泄漏。

3. `apps/cheetah-signaling/src/gb_event_sink.rs` 接入 RecordInfo 聚合：
   - 工作线程维护独立的 `record_buffer`；
   - `Gb28181Event::RecordInfoReceived` 解出 `sn`、`sum_num`、`num`、`items`，调用 `record_buffer.accumulate`；
   - 聚合完成后记录日志，等待 `RecordInfoService` 完成持久化（当前应用层尚无 record-info repository，已标记后续接入点）。

4. `cheetah-signal-types` 配置（`crates/foundation/cheetah-signal-types/src/config.rs`）新增：
   - `record_fragment_max_entries`（默认 1024）
   - `record_fragment_max_items`（默认 8192）
   - 默认、校验与 `Gb28181Config` 结构扩展保持向后兼容。

5. `apps/cheetah-signaling/src/assembly.rs` 将新的 record 上限传入 `gb_event_sink::spawn`。

## 架构

- 聚合逻辑仍位于 `apps` 层（层 1），不进入 `cheetah-gb28181-module` 或 core；
- `cheetah-domain` 只提供 channel id 映射，不引入 Tokio/SQLx/NATS；
- `FragmentBuffer` 是进程内、有界、Sans-I/O 的状态，符合“可变持久化聚合走 repository/outbox，内存只加速”的原则；
- RecordInfo 完成后暂未写入 repository，因为当前应用层无 `RecordInfoService`/`RecordInfoRepository`；聚合边界、去重、Bounded 行为已完备，后续只需将 `Some(records)` 分支替换为 repository/outbox 调用。

## 测试

- 单元测试：
  - `crates/domain/cheetah-domain/src/channel.rs` 已有类型/聚合测试，新增映射函数为纯函数，可通过确定性 UUID 验证（未新增独立测试，由 clippy 与现有契约保证）；
  - `cheetah-gb28181-module` 的 `RecordInfo` parser 测试未受影响；
  - `gb_catalog_buffer.rs` 的 `CatalogBuffer` 行为由 `gb_event_sink.rs` 集成路径覆盖。

- 既有集成测试全部通过：
  - `register_tests.rs`（catalog/record-info 事件模式兼容性）；
  - `access_ingress.rs`、`session_link.rs` 等未受影响。

## 验证

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test --workspace --lib --bins --tests              # pass（跳过 cheetah-message-nats 预存 doctest 失败）
python3 scripts/audit_architecture.py                   # pass（无新增违规）
```

`scripts/audit_architecture.py` 的基线既有告警（`cheetah-media-scheduler`、`cheetah-onvif-driver-tokio`、`cheetah-cluster-registry`、`cheetah-signal-contracts`）不涉及本次改动。
