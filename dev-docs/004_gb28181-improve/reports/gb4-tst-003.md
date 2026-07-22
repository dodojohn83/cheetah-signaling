# GB4-TST-003：Driver-Core-Module 与架构契约测试

- 任务：`GB4-TST-003`
- 状态：`Partial`（架构契约部分完成，repository/message/media 契约待后续补充）
- 日期：2026-07-21

## 1. 已完成的架构契约测试

在 `cheetah-gb28181-driver-tokio` 和 `cheetah-gb28181-module` 中新增 `tests/architecture.rs`：

- `driver-tokio` 生产依赖必须包含 `cheetah-gb28181-core`；
- `driver-tokio` 生产依赖禁止包含 `cheetah-gb28181-module`；
- `module` 生产依赖必须包含 `cheetah-gb28181-core`；
- `module` 生产依赖禁止包含 `tokio`、`cheetah-plugin-sdk`、`async-trait`。

测试通过读取 `Cargo.toml` 并限定只扫描 `[dependencies]` 区段，不误判 `[dev-dependencies]`。

## 2. 验证命令

```bash
cargo test -p cheetah-gb28181-driver-tokio --test architecture
cargo test -p cheetah-gb28181-module --test architecture
```

## 3. 待补充的 repository/message/media 契约

- `GB4-ACC-001` 完成后，补充 `ProtocolSession` repository 契约测试（SQLite/PostgreSQL 共享）；
- 消息总线契约测试在 `GB4-EVT` 阶段补充；
- `MediaPort` contract 测试在 `GB4-MED` 阶段补充。
