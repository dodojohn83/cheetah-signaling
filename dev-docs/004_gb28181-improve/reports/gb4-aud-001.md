# GB4-AUD：003 完成度与 GB28181 现状审计

- 任务：`GB4-AUD-001`、`GB4-AUD-002`、`GB4-AUD-003`
- 结论：`Partial`；报告与注册表工具已落地，架构/占位依赖需在 `GB4-ARC/SIP` 阶段修复
- 审计日期：2026-07-21
- 审计 commit：`96c76efc9b6c5bdf4956ab6a4c100429d0e8e8da`
- 审计人：Devin 自动执行

## 1. 审计范围

依据 `dev-docs/004_gb28181-improve/01_003_completion_audit.md` 对仓库当前状态进行复验，覆盖：

1. 代码格式化、GB28181 定向 clippy 与单元测试；
2. `scripts/audit_architecture.py` 依赖层/占位/panic 审计；
3. `dev-docs/004_gb28181-improve/91_003_requirement_registry.md` 与 Phase 文档的 `GB4-*` 任务 ID 一致性；
4. 相对链接有效性。

## 2. 环境

- Rust：`rustc 1.96.1`（由 `rust-toolchain.toml` 指定）
- 操作系统：Ubuntu（workspace）
- 缺少工具：`buf`（v1.50.0 预期）和 `cargo-nextest` 未安装，因此 `buf format/lint` 与 `cargo nextest run` 未执行；已在 `GB4-SYS` 阶段前补齐。

## 3. 执行命令与结果

### 3.1 `cargo fmt --all -- --check`

```text
Exit code: 0
Result: PASS
```

### 3.2 定向 clippy

```bash
cargo clippy \
  -p cheetah-gb28181-core \
  -p cheetah-gb28181-module \
  -p cheetah-gb28181-driver-tokio \
  -p cheetah-gb28181-simulator \
  --all-targets -- -D warnings
```

```text
Exit code: 0
Result: PASS
```

### 3.3 定向单元测试

```bash
cargo test \
  -p cheetah-gb28181-core \
  -p cheetah-gb28181-module \
  -p cheetah-gb28181-driver-tokio \
  -p cheetah-gb28181-simulator
```

```text
Exit code: 0
Result: PASS
Summary:
  cheetah-gb28181-core: 130+ parser/transaction/dialog/digest tests passed
  cheetah-gb28181-module: 37 access tests passed
  driver-tokio / simulator: 0/0 tests passed
```

### 3.4 架构审计

```bash
python3 scripts/audit_architecture.py
```

结果：

```text
Dependency layer violations: 3
   LAYER VIOLATION: cheetah-gb28181-driver-tokio (layer 5) depends on cheetah-gb28181-module (layer 4)
   LAYER VIOLATION: cheetah-media-scheduler (layer 3) depends on cheetah-media-client (layer 2)
   LAYER VIOLATION: cheetah-onvif-driver-tokio (layer 5) depends on cheetah-onvif-module (layer 4)
Forbidden dependency warnings: 3
   FORBIDDEN DEP: cheetah-cluster-registry (layer 6) -> tokio
   FORBIDDEN DEP: cheetah-signal-contracts (layer 6) -> tonic
   FORBIDDEN DEP: cheetah-signal-contracts (layer 6) -> tonic-prost
Production todo!/unimplemented! hits: 0
Production panic! warnings: 2
  crates/protocols/cheetah-onvif-driver-tokio/src/protocol_driver.rs:130 panic!
  crates/storage/cheetah-storage-api/src/phase_migration.rs:105 panic!
Test-fake todo!/unimplemented! hits: 0
Direct SQL outside storage crates: 0
```

与 GB28181 直接相关的唯一层违例为 `cheetah-gb28181-driver-tokio -> cheetah-gb28181-module`，归属 `GB4-ARC-001` 修复。

### 3.5 注册表一致性（GB4-AUD-002）

```bash
python3 scripts/verify_gb4_registry.py
```

```text
Exit code: 0
Result: OK: 68 unique GB4 task IDs, registry cross-check passed, links valid.
```

新增 `scripts/verify_gb4_registry.py` 可在基线变化时重新运行，覆盖 `GB4-AUD-003` 的复验要求。

## 4. 003 完成度判定

| 003 Phase | 当前状态 | 004 归属 |
| --- | --- | --- |
| 00 completion audit | `Partial` | GB4-AUD（报告已落地，但后续基线变化需复验） |
| 01 baseline/CI | `Partial` | GB4-AUD、GB4-ARC、GB4-SYS |
| 02 assembly | `Partial` | GB4-ARC、GB4-ACC |
| 03 media contract/runtime | `Not Completed` | 003 MED-C/MED-R + GB4-MED |
| 04 media workflow | `Not Completed` | 003 WF + GB4-MED |
| 05 GB/ONVIF | `Partial` | GB4-SIP..GB4-CAS |
| 06 API/cluster/security | `Partial` | 003 PROD、GB4-OPS、GB4-SYS |
| 07 system/release | `Not Implemented` | GB4-SYS |
| 全局 DoD | `Not Completed` | 全部 |

## 5. 生产链路缺口（与 01 审计一致）

1. **双入口与错误命令结果**：`Gb28181UdpDriver` 真实 listener 与内置 `cheetah/gb28181` plugin `process_sip` 路径并存；OwnerCommandHandler 将 plugin 的 `Unsupported` 映射为 `Completed/unknown`，不符合 `UnknownOutcome` 语义。
2. **Driver 未执行 core transaction/dialog**：UDP driver 仅解析 datagram 后调用 `Gb28181Access`，没有 TCP、事务/dialog 路由、固定分片运行时、命令发送路径。
3. **Event sink 未形成权威状态**：`default_tenant_id`、队列满静默丢弃、每次输入生成新 MessageId/CorrelationId，缺乏去重与事务。
4. **Media 与 cascade 仅存在局部状态机**：生产代码未实例化 `Gb28181Media`/`Gb28181Cascade`。
5. **文档与代码边界不一致**：driver README 声明不依赖 module 但实际依赖；module 依赖 Tokio/plugin transport；app assembly 包含业务映射。

## 6. 未运行/阻塞项

- `buf format --diff --exit-code`、`buf lint`、`cargo nextest run --workspace`、`cargo deny check`：环境缺少 `buf` 和 `cargo-nextest`，计划由 `GB4-SYS-001` 补齐工具链并在后续 PR 复验。
- 真实媒体节点、真实设备/平台联调：依赖 `UP-MEDIA-P0` 与外部硬件环境，当前 `Blocked`，不纳入本阶段。

## 7. 证据文件

- `scripts/audit_architecture.py`：现有架构审计脚本
- `scripts/verify_gb4_registry.py`：新增注册表/链接一致性校验脚本
- `target/reports/bas-004-architecture-audit.md`：架构审计原始输出
- 本报告：`dev-docs/004_gb28181-improve/reports/gb4-aud-001.md`
