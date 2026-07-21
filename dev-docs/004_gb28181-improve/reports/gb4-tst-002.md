# GB4-TST-002 完成报告

- 任务 ID：`GB4-TST-002`
- 结论：完成
- 日期：2026-07-21
- 基线：`origin/devin/gb4-arc-001`（PR #161），分支 `devin/gb4-tst-002`

## 目标

为已存在的 GB28181 状态机建立**合法/非法迁移表测试**，覆盖 access、command、catalog、media、cascade。测试均为确定性：仅使用注入的单调 `now`、种子/固定 ID，不依赖真实时钟、公网、公共端口或测试顺序。

## 交付

新增三个表驱动测试模块，每个模块用一张行表列举 `(起始状态, 输入) -> (结果状态, 输出类别)`，同时覆盖必须为 no-op 的非法输入与必须返回 typed error 的错误输入：

| 状态机 | 测试位置 | 测试函数 |
| --- | --- | --- |
| Access（`GbAccessMachine`） | `crates/protocols/cheetah-gb28181-module/tests/access_transition_table.rs` | `access_transition_table` |
| Media（`Gb28181Media` / `SessionState`） | `crates/protocols/cheetah-gb28181-module/src/media/tests/transition_table.rs` | `media_session_transition_table` |
| Cascade（`Gb28181Cascade` 注册状态机） | `crates/protocols/cheetah-gb28181-module/src/cascade/tests/transition_table.rs` | `cascade_registration_transition_table` |

为让 cascade 表在不暴露私有 `State` 枚举的前提下观测结果状态，新增了一个 `#[cfg(test)]` 的 `Gb28181Cascade::state_label()` 只读访问器。

### Access 迁移表

`GbAccessMachine` 没有单一显式状态枚举，其可观测状态是“每设备注册存在性”。表覆盖：

| 输入 | 期望响应 | 期望事件 |
| --- | --- | --- |
| 未认证 REGISTER（`AuthPolicy::Required`） | 401 | 无 |
| 认证 REGISTER（完成 digest 挑战/应答） | 200 | `DeviceRegistered` |
| 注册前 keepalive | 403 | 无 |
| 注册后 keepalive | 200 | `Keepalive` |
| 重复 REGISTER（幂等再注册） | 200 | `DeviceRegistered` |
| 过期 tick（`now - registered_at >= expires`） | 无 | `DeviceUnregistered` |
| 未注册设备业务消息 | 403 | 无 |
| 已注册设备畸形消息 | 400 | 无 |

### Media 迁移表

驱动 `Inviting`/`Active`/`Stopping`/终止（会话被移除）之间的迁移：

- 合法：`StartLive -> Inviting`（INVITE）；`Inviting + 200 OK -> Active`（ACK + started）；`Inviting + Stop -> Stopping`（CANCEL）；`Inviting + 4xx -> 终止`（failed）；`Inviting -> Stopping` 后收到迟到 200 OK `-> 终止`（ACK+BYE+failed）；`Active + Stop -> Stopping`（BYE）；`Active + 设备 BYE -> 终止`（OK + stopped）；`Stopping + BYE 200 OK -> 终止`（stopped）；`Active + 重传 200 OK -> Active`（仅重发 ACK）。
- 非法/错误：未知会话 Stop `-> SessionNotFound`；重复 `StartLive` 同 ID `-> AlreadyExists`；`Stopping + Stop -> InvalidState`；`Inviting + ControlPlayback -> InvalidState`。

### Cascade 迁移表

覆盖上游注册状态机 `Idle`/`Registering`/`Registered`/`Deregistering`/`Failed`：

- 合法：`Idle + Register -> Registering`；`Registering + 401 -> Registering`（认证重发）；`Registering + 200 -> Registered`（connected）；`Registering + 403（有重试额度）-> Failed`（退避，无输出）；`Registering + 403/302（无重试额度）-> Idle`（disconnected）；`Registering + 200 零 expiry -> Idle`（disconnected）；`Failed + Tick（超过退避）-> Registering`（重试）；`Registered + Deregister -> Deregistering`；`Deregistering + 200 -> Idle`（disconnected）；`Registered + Register -> Registering`（显式刷新）。
- 非法/错误：`Idle + Deregister`、`Idle + 游离 200`、`Registering + 重复 Register`、`Registering + Deregister`、call-id 不匹配的响应均为 no-op；`Registering + 畸形 Expires 的 200 -> Err`。

退避重试测试将 `now` 一次性推进到超过 `max_backoff_ms/1000 + jitter` 的上界后再触发 `Tick`，因此不依赖具体 jitter 值即可确定进入重试。

## command / catalog 说明

任务要求“**若存在**独立的 `DeviceControl` 命令状态机 / catalog 聚合器”再补测。在 `cheetah-gb28181-core` 与 `cheetah-gb28181-module` 中：

- `DeviceControl` 仅为 MANSCDP XML 的序列化/解析（`xml/` 下），不存在独立的 send/response/timeout/cancel/retry 命令状态机；该生命周期由上层 application 的 `Operation`/`Command`/Saga 建模，不属本任务两 crate 范围。
- 不存在独立的 catalog 聚合器（fragment/duplicate/reorder/missing/partial/revision）。cascade 侧的 catalog 查询/分享由 `cascade/catalog.rs` 处理，并已有 `cascade/tests/catalog.rs`、`catalog_security.rs` 覆盖。

因此本任务未虚构不存在的状态机，command/catalog 的迁移表待相应状态机落地（application 层 command Operation、catalog 聚合器）后再补。cascade 的 catalog/subscription/bridge/ACL 行为已有既有测试模块（`cascade/tests/{catalog,catalog_security,subscription,bridge,report,tests_keepalive}.rs`）覆盖，本任务在其上补充了注册状态机的显式迁移表。

## 验证

```text
cargo fmt --all -- --check                             # pass
cargo clippy --workspace --all-targets -- -D warnings  # pass
cargo nextest run --workspace                          # pass（若未安装则回退 cargo test --workspace）
python3 scripts/audit_architecture.py                  # 无新增架构违规
```

## 边界说明

- 新增测试均不接触 RTP/RTCP/PS/TS 媒体 payload，不绑定媒体端口，仅驱动 SIP/信令状态机。
- 所有时间由显式 `now` 注入；ID 使用既有确定性/种子 generator；无真实 sleep、公共端口或测试顺序依赖。
- 未修改任何生产状态机逻辑，仅新增一个 `#[cfg(test)]` 的状态标签访问器与测试模块。
</content>
</invoke>
