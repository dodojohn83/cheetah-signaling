# GB4-SIP-002 / GB4-SIP-003 完成报告

- 任务 ID：`GB4-SIP-002`、`GB4-SIP-003`
- 结论：完成
- 日期：2026-07-21
- 基线分支：`origin/devin/gb4-arc-sip`（PR #173）
- 工作分支：`devin/gb4-sip-002`（目标 `main`，stacked on #173）

## 1. 变更摘要

### 1.1 Transaction manager 与 FSM 接入（core）

- 新增 `cheetah_gb28181_core::sip::transaction::manager`，导出 `TransactionManager<T>`、`ManagerConfig`、`ManagerOutput<T>`、`RequestOutcome<T>`、`DEFAULT_MAX_TRANSACTIONS`、`DEFAULT_TRANSACTION_TTL`。
- `TransactionManager<T>` 在既有 client/server transaction FSM 之上提供有界注册表：按 `TransactionKey` 索引，泛型化路由目标 `T`（driver 中为 `SocketAddr`），保持 Sans-I/O——`now` 由调用方以单调 `Duration` 传入，manager 自身不读时钟。
- 入口方法：`handle_request`（服务端）、`provide_response`（回写 TU 响应并缓存）、`handle_response`（客户端）、`start_client_transaction`（出站请求）、`tick`（推进 timer / 回收）。
- 请求分发结果用 `RequestOutcome { key, deliver, outputs }` 表达：`deliver` 由底层 FSM 是否产出 `Deliver` 推导，`outputs` 只携带即时的 send/failure，避免把待投递请求重复塞进 outputs。

### 1.2 Dialog 接入与路由（core）

- 复用 #173 已实现的 `Dialog` 状态机（保存 `DialogId`、route set、remote target、local/remote CSeq、状态），新增有界 `DialogManager`（`cheetah_gb28181_core::sip::dialog_manager`），导出 `DialogManager`、`DialogManagerConfig`、`DialogRouting`、`DEFAULT_MAX_DIALOGS`、`DEFAULT_DIALOG_TTL`。
- `DialogManager` 按 `Call-ID` + local/remote tag 建立/匹配 dialog：入站请求以 `To`-tag 为 local、`From`-tag 为 remote；入站响应反之。`establish_uas` / `establish_uac` 在 2xx INVITE 建立 dialog。
- `handle_request` / `handle_response` 通过 `Dialog::process` 路由并返回 `DialogRouting`：`Deliver`（新的 in-dialog 消息交 TU）、`Terminated`（BYE 终止并移除 dialog）、`Absorbed`（CSeq 重复/乱序被吸收，不重复投递）、`NoDialog`（out-of-dialog，交由调用方处理）、`Failure`。
- Re-INVITE 更新 remote target；BYE 终止 dialog；INFO/SUBSCRIBE/NOTIFY 走 dialog 序列检查而非 REGISTER 路由。

### 1.3 Method 路由（core + driver, GB4-SIP-003）

- 新增 `cheetah_gb28181_core::sip::routing`，`route_request(&SipMessage) -> Option<RequestRoute>` 将
  `REGISTER/MESSAGE/INVITE/ACK/CANCEL/BYE/INFO/SUBSCRIBE/NOTIFY/OPTIONS` 映射到 `RequestRoute` 分类，未知方法映射为 `Unsupported`（不会被误判为受支持方法）。
- `RequestRoute` 提供 `delivers_to_tu()`、`is_dialog()`、`creates_server_transaction()` 供 driver 决策。
- driver `handle_incoming_request` 依据 `route_request` 分派：ACK 无匹配事务时不新建服务端事务而交 dialog/无状态处理；in-dialog 方法先经 `DialogManager` 决定投递/吸收/终止；其余交 access 状态机。

### 1.4 Timer 与可靠传输语义

- Timer A/E（重传）在 UDP 上指数退避、在可靠传输上归零（不重传）；Timer B/F（`64*T1` deadline）两类传输都保留，超时以 `ManagerOutput::Failure` 上抛。Timer D（UDP 32s / 可靠 0）、Timer K（UDP T4 / 可靠 0）用于 completed 状态吸收。
- driver 按传输类别拆分为 `udp_txns`（`Unreliable`）与 `tcp_txns`（`Reliable`）两张事务表；TCP 抑制应用层重传但保留事务 deadline 与完成语义。

