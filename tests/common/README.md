# cheetah-signaling-testkit

Test doubles and fixtures for the signaling workspace.

## 职责

- 提供 `FakeClock`、确定性 ID 生成器、fake secret/media/bus/storage 实现。
- 为单元测试和集成测试提供可复用测试组件。
- 不参与生产运行时。

## 允许依赖

- 任何 workspace crate（仅 dev）。
- `tokio`、`serde`、`thiserror`、`uuid`、`chrono` 等测试辅助库。

## 禁止依赖

- 生产专用实现不在本 crate 中实现。
- 不得依赖外部真实数据库、NATS 或媒体服务。

## feature

- `default`：测试工具集合。

## 公共入口

- `src/lib.rs`：公开 `fake_clock`、`fake_id`、`fake_ports` 等模块（WP-05 起填充）。
