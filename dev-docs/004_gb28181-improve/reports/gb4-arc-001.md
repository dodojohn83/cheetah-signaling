# GB4-ARC-001 完成报告

- 任务 ID：`GB4-ARC-001`
- 结论：完成
- 日期：2026-07-21
- 仓库 commit：`dodojohn83/cheetah-signaling`（当前 PR HEAD）

## 变更摘要

1. 在 `cheetah-gb28181-core` 新增 `access` 模块，定义 `AccessInput`、泛型 `AccessOutput<E>` 与 `GbAccessMachine` trait。`GbAccessMachine` 是 GB28181 access 状态机的契约层，所有实现必须是无 I/O 的。
2. `cheetah-gb28181-module` 的 `Gb28181Access` 实现 `GbAccessMachine`，并重新导出核心类型。
3. `cheetah-gb28181-driver-tokio` 改为仅依赖 `cheetah-gb28181-core`：UDP 驱动通过 `GbAccessMachine` trait 泛型执行状态机，事件通过 `EventSink<M::Event>` 泛化输出；测试时以 `cheetah-gb28181-module` 作为 dev-dependency。
4. 删除 `cheetah-gb28181-module` 中的 `driver.rs`、Tokio、`cheetah-plugin-sdk` 与 `async-trait` 依赖。
5. 更新 `apps/cheetah-signaling`：
   - `gb_event_sink.rs` 实现泛型 `EventSink<Gb28181Event>`；
   - `assembly.rs` 不再注册内置 `cheetah/gb28181` plugin driver，改为直接构造 `Gb28181Access` 并交给 `Gb28181UdpDriver`。
6. 更新三个 crate 的 README，明确依赖边界与公共入口。

## 验证

```text
cargo fmt --all -- --check       # pass (after cargo fmt --all)
cargo clippy --workspace --all-targets -- -D warnings  # pass
cargo test -p cheetah-gb28181-core -p cheetah-gb28181-module -p cheetah-gb28181-driver-tokio -p cheetah-signaling  # pass
python3 scripts/audit_architecture.py  # GB28181 driver -> module 与 module -> Tokio/plugin SDK 违规已消失
```

`scripts/audit_architecture.py` 剩余两项违规为 `cheetah-media-scheduler` 与 `cheetah-onvif-driver-tokio`，不在本任务范围内。

## 未运行项

- `cargo nextest`：当前环境未安装 `cargo-nextest`；已使用 `cargo test` 覆盖变更 crate 与主应用。
- `buf format/lint`：`buf` 二进制未安装；本 PR 未修改 `.proto`。

## 边界说明

- 本 PR 未改变信令进程对媒体 payload 的处理；`Gb28181UdpDriver` 仍只处理 SIP datagram，不接收或解析 RTP/RTCP/PS/TS。
- `cheetah-gb28181-core` 仍未引入 `quick-xml` 等 XML 解析库；XML 解析保留在 `module` 中，`core` 只负责 SIP 状态机契约。
