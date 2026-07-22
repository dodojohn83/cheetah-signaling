# GB4-TST-002：状态机迁移表测试

- 任务：`GB4-TST-002`
- 状态：`Done`
- 日期：2026-07-22

## 1. 新增 crate

新增 `crates/testing/cheetah-state-machine-tests`（`publish = false`，仅测试代码，附
`README.md` 说明职责与依赖边界）。所有测试使用
`cheetah_domain::in_memory::InMemoryClock` / `InMemoryIdGenerator` 等确定性夹具，
不依赖真实设备、网络、定时器或媒体负载。

## 2. 覆盖范围

| 领域    | 文件               | 覆盖 |
|---------|--------------------|------|
| access  | `tests/access.rs`  | 注册/刷新/keepalive/注销/owner 分配与 epoch fencing；CSeq 单调；绝对过期时间；ingress 端点更新授权表（Register/Keepalive/Message × 认证 × in-dialog） |
| command | `tests/command.rs` | `Operation` 全状态迁移矩阵（Start/Complete/Cancel/Timeout）；dispatch attempt sent/acked/nack 诊断；timeout 置当前 step 失败；重复命令共享幂等域；重复 start 拒绝；旧 owner epoch fencing |
| catalog | `tests/catalog.rs` | 基于无状态 `parse_catalog`/`build_catalog_response`：fragment、partial（`num < sum_num`）、乱序无关、重复设备去重、缺失必需元素、声明数量不匹配、非 Catalog cmdType 拒绝、crash 后重放确定性 |
| media   | `tests/media.rs`   | `MediaSession` saga（Requested→Allocating→Inviting→Active→Stopping→Stopped）与全迁移矩阵；early media；CANCEL 后 late 200 拒绝；BYE 停止；`MediaBinding` 迁移矩阵与终态不可逆；旧 media-node instance epoch fencing |
| cascade | `tests/cascade.rs` | 通过公共 `Gb28181Cascade::process(CascadeInput)` 驱动：register 发出 REGISTER、注册中重复 Register 被忽略、成功注册发 `CascadePlatformConnected`、Deregister 发 Expires:0、重试耗尽后 `CascadePlatformDisconnected` 退避、内网上游 ACL 拒绝/开关 |

### 关于 catalog 与 cascade 的取舍

- 生产端未提供独立的“catalog collector”聚合，catalog 迁移语义落在实际存在的两个
  层：无状态逐包 parser 与其消费契约。存储层 revision-conflict 由 GB4-TST-003 的
  `cheetah-storage-tests` 合同套件覆盖。
- cascade 的 subscription/bridge/loop 迁移需要 crate 内部夹具，已由
  `cheetah-gb28181-module/src/cascade/tests` 覆盖；本 crate 固定 register/backoff/
  deregister/ACL 的公共 API 契约，避免重复内部实现。
- owner epoch / media-node instance epoch 的 fencing 发生在应用/handler 层而非聚合
  上，聚合仅暴露访问器；相关测试以真实访问器值 pin 比较序语义。

## 3. 验证命令

```bash
cargo test -p cheetah-state-machine-tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

结果：access 8、command 7、catalog 7、media 7、cascade 6，全部通过；fmt/clippy 无告警。