### 1.5 Duplicate / late / out-of-order 处理

- 服务端：重复请求命中既有事务时**重放缓存响应**且 `deliver=false`，不再触发鉴权 / 持久化 / 业务事件。
- 客户端：无匹配事务的响应（late / 重传 / forked）被丢弃并附有界诊断。
- dialog 层：CSeq 不大于上次值的 in-dialog 请求被 `Absorbed`。

### 1.6 有界性与 TTL

- 每类事务表和 dialog 表都有 `max_*` 容量与 `ttl`；超容量时优先淘汰已终止条目，否则淘汰最久未活跃条目；`tick` 回收终止/空闲条目。无无界 map / channel / task。

### 1.7 响应路由修正

- driver 不再把所有响应盲发回当前 source：TU 响应经 `provide_response` 由服务端事务缓存后按事务 target 发送；UDP 重传由 ticker 通过绑定的 UDP socket 发出，TCP 响应写回当前连接。

### 1.8 Compact / unknown header

- 补充 golden 测试：compact 头（`v/f/t/i/m/c/l/s`）大小写不敏感映射到规范名；未知头以 `HeaderName::Other` 保留原始大小写但等值比较大小写不敏感；未知头计入 `max_headers` 上限，超限返回 `TooManyHeaders`。

## 2. 测试

- core 单元/golden 测试（`cargo test -p cheetah-gb28181-core`，61 passed），新增：
  - transaction manager：新建服务端事务投递、重复请求重放缓存响应且不重投、客户端重传+Timer F 超时、无匹配响应丢弃、INVITE 非 2xx 生成 ACK、服务端容量有界、TTL 回收。
  - dialog manager：UAS 建立并路由 in-dialog、out-of-dialog 不匹配、重复/乱序吸收、BYE 终止移除、容量有界、TTL 回收。
  - routing：十种方法 golden 映射、ACK 特例、dialog 方法分类、响应不按方法路由。
  - parser/headers：compact 头映射、未知头保留、未知头计入上限。
- driver 集成测试（`tests/transaction_routing.rs`）：重复 REGISTER 只触发一次注册事件且两次都收到缓存 200；OPTIONS 被路由并获得最终响应。
- 全部 core/driver 相关测试确定性、无真实 sleep（核心用注入的单调 `Duration`）。

## 3. 验证

```text
cargo fmt --all -- --check                               # pass
cargo clippy --workspace --all-targets -- -D warnings    # pass
cargo test --workspace                                   # GB28181 全部 pass；见“已知无关失败”
python3 scripts/audit_architecture.py                    # exit 0；GB28181 路径无 violation/warning
```

### 已知无关失败

- `cargo nextest`：环境未安装 `cargo-nextest`，改用 `cargo test --workspace`。
- `cheetah-message-nats` 的 README doctest（`src/lib.rs` line 32）编译失败，属基线 `origin/devin/gb4-arc-sip` 既有问题（本分支未触及该 crate，已在基线 worktree 复现），与本任务无关。
- `audit_architecture.py` 剩余的 `cheetah-media-scheduler`、`cheetah-onvif-driver-tokio` 层级违规与两处 `panic!` 均为既有项，不在本任务范围。

## 4. 架构影响与边界

- 事务/dialog 状态只存在于 `core`，`driver` 仅持有有界注册表实例并执行 socket/timer I/O；`module` 不重复实现 wire state machine。
- `core` 未新增 Tokio/socket/DB 依赖；`driver` 未引入 SQLx 或业务持久化。
- 响应目标来自事务/dialog 路由状态，不再以当前 source 作为通用默认。
- dialog 建立依赖 TU 产出 2xx INVITE；当前 `module` 对 INVITE 仍返回 501（属 `GB4-SIP-004+` 业务范围），故 driver 侧 dialog 建立目前不会被真实业务触发，但 core 层 `DialogManager` 已具备完整能力与测试，wiring 已就绪。
- 未改变信令进程对媒体 payload 的处理；driver 仍只处理 SIP，不接收/解析 RTP/RTCP/PS/TS。
